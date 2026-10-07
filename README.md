# Print preview for Omarchy

See what you're printing before you print it. Omarchy's image viewer (imv)
binds `Ctrl+P` to `lp`, which sends the image straight to the printer with no
preview and no settings. This replaces that with a small preview window, and
gives document viewers' `Ctrl+P` and GTK print dialogs' Preview button the same
window. One small Rust program (GTK 4, Poppler, Cairo).

![Print preview](docs/preview.png)

- **Images and PDFs**, including multi-page PDFs (and PostScript from older apps).
- **What you see is what prints.** Each page is drawn into a PDF at the exact
  paper size and sent with `lp -o print-scaling=none`, so the printer doesn't
  rescale it.
- **Thermal and black-only printers** (shipping label printers, mono lasers):
  the page is rendered at the printer's own dpi and turned into pure black and
  white, so the preview shows the actual dots. The tests check that the printed
  dots match the preview exactly. Each page is judged on its own: labels get a
  sharp cut-off, photos a dotted pattern.
- **Barcode check.** Shrinking a low-resolution label onto the printer's dots
  can make bars a dot too fat or thin, enough that a barcode stops scanning.
  Every barcode that reads on the page is read back from the black-and-white
  dots. If one broke, its own area gets a different black/white cut-off until it
  scans again, and a badge says so: *✓ 2 barcodes checked: will scan*, or a
  warning if one can't be saved.
- **Printer offline warning**, with a button for printer settings, or your own
  script (see below).
- One row of controls: printer, paper size (common label sizes first, the rest
  under *Other sizes…*), *Fit whole image*, copies. `⋮` opens rotate (auto, 0,
  90, 180, 270°), margin and black & white options. Remembers the last printer
  you used.

![More options](docs/options.png)

## Install

Needs Omarchy with Hyprland's Lua config (`~/.config/hypr/hyprland.lua`).

```bash
git clone https://github.com/kenners22/omarchy-print-preview ~/.local/share/omarchy-print-preview
~/.local/share/omarchy-print-preview/install.sh
```

That gives you:

| Where | What |
|---|---|
| imv | `Ctrl+P` opens the preview (a backup of `~/.config/imv/config` is kept) |
| Files | right-click an image or PDF → **Print preview…** (restart Files once: `nautilus -q`) |
| Anywhere | **Open with → Print preview** |
| Terminal | `print-preview file.png label.pdf` |
| Document viewers | `Ctrl+P` in Document Viewer (Evince) or Papers opens this preview on the open file, skipping the print dialog. Other apps get `Ctrl+P` as usual (the binding passes it on, as Omarchy's universal copy/paste does). |
| Print dialogs | the **Preview** button in GTK print dialogs (Document Viewer, LibreOffice, Files…) opens this preview instead of GNOME's full-screen one; **Print** still prints as before |

It installs `gtk4 poppler-glib ghostscript`, and Rust if you don't have it,
then builds the app in the clone and links it into `~/.local/bin`. After
`git pull`, run `./install.sh` again to rebuild.

Older versions could add a **Preview** printer that caught every app's
`Ctrl+P → Print`. That's gone; if you set it up, take it out with:

```bash
~/.local/share/omarchy-print-preview/install.sh --no-printer
```

### Undo

```bash
~/.local/share/omarchy-print-preview/install.sh --undo
```

This puts back the `Ctrl+P` line imv had before (yours, or Omarchy's), your
previous GTK preview command, and removes the window rules, the `Ctrl+P`
binding, the links and any old Preview printer. The libraries stay installed.

## Keys

`Enter` print · `Esc` cancel · `R` rotate · `F` fit/fill · `+`/`-` copies ·
`O` more options · `Page Up`/`Page Down` pages

## Fixing a printer that went missing

When the printer doesn't answer, a bar offers **Printer settings**. If your
printer is on Wi-Fi and its address keeps changing, put a script at
`~/.config/print-preview/find-printer` and make it executable. The button then
says **Find printer** and runs it in a terminal with the printer name as its
argument, so it can rescan and repoint the queue with `lpadmin`.

![Offline warning](docs/offline.png)

## Tests

```bash
cargo test
```

The tests never print. They make their own sample files and use a stand-in
203 dpi black-only label printer, so they don't need one set up. A few compare
against Poppler's `pdfinfo`/`pdfimages`, and the barcode repair test uses
`zint` to make a barcode (`sudo pacman -S zint`); those are skipped if missing.

## Notes

- Tested on Omarchy 4 (Hyprland 0.56, Lua config) with a 203 dpi 4×6 thermal
  label printer using CUPS's Zebra ZPL driver. On colour printers the page is
  redrawn as a PDF at the chosen size and placement (vector stays vector); it is
  not the app's original file, so print-specific extras like ICC profiles or
  overprint aren't carried over.
- Paper sizes come from the printer's PPD or IPP names (`A4`, `w288h432`,
  `iso_a4_210x297mm`, `na_letter_8.5x11in`, `4x6`…). If CUPS doesn't answer,
  the preview assumes an ordinary colour printer rather than guessing.
- Offline detection works for `socket://`, `ipp(s)://`, `http(s)://` and
  `lpd://` printers (IPv6 too). USB and `dnssd://` printers show no warning.
- Multi-page TIFFs show only the first page.
- The window floats centred at 760×820. Omarchy applies its slight window
  transparency before a rule can drop the tag, so the rule also sets
  `opacity = "1 1"`.
- Barcodes are read with [rxing](https://crates.io/crates/rxing) (a Rust port
  of ZXing), limited to the kinds shipping labels use: Code 128/39/93, ITF,
  EAN-13/UPC-A, QR, Data Matrix, PDF417, MaxiCode and Aztec. The short product
  codes (UPC-E, EAN-8) are left out, as ordinary text can read as one.
- Printers and their paper sizes are read from CUPS's web server on
  localhost, because `lpstat` and `lpoptions` spend a second browsing the
  network before they answer. The window opens in about half a second.

Community project, not part of Omarchy. MIT licensed.
