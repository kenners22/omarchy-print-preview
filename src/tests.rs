//! Whole-program checks, ported from the old tests.py. Each test makes its own
//! sample files in a temp folder and never prints anything. Label checks use a
//! stand-in 203 dpi black-only printer, so they run without one set up.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cups::{self, PrinterInfo};
use crate::job::Job;
use crate::raster::{self, Grey, Mode};
use crate::source;

/// A temp folder for one test, removed afterwards.
struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Dir {
        let d = std::env::temp_dir().join(format!("print-preview-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Dir(d)
    }
    fn path(&self, f: &str) -> PathBuf {
        self.0.join(f)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Repeatable "random" numbers, so the sample barcodes are the same every run.
struct Lcg(u64);

impl Lcg {
    fn pick<T: Copy>(&mut self, v: &[T]) -> T {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        v[(self.0 >> 33) as usize % v.len()]
    }
}

#[allow(clippy::too_many_arguments)]
fn bars(c: &cairo::Context, rng: &mut Lcg, x0: f64, x1: f64, y: f64, h: f64, widths: &[f64], gaps: &[f64]) {
    let mut x = x0;
    while x < x1 {
        let w = rng.pick(widths);
        c.rectangle(x, y, w, h);
        c.fill().unwrap();
        x += w + rng.pick(gaps);
    }
}

/// label.png (landscape, text and bars), photo.png (all mid-tones), two.pdf (4x6, 2 pages),
/// doc.ps, junk.png (not an image) and empty.pdf.
fn samples(d: &Dir) {
    let mut rng = Lcg(3);
    let s = cairo::ImageSurface::create(cairo::Format::Rgb24, 1200, 800).unwrap();
    let c = cairo::Context::new(&s).unwrap();
    c.set_source_rgb(1.0, 1.0, 1.0);
    c.paint().unwrap();
    c.set_source_rgb(0.0, 0.0, 0.0);
    c.set_font_size(64.0);
    c.move_to(60.0, 120.0);
    c.show_text("SAMPLE").unwrap();
    bars(&c, &mut rng, 60.0, 1140.0, 520.0, 220.0, &[4.0, 6.0, 10.0], &[4.0, 6.0, 8.0]);
    drop(c);
    s.write_to_png(&mut std::fs::File::create(d.path("label.png")).unwrap()).unwrap();

    let s = cairo::ImageSurface::create(cairo::Format::Rgb24, 300, 400).unwrap();
    let c = cairo::Context::new(&s).unwrap();
    let g = cairo::LinearGradient::new(0.0, 0.0, 300.0, 400.0);
    g.add_color_stop_rgb(0.0, 1.0, 0.6, 0.2);
    g.add_color_stop_rgb(1.0, 0.1, 0.2, 0.6);
    c.set_source(&g).unwrap();
    c.paint().unwrap();
    drop(c);
    s.write_to_png(&mut std::fs::File::create(d.path("photo.png")).unwrap()).unwrap();

    let s = cairo::PdfSurface::new(288.0, 432.0, d.path("two.pdf")).unwrap();
    let c = cairo::Context::new(&s).unwrap();
    for n in 1..=2 {
        c.set_font_size(28.0);
        c.move_to(20.0, 50.0);
        c.show_text(&format!("PAGE {n}")).unwrap();
        bars(&c, &mut rng, 30.0, 258.0, 300.0, 100.0, &[1.0, 1.5, 2.5], &[1.0, 1.5, 2.0]);
        c.show_page().unwrap();
    }
    drop(c);
    s.finish();

    std::fs::write(d.path("doc.ps"), "%!PS\n/Helvetica findfont 30 scalefont setfont 50 700 moveto (PS) show showpage\n").unwrap();
    std::fs::write(d.path("junk.png"), "not an image").unwrap();
    std::fs::write(d.path("empty.pdf"), "").unwrap();
}

/// A 203 dpi black-only label printer with 4x6 labels (and Letter, for big pages).
fn label_printer() -> PrinterInfo {
    PrinterInfo {
        sizes: vec![("w288h432".into(), (288.0, 432.0)), ("w144h72".into(), (144.0, 72.0)), ("Letter".into(), (612.0, 792.0))],
        default: "w288h432".into(),
        dpi: 203,
        mono: true,
    }
}

fn label_job(files: &[PathBuf]) -> Job {
    let mut job = Job::new(files, None, 1, None).unwrap();
    job.set_printer_info(label_printer());
    job
}

fn has(cmd: &str) -> bool {
    Command::new("sh").args(["-c", &format!("command -v {cmd}")]).output().is_ok_and(|o| o.status.success())
}

/// A copy of a surface's pixels (it may be shared with the preview's cache).
fn surface_bytes(s: &mut cairo::ImageSurface) -> (Vec<u8>, usize) {
    let stride = s.stride() as usize;
    s.flush();
    let mut copy = Vec::new();
    s.with_data(|d| copy = d.to_vec()).unwrap();
    (copy, stride)
}

#[test]
fn loading() {
    let d = Dir::new("loading");
    samples(&d);
    assert_eq!(source::load(&d.path("label.png")).unwrap().len(), 1, "image loads");
    assert_eq!(source::load(&d.path("two.pdf")).unwrap().len(), 2, "PDF loads both pages");
    if has("ps2pdf") {
        assert_eq!(source::load(&d.path("doc.ps")).unwrap().len(), 1, "PostScript converts");
    }
    for bad in ["junk.png", "empty.pdf", "missing.png"] {
        assert!(source::load(&d.path(bad)).is_err(), "{bad} rejected");
    }
}

#[test]
fn job_takes_dialog_pages_and_copies() {
    let d = Dir::new("dialog");
    samples(&d);
    let two = Job::new(&[d.path("two.pdf")], None, 5, Some("2")).unwrap();
    assert_eq!(two.pages.len(), 1, "keeps only the chosen pages");
    assert_eq!(two.copies, 5, "takes the dialog's copies");
}

#[test]
fn printer_choice() {
    let d = Dir::new("printers");
    // Safety: this is the only test that touches XDG_STATE_HOME.
    unsafe { std::env::set_var("XDG_STATE_HOME", &d.0) };
    let (names, default) = cups::printers();
    assert!(!names.iter().any(|n| n == "Preview"), "Preview queue never offered");
    std::fs::create_dir_all(cups::last_file().parent().unwrap()).unwrap();
    std::fs::write(cups::last_file(), "NoSuchPrinter\n").unwrap();
    assert_eq!(cups::printers().1, default, "stale remembered printer ignored");
    if let Some(last) = names.last() {
        cups::remember_printer(last);
        assert_eq!(&cups::printers().1, last, "remembered printer used");
    }
}

#[test]
fn black_and_white() {
    let d = Dir::new("bw");
    samples(&d);
    let job = label_job(&[d.path("label.png")]);
    let mut r = job.raster(0);
    let (pw, ph) = job.page();
    let dots = |pt: f64| (pt * job.dpi as f64 / 72.0).round() as i32;
    assert_eq!((r.width(), r.height()), (dots(pw), dots(ph)), "raster is the page at printer dpi");
    let (data, stride) = surface_bytes(&mut r);
    let pure = (0..r.height() as usize).all(|y| {
        (0..r.width() as usize).all(|x| {
            let v = u32::from_ne_bytes(data[y * stride + x * 4..y * stride + x * 4 + 4].try_into().unwrap()) & 0xFF_FFFF;
            v == 0 || v == 0xFF_FFFF
        })
    });
    assert!(pure, "raster is pure black and white");
    assert_eq!(job.mode(0), Some(Mode::Sharp), "text label → sharp");
    let photo = label_job(&[d.path("photo.png")]);
    assert_eq!(photo.mode(0), Some(Mode::Dither), "photo → dotted");
}

#[test]
fn rotation() {
    let d = Dir::new("rotation");
    samples(&d);
    let mut job = label_job(&[d.path("label.png")]);
    assert!(job.rotated(0), "landscape image auto-rotates onto portrait label");
    job.rotate = "0".into();
    assert!(!job.rotated(0), "rotate off");
    for a in ["90", "180", "270"] {
        job.rotate = a.into();
        assert_eq!(job.angle(0), a.parse::<i32>().unwrap(), "rotate {a}°");
    }
    let pdf = label_job(&[d.path("two.pdf")]);
    assert!(!pdf.rotated(0), "4x6 PDF not rotated");
}

#[test]
fn pdf_sent_to_printer() {
    if !has("pdfinfo") || !has("pdfimages") {
        return eprintln!("SKIP: needs poppler's pdfinfo and pdfimages");
    }
    let d = Dir::new("pdfout");
    samples(&d);
    let job = label_job(&[d.path("two.pdf")]);
    let pdf = d.path("out.pdf");
    job.write_pdf(&pdf).unwrap();
    let info = String::from_utf8(Command::new("pdfinfo").arg(&pdf).output().unwrap().stdout).unwrap();
    assert!(info.contains("Pages:           2"), "output has both pages");
    let (pw, ph) = job.page();
    assert!(info.contains(&format!("{pw} x {ph} pts")), "output page is the paper size: {info}");

    // The dots on paper are exactly the dots in the preview.
    let out = d.path("img");
    std::fs::create_dir_all(&out).unwrap();
    assert!(Command::new("pdfimages").args(["-f", "1", "-l", "1", "-png"]).arg(&pdf).arg(out.join("i")).status().unwrap().success());
    let mut imgs: Vec<_> = std::fs::read_dir(&out).unwrap().flatten().map(|e| e.path()).collect();
    imgs.sort();
    let pb = gtk::gdk_pixbuf::Pixbuf::from_file(&imgs[0]).unwrap();
    let mut want = job.raster(0);
    let (data, stride) = surface_bytes(&mut want);
    assert_eq!((pb.width(), pb.height()), (want.width(), want.height()), "printed image is the preview's size");
    let (n, rs, px) = (pb.n_channels() as usize, pb.rowstride() as usize, pb.read_pixel_bytes());
    let same = (0..want.height() as usize)
        .all(|y| (0..want.width() as usize).all(|x| px[y * rs + x * n] == data[y * stride + x * 4]));
    assert!(same, "printed dots match the preview exactly");
    let list = String::from_utf8(Command::new("pdfimages").arg("-list").arg(&pdf).output().unwrap().stdout).unwrap();
    assert!(list.lines().nth(2).is_some_and(|l| l.contains(" no ")), "no smoothing on paper (interpolate no)");
}

#[test]
fn margins_and_whole_dots() {
    let d = Dir::new("margins");
    samples(&d);
    let mut job = label_job(&[d.path("label.png")]);
    job.size = "w144h72".into();
    job.margin_mode = "36".into();
    assert!((0.0..36.0).contains(&job.margin()), "big margin on a 2x1 label is clamped");
    if !has("pdfimages") {
        return;
    }
    job.size = "Letter".into();
    job.margin_mode = "auto".into();
    let pdf = d.path("letter.pdf");
    job.write_pdf(&pdf).unwrap();
    let (w, h) = job.dots();
    let list = String::from_utf8(Command::new("pdfimages").arg("-list").arg(&pdf).output().unwrap().stdout).unwrap();
    let f: Vec<&str> = list.lines().nth(2).unwrap().split_whitespace().collect();
    assert_eq!((f[3], f[4]), (w.to_string().as_str(), h.to_string().as_str()), "Letter at 203 dpi: image is the page in dots");
    assert_eq!((f[12], f[13]), ("203", "203"), "and exactly 203 dpi");
}

#[test]
fn label_off_an_a4_page_fills_the_label() {
    let d = Dir::new("trim");
    // What a browser sends: a 4x6 label in the top-left corner of an A4 page.
    let s = cairo::PdfSurface::new(595.0, 842.0, d.path("a4label.pdf")).unwrap();
    let c = cairo::Context::new(&s).unwrap();
    c.set_line_width(6.0);
    c.rectangle(12.0, 12.0, 264.0, 408.0);
    c.stroke().unwrap();
    c.rectangle(40.0, 200.0, 200.0, 100.0);
    c.fill().unwrap();
    c.show_page().unwrap();
    drop(c);
    s.finish();
    let mut job = label_job(&[d.path("a4label.pdf")]);
    let (x, y, w, h) = job.area(0);
    assert!(x < 12.0 && y < 12.0 && w < 300.0 && h < 440.0, "label area found: {:?}", (x, y, w, h));
    job.set_printer_info(PrinterInfo { sizes: vec![("A4".into(), (595.0, 842.0))], default: "A4".into(), dpi: 300, mono: false });
    assert_eq!(job.area(0), (0.0, 0.0, 595.0, 842.0), "paper printers print the whole page");
}

/// A Code 128 barcode made by zint, as a greyscale image.
fn zint_barcode(d: &Dir) -> Option<Grey> {
    if !has("zint") {
        return None;
    }
    let png = d.path("bc.png");
    let ok = Command::new("zint")
        .args(["-b", "20", "--notext", "--scale=2", "--height=40", "-d", "TEST-1234567890", "-o"])
        .arg(&png)
        .status()
        .ok()?
        .success();
    let pb = gtk::gdk_pixbuf::Pixbuf::from_file(&png).ok().filter(|_| ok)?;
    let (n, rs, px) = (pb.n_channels() as usize, pb.rowstride() as usize, pb.read_pixel_bytes());
    let (w, h) = (pb.width() as usize, pb.height() as usize);
    Some(Grey { w, h, px: (0..h).flat_map(|y| (0..w).map(move |x| (y, x))).map(|(y, x)| px[y * rs + x * n]).collect() })
}

#[test]
fn barcodes_survive_black_and_white() {
    let d = Dir::new("barcodes");
    let Some(bars) = zint_barcode(&d) else { return eprintln!("SKIP: install zint") };
    // The bars printed mid-grey, with a white border.
    let (w, h) = (bars.w + 80, bars.h + 80);
    let mut lum = Grey { w, h, px: vec![255; w * h] };
    for y in 0..bars.h {
        for x in 0..bars.w {
            lum.px[(y + 40) * w + x + 40] = if bars.px[y * bars.w + x] < 128 { 140 } else { 255 };
        }
    }
    assert_eq!(raster::scan(&lum).len(), 1, "the grey barcode reads");
    let mut black = lum.to_black(Mode::Sharp);
    let lost = Grey { w, h, px: black.iter().map(|&b| if b { 0 } else { 255 }).collect() };
    assert!(raster::scan(&lost).is_empty(), "grey barcode lost by a plain cut");
    assert_eq!(raster::keep_barcodes(&lum, &mut black), Some((1, 1)), "barcode check repairs it");
}

#[test]
fn example_label_barcodes_scan() {
    let label = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/example-label.png");
    let job = label_job(&[label]);
    assert_eq!(job.mode(0), Some(Mode::Sharp));
    assert_eq!(job.barcodes(0), Some((2, 2)), "both barcodes scan from the printed dots");
}

