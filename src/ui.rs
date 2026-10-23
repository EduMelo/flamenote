//! Caixa de ferramentas e bilhetes flutuantes.

use gdk::EventMask;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const SHADOW: f64 = 16.0;
const CARD_PAD: f64 = 16.0;
const TOOLBOX: i32 = 96;

pub struct Toolbox {
    window: gtk::Window,
    enabled: Rc<Cell<bool>>,
    area: gtk::EventBox,
}

pub fn install_style() {
    let provider = gtk::CssProvider::new();
    let css = br#"
        window, box, eventbox, label, drawingarea {
            background-color: transparent;
            border: none;
            box-shadow: none;
        }
        scrolledwindow, viewport {
            background-color: #ffffff;
            border: none;
            box-shadow: none;
        }
        label {
            color: #1e1e2e;
            font-family: Inter, "DejaVu Sans", sans-serif;
            font-size: 14px;
        }
    "#;
    if provider.load_from_data(css).is_err() {
        return;
    }
    if let Some(screen) = gdk::Screen::default() {
        gtk::StyleContext::add_provider_for_screen(
            &screen,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_USER,
        );
    }
}

pub fn build_toolbox(on_activate: Rc<dyn Fn()>) -> Toolbox {
    let window = gtk::Window::new(gtk::WindowType::Popup);
    prepare_floating(&window, false, 18);
    window.set_title("flamenote");
    window.set_default_size(TOOLBOX, TOOLBOX);
    window.set_resizable(false);
    window.add_events(
        EventMask::BUTTON_PRESS_MASK
            | EventMask::POINTER_MOTION_MASK
            | EventMask::LEAVE_NOTIFY_MASK,
    );

    let enabled = Rc::new(Cell::new(false));
    let hover = Rc::new(Cell::new(false));
    let area = gtk::EventBox::new();
    area.set_size_request(TOOLBOX, TOOLBOX);
    area.set_visible_window(true);
    area.set_app_paintable(true);
    use_rgba(&area);
    area.add_events(
        EventMask::BUTTON_PRESS_MASK
            | EventMask::POINTER_MOTION_MASK
            | EventMask::LEAVE_NOTIFY_MASK,
    );
    area.connect_realize(|widget| {
        if let Some(gdk_win) = widget.window() {
            punch_transparency(&gdk_win);
            select_pointer_events(&gdk_win);
        }
    });
    area.set_tooltip_text(Some("Nenhum texto selecionado na janela em foco"));

    let enabled_draw = enabled.clone();
    let hover_draw = hover.clone();
    area.connect_draw(move |widget, cr| {
        let alloc = widget.allocation();
        paint_toolbox(
            cr,
            alloc.width() as f64,
            alloc.height() as f64,
            enabled_draw.get(),
            hover_draw.get(),
        );
        glib::Propagation::Stop
    });

    let enabled_motion = enabled.clone();
    let hover_motion = hover.clone();
    area.connect_motion_notify_event(move |widget, event| {
        let (x, y) = event.position();
        let alloc = widget.allocation();
        let inside = point_in_button(x, y, alloc.width() as f64, alloc.height() as f64);
        let next = inside && enabled_motion.get();
        if next != hover_motion.get() {
            hover_motion.set(next);
            widget.queue_draw();
            let cursor = if next {
                gdk::CursorType::Hand1
            } else {
                gdk::CursorType::Arrow
            };
            if let (Some(win), Some(display)) = (widget.window(), gdk::Display::default()) {
                if let Some(cursor) = gdk::Cursor::for_display(&display, cursor) {
                    win.set_cursor(Some(&cursor));
                }
            }
        }
        glib::Propagation::Proceed
    });

    let hover_leave = hover.clone();
    area.connect_leave_notify_event(move |widget, _| {
        if hover_leave.get() {
            hover_leave.set(false);
            widget.queue_draw();
        }
        glib::Propagation::Proceed
    });

    let enabled_click = enabled.clone();
    area.connect_button_press_event(move |widget, event| {
        if event.button() != 1 || event.event_type() != gdk::EventType::ButtonPress {
            return glib::Propagation::Proceed;
        }
        let (x, y) = event.position();
        let alloc = widget.allocation();
        if enabled_click.get() && point_in_button(x, y, alloc.width() as f64, alloc.height() as f64)
        {
            on_activate();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });

    window.add(&area);
    Toolbox {
        window,
        enabled,
        area,
    }
}

impl Toolbox {
    pub fn set_enabled(&self, enabled: bool) {
        if self.enabled.get() == enabled {
            return;
        }
        self.enabled.set(enabled);
        self.area.set_tooltip_text(Some(if enabled {
            "Capturar o texto selecionado"
        } else {
            "Nenhum texto selecionado na janela em foco"
        }));
        self.area.queue_draw();
    }

    pub fn show_near(&self, anchor_x: i32, anchor_y: i32) {
        let (x, y) = place_near(anchor_x, anchor_y, TOOLBOX, TOOLBOX);
        self.window.move_(x, y);
        self.window.show_all();
        if let Some(win) = self.window.window() {
            select_pointer_events(&win);
            win.raise();
        }
    }

    pub fn hide(&self) {
        self.window.hide();
    }
}

pub fn spawn_note(text: &str, seq: i32) -> gtk::Window {
    let text = truncate_note(text);
    let window = gtk::Window::new(gtk::WindowType::Popup);
    prepare_floating(&window, false, 16);
    window.set_title("flamenote");

    let label = gtk::Label::new(Some(&text));
    label.set_line_wrap(true);
    label.set_line_wrap_mode(pango::WrapMode::WordChar);
    label.set_xalign(0.0);
    label.set_yalign(0.0);
    label.set_selectable(false);
    label.set_can_focus(false);
    label.style_context().add_class("flamenote-text");

    let longest = text
        .lines()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(1);
    let content_w = ((longest as i32) * 8 + 24).clamp(180, 420);
    label.set_size_request(content_w, -1);
    let natural_h = label.preferred_height_for_width(content_w).1;
    let text_h = natural_h.clamp(22, 300);
    let scroll = natural_h > 300;

    let sw = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    sw.set_policy(
        gtk::PolicyType::Never,
        if scroll {
            gtk::PolicyType::Automatic
        } else {
            gtk::PolicyType::Never
        },
    );
    sw.set_shadow_type(gtk::ShadowType::None);
    sw.set_size_request(content_w, text_h);
    sw.set_can_focus(false);
    sw.add(&label);

    let margin = (SHADOW + CARD_PAD) as i32;
    let inner = gtk::Box::new(gtk::Orientation::Vertical, 0);
    inner.set_margin_top(margin);
    inner.set_margin_bottom(margin);
    inner.set_margin_start(margin);
    inner.set_margin_end(margin);
    inner.add(&sw);

    let events = gtk::EventBox::new();
    events.set_above_child(true);
    events.set_visible_window(true);
    events.set_app_paintable(true);
    use_rgba(&events);
    events.connect_realize(|widget| {
        if let Some(gdk_win) = widget.window() {
            punch_transparency(&gdk_win);
            gdk_win.set_events(
                gdk_win.events()
                    | EventMask::BUTTON_PRESS_MASK
                    | EventMask::BUTTON_RELEASE_MASK
                    | EventMask::POINTER_MOTION_MASK
                    | EventMask::SCROLL_MASK,
            );
        }
    });
    events.add_events(
        EventMask::BUTTON_PRESS_MASK
            | EventMask::BUTTON_RELEASE_MASK
            | EventMask::POINTER_MOTION_MASK
            | EventMask::SCROLL_MASK,
    );
    events.add(&inner);
    window.add(&events);
    events.connect_draw(|widget, cr| {
        let alloc = widget.allocation();
        paint_note(cr, alloc.width() as f64, alloc.height() as f64);
        if let Some(child) = widget.child() {
            widget.propagate_draw(&child, cr);
        }
        glib::Propagation::Stop
    });

    window.connect_draw(|widget, cr| {
        let alloc = widget.allocation();
        paint_note(cr, alloc.width() as f64, alloc.height() as f64);
        if let Some(child) = widget.child() {
            widget.propagate_draw(&child, cr);
        }
        glib::Propagation::Stop
    });

    let drag = Rc::new(RefCell::new(Drag::default()));
    let drag_press = drag.clone();
    let win_press = window.clone();
    events.connect_button_press_event(move |_, event| {
        if event.button() != 1 {
            return glib::Propagation::Proceed;
        }
        if event.event_type() == gdk::EventType::DoubleButtonPress {
            win_press.close();
            return glib::Propagation::Stop;
        }
        if event.event_type() != gdk::EventType::ButtonPress {
            return glib::Propagation::Proceed;
        }
        let (root_x, root_y) = event.root();
        let (win_x, win_y) = win_press.position();
        *drag_press.borrow_mut() = Drag {
            armed: true,
            moved: false,
            press_x: root_x,
            press_y: root_y,
            origin_x: win_x,
            origin_y: win_y,
        };
        if let Some(gdk_win) = win_press.window() {
            gdk_win.raise();
        }
        glib::Propagation::Stop
    });

    let drag_motion = drag.clone();
    let win_motion = window.clone();
    events.connect_motion_notify_event(move |_, event| {
        let mut drag = drag_motion.borrow_mut();
        if !drag.armed {
            return glib::Propagation::Proceed;
        }
        let (root_x, root_y) = event.root();
        let dx = root_x - drag.press_x;
        let dy = root_y - drag.press_y;
        if dx * dx + dy * dy > 9.0 {
            drag.moved = true;
        }
        if drag.moved {
            win_motion.move_(drag.origin_x + dx as i32, drag.origin_y + dy as i32);
        }
        glib::Propagation::Stop
    });

    let drag_release = drag.clone();
    events.connect_button_release_event(move |_, event| {
        if event.button() == 1 {
            drag_release.borrow_mut().armed = false;
        }
        glib::Propagation::Proceed
    });

    let sw_scroll = sw.clone();
    events.connect_scroll_event(move |_, event| {
        let adj = sw_scroll.vadjustment();
        let delta = match event.direction() {
            gdk::ScrollDirection::Up => -48.0,
            gdk::ScrollDirection::Down => 48.0,
            gdk::ScrollDirection::Smooth => event.delta().1 * 48.0,
            _ => 0.0,
        };
        if delta == 0.0 {
            return glib::Propagation::Proceed;
        }
        let max = (adj.upper() - adj.page_size()).max(adj.lower());
        adj.set_value((adj.value() + delta).clamp(adj.lower(), max));
        glib::Propagation::Stop
    });

    let ww = content_w + margin * 2;
    let wh = text_h + margin * 2;
    window.set_default_size(ww, wh);
    let (x, y) = place_note(ww, wh, seq);
    window.move_(x, y);
    window.show_all();
    if let Some(gdk_win) = window.window() {
        select_pointer_events(&gdk_win);
        gdk_win.raise();
    }
    if let Some(gdk_win) = events.window() {
        select_pointer_events(&gdk_win);
    }
    window
}

#[derive(Default)]
struct Drag {
    armed: bool,
    moved: bool,
    press_x: f64,
    press_y: f64,
    origin_x: i32,
    origin_y: i32,
}

fn use_rgba(widget: &impl gtk::prelude::WidgetExt) {
    let screen = widget.screen().or_else(gdk::Screen::default);
    if let Some(screen) = screen {
        if let Some(visual) = screen.rgba_visual() {
            widget.set_visual(Some(&visual));
        }
    }
}

fn select_pointer_events(window: &gdk::Window) {
    window.set_events(
        window.events()
            | EventMask::BUTTON_PRESS_MASK
            | EventMask::BUTTON_RELEASE_MASK
            | EventMask::POINTER_MOTION_MASK
            | EventMask::LEAVE_NOTIFY_MASK
            | EventMask::SCROLL_MASK,
    );
}

fn punch_transparency(window: &gdk::Window) {
    window.set_background_pattern(None);
    let empty = cairo::Region::create();
    window.set_opaque_region(Some(&empty));
}

/// Sem compositor, alfa vira preto. A máscara recorta o cartão arredondado.
fn apply_rounded_shape(window: &gtk::Window, inset: i32, radius: i32) {
    let Some(gdk_win) = window.window() else {
        return;
    };
    let composited = gtk::prelude::WidgetExt::screen(window)
        .map(|screen| screen.is_composited())
        .unwrap_or(false);
    if composited {
        gdk_win.shape_combine_region(None, 0, 0);
        return;
    }
    let alloc = window.allocation();
    let region = rounded_region(
        inset,
        inset,
        alloc.width() - inset * 2,
        alloc.height() - inset * 2,
        radius,
    );
    gdk_win.shape_combine_region(Some(&region), 0, 0);
}

fn rounded_region(x: i32, y: i32, width: i32, height: i32, radius: i32) -> cairo::Region {
    let region = cairo::Region::create();
    if width <= 1 || height <= 1 {
        return region;
    }
    let radius = radius.max(0).min(width / 2).min(height / 2);
    for row in 0..height {
        let inset = rounded_inset(row, height, radius);
        let span = width - inset * 2;
        if span <= 0 {
            continue;
        }
        let _ = region.union_rectangle(&cairo::RectangleInt::new(x + inset, y + row, span, 1));
    }
    region
}

fn rounded_inset(row: i32, height: i32, radius: i32) -> i32 {
    if radius <= 0 {
        return 0;
    }
    let dist = if row < radius {
        radius as f64 - (row as f64 + 0.5)
    } else if row >= height - radius {
        (row as f64 + 0.5) - (height - radius) as f64
    } else {
        return 0;
    };
    if dist <= 0.0 {
        return 0;
    }
    let remain = radius as f64 * radius as f64 - dist * dist;
    if remain <= 0.0 {
        radius
    } else {
        (radius as f64 - remain.sqrt()).ceil() as i32
    }
}

fn prepare_floating(window: &gtk::Window, accept_focus: bool, corner: i32) {
    window.set_decorated(false);
    window.set_skip_taskbar_hint(true);
    window.set_skip_pager_hint(true);
    window.set_keep_above(true);
    window.set_accept_focus(accept_focus);
    window.set_focus_on_map(false);
    window.set_app_paintable(true);
    window.set_type_hint(gdk::WindowTypeHint::PopupMenu);
    if let Some(screen) = gtk::prelude::WidgetExt::screen(window).or_else(gdk::Screen::default) {
        if let Some(visual) = screen.rgba_visual() {
            window.set_visual(Some(&visual));
        }
        let shaped = window.clone();
        screen.connect_composited_changed(move |_| {
            apply_rounded_shape(&shaped, SHADOW as i32, corner);
        });
    }
    window.connect_realize(move |widget| {
        if let Some(gdk_win) = widget.window() {
            gdk_win.set_override_redirect(true);
            gdk_win.set_accept_focus(false);
            punch_transparency(&gdk_win);
            select_pointer_events(&gdk_win);
        }
        apply_rounded_shape(widget, SHADOW as i32, corner);
    });
    window.connect_size_allocate(move |widget, _| {
        apply_rounded_shape(widget, SHADOW as i32, corner);
    });
    window.connect_screen_changed(|widget, _| {
        if let Some(screen) = gtk::prelude::WidgetExt::screen(widget) {
            if let Some(visual) = screen.rgba_visual() {
                widget.set_visual(Some(&visual));
            }
        }
    });
}

fn truncate_note(text: &str) -> String {
    const MAX: usize = 20_000;
    let count = text.chars().count();
    if count <= MAX {
        return text.to_string();
    }
    let mut out: String = text.chars().take(MAX).collect();
    out.push_str("\n…");
    out
}

fn place_near(anchor_x: i32, anchor_y: i32, width: i32, height: i32) -> (i32, i32) {
    let (mx, my, mw, mh) = monitor_at(anchor_x, anchor_y);
    let gap = 14;
    let mut x = anchor_x + gap;
    let mut y = anchor_y + gap;
    if x + width > mx + mw - 8 {
        x = anchor_x - width - gap;
    }
    if y + height > my + mh - 8 {
        y = anchor_y - height - gap;
    }
    (
        clamp(x, mx + 8, mx + mw - width - 8),
        clamp(y, my + 8, my + mh - height - 8),
    )
}

fn place_note(width: i32, height: i32, seq: i32) -> (i32, i32) {
    let (px, py) = pointer();
    let (mx, my, mw, mh) = monitor_at(px, py);
    let stagger = (seq % 5) * 16;
    let x = px - width / 2 + stagger;
    let mut y = py - height - 8;
    if y < my + 8 {
        y = py + 28;
    }
    (
        clamp(x, mx + 8, mx + mw - width - 8),
        clamp(y, my + 8, my + mh - height - 8),
    )
}

fn pointer() -> (i32, i32) {
    let Some(display) = gdk::Display::default() else {
        return (0, 0);
    };
    let Some(seat) = display.default_seat() else {
        return (0, 0);
    };
    let Some(pointer) = seat.pointer() else {
        return (0, 0);
    };
    let (_, x, y) = pointer.position();
    (x, y)
}

fn monitor_at(x: i32, y: i32) -> (i32, i32, i32, i32) {
    let Some(display) = gdk::Display::default() else {
        return (0, 0, 1920, 1080);
    };
    let Some(monitor) = display.monitor_at_point(x, y) else {
        let width = display
            .primary_monitor()
            .map(|monitor| monitor.geometry().width())
            .unwrap_or(1920);
        return (0, 0, width, 1080);
    };
    let g = monitor.geometry();
    (g.x(), g.y(), g.width(), g.height())
}

fn clamp(value: i32, min: i32, max: i32) -> i32 {
    if max < min {
        min
    } else {
        value.clamp(min, max)
    }
}

fn button_rect(w: f64, h: f64) -> (f64, f64, f64, f64) {
    let pad = 12.0;
    let side = (w - SHADOW * 2.0 - pad * 2.0).min(h - SHADOW * 2.0 - pad * 2.0);
    (SHADOW + pad, SHADOW + pad, side, side)
}

fn point_in_button(x: f64, y: f64, w: f64, h: f64) -> bool {
    let (bx, by, bs, _) = button_rect(w, h);
    x >= bx && y >= by && x <= bx + bs && y <= by + bs
}

fn paint_toolbox(cr: &cairo::Context, w: f64, h: f64, enabled: bool, hover: bool) {
    cr.set_operator(cairo::Operator::Source);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
    let _ = cr.paint();
    cr.set_operator(cairo::Operator::Over);

    for i in 0..8 {
        let inset = i as f64;
        rounded(
            cr,
            inset,
            inset + 2.0,
            w - inset * 2.0,
            h - inset * 2.0,
            20.0,
        );
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.035);
        let _ = cr.fill();
    }

    rounded(cr, SHADOW, SHADOW, w - SHADOW * 2.0, h - SHADOW * 2.0, 18.0);
    cr.set_source_rgba(0.192, 0.196, 0.267, 0.97);
    let _ = cr.fill_preserve();
    cr.set_source_rgba(0.345, 0.357, 0.439, 1.0);
    cr.set_line_width(1.0);
    let _ = cr.stroke();

    let (bx, by, bs, _) = button_rect(w, h);
    rounded(cr, bx, by, bs, bs, 12.0);
    if enabled && hover {
        cr.set_source_rgba(0.271, 0.278, 0.353, 1.0);
    } else if enabled {
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.03);
    } else {
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.12);
    }
    let _ = cr.fill();
    paint_tool_icon(cr, bx, by, bs, enabled);
}

fn paint_tool_icon(cr: &cairo::Context, x: f64, y: f64, side: f64, enabled: bool) {
    let paper = if enabled {
        (0.980, 0.702, 0.529)
    } else {
        (0.345, 0.357, 0.439)
    };
    let ink = if enabled {
        (0.118, 0.118, 0.180)
    } else {
        (0.192, 0.196, 0.267)
    };
    let px = x + side * 0.24;
    let py = y + side * 0.20;
    let pw = side * 0.52;
    let ph = side * 0.60;
    rounded(cr, px, py, pw, ph, 4.0);
    cr.set_source_rgb(paper.0, paper.1, paper.2);
    let _ = cr.fill();

    cr.set_source_rgb(ink.0, ink.1, ink.2);
    let line_h = (side * 0.045).max(1.5);
    for (index, frac) in [0.34_f64, 0.50, 0.66].into_iter().enumerate() {
        let ly = py + ph * frac;
        let lw = if index == 1 { pw * 0.72 } else { pw * 0.58 };
        cr.rectangle(px + pw * 0.14, ly, lw, line_h);
        let _ = cr.fill();
    }
}

fn paint_note(cr: &cairo::Context, w: f64, h: f64) {
    cr.set_operator(cairo::Operator::Source);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
    let _ = cr.paint();
    cr.set_operator(cairo::Operator::Over);

    for i in 0..10 {
        let inset = i as f64 * 1.15;
        rounded(
            cr,
            inset,
            inset + 3.0,
            (w - inset * 2.0).max(1.0),
            (h - inset * 2.0).max(1.0),
            22.0,
        );
        cr.set_source_rgba(0.05, 0.05, 0.08, 0.028);
        let _ = cr.fill();
    }

    let card_x = SHADOW;
    let card_y = SHADOW;
    let card_w = (w - SHADOW * 2.0).max(1.0);
    let card_h = (h - SHADOW * 2.0).max(1.0);
    rounded(cr, card_x, card_y, card_w, card_h, 16.0);
    cr.set_source_rgb(1.0, 1.0, 1.0);
    let _ = cr.fill_preserve();
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.08);
    cr.set_line_width(1.0);
    let _ = cr.stroke();
}

fn rounded(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    const PI: f64 = std::f64::consts::PI;
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
    cr.arc(x + r, y + r, r, PI, PI * 1.5);
    cr.close_path();
}
