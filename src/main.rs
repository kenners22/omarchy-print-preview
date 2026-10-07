//! print-preview — preview images and PDFs before printing them.
//!
//!     print-preview [--more] FILE...        preview and print images / PDFs
//!     print-preview --unlink-tempfile FILE  the same, then delete FILE (GTK print dialog's Preview button)
//!     print-preview --active-window         preview the document open in the focused viewer window
//!
//! Opened by imv's Ctrl+P, Ctrl+P in document viewers, Files' right-click
//! "Print preview…", "Open with", and the Preview button in GTK print dialogs.
//!
//! Keys: Enter print, Esc cancel, R rotate, F fit/fill, +/- copies, O more options,
//! Page Up/Down pages.

mod cups;
mod job;
mod raster;
mod source;
mod ui;

#[cfg(test)]
mod tests;

use std::path::PathBuf;

const USAGE: &str = "print-preview — preview images and PDFs before printing them.

  print-preview [--more] FILE...        preview and print images / PDFs
  print-preview --unlink-tempfile FILE  the same, then delete FILE (GTK print dialog's Preview button)
  print-preview --active-window         preview the document open in the focused viewer window

Keys: Enter print, Esc cancel, R rotate, F fit/fill, +/- copies, O more options, Page Up/Down pages.";

#[derive(Default)]
struct Args {
    more: bool,
    unlink: bool,
    active_window: bool,
    screenshot: Option<PathBuf>,
    files: Vec<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args::default();
    let mut it = std::env::args_os().skip(1);
    while let Some(arg) = it.next() {
        match arg.to_str() {
            Some("--more") => a.more = true,
            Some("--unlink-tempfile") => a.unlink = true,
            Some("--active-window") => a.active_window = true,
            Some("--screenshot") => a.screenshot = Some(it.next().ok_or("--screenshot needs a file")?.into()),
            Some("-h" | "--help") => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            Some(s) if s.starts_with("--") => return Err(format!("unknown option {s}")),
            _ => a.files.push(arg.into()),
        }
    }
    Ok(a)
}

/// The file shown in the focused window (Ctrl+P in a document viewer).
/// Viewers keep the file open, so look at what its process has open, and pick
/// the one whose name starts the window title ("file.pdf — Title").
fn active_document() -> Option<PathBuf> {
    let win: serde_json::Value = serde_json::from_str(&cups::sh("hyprctl", &["activewindow", "-j"])?).ok()?;
    let pid = win.get("pid")?.as_i64()?;
    let title = win.get("title").and_then(|t| t.as_str()).unwrap_or_default();
    let mut found: Vec<PathBuf> = Vec::new();
    if let Ok(cmdline) = std::fs::read(format!("/proc/{pid}/cmdline")) {
        use std::os::unix::ffi::OsStrExt;
        found.extend(cmdline.split(|&b| b == 0).skip(1).filter(|a| !a.is_empty()).map(|a| PathBuf::from(std::ffi::OsStr::from_bytes(a))));
    }
    if let Ok(fds) = std::fs::read_dir(format!("/proc/{pid}/fd")) {
        found.extend(fds.flatten().filter_map(|fd| std::fs::read_link(fd.path()).ok()));
    }
    let mut docs: Vec<PathBuf> = Vec::new();
    for f in found {
        let f = match f.to_str().filter(|s| s.starts_with("file://")) {
            Some(uri) => gtk::glib::filename_from_uri(uri).map(|(p, _)| p).unwrap_or(f),
            None => f,
        };
        let printable = f.extension().and_then(|e| e.to_str()).is_some_and(|e| {
            matches!(e.to_ascii_lowercase().as_str(), "pdf" | "ps" | "png" | "jpg" | "jpeg" | "tif" | "tiff")
        });
        if printable && f.is_file() && !docs.contains(&f) {
            docs.push(f);
        }
    }
    let named = docs.iter().find(|d| d.file_name().is_some_and(|n| title.starts_with(&*n.to_string_lossy()))).cloned();
    named.or_else(|| docs.into_iter().next())
}

fn main() {
    let mut a = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("print-preview: {e}\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    if a.active_window {
        match active_document() {
            Some(doc) => a.files = vec![doc],
            None => return job::notify("Couldn't tell which file this window has open. Use its print dialog instead.", true),
        }
    }
    if a.files.is_empty() {
        eprintln!("print-preview: give at least one image or PDF\n\n{USAGE}");
        std::process::exit(2);
    }
    // The Wayland app id, which Hyprland's window rules match on.
    gtk::glib::set_prgname(Some("print-preview"));
    if gtk::init().is_err() {
        eprintln!("print-preview: no display");
        std::process::exit(1);
    }
    match job::Job::new(&a.files, None, 1, None) {
        Ok(job) => ui::run(job, a.more, a.screenshot.as_deref()),
        Err(e) => {
            let names: Vec<_> = a.files.iter().map(|f| f.file_name().unwrap_or_default().to_string_lossy()).collect();
            job::notify(&format!("Can't preview {}: {e}", names.join(", ")), true);
        }
    }
    if a.unlink {
        // GTK leaves its temporary PDF for the previewer to remove.
        for f in &a.files {
            let _ = std::fs::remove_file(f);
        }
    }
}

