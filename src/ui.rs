//! The preview window: the page, one row of controls, and a ⋮ sidebar of more options.

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};

use crate::cups;
use crate::job::Job;

const W: i32 = 760;
const H: i32 = 820;
const SIDE: i32 = 300;

const CSS: &str = "
.side-title { font-weight: bold; }
.muted { opacity: .65; }
.pager { background: rgba(0,0,0,.6); border-radius: 8px; padding: 2px 6px; }
.pager label { color: #eee; }
.badge { background: rgba(0,0,0,.72); color: #eee; border-radius: 8px; padding: 4px 9px; }
.warn { background: alpha(@warning_color, .25); padding: 6px 10px; }
";

/// A drop-down whose entries have ids, like GTK 3's ComboBoxText.
#[derive(Clone)]
struct Choice {
    widget: gtk::DropDown,
    model: gtk::StringList,
    ids: Rc<RefCell<Vec<String>>>,
    quiet: Rc<Cell<bool>>,
}

impl Choice {
    fn new(items: &[(&str, &str)], active: &str) -> Choice {
        let model = gtk::StringList::new(&[]);
        let c = Choice {
            widget: gtk::DropDown::new(Some(model.clone()), gtk::Expression::NONE),
            model,
            ids: Rc::default(),
            quiet: Rc::default(),
        };
        c.fill(&items.iter().map(|(k, l)| (k.to_string(), l.to_string())).collect::<Vec<_>>(), active);
        c
    }

    /// Replace the entries without firing on_change.
    fn fill(&self, items: &[(String, String)], active: &str) {
        self.quiet.set(true);
        let labels: Vec<&str> = items.iter().map(|(_, l)| l.as_str()).collect();
        self.model.splice(0, self.model.n_items(), &labels);
        *self.ids.borrow_mut() = items.iter().map(|(k, _)| k.clone()).collect();
        self.select(active);
        self.quiet.set(false);
    }

    fn select(&self, id: &str) {
        if let Some(i) = self.ids.borrow().iter().position(|k| k == id) {
            self.widget.set_selected(i as u32);
        }
    }

    fn active(&self) -> Option<String> {
        self.ids.borrow().get(self.widget.selected() as usize).cloned()
    }

    fn on_change(&self, f: impl Fn(String) + 'static) {
        let me = self.clone();
        self.widget.connect_selected_notify(move |_| {
            if !me.quiet.get()
                && let Some(id) = me.active()
            {
                f(id);
            }
        });
    }
}

struct Ui {
    job: RefCell<Job>,
    window: gtk::Window,
    view: gtk::DrawingArea,
    badge: gtk::Label,
    page_label: Option<gtk::Label>,
    warn: gtk::Box,
    warn_text: gtk::Label,
    fix_btn: gtk::Button,
    paper: Choice,
    bw: Choice,
    rotate: Choice,
    fit: gtk::CheckButton,
    spin: gtk::SpinButton,
    more: gtk::ToggleButton,
    print_btn: gtk::Button,
    side: gtk::Box,
    all_sizes: Cell<bool>,
    base_w: Cell<i32>,
}

pub fn run(job: Job, more: bool, screenshot: Option<&Path>) {
    let main_loop = glib::MainLoop::new(None, false);
    let ui = build(job, more);
    let ml = main_loop.clone();
    ui.window.connect_close_request(move |_| {
        ml.quit();
        glib::Propagation::Proceed
    });
    ui.window.present();
    if let Some(out) = screenshot {
        let out = out.to_path_buf();
        let (ui2, ml) = (ui.clone(), main_loop.clone());
        glib::timeout_add_local_once(Duration::from_millis(800), move || {
            save_screenshot(&ui2.window, &out);
            ml.quit();
        });
    }
    main_loop.run();
}

/// The window's contents as a PNG (for docs and checks).
fn save_screenshot(window: &gtk::Window, out: &Path) {
    let (w, h) = (window.width(), window.height());
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let snap = gtk::Snapshot::new();
    paintable.snapshot(&snap, w as f64, h as f64);
    if let (Some(node), Some(native)) = (snap.to_node(), window.native())
        && let Some(renderer) = native.renderer()
    {
        let texture = renderer.render_texture(node, None);
        let _ = texture.save_to_png(out);
    }
}

fn build(job: Job, more: bool) -> Rc<Ui> {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(CSS);
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }

    let window = gtk::Window::builder().title(format!("Print preview — {}", job.title)).default_width(W).default_height(H).build();
    let outer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let main = gtk::Box::new(gtk::Orientation::Vertical, 0);
    main.set_hexpand(true);

    // Offline warning.
    let warn = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    warn.add_css_class("warn");
    warn.set_visible(false);
    let warn_text = gtk::Label::builder().xalign(0.0).wrap(true).hexpand(true).build();
    let fix_btn = gtk::Button::with_label("Find printer");
    let again_btn = gtk::Button::with_label("Check again");
    warn.append(&warn_text);
    warn.append(&fix_btn);
    warn.append(&again_btn);
    main.append(&warn);

    // The page, with a barcode badge and a pager on top.
    let overlay = gtk::Overlay::new();
    let view = gtk::DrawingArea::builder().hexpand(true).vexpand(true).build();
    overlay.set_child(Some(&view));
    let badge = gtk::Label::builder().halign(gtk::Align::Start).valign(gtk::Align::Start).margin_top(10).margin_start(10).build();
    badge.add_css_class("badge");
    badge.set_visible(false);
    overlay.add_overlay(&badge);
    let n_pages = job.pages.len();
    let mut pager_parts = None;
    if n_pages > 1 {
        let pager = gtk::Box::builder().spacing(4).halign(gtk::Align::Center).valign(gtk::Align::End).margin_bottom(8).build();
        pager.add_css_class("pager");
        let prev = gtk::Button::from_icon_name("go-previous-symbolic");
        let next = gtk::Button::from_icon_name("go-next-symbolic");
        prev.add_css_class("flat");
        next.add_css_class("flat");
        let label = gtk::Label::new(None);
        pager.append(&prev);
        pager.append(&label);
        pager.append(&next);
        overlay.add_overlay(&pager);
        pager_parts = Some((prev, next, label));
    }
    main.append(&overlay);

    // The one row of controls.
    let bar = gtk::Box::builder().spacing(6).margin_top(10).margin_bottom(10).margin_start(10).margin_end(10).build();
    let printer_items: Vec<(&str, &str)> = job.printers.iter().map(|p| (p.as_str(), p.as_str())).collect();
    let printer = Choice::new(&printer_items, &job.printer);
    printer.widget.set_tooltip_text(Some("Printer"));
    let paper = Choice::new(&[], "");
    paper.widget.set_tooltip_text(Some("Paper size"));
    let fit = gtk::CheckButton::with_label("Fit whole image");
    fit.set_active(job.scale == "fit");
    let spin = gtk::SpinButton::with_range(1.0, 99.0, 1.0);
    spin.set_value(job.copies as f64);
    spin.set_tooltip_text(Some("Copies"));
    let more_btn = gtk::ToggleButton::builder().icon_name("view-more-symbolic").tooltip_text("More options (O)").build();
    let cancel = gtk::Button::with_label("Cancel");
    let print_btn = gtk::Button::with_label("Print");
    print_btn.add_css_class("suggested-action");
    print_btn.set_sensitive(!job.printer.is_empty());
    let spacer = gtk::Box::builder().hexpand(true).build();
    bar.append(&printer.widget);
    bar.append(&paper.widget);
    bar.append(&fit);
    bar.append(&gtk::Label::builder().label("Copies").margin_start(6).build());
    bar.append(&spin);
    bar.append(&more_btn);
    bar.append(&spacer);
    bar.append(&cancel);
    bar.append(&print_btn);
    main.append(&bar);
    outer.append(&main);

    // The ⋮ sidebar.
    let side_wrap = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    side_wrap.set_visible(false);
    side_wrap.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    let side = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .width_request(SIDE - 32)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();
    let t = gtk::Label::builder().label("More options").xalign(0.0).build();
    t.add_css_class("side-title");
    side.append(&t);
    let f = gtk::Label::builder().label(&job.title).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).margin_bottom(4).build();
    f.add_css_class("muted");
    side.append(&f);
    let grid = gtk::Grid::builder().row_spacing(10).column_spacing(12).build();
    let rotate = Choice::new(&[("auto", "Auto"), ("0", "None"), ("90", "90°"), ("180", "180°"), ("270", "270°")], &job.rotate);
    let margin = Choice::new(&[("auto", "Auto"), ("0", "None"), ("18", "¼ in"), ("36", "½ in")], &job.margin_mode);
    let bw = Choice::new(&[("auto", "Auto"), ("sharp", "Sharp (labels)"), ("dither", "Dotted (photos)"), ("off", "Off (colour)")], &job.bw);
    bw.widget.set_tooltip_text(Some(
        "How the page is turned into black and white for a black-only printer.\nThe preview shows the exact dots that will print.",
    ));
    for (row, (label, c)) in [("Rotate", &rotate), ("Margin", &margin), ("Black & white", &bw)].into_iter().enumerate() {
        grid.attach(&gtk::Label::builder().label(label).xalign(0.0).build(), 0, row as i32, 1, 1);
        c.widget.set_hexpand(true);
        grid.attach(&c.widget, 1, row as i32, 1, 1);
    }
    side.append(&grid);
    let keys = gtk::Label::builder().xalign(0.0).vexpand(true).valign(gtk::Align::End).build();
    keys.set_markup(
        "<small><b>R</b> rotate · <b>F</b> fit/fill · <b>+ −</b> copies\n\
         <b>O</b> options · <b>PgUp PgDn</b> pages\n<b>Enter</b> print · <b>Esc</b> cancel</small>",
    );
    keys.add_css_class("muted");
    side.append(&keys);
    side_wrap.append(&side);
    outer.append(&side_wrap);
    window.set_child(Some(&outer));

    let ui = Rc::new(Ui {
        job: RefCell::new(job),
        window: window.clone(),
        view: view.clone(),
        badge,
        page_label: pager_parts.as_ref().map(|p| p.2.clone()),
        warn,
        warn_text,
        fix_btn: fix_btn.clone(),
        paper: paper.clone(),
        bw: bw.clone(),
        rotate: rotate.clone(),
        fit: fit.clone(),
        spin: spin.clone(),
        more: more_btn.clone(),
        print_btn: print_btn.clone(),
        side: side_wrap,
        all_sizes: Cell::new(false),
        base_w: Cell::new(W),
    });
    fill_paper(&ui);

    // Drawing: the page centred, scaled to fit, with a soft shadow.
    let weak = Rc::downgrade(&ui);
    view.set_draw_func(move |_, cr, w, h| {
        let Some(ui) = weak.upgrade() else { return };
        let job = ui.job.borrow();
        cr.set_source_rgb(0.16, 0.16, 0.18);
        let _ = cr.paint();
        let pad = 20.0;
        let (pw, ph) = job.page();
        let z = ((w as f64 - 2.0 * pad) / pw).min((h as f64 - 2.0 * pad) / ph);
        if z <= 0.0 {
            return;
        }
        let (x, y) = ((w as f64 - pw * z) / 2.0, (h as f64 - ph * z) / 2.0);
        for (i, a) in [(6.0, 0.06), (3.0, 0.10), (1.0, 0.18)] {
            cr.set_source_rgba(0.0, 0.0, 0.0, a);
            cr.rectangle(x - i + 2.0, y - i + 4.0, pw * z + 2.0 * i, ph * z + 2.0 * i);
            let _ = cr.fill();
        }
        cr.translate(x, y);
        cr.scale(z, z);
        job.draw_page(cr, job.index, true);
        drop(job);
        let weak = Rc::downgrade(&ui);
        glib::idle_add_local_once(move || {
            if let Some(ui) = weak.upgrade() {
                show_barcodes(&ui);
            }
        });
    });

    // Wiring.
    if let Some((prev, next, _)) = pager_parts {
        let u = ui.clone();
        prev.connect_clicked(move |_| go(&u, -1));
        let u = ui.clone();
        next.connect_clicked(move |_| go(&u, 1));
        go(&ui, 0);
    }
    let u = ui.clone();
    printer.on_change(move |p| set_printer(&u, p));
    let u = ui.clone();
    paper.on_change(move |id| {
        if id == "__other" {
            u.all_sizes.set(true);
            fill_paper(&u);
            u.paper.widget.emit_by_name::<()>("activate", &[]);
        } else {
            u.job.borrow_mut().size = id;
            u.view.queue_draw();
        }
    });
    let u = ui.clone();
    fit.connect_toggled(move |w| {
        u.job.borrow_mut().scale = if w.is_active() { "fit" } else { "fill" }.into();
        u.view.queue_draw();
    });
    let u = ui.clone();
    spin.connect_value_changed(move |w| u.job.borrow_mut().copies = w.value_as_int() as u32);
    let u = ui.clone();
    more_btn.connect_toggled(move |w| show_more(&u, w.is_active()));
    let u = ui.clone();
    rotate.on_change(move |v| {
        u.job.borrow_mut().rotate = v;
        u.view.queue_draw();
    });
    let u = ui.clone();
    margin.on_change(move |v| {
        u.job.borrow_mut().margin_mode = v;
        u.view.queue_draw();
    });
    let u = ui.clone();
    bw.on_change(move |v| {
        u.job.borrow_mut().bw = v;
        u.view.queue_draw();
    });
    let w = window.clone();
    cancel.connect_clicked(move |_| w.close());
    let u = ui.clone();
    print_btn.connect_clicked(move |_| do_print(&u));
    let u2 = ui.clone();
    fix_btn.connect_clicked(move |_| find_printer(&u2));
    let u2 = ui.clone();
    again_btn.connect_clicked(move |_| check_printer(&u2));

    let keys = gtk::EventControllerKey::new();
    let u2 = ui.clone();
    keys.connect_key_pressed(move |_, key, _, _| on_key(&u2, key));
    window.add_controller(keys);

    if more {
        ui.more.set_active(true);
    }
    check_printer(&ui);
    ui
}

fn do_print(ui: &Rc<Ui>) {
    let ok = {
        let job = ui.job.borrow();
        !job.printer.is_empty() && job.print()
    };
    if ok {
        ui.window.close();
    }
}

fn show_barcodes(ui: &Ui) {
    let job = ui.job.borrow();
    match job.barcodes(job.index) {
        None => ui.badge.set_visible(false),
        Some((ok, n)) => {
            let word = if n == 1 { "barcode" } else { "barcodes" };
            if ok == n {
                ui.badge.set_markup(&format!("<span foreground='#7fd88f'>✓</span> {n} {word} checked: will scan"));
                ui.badge.set_tooltip_text(Some("Read back from the exact dots that will print."));
            } else {
                ui.badge.set_markup(&format!("<span foreground='#ffb454'>⚠</span> {} of {n} {word} may not scan", n - ok));
                ui.badge.set_tooltip_text(Some(
                    "The image is too low-resolution for this label size.\nPrint from the original PDF label if you have it.",
                ));
            }
            ui.badge.set_visible(true);
        }
    }
}

fn go(ui: &Ui, step: i32) {
    let n = {
        let mut job = ui.job.borrow_mut();
        let n = job.pages.len() as i32;
        job.index = (job.index as i32 + step).clamp(0, n - 1) as usize;
        n
    };
    if let Some(l) = &ui.page_label {
        l.set_text(&format!("Page {} of {n}", ui.job.borrow().index + 1));
    }
    ui.view.queue_draw();
}

fn fill_paper(ui: &Ui) {
    let job = ui.job.borrow();
    let mut common: Vec<String> =
        cups::COMMON.iter().filter(|n| job.sizes.iter().any(|(s, _)| s == *n)).map(|s| s.to_string()).collect();
    if !common.contains(&job.size) {
        common.push(job.size.clone());
    }
    let all: Vec<String> = job.sizes.iter().map(|(n, _)| n.clone()).collect();
    let shown = if ui.all_sizes.get() || all.len() <= 12 { all.clone() } else { common };
    let dims = |n: &str| job.sizes.iter().find(|(s, _)| s == n).map(|(_, d)| *d).unwrap_or((595.0, 842.0));
    let mut items: Vec<(String, String)> = shown.iter().map(|n| (n.clone(), cups::size_label(n, dims(n)))).collect();
    if shown.len() < all.len() {
        items.push(("__other".into(), "Other sizes…".into()));
    }
    let size = job.size.clone();
    drop(job);
    ui.paper.fill(&items, &size);
}

fn set_printer(ui: &Rc<Ui>, p: String) {
    {
        let mut job = ui.job.borrow_mut();
        job.printer = p;
        job.load_printer();
    }
    ui.print_btn.set_sensitive(true);
    ui.all_sizes.set(false);
    fill_paper(ui);
    let bw = ui.job.borrow().bw.clone();
    ui.bw.quiet.set(true);
    ui.bw.select(&bw);
    ui.bw.quiet.set(false);
    ui.warn.set_visible(false);
    check_printer(ui);
    ui.view.queue_draw();
}

// --- printer status ---------------------------------------------------------

fn check_printer(ui: &Rc<Ui>) {
    let printer = ui.job.borrow().printer.clone();
    if printer.is_empty() {
        return show_warning(ui, Some(false), "No printer is set up.");
    }
    let ui = ui.clone();
    glib::MainContext::default().spawn_local(async move {
        let p = printer.clone();
        let Ok((ok, msg)) = gio::spawn_blocking(move || cups::printer_status(&p)).await else { return };
        if ui.job.borrow().printer == printer {
            show_warning(&ui, ok, &msg);
        }
    });
}

fn hook_ready() -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(cups::fix_hook()).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

fn on_path(cmd: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(cmd).is_file()))
}

fn show_warning(ui: &Ui, ok: Option<bool>, msg: &str) {
    if ok == Some(false) {
        ui.warn_text.set_text(&format!("{msg} It may be off, asleep, or on a new network address."));
        let hook = hook_ready();
        ui.fix_btn.set_label(if hook { "Find printer" } else { "Printer settings" });
        ui.fix_btn.set_visible(hook || on_path("system-config-printer"));
        ui.warn.set_visible(true);
    } else {
        ui.warn.set_visible(false);
    }
}

/// Your own fixer (it may ask for sudo, so in a terminal), else the printer settings app.
fn find_printer(ui: &Rc<Ui>) {
    let printer = ui.job.borrow().printer.clone();
    let mut cmd = if hook_ready() {
        ui.warn_text.set_text("Looking for the printer in the terminal window…");
        let mut c = std::process::Command::new("xdg-terminal-exec");
        c.args(["bash", "-c", r#""$0" "$1"; echo; read -n1 -rsp "Press any key to close…""#]);
        c.arg(cups::fix_hook()).arg(printer);
        c
    } else {
        std::process::Command::new("system-config-printer")
    };
    let ui = ui.clone();
    glib::MainContext::default().spawn_local(async move {
        let _ = gio::spawn_blocking(move || cmd.status()).await;
        check_printer(&ui);
    });
}

// --- sidebar and keys ---------------------------------------------------------

fn show_more(ui: &Rc<Ui>, on: bool) {
    if !ui.window.is_realized() {
        return ui.side.set_visible(on);
    }
    let (w, h) = (ui.window.width(), ui.window.height());
    let target = if on {
        ui.base_w.set(w);
        w + ui.side.measure(gtk::Orientation::Horizontal, -1).1.max(SIDE)
    } else {
        ui.base_w.get()
    };
    ui.side.set_visible(on);
    // Resize only after GTK has laid out the new contents; shrinking first
    // makes Hyprland squash the old, wider frame into the narrow window.
    glib::timeout_add_local_once(Duration::from_millis(80), move || hypr_resize(target, h));
}

/// Hyprland lets a window grow by itself but not shrink, so ask it directly.
fn hypr_resize(width: i32, height: i32) {
    let win = format!("pid:{}", std::process::id());
    for cmd in [
        format!(r#"hl.dsp.window.resize({{ window = "{win}", x = {width}, y = {height} }})"#), // Lua config
        format!("resizewindowpixel exact {width} {height},{win}"),                            // older Hyprland
    ] {
        if std::process::Command::new("hyprctl").args(["dispatch", &cmd]).output().is_ok_and(|o| o.status.success()) {
            return;
        }
    }
}

fn on_key(ui: &Rc<Ui>, key: gdk::Key) -> glib::Propagation {
    use gdk::Key;
    match key {
        Key::Escape => ui.window.close(),
        Key::Return | Key::KP_Enter => do_print(ui),
        Key::r | Key::R => {
            let order = ["auto", "90", "180", "270", "0"];
            let now = ui.rotate.active().unwrap_or_default();
            let i = order.iter().position(|o| *o == now).unwrap_or(0);
            ui.rotate.select(order[(i + 1) % order.len()]);
        }
        Key::f | Key::F => ui.fit.set_active(!ui.fit.is_active()),
        Key::o | Key::O => ui.more.set_active(!ui.more.is_active()),
        Key::plus | Key::equal | Key::KP_Add => ui.spin.spin(gtk::SpinType::StepForward, 1.0),
        Key::minus | Key::KP_Subtract => ui.spin.spin(gtk::SpinType::StepBackward, 1.0),
        Key::Page_Down => go(ui, 1),
        Key::Page_Up => go(ui, -1),
        _ => return glib::Propagation::Proceed,
    }
    glib::Propagation::Stop
}
