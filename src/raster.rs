//! Turning a page into pure black and white for a black-only printer, without
//! breaking its barcodes.

/// A greyscale image, 0 = black, 255 = white, row by row.
#[derive(Clone)]
pub struct Grey {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

/// Which pixels print black, row by row.
pub type Black = Vec<bool>;

/// 8x8 ordered-dither thresholds (0-255), for photos on a black-only printer.
const BAYER: [[u8; 8]; 8] = [
    [0, 32, 8, 40, 2, 34, 10, 42],
    [48, 16, 56, 24, 50, 18, 58, 26],
    [12, 44, 4, 36, 14, 46, 6, 38],
    [60, 28, 52, 20, 62, 30, 54, 22],
    [3, 35, 11, 43, 1, 33, 9, 41],
    [51, 19, 59, 27, 49, 17, 57, 25],
    [15, 47, 7, 39, 13, 45, 5, 37],
    [63, 31, 55, 23, 61, 29, 53, 21],
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// One cut-off: text, line art, labels.
    Sharp,
    /// Ordered dots: photos.
    Dither,
}

impl Grey {
    /// Luminance of a Cairo RGB24/ARGB32 surface's pixels.
    pub fn from_surface_data(data: &[u8], stride: usize, w: usize, h: usize) -> Grey {
        let mut px = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let v = u32::from_ne_bytes(data[y * stride + x * 4..y * stride + x * 4 + 4].try_into().unwrap());
                let (r, g, b) = ((v >> 16) & 255, (v >> 8) & 255, v & 255);
                px.push(((r * 299 + g * 587 + b * 114) / 1000) as u8);
            }
        }
        Grey { w, h, px }
    }

    /// Mostly mid-tones (a photo) looks better as dots; labels and text stay sharp.
    pub fn auto_mode(&self) -> Mode {
        let (mut mid, mut n) = (0usize, 0usize);
        for y in (0..self.h).step_by(4) {
            for x in (0..self.w).step_by(4) {
                let v = self.px[y * self.w + x];
                n += 1;
                if v > 40 && v < 215 {
                    mid += 1;
                }
            }
        }
        if n > 0 && mid as f64 / n as f64 > 0.3 { Mode::Dither } else { Mode::Sharp }
    }

    pub fn to_black(&self, mode: Mode) -> Black {
        let mut black = Vec::with_capacity(self.px.len());
        for y in 0..self.h {
            for x in 0..self.w {
                let v = self.px[y * self.w + x];
                let cut = match mode {
                    Mode::Sharp => 128,
                    Mode::Dither => BAYER[y % 8][x % 8] as u16 * 4 + 2,
                };
                black.push((v as u16) < cut);
            }
        }
        black
    }

    fn crop(&self, x0: usize, y0: usize, x1: usize, y1: usize) -> Grey {
        let mut px = Vec::with_capacity((x1 - x0) * (y1 - y0));
        for y in y0..y1 {
            px.extend_from_slice(&self.px[y * self.w + x0..y * self.w + x1]);
        }
        Grey { w: x1 - x0, h: y1 - y0, px }
    }
}

/// Black and white as a greyscale image (for reading barcodes back).
fn grey_of(black: &[bool], w: usize, h: usize) -> Grey {
    Grey { w, h, px: black.iter().map(|&b| if b { 0 } else { 255 }).collect() }
}

/// A barcode found in an image: its text and the box around it (pixels).
#[derive(Clone, Debug, PartialEq)]
pub struct Code {
    pub data: String,
    pub x0: usize,
    pub y0: usize,
    pub x1: usize,
    pub y1: usize,
}

/// Barcodes that parcel and shipping labels use. The short product codes
/// (UPC-E, EAN-8) and Codabar are left out: with so few bars, ordinary text can
/// read as one, which would raise a false "may not scan" warning.
const LABEL_FORMATS: &[rxing::BarcodeFormat] = &[
    rxing::BarcodeFormat::CODE_128,
    rxing::BarcodeFormat::CODE_39,
    rxing::BarcodeFormat::CODE_93,
    rxing::BarcodeFormat::ITF,
    rxing::BarcodeFormat::EAN_13,
    rxing::BarcodeFormat::UPC_A,
    rxing::BarcodeFormat::QR_CODE,
    rxing::BarcodeFormat::DATA_MATRIX,
    rxing::BarcodeFormat::PDF_417,
    rxing::BarcodeFormat::MAXICODE,
    rxing::BarcodeFormat::AZTEC,
];

/// Every barcode readable in a greyscale image.
pub fn scan(g: &Grey) -> Vec<Code> {
    if g.w == 0 || g.h == 0 {
        return Vec::new();
    }
    let mut hints = rxing::DecodeHints {
        TryHarder: Some(true),
        PossibleFormats: Some(LABEL_FORMATS.iter().copied().collect()),
        ..Default::default()
    };
    let found = rxing::helpers::detect_multiple_in_luma_with_hints(g.px.clone(), g.w as u32, g.h as u32, &mut hints)
        .unwrap_or_default();
    let mut codes: Vec<Code> = Vec::new();
    for r in found {
        let pts = r.getPoints();
        if pts.is_empty() {
            continue;
        }
        let clamp = |v: f32, max: usize| (v.max(0.0) as usize).min(max);
        let x0 = clamp(pts.iter().map(|p| p.x).fold(f32::MAX, f32::min), g.w);
        let x1 = clamp(pts.iter().map(|p| p.x).fold(f32::MIN, f32::max), g.w);
        let y0 = clamp(pts.iter().map(|p| p.y).fold(f32::MAX, f32::min), g.h);
        let y1 = clamp(pts.iter().map(|p| p.y).fold(f32::MIN, f32::max), g.h);
        let code = Code { data: r.getText().to_string(), x0, y0, x1, y1 };
        if !codes.iter().any(|c| c.data == code.data) {
            codes.push(code);
        }
    }
    codes
}

/// Make every barcode that reads in the grey page still read once it's black and white.
///
/// Squeezing an image onto the printer's dots and cutting it to black/white can
/// make bars a dot too fat or thin, enough to stop a barcode scanning. For each
/// barcode that broke, try other black/white cut-offs in just its area until it
/// reads again. Changes `black` in place; returns (scanning, found), or None if
/// the page has no barcodes.
pub fn keep_barcodes(lum: &Grey, black: &mut Black) -> Option<(usize, usize)> {
    let want = scan(lum);
    if want.is_empty() {
        return None;
    }
    let (w, h) = (lum.w, lum.h);
    let have: Vec<String> = scan(&grey_of(black, w, h)).into_iter().map(|c| c.data).collect();
    for code in &want {
        if have.contains(&code.data) {
            continue;
        }
        // The quiet zone either side matters to the scanner. A linear barcode is
        // reported as a line across it, so reach up and down to cover its bars.
        let pad = 24;
        let reach = if code.y1 - code.y0 < pad { ((code.x1 - code.x0) / 2).max(pad) } else { pad };
        let (x0, x1) = (code.x0.saturating_sub(pad), (code.x1 + pad + 1).min(w));
        let (y0, y1) = (code.y0.saturating_sub(reach), (code.y1 + reach + 1).min(h));
        let area = lum.crop(x0, y0, x1, y1);
        for cut in [112u8, 144, 100, 160, 90, 176, 80, 192] {
            let trial: Vec<bool> = area.px.iter().map(|&v| v < cut).collect();
            if scan(&grey_of(&trial, area.w, area.h)).iter().any(|c| c.data == code.data) {
                for y in y0..y1 {
                    let row = &trial[(y - y0) * area.w..(y - y0 + 1) * area.w];
                    black[y * w + x0..y * w + x1].copy_from_slice(row);
                }
                break;
            }
        }
    }
    // Check the whole page as it will print.
    let finals: Vec<String> = scan(&grey_of(black, w, h)).into_iter().map(|c| c.data).collect();
    Some((want.iter().filter(|c| finals.contains(&c.data)).count(), want.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn photo_dithers_text_stays_sharp() {
        let flat = Grey { w: 64, h: 64, px: vec![128; 64 * 64] };
        assert_eq!(flat.auto_mode(), Mode::Dither);
        let mut text = Grey { w: 64, h: 64, px: vec![255; 64 * 64] };
        text.px[..64 * 8].fill(0);
        assert_eq!(text.auto_mode(), Mode::Sharp);
    }

    #[test]
    fn dither_makes_dots_cut_is_plain() {
        let g = Grey { w: 16, h: 16, px: vec![128; 256] };
        let dots = g.to_black(Mode::Dither).iter().filter(|&&b| b).count();
        assert!(dots > 64 && dots < 192, "about half black, got {dots}");
        assert!(g.to_black(Mode::Sharp).iter().all(|&b| !b));
    }
}
