//! Everything the preview and the printout share: the pages, the printer and
//! paper, and how each page is placed on it.
//!
//! What you see is what prints: every page is drawn into a PDF at the exact
//! paper size and sent with `lp -o print-scaling=none`. On black-only (thermal)
//! printers the page is first rendered at the printer's own dpi and turned into
//! pure black and white, so the preview shows the real dots and barcodes stay sharp.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cups::{self, PrinterInfo};
use crate::raster::{self, Grey, Mode};
use crate::source::{self, Source};

/// A page rendered for a black-only printer, and what was learned doing it.
struct Raster {
    key: Key,
    surface: cairo::ImageSurface,
    codes: Option<(usize, usize)>,
    #[cfg_attr(not(test), allow(dead_code))]
    mode: Mode,
}

/// Everything a raster depends on.
#[derive(Clone, PartialEq)]
struct Key {
    page: usize,
    size: String,
    dpi: u32,
    rotate: String,
    scale: String,
    margin: f64,
    bw: String,
}

pub struct Job {
    pub title: String,
    pub pages: Vec<Source>,
    pub index: usize,
    pub printers: Vec<String>,
    pub printer: String,
    /// auto | 0 | 90 | 180 | 270
    pub rotate: String,
    /// fit | fill
    pub scale: String,
    /// auto | a margin in points
    pub margin_mode: String,
    pub copies: u32,
    pub sizes: Vec<(String, (f64, f64))>,
    pub size: String,
    pub dpi: u32,
    /// auto (per page) | sharp | dither | off
    pub bw: String,
    cache: RefCell<Option<Raster>>,
}

/// Page numbers in a CUPS page-ranges value like "1-3,5,8-".
pub fn pick_pages(ranges: &str, n: usize) -> BTreeSet<usize> {
    let mut keep = BTreeSet::new();
    for part in ranges.split(',') {
        let part: String = part.chars().filter(|c| !c.is_whitespace()).collect();
        let (a, dash, b) = match part.split_once('-') {
            Some((a, b)) => (a, true, b),
            None => (part.as_str(), false, ""),
        };
        let ok = |s: &str| s.chars().all(|c| c.is_ascii_digit());
        if !ok(a) || !ok(b) || (a.is_empty() && b.is_empty()) {
            continue;
        }
        let a: usize = a.parse().unwrap_or(1);
        let b: usize = if dash { b.parse().unwrap_or(n) } else { a };
        keep.extend(a.max(1)..=b.min(n));
    }
    keep
}

impl Job {
    pub fn new(paths: &[PathBuf], title: Option<String>, copies: u32, page_ranges: Option<&str>) -> Result<Job, String> {
        let title = title.unwrap_or_else(|| {
            paths.iter().map(|p| p.file_name().unwrap_or_default().to_string_lossy()).collect::<Vec<_>>().join(", ")
        });
        let mut pages = Vec::new();
        for p in paths {
            pages.extend(source::load(p)?);
        }
        if let Some(r) = page_ranges.filter(|r| !r.is_empty()) {
            // e.g. "2-3,5" from an app's print dialog
            let keep = pick_pages(r, pages.len());
            if !keep.is_empty() {
                pages = pages.into_iter().enumerate().filter(|(i, _)| keep.contains(&(i + 1))).map(|(_, p)| p).collect();
            }
        }
        let (printers, printer) = cups::printers();
        let mut job = Job {
            title,
            pages,
            index: 0,
            printers,
            printer,
            rotate: "auto".into(),
            scale: "fit".into(),
            margin_mode: "auto".into(),
            copies: copies.clamp(1, 99),
            sizes: Vec::new(),
            size: String::new(),
            dpi: 300,
            bw: "off".into(),
            cache: RefCell::new(None),
        };
        job.load_printer();
        Ok(job)
    }

    /// Paper sizes, dpi and colour for the chosen printer.
    pub fn load_printer(&mut self) {
        let info = if self.printer.is_empty() { PrinterInfo::plain() } else { cups::printer_info(&self.printer) };
        self.set_printer_info(info);
    }

    pub fn set_printer_info(&mut self, info: PrinterInfo) {
        self.sizes = info.sizes;
        self.size = info.default;
        self.dpi = info.dpi;
        self.bw = if info.mono { "auto" } else { "off" }.into();
        self.cache.replace(None);
    }

    /// The paper, in points.
    pub fn page(&self) -> (f64, f64) {
        self.sizes.iter().find(|(n, _)| *n == self.size).map(|(_, d)| *d).unwrap_or((595.0, 842.0))
    }

    /// Auto: labels (4 in wide or less) edge to edge, paper a quarter inch. Never eats the whole page.
    pub fn margin(&self) -> f64 {
        let (pw, ph) = self.page();
        let m: f64 = match self.margin_mode.as_str() {
            "auto" => {
                if pw.min(ph) <= 300.0 {
                    0.0
                } else {
                    18.0
                }
            }
            v => v.parse().unwrap_or(0.0),
        };
        m.min(pw.min(ph) / 2.0 - 9.0).max(0.0)
    }

    /// The part of page `i` to print, (x, y, w, h). Normally all of it; but a
    /// label-sized paper fed a much bigger page (a 4x6 label that a browser
    /// printed onto A4) gets just the printed part, not the whole sheet shrunk.
    pub fn area(&self, i: usize) -> (f64, f64, f64, f64) {
        let src = &self.pages[i];
        let (iw, ih) = src.size();
        let (pw, ph) = self.page();
        if pw.min(ph) <= 324.0
            && iw * ih > 1.5 * pw * ph
            && let Some(b) = src.content_box()
            && b.2 * b.3 < 0.6 * iw * ih
        {
            return b;
        }
        (0.0, 0.0, iw, ih)
    }

    /// Rotation in degrees for page `i`; auto turns it to match the paper.
    pub fn angle(&self, i: usize) -> i32 {
        if self.rotate != "auto" {
            return self.rotate.parse().unwrap_or(0);
        }
        let (_, _, iw, ih) = self.area(i);
        let (pw, ph) = self.page();
        if (iw > ih) != (pw > ph) && (iw - ih).abs() > 1.0 { 90 } else { 0 }
    }

    #[cfg_attr(not(test), allow(dead_code))] // the window shows it; the tests ask
    pub fn rotated(&self, i: usize) -> bool {
        matches!(self.angle(i), 90 | 270)
    }

    /// Page `i` at 1 unit = 1 pt, origin top-left, in full colour.
    pub fn draw_content(&self, cr: &cairo::Context, i: usize) {
        let (pw, ph) = self.page();
        let _ = cr.save();
        cr.rectangle(0.0, 0.0, pw, ph);
        cr.set_source_rgb(1.0, 1.0, 1.0);
        let _ = cr.fill_preserve();
        cr.clip();
        let (x0, y0, iw, ih) = self.area(i);
        let a = self.angle(i);
        let (ew, eh) = if matches!(a, 90 | 270) { (ih, iw) } else { (iw, ih) };
        let m = self.margin();
        let (sx, sy) = ((pw - 2.0 * m) / ew, (ph - 2.0 * m) / eh);
        let s = if self.scale == "fill" { sx.max(sy) } else { sx.min(sy) };
        if s > 0.0 {
            cr.translate(pw / 2.0, ph / 2.0);
            cr.rotate((a as f64).to_radians());
            cr.scale(s, s);
            cr.translate(-iw / 2.0, -ih / 2.0);
            cr.rectangle(0.0, 0.0, iw, ih);
            cr.clip();
            cr.translate(-x0, -y0);
            self.pages[i].paint(cr);
        }
        let _ = cr.restore();
    }

    fn key(&self, i: usize) -> Key {
        Key {
            page: i,
            size: self.size.clone(),
            dpi: self.dpi,
            rotate: self.rotate.clone(),
            scale: self.scale.clone(),
            margin: self.margin(),
            bw: self.bw.clone(),
        }
    }

    /// The page in printer dots. The PDF page is made exactly this many dots, so one
    /// image pixel lands on one printer dot even when the paper isn't a whole number of dots.
    pub fn dots(&self) -> (i32, i32) {
        let (pw, ph) = self.page();
        let d = |pt: f64| ((pt * self.dpi as f64 / 72.0).round() as i32).max(1);
        (d(pw), d(ph))
    }

    /// Page `i` at the printer's dpi in pure black and white (only the latest is kept).
    pub fn raster(&self, i: usize) -> cairo::ImageSurface {
        let key = self.key(i);
        if let Some(r) = self.cache.borrow().as_ref().filter(|r| r.key == key) {
            return r.surface.clone();
        }
        let (w, h) = self.dots();
        let mut surface = cairo::ImageSurface::create(cairo::Format::Rgb24, w, h).expect("page surface");
        {
            let cr = cairo::Context::new(&surface).expect("cairo context");
            cr.scale(self.dpi as f64 / 72.0, self.dpi as f64 / 72.0);
            self.draw_content(&cr, i);
        }
        surface.flush();
        let stride = surface.stride() as usize;
        let (w, h) = (w as usize, h as usize);
        let lum = Grey::from_surface_data(&surface.data().expect("page pixels"), stride, w, h);
        let mode = match self.bw.as_str() {
            "dither" => Mode::Dither,
            "sharp" => Mode::Sharp,
            _ => lum.auto_mode(),
        };
        let mut black = lum.to_black(mode);
        let codes = raster::keep_barcodes(&lum, &mut black); // barcodes on a photo-ish page need saving too
        {
            let mut data = surface.data().expect("page pixels");
            for y in 0..h {
                for x in 0..w {
                    let v: u32 = if black[y * w + x] { 0xFF00_0000 } else { 0xFFFF_FFFF };
                    data[y * stride + x * 4..y * stride + x * 4 + 4].copy_from_slice(&v.to_ne_bytes());
                }
            }
        }
        surface.mark_dirty();
        self.cache.replace(Some(Raster { key, surface: surface.clone(), codes, mode }));
        surface
    }

    /// How page `i` becomes black and white.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn mode(&self, i: usize) -> Option<Mode> {
        match self.bw.as_str() {
            "off" => None,
            "sharp" => Some(Mode::Sharp),
            "dither" => Some(Mode::Dither),
            _ => {
                self.raster(i);
                self.cache.borrow().as_ref().map(|r| r.mode)
            }
        }
    }

    /// (scanning, found) for page `i` as it will print, or None if not checked.
    pub fn barcodes(&self, i: usize) -> Option<(usize, usize)> {
        if self.bw == "off" {
            return None;
        }
        self.raster(i);
        self.cache.borrow().as_ref().and_then(|r| r.codes)
    }

    pub fn draw_page(&self, cr: &cairo::Context, i: usize, preview: bool) {
        if self.bw == "off" {
            return self.draw_content(cr, i);
        }
        let _ = cr.save();
        cr.scale(72.0 / self.dpi as f64, 72.0 / self.dpi as f64);
        let _ = cr.set_source_surface(self.raster(i), 0.0, 0.0);
        // On paper every dot stays a dot; on screen smooth it so it isn't jagged.
        cr.source().set_filter(if preview { cairo::Filter::Good } else { cairo::Filter::Nearest });
        let _ = cr.paint();
        let _ = cr.restore();
    }

    pub fn write_pdf(&self, path: &Path) -> Result<(), String> {
        let (mut pw, mut ph) = self.page();
        if self.bw != "off" {
            let (w, h) = self.dots();
            (pw, ph) = (w as f64 * 72.0 / self.dpi as f64, h as f64 * 72.0 / self.dpi as f64);
        }
        let surface = cairo::PdfSurface::new(pw, ph, path).map_err(|e| e.to_string())?;
        let cr = cairo::Context::new(&surface).map_err(|e| e.to_string())?;
        for i in 0..self.pages.len() {
            self.draw_page(&cr, i, false);
            cr.show_page().map_err(|e| e.to_string())?;
        }
        drop(cr);
        surface.finish();
        Ok(())
    }

    /// Send it to the printer. Tells the desktop how it went.
    pub fn print(&self) -> bool {
        let result = (|| -> Result<(), String> {
            let pdf = source::temp_file("pdf").map_err(|e| e.to_string())?;
            let sent = self.write_pdf(&pdf).and_then(|_| {
                let out = Command::new("lp")
                    .args(["-d", &self.printer, "-n", &self.copies.to_string(), "-o", "collate=true"])
                    .args(["-o", &format!("PageSize={}", self.size), "-o", "print-scaling=none", "-t", &self.title])
                    .arg(&pdf)
                    .output()
                    .map_err(|e| e.to_string())?;
                if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_string()) }
            });
            let _ = std::fs::remove_file(&pdf);
            sent
        })();
        match result {
            Ok(()) => {
                cups::remember_printer(&self.printer);
                notify(&format!("Printing {} on {}", self.title, self.printer), false);
                true
            }
            Err(e) => {
                notify(&format!("Print failed: {e}"), true);
                false
            }
        }
    }
}

pub fn notify(msg: &str, critical: bool) {
    let mut cmd = Command::new("notify-send");
    cmd.args(["-a", "Print"]);
    if critical {
        cmd.args(["-u", "critical"]);
        eprintln!("{msg}");
    }
    let _ = cmd.arg(msg).status();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_ranges() {
        assert_eq!(pick_pages("1-2,4,9-", 10), BTreeSet::from([1, 2, 4, 9, 10]));
        assert_eq!(pick_pages("x,,-", 3), BTreeSet::new());
        assert_eq!(pick_pages(" 2 - 3 ", 5), BTreeSet::from([2, 3]));
        assert_eq!(pick_pages("-2", 5), BTreeSet::from([1, 2]));
    }
}
