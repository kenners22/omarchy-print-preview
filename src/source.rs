//! The pages to print: PDF pages (PostScript is turned into PDF first) and images.

use std::cell::OnceCell;
use std::path::Path;
use std::process::Command;

use gtk::gdk_pixbuf::Pixbuf;
use gtk::glib;

pub enum Source {
    Pdf { page: poppler::Page, size: (f64, f64), content: OnceCell<Option<(f64, f64, f64, f64)>> },
    Image { surface: cairo::ImageSurface, size: (f64, f64) },
}

impl Source {
    /// Width and height: points for PDF pages, pixels for images.
    pub fn size(&self) -> (f64, f64) {
        match self {
            Source::Pdf { size, .. } | Source::Image { size, .. } => *size,
        }
    }

    /// Draw it at 1 unit per point (PDF) or pixel (image), origin top-left.
    pub fn paint(&self, cr: &cairo::Context) {
        match self {
            Source::Pdf { page, .. } => page.render_for_printing(cr),
            Source::Image { surface, .. } => {
                let _ = cr.set_source_surface(surface, 0.0, 0.0);
                cr.source().set_filter(cairo::Filter::Good);
                let _ = cr.paint();
            }
        }
    }

    /// (x, y, w, h) in points around everything printed on a PDF page, or None.
    /// Browsers print a 4x6 label PDF onto an A4 page, the label in a corner.
    pub fn content_box(&self) -> Option<(f64, f64, f64, f64)> {
        let Source::Pdf { page, size, content } = self else { return None };
        *content.get_or_init(|| {
            // 36 dpi is plenty to find it.
            let w = ((size.0 / 2.0).round() as i32).max(1);
            let h = ((size.1 / 2.0).round() as i32).max(1);
            let mut surf = cairo::ImageSurface::create(cairo::Format::Rgb24, w, h).ok()?;
            {
                let cr = cairo::Context::new(&surf).ok()?;
                cr.set_source_rgb(1.0, 1.0, 1.0);
                cr.paint().ok()?;
                cr.scale(0.5, 0.5);
                page.render_for_printing(&cr);
            }
            surf.flush();
            let stride = surf.stride() as usize;
            let data = surf.data().ok()?;
            let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, -1, -1);
            for y in 0..h {
                let row = &data[y as usize * stride..];
                for x in 0..w {
                    let p = &row[x as usize * 4..x as usize * 4 + 3];
                    if p.iter().any(|&c| c < 230) {
                        x0 = x0.min(x);
                        y0 = y0.min(y);
                        x1 = x1.max(x + 1);
                        y1 = y1.max(y + 1);
                    }
                }
            }
            if x1 < 0 {
                return None;
            }
            let pad = 2; // px = 4 pt
            let (x0, y0, x1, y1) = ((x0 - pad).max(0), (y0 - pad).max(0), (x1 + pad).min(w), (y1 + pad).min(h));
            Some((x0 as f64 * 2.0, y0 as f64 * 2.0, (x1 - x0) as f64 * 2.0, (y1 - y0) as f64 * 2.0))
        })
    }
}

/// Every page in a file: PDF, PostScript or an image.
pub fn load(path: &Path) -> Result<Vec<Source>, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let head = &data[..data.len().min(1024)];
    if head.starts_with(b"%!") {
        // PostScript (some apps print it): convert to PDF first.
        let pdf = temp_file("pdf").map_err(|e| e.to_string())?;
        let ok = Command::new("ps2pdf").arg(path).arg(&pdf).output().map(|o| o.status.success()).unwrap_or(false);
        let pages = if ok { load(&pdf) } else { Err("couldn't convert the PostScript (is ghostscript installed?)".into()) };
        let _ = std::fs::remove_file(&pdf);
        return pages;
    }
    if head.windows(5).any(|w| w == b"%PDF-") {
        // Kept in memory, so the file can go while its pages are shown.
        let doc = poppler::Document::from_bytes(&glib::Bytes::from_owned(data), None).map_err(|e| e.to_string())?;
        let pages: Vec<Source> = (0..doc.n_pages())
            .filter_map(|i| doc.page(i))
            .map(|page| {
                let size = page.size();
                Source::Pdf { page, size, content: OnceCell::new() }
            })
            .collect();
        if pages.is_empty() {
            return Err("the PDF has no pages".into());
        }
        return Ok(pages);
    }
    let pixbuf = Pixbuf::from_file(path).map_err(|e| e.to_string())?;
    let pixbuf = pixbuf.apply_embedded_orientation().unwrap_or(pixbuf);
    let surface = pixbuf_to_surface(&pixbuf)?;
    let size = (pixbuf.width() as f64, pixbuf.height() as f64);
    Ok(vec![Source::Image { surface, size }])
}

/// A Cairo surface with the pixbuf's pixels (premultiplied ARGB, as Cairo wants).
fn pixbuf_to_surface(pb: &Pixbuf) -> Result<cairo::ImageSurface, String> {
    let (w, h) = (pb.width(), pb.height());
    let mut surf = cairo::ImageSurface::create(cairo::Format::ARgb32, w, h).map_err(|e| e.to_string())?;
    let n = pb.n_channels() as usize;
    let alpha = pb.has_alpha();
    let src_stride = pb.rowstride() as usize;
    let dst_stride = surf.stride() as usize;
    let bytes = pb.read_pixel_bytes();
    {
        let mut dst = surf.data().map_err(|e| e.to_string())?;
        for y in 0..h as usize {
            for x in 0..w as usize {
                let s = &bytes[y * src_stride + x * n..];
                let a = if alpha { s[3] as u32 } else { 255 };
                let pm = |c: u8| ((c as u32 * a + 127) / 255) as u8;
                let d = &mut dst[y * dst_stride + x * 4..y * dst_stride + x * 4 + 4];
                // Cairo's ARGB32 is native-endian: B, G, R, A in memory on little-endian.
                let px = (a << 24) | ((pm(s[0]) as u32) << 16) | ((pm(s[1]) as u32) << 8) | pm(s[2]) as u32;
                d.copy_from_slice(&px.to_ne_bytes());
            }
        }
    }
    surf.mark_dirty();
    Ok(surf)
}

/// A new, empty file in the temp folder that only we can read (like mkstemp):
/// created exclusively, so nothing else can have put a file or link there first.
pub fn temp_file(ext: &str) -> std::io::Result<std::path::PathBuf> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    loop {
        let n = N.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
        let path = std::env::temp_dir().join(format!("print-preview-{}-{nanos}-{n}.{ext}", std::process::id()));
        match std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path) {
            Ok(_) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
}
