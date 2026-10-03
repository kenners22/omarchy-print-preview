#!/usr/bin/env python3
"""Tests for print-preview. Run: python3 print-preview/tests.py
Makes its own sample files in a temp folder. Never prints anything."""
import glob, importlib.machinery, importlib.util, os, random, subprocess, sys, tempfile

import cairo
import numpy as np

here = os.path.dirname(os.path.abspath(__file__))
loader = importlib.machinery.SourceFileLoader("pp", os.path.join(here, "print-preview"))
spec = importlib.util.spec_from_loader("pp", loader)
pp = importlib.util.module_from_spec(spec)
loader.exec_module(pp)
from gi.repository import GdkPixbuf

os.chdir(tempfile.mkdtemp())
random.seed(3)


def barcode(c, x0, x1, y, h, widths, gaps):
    x = x0
    while x < x1:
        w = random.choice(widths)
        c.rectangle(x, y, w, h)
        c.fill()
        x += w + random.choice(gaps)


s = cairo.ImageSurface(cairo.FORMAT_RGB24, 1200, 800)  # landscape label image
c = cairo.Context(s)
c.set_source_rgb(1, 1, 1)
c.paint()
c.set_source_rgb(0, 0, 0)
c.set_font_size(64)
c.move_to(60, 120)
c.show_text("SAMPLE")
barcode(c, 60, 1140, 520, 220, [4, 6, 10], [4, 6, 8])
s.write_to_png("label.png")

s = cairo.ImageSurface(cairo.FORMAT_RGB24, 300, 400)  # "photo": all mid-tones
c = cairo.Context(s)
g = cairo.LinearGradient(0, 0, 300, 400)
g.add_color_stop_rgb(0, 1, .6, .2)
g.add_color_stop_rgb(1, .1, .2, .6)
c.set_source(g)
c.paint()
s.write_to_png("photo.png")

s = cairo.PDFSurface("two.pdf", 288, 432)  # 4x6 PDF, 2 pages
c = cairo.Context(s)
for n in (1, 2):
    c.set_font_size(28)
    c.move_to(20, 50)
    c.show_text(f"PAGE {n}")
    barcode(c, 30, 258, 300, 100, [1, 1.5, 2.5], [1, 1.5, 2])
    c.show_page()
s.finish()

with open("doc.ps", "w") as f:
    f.write("%!PS\n/Helvetica findfont 30 scalefont setfont 50 700 moveto (PS) show showpage\n")
with open("junk.png", "w") as f:
    f.write("not an image")
open("empty.pdf", "w").close()

fails = []


def check(name, cond, extra=""):
    print("PASS" if cond else "FAIL", name, extra)
    if not cond:
        fails.append(name)


def pixels(surf):
    return np.ndarray((surf.get_height(), surf.get_stride() // 4), np.uint32, surf.get_data())[:, :surf.get_width()]


# --- loading -------------------------------------------------------------------
check("image loads", len(pp.load("label.png")) == 1)
check("PDF loads both pages", len(pp.load("two.pdf")) == 2)
check("PostScript converts", len(pp.load("doc.ps")) == 1)
for bad in ("junk.png", "empty.pdf", "missing.png"):
    try:
        pp.load(bad)
        check(f"{bad} rejected", False)
    except Exception as e:
        check(f"{bad} rejected", True, type(e).__name__)

# --- printer choice --------------------------------------------------------------
real_last = pp.LAST
pp.LAST = tempfile.mktemp()
names, default = pp.printers()
check("Preview queue never offered", "Preview" not in names)
with open(pp.LAST, "w") as f:
    f.write("NoSuchPrinter\n")
check("stale remembered printer ignored", pp.printers()[1] == default)
if names:
    pp.remember_printer(names[-1])
    check("remembered printer used", pp.printers()[1] == names[-1])
pp.LAST = real_last

# --- paper names, page ranges, dialog options, zbar output, margins --------------
check("paper: iso_a4_210x297mm", pp.media_dims("iso_a4_210x297mm") == (595.28, 841.89), pp.media_dims("iso_a4_210x297mm"))
check("paper: na_letter_8.5x11in", pp.media_dims("na_letter_8.5x11in") == (612, 792))
check("paper: A4.Fullbleed", pp.media_dims("A4.Fullbleed") == pp.NAMED["A4"])
check("paper: 4x6", pp.media_dims("4x6") == (288, 432))
check("paper: w288h432", pp.media_dims("w288h432") == (288, 432))
check("paper: nonsense", pp.media_dims("Custom.WIDTHxHEIGHT") is None)
check("page ranges", pp.pick_pages("1-2,4,9-", 10) == {1, 2, 4, 9, 10})
check("page ranges junk", pp.pick_pages("x,,-", 3) == set())
check("dialog options", pp.job_options("3\npage-ranges=2-3 media=A4 job-uuid=urn:x\n") == (3, "2-3"))
check("dialog options empty", pp.job_options("") == (1, None))
two = pp.Job(["two.pdf"], copies=5, page_ranges="2")
check("job keeps only the chosen pages", len(two.pages) == 1)
check("job takes the dialog's copies", two.copies == 5)
xml_a = "<symbol type='CODE-128'><polygon points='+1,+2 +30,+40'/><data><![CDATA[ABC]]></data></symbol>"
xml_b = "<symbol type='CODE-128'><data><![CDATA[ABC]]></data><polygon points='+1,+2 +30,+40'/></symbol>"
real_run = pp.subprocess.run
for name, xml in (("polygon first", xml_a), ("data first", xml_b)):
    pp.subprocess.run = lambda *a, **k: type("R", (), {"stdout": xml, "returncode": 0})()
    check(f"zbar output, {name}", pp.scan(np.full((50, 50), 255, np.uint8)) == [("ABC", 1, 2, 30, 40)])
pp.subprocess.run = real_run

# The rest needs a black-only label printer such as a thermal label printer.
job = pp.Job(["label.png"])
if not (job.printer and job.bw != "off"):
    print("SKIP raster tests: no black-only printer set up")
    sys.exit(1 if fails else 0)

# --- black and white -------------------------------------------------------------
r = job.raster(job.pages[0])
pw, ph = job.page
check("raster is the page at printer dpi", (r.get_width(), r.get_height()) == (round(pw * job.dpi / 72), round(ph * job.dpi / 72)))
check("raster is pure black and white", set(np.unique(pixels(r) & 0xFFFFFF).tolist()) <= {0, 0xFFFFFF})
check("text label → sharp", job.mode(job.pages[0]) == "sharp")
photo = pp.Job(["photo.png"])
check("photo → dotted", photo.mode(photo.pages[0]) == "dither")

# --- rotation --------------------------------------------------------------------
check("landscape image auto-rotates onto portrait label", job.rotated(job.pages[0]))
job.rotate = "0"
check("rotate off", not job.rotated(job.pages[0]))
for a in ("90", "180", "270"):
    job.rotate = a
    check(f"rotate {a}°", job.angle(job.pages[0]) == int(a))
job.rotate = "auto"

# --- PDF sent to the printer -------------------------------------------------------
job = pp.Job(["two.pdf"])
check("4x6 PDF not rotated", not job.rotated(job.pages[0]))
pdf = tempfile.mktemp(suffix=".pdf")
job.write_pdf(pdf)
info = subprocess.run(["pdfinfo", pdf], capture_output=True, text=True).stdout
check("output has both pages", "Pages:           2" in info)
check("output page is the paper size", f"{pw:g} x {ph:g} pts" in info)
out = tempfile.mkdtemp()
subprocess.run(["pdfimages", "-f", "1", "-l", "1", "-png", pdf, out + "/i"], check=True)
pb = GdkPixbuf.Pixbuf.new_from_file(sorted(glob.glob(out + "/i*"))[0])
n = pb.get_n_channels()
got = np.frombuffer(pb.get_pixels(), np.uint8).reshape(pb.get_height(), pb.get_rowstride())[:, :pb.get_width() * n:n]
want = (pixels(job.raster(job.pages[0])) & 0xFF).astype(np.uint8)
check("printed dots match the preview exactly", got.shape == want.shape and (got == want).all())
lst = subprocess.run(["pdfimages", "-list", pdf], capture_output=True, text=True).stdout
check("no smoothing on paper (interpolate no)", " no " in lst.splitlines()[2])

# --- margins never eat the page; PDF page is a whole number of dots ---------------
job = pp.Job(["label.png"])
job.sizes, job.size, job.dpi = [("w144h72", (144, 72)), ("Letter", (612, 792))], "w144h72", 203
job.margin_mode = "36"
check("big margin on a 2x1 label is clamped", 0 <= job.margin < 36)
job.size, job.margin_mode = "Letter", "auto"
pdf = tempfile.mktemp(suffix=".pdf")
job.write_pdf(pdf)
W, H = job.dots
lst = subprocess.run(["pdfimages", "-list", pdf], capture_output=True, text=True).stdout.splitlines()[2].split()
check("Letter at 203 dpi: image is exactly the page in dots", (int(lst[3]), int(lst[4])) == (W, H) and lst[12] == lst[13] == "203", lst[3:5] + lst[12:14])
os.unlink(pdf)

# --- barcodes survive black and white (needs zbar; zint makes the test barcode) ----
import shutil
if shutil.which("zbarimg") and shutil.which("zint"):
    subprocess.run(["zint", "-b", "20", "--notext", "--scale=2", "--height=40", "-d", "TEST-1234567890", "-o", "bc.png"], check=True)
    pb = GdkPixbuf.Pixbuf.new_from_file("bc.png")
    n = pb.get_n_channels()
    bars = np.frombuffer(pb.get_pixels(), np.uint8).reshape(pb.get_height(), pb.get_rowstride())[:, :pb.get_width() * n:n]
    lum = np.full((bars.shape[0] + 80, bars.shape[1] + 80), 255, np.int64)
    lum[40:-40, 40:-40] = np.where(bars < 128, 140, 255)  # bars printed mid-grey
    black = lum < 128                                      # a plain cut loses them all
    check("grey barcode lost by a plain cut", not pp.scan(np.where(black, 0, 255).astype(np.uint8)))
    check("barcode check repairs it", pp.keep_barcodes(lum, black) == (1, 1))
    job = pp.Job([os.path.join(here, "docs", "example-label.png")])
    if job.mode(job.pages[0]) == "sharp":
        check("example label: both barcodes scan from the printed dots", job.barcodes(job.pages[0]) == (2, 2))
else:
    print("SKIP barcode tests: install zbar and zint")

print("\nFAILED: " + ", ".join(fails) if fails else "\nALL PASS")
sys.exit(1 if fails else 0)
