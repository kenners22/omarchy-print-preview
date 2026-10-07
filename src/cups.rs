//! Asking CUPS about printers: which are set up, their paper sizes and dots
//! per inch, whether they print colour, and whether they're answering.
//!
//! `lpstat -a/-e/-v` and `lpoptions -l` each spend a second browsing the
//! network before they answer, so the printer list and each printer's options
//! come from CUPS's own web server on localhost instead (instant), with those
//! commands kept only as a fallback.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::process::Command;
use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;

/// Our own "Preview" printer from older installs: never print back into it.
pub const VIRTUAL: &[&str] = &["Preview"];

/// Paper sizes known by name, in points.
pub const NAMED: &[(&str, (f64, f64))] = &[
    ("A3", (842.0, 1191.0)),
    ("A4", (595.0, 842.0)),
    ("A5", (420.0, 595.0)),
    ("A5Rotated", (595.0, 420.0)),
    ("A6", (297.0, 420.0)),
    ("Letter", (612.0, 792.0)),
    ("Legal", (612.0, 1008.0)),
    ("Tabloid", (792.0, 1224.0)),
    ("Executive", (522.0, 756.0)),
];

/// Label sizes shown first; a printer's other sizes go under "Other sizes".
pub const COMMON: &[&str] =
    &["w288h432", "w288h288", "w288h216", "w288h144", "w144h72", "w162h90", "A4", "A5", "A6", "Letter"];

pub fn named(name: &str) -> Option<(f64, f64)> {
    NAMED.iter().find(|(n, _)| *n == name).map(|(_, d)| *d)
}

/// stdout of a command, or None if it failed (CUPS can be slow while printers wake).
pub fn sh(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| home().join(fallback))
}

/// Optional: an executable here makes the offline warning's button run it (with
/// the printer name), e.g. a script that re-finds a Wi-Fi printer that moved.
pub fn fix_hook() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join("print-preview/find-printer")
}

/// Where the last printer used is remembered.
pub fn last_file() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state").join("print-preview/printer")
}

/// GET a path from CUPS on localhost:631. Body only, None unless it's a 200.
fn cups_get(path: &str) -> Option<Vec<u8>> {
    let addr = ("localhost", 631).to_socket_addrs().ok()?.next()?;
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(3)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
    write!(s, "GET {path} HTTP/1.0\r\nHost: localhost:631\r\nConnection: close\r\n\r\n").ok()?;
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).ok()?;
    let split = buf.windows(4).position(|w| w == b"\r\n\r\n")?;
    let status = String::from_utf8_lossy(&buf[..split]);
    status.lines().next()?.split_whitespace().nth(1).filter(|c| *c == "200")?;
    Some(buf[split + 4..].to_vec())
}

/// Queues set up in CUPS, in the order CUPS lists them.
pub fn installed_printers() -> Vec<String> {
    static HREF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)href="/printers/([^"/?#]+)""#).unwrap());
    if let Some(page) = cups_get("/printers/") {
        let page = String::from_utf8_lossy(&page);
        let mut names: Vec<String> = Vec::new();
        for c in HREF.captures_iter(&page) {
            let n = urlencoding::decode(&c[1]).map(|s| s.into_owned()).unwrap_or_else(|_| c[1].to_string());
            if !names.contains(&n) {
                names.push(n);
            }
        }
        if !names.is_empty() {
            return names;
        }
    }
    static DEVICE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^device for ([^:\s]+):").unwrap());
    DEVICE.captures_iter(&sh("lpstat", &["-v"]).unwrap_or_default()).map(|c| c[1].to_string()).collect()
}

/// Real printers, and the one to start on: last used, else the CUPS default, else the first.
/// Printers CUPS merely found on the network are only offered when nothing is installed,
/// so a printer you've set up doesn't show twice.
pub fn printers() -> (Vec<String>, String) {
    let mut installed = installed_printers();
    if installed.iter().all(|p| VIRTUAL.contains(&p.as_str())) {
        installed = sh("lpstat", &["-e"]).unwrap_or_default().split_whitespace().map(String::from).collect();
    }
    let names: Vec<String> = installed.into_iter().filter(|p| !VIRTUAL.contains(&p.as_str())).collect();
    let last = std::fs::read_to_string(last_file()).ok().map(|s| s.trim().to_string());
    static DEST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"destination: (\S+)").unwrap());
    let default = DEST.captures(&sh("lpstat", &["-d"]).unwrap_or_default()).map(|c| c[1].to_string());
    let pick = [last, default, names.first().cloned()].into_iter().flatten().find(|p| names.contains(p));
    (names, pick.unwrap_or_default())
}

pub fn remember_printer(printer: &str) {
    let f = last_file();
    if let Some(dir) = f.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(f, format!("{printer}\n"));
}

/// Size in points of a CUPS/IPP paper name: A4, A4.Fullbleed, w288h432, iso_a4_210x297mm, 4x6…
pub fn media_dims(name: &str) -> Option<(f64, f64)> {
    static SUFFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.[A-Za-z]+$").unwrap());
    static WH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^w(\d+(?:\.\d+)?)h(\d+(?:\.\d+)?)$").unwrap());
    static XY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+(?:\.\d+)?)x(\d+(?:\.\d+)?)(mm|in)?$").unwrap());
    let base = SUFFIX.replace(name, ""); // A4.Fullbleed → A4, but keep 8.5x11in whole
    if let Some(d) = named(&base) {
        return Some(d);
    }
    if let Some(c) = WH.captures(&base) {
        return Some((c[1].parse().ok()?, c[2].parse().ok()?));
    }
    let c = XY.captures(&base)?;
    let k = if c.get(3).is_some_and(|m| m.as_str() == "mm") { 72.0 / 25.4 } else { 72.0 };
    let round2 = |v: f64| (v * 100.0).round() / 100.0;
    Some((round2(c[1].parse::<f64>().ok()? * k), round2(c[2].parse::<f64>().ok()? * k)))
}

/// The printer's options in `lpoptions -l` form ("PageSize/Media Size: A4 *Letter"),
/// from the PPD that CUPS serves on localhost. None if it has no PPD there.
fn ppd_options(printer: &str) -> Option<String> {
    let ppd = cups_get(&format!("/printers/{}.ppd", urlencoding::encode(printer)))?;
    let ppd: String = ppd.iter().map(|&b| b as char).collect(); // PPDs are Latin-1
    Some(ppd_to_options(&ppd, &user_defaults(printer))).filter(|s| !s.is_empty())
}

/// This user's own defaults (`lpoptions -p X -o key=value`), which win, as with lpoptions.
fn user_defaults(printer: &str) -> HashMap<String, String> {
    let mut mine = HashMap::new();
    let text = std::fs::read_to_string(home().join(".cups/lpoptions")).unwrap_or_default();
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() > 2 && (f[0] == "Dest" || f[0] == "Default") && f[1] == printer {
            for o in &f[2..] {
                if let Some((k, v)) = o.split_once('=') {
                    mine.insert(k.to_string(), v.to_string());
                }
            }
        }
    }
    mine
}

/// The OpenUI options of a PPD, as `lpoptions -l` prints them.
pub fn ppd_to_options(ppd: &str, mine: &HashMap<String, String>) -> String {
    static OPEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^\*OpenUI \*([^/:\s]+)(?:/([^:]*))?:").unwrap());
    let mut out = String::new();
    for c in OPEN.captures_iter(ppd) {
        let key = &c[1];
        let text = c.get(2).map_or(key, |m| m.as_str());
        let start = c.get(0).unwrap().end();
        let body = &ppd[start..start + ppd[start..].find("*CloseUI").unwrap_or(ppd.len() - start)];
        let default = mine.get(key).cloned().or_else(|| {
            let re = Regex::new(&format!(r"(?m)^\*Default{}:\s*(\S+)", regex::escape(key))).ok()?;
            re.captures(ppd).map(|d| d[1].to_string())
        });
        let choice = Regex::new(&format!(r"(?m)^\*{}\s+([^/:\s]+)", regex::escape(key))).unwrap();
        let choices: Vec<String> = choice
            .captures_iter(body)
            .map(|m| if default.as_deref() == Some(&m[1]) { format!("*{}", &m[1]) } else { m[1].to_string() })
            .collect();
        out.push_str(&format!("{key}/{text}: {}\n", choices.join(" ")));
    }
    out
}

/// What the preview needs to know about a printer.
#[derive(Clone, Debug)]
pub struct PrinterInfo {
    /// Paper sizes: (CUPS name, (w, h) in points).
    pub sizes: Vec<(String, (f64, f64))>,
    pub default: String,
    pub dpi: u32,
    /// Black-only: thermal label printers, mono lasers.
    pub mono: bool,
}

impl PrinterInfo {
    /// An ordinary colour printer: what to assume when CUPS can't be asked.
    pub fn plain() -> Self {
        PrinterInfo {
            sizes: vec![("A4".into(), named("A4").unwrap()), ("Letter".into(), named("Letter").unwrap())],
            default: "A4".into(),
            dpi: 300,
            mono: false,
        }
    }
}

pub fn printer_info(printer: &str) -> PrinterInfo {
    match ppd_options(printer).or_else(|| sh("lpoptions", &["-p", printer, "-l"])) {
        Some(opts) => parse_options(&opts),
        None => PrinterInfo::plain(),
    }
}

/// Paper sizes, default size, dpi and black-only from `lpoptions -l` text.
pub fn parse_options(opts: &str) -> PrinterInfo {
    static SIZES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^(?:PageSize|media)/[^:]*:(.*)$").unwrap());
    static DPI: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?m)^(?:Resolution|printer-resolution)/[^:]*:.*\*(\d+)(?:x\d+)?dpi").unwrap());
    static COLOUR_LINES: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?m)^(?:ColorModel|print-color-mode)/[^:]*:(.*)$").unwrap());
    // A colour choice: RGB, CMYK, CMY, Colour/Color or auto, not inside another word.
    static COLOUR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(?:^|[^A-Za-z])\*?(RGB|CMYK|CMY|Colou?r|auto)").unwrap());
    let mut sizes = Vec::new();
    let mut default = None;
    if let Some(c) = SIZES.captures(opts) {
        for tok in c[1].split_whitespace() {
            let name = tok.trim_start_matches('*');
            if let Some(d) = media_dims(name).filter(|d| d.0 > 0.0 && d.1 > 0.0) {
                sizes.push((name.to_string(), d));
                if tok.starts_with('*') {
                    default = Some(name.to_string());
                }
            }
        }
    }
    if sizes.is_empty() {
        sizes.push(("A4".into(), named("A4").unwrap()));
    }
    let dpi = DPI.captures(opts).and_then(|c| c[1].parse().ok()).unwrap_or(300);
    let mono = !COLOUR_LINES.captures_iter(opts).any(|c| COLOUR.is_match(&c[1]));
    let default = default.unwrap_or_else(|| sizes[0].0.clone());
    PrinterInfo { sizes, default, dpi, mono }
}

/// Some(true) answering, Some(false) not (with why), None when we can't tell
/// (USB, dnssd://, CUPS not answering).
pub fn printer_status(printer: &str) -> (Option<bool>, String) {
    if sh("lpstat", &["-p", printer]).unwrap_or_default().contains("disabled") {
        return (Some(false), format!("{printer} is paused in CUPS."));
    }
    static URI: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\b(socket|ipps?|https?|lpd)://(\[[0-9A-Fa-f:.]+\]|[^:/\s\[\]]+)(?::(\d+))?").unwrap());
    let v = sh("lpstat", &["-v", printer]).unwrap_or_default();
    let Some(c) = URI.captures(&v) else { return (None, String::new()) };
    let host = c[2].trim_matches(|ch| ch == '[' || ch == ']').to_string();
    let port: u16 = c.get(3).and_then(|p| p.as_str().parse().ok()).unwrap_or(match &c[1] {
        "socket" => 9100,
        "http" => 80,
        "https" => 443,
        "lpd" => 515,
        _ => 631,
    });
    let up = (host.as_str(), port)
        .to_socket_addrs()
        .ok()
        .and_then(|mut a| a.next())
        .is_some_and(|a| TcpStream::connect_timeout(&a, Duration::from_secs(2)).is_ok());
    if up { (Some(true), String::new()) } else { (Some(false), format!("{printer} isn't answering at {host}.")) }
}

/// A paper size for menus: "A4", "A5 landscape", "4 × 6 in".
pub fn size_label(name: &str, dims: (f64, f64)) -> String {
    static SUFFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.[A-Za-z]+$").unwrap());
    let (base, extra) = if SUFFIX.is_match(name) { name.split_once('.').unwrap_or((name, "")) } else { (name, "") };
    if named(base).is_some() {
        let extra = if extra.is_empty() { String::new() } else { format!(" ({})", extra.to_lowercase()) };
        return base.replace("Rotated", " landscape") + &extra;
    }
    let f = |pt: f64| {
        let s = format!("{:.2}", pt / 72.0);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    };
    format!("{} × {} in", f(dims.0), f(dims.1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paper_names() {
        assert_eq!(media_dims("iso_a4_210x297mm"), Some((595.28, 841.89)));
        assert_eq!(media_dims("na_letter_8.5x11in"), Some((612.0, 792.0)));
        assert_eq!(media_dims("A4.Fullbleed"), named("A4"));
        assert_eq!(media_dims("4x6"), Some((288.0, 432.0)));
        assert_eq!(media_dims("w288h432"), Some((288.0, 432.0)));
        assert_eq!(media_dims("Custom.WIDTHxHEIGHT"), None);
    }

    #[test]
    fn size_labels() {
        assert_eq!(size_label("A4", (595.0, 842.0)), "A4");
        assert_eq!(size_label("A5Rotated", (595.0, 420.0)), "A5 landscape");
        assert_eq!(size_label("A4.Fullbleed", (595.0, 842.0)), "A4 (fullbleed)");
        assert_eq!(size_label("w288h432", (288.0, 432.0)), "4 × 6 in");
        assert_eq!(size_label("w162h90", (162.0, 90.0)), "2.25 × 1.25 in");
    }

    const PPD: &str = "*PPD-Adobe: \"4.3\"\n*ColorDevice: False\n\
        *OpenUI *PageSize/Media Size: PickOne\n*DefaultPageSize: w288h432\n\
        *PageSize w288h432/4x6: \"\"\n*PageSize A4/A4: \"\"\n*CloseUI: *PageSize\n\
        *OpenUI *Resolution/Resolution: PickOne\n*DefaultResolution: 203dpi\n\
        *Resolution 203dpi/203 DPI: \"\"\n*CloseUI: *Resolution\n";

    #[test]
    fn ppd_options_like_lpoptions() {
        let o = ppd_to_options(PPD, &HashMap::new());
        assert_eq!(o, "PageSize/Media Size: *w288h432 A4\nResolution/Resolution: *203dpi\n");
        let mine = HashMap::from([("PageSize".to_string(), "A4".to_string())]);
        assert!(ppd_to_options(PPD, &mine).starts_with("PageSize/Media Size: w288h432 *A4"));
    }

    #[test]
    fn label_printer_from_options() {
        let i = parse_options(&ppd_to_options(PPD, &HashMap::new()));
        assert_eq!((i.default.as_str(), i.dpi, i.mono), ("w288h432", 203, true));
        let colour = parse_options("PageSize/Media Size: *A4 Letter\nColorModel/Output Mode: *RGB Gray\n");
        assert!(!colour.mono);
        let ipp = parse_options("media/Media: *iso_a4_210x297mm\nprint-color-mode/Color: *auto monochrome\n");
        assert!(!ipp.mono);
        let mono = parse_options("media/Media: *iso_a4_210x297mm\nprint-color-mode/Color: *monochrome\n");
        assert!(mono.mono);
    }
}
