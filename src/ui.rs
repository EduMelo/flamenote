//! Caixa de ferramentas, bilhetes flutuantes e a janela de configuração.

use crate::config::{self, CaptureMods, Hotkey, LlmConfig, Query, QueryDestination, MAX_QUERIES};
use gdk::EventMask;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const SHADOW: f64 = 16.0;
const CARD_PAD: f64 = 16.0;
const TOOLBOX_H: i32 = 96;
const BUTTON_SIDE: f64 = 40.0;
const BUTTON_GAP: f64 = 10.0;
const BUTTON_PAD: f64 = 12.0;

const TOOLTIP_DISABLED: &str = "Nenhum texto selecionado na janela em foco";
const TOOLTIP_ENABLED: &str = "Usar o texto selecionado";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolIcon {
    Note,
    File,
    Llm,
}

#[derive(Clone)]
pub struct ToolAction {
    pub icon: ToolIcon,
    pub tooltip: String,
    pub run: Rc<dyn Fn()>,
}

impl ToolAction {
    pub fn new(icon: ToolIcon, tooltip: impl Into<String>, run: impl Fn() + 'static) -> Self {
        Self {
            icon,
            tooltip: tooltip.into(),
            run: Rc::new(run),
        }
    }
}

pub struct Toolbox {
    window: gtk::Window,
    enabled: Rc<Cell<bool>>,
    area: gtk::EventBox,
    width: i32,
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
        window.flamenote-settings,
        window.flamenote-settings box {
            background-color: #f7f5f8;
        }
        window.flamenote-settings label.title {
            font-size: 18px;
            font-weight: 600;
        }
        window.flamenote-settings label.section {
            color: #6c7086;
            font-size: 13px;
        }
        window.flamenote-settings entry.capture {
            background-image: none;
            background-color: #ffffff;
            color: #1e1e2e;
            border: 1px solid #d5d3dc;
            border-radius: 8px;
            padding: 8px 10px;
            font-size: 15px;
            caret-color: transparent;
        }
        window.flamenote-settings entry.capture.listening {
            border-color: #fab387;
            background-color: #fff8f3;
        }
        window.flamenote-settings entry.path {
            background-image: none;
            background-color: #ffffff;
            color: #1e1e2e;
            border: 1px solid #d5d3dc;
            border-radius: 8px;
            padding: 8px 10px;
            font-size: 15px;
        }
        window.flamenote-settings label.field {
            color: #6c7086;
            font-size: 13px;
        }
        window.flamenote-settings scrolledwindow.prompt {
            background-color: #ffffff;
            border: 1px solid #d5d3dc;
            border-radius: 8px;
        }
        window.flamenote-settings textview,
        window.flamenote-settings textview text {
            background-color: #ffffff;
            color: #1e1e2e;
            font-size: 14px;
        }
        window.flamenote-settings label.hint {
            color: #6c7086;
            font-size: 12px;
        }
        window.flamenote-settings label.hint.error {
            color: #c45c4a;
        }
        menu {
            background-color: #f7f5f8;
            color: #1e1e2e;
            border: 1px solid #d5d3dc;
        }
        menuitem:hover {
            background-color: #ebe7ee;
        }
        separator {
            background-color: #d5d3dc;
            min-height: 1px;
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

pub fn build_toolbox(actions: Vec<ToolAction>) -> Toolbox {
    let actions = Rc::new(actions);
    let count = actions.len().max(1);
    let width = toolbox_width(count);
    let window = gtk::Window::new(gtk::WindowType::Popup);
    prepare_floating(&window, false, 18);
    window.set_title("flamenote");
    window.set_default_size(width, TOOLBOX_H);
    window.set_resizable(false);
    window.add_events(
        EventMask::BUTTON_PRESS_MASK
            | EventMask::POINTER_MOTION_MASK
            | EventMask::LEAVE_NOTIFY_MASK,
    );

    let enabled = Rc::new(Cell::new(false));
    let hover = Rc::new(Cell::new(None::<usize>));
    let area = gtk::EventBox::new();
    area.set_size_request(width, TOOLBOX_H);
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
    area.set_tooltip_text(Some(TOOLTIP_DISABLED));

    let enabled_draw = enabled.clone();
    let hover_draw = hover.clone();
    let actions_draw = actions.clone();
    area.connect_draw(move |widget, cr| {
        let alloc = widget.allocation();
        paint_toolbox(
            cr,
            alloc.width() as f64,
            alloc.height() as f64,
            enabled_draw.get(),
            hover_draw.get(),
            &actions_draw,
        );
        glib::Propagation::Stop
    });

    let enabled_motion = enabled.clone();
    let hover_motion = hover.clone();
    let actions_motion = actions.clone();
    area.connect_motion_notify_event(move |widget, event| {
        let (x, y) = event.position();
        let alloc = widget.allocation();
        let inside = button_at(x, y, alloc.width() as f64, alloc.height() as f64, count);
        let next = if enabled_motion.get() { inside } else { None };
        if next != hover_motion.get() {
            hover_motion.set(next);
            widget.queue_draw();
            let tooltip = match next.and_then(|index| actions_motion.get(index)) {
                Some(action) => action.tooltip.clone(),
                None => TOOLTIP_ENABLED.to_string(),
            };
            widget.set_tooltip_text(Some(&tooltip));
            let cursor = if next.is_some() {
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
        if hover_leave.get().is_some() {
            hover_leave.set(None);
            widget.queue_draw();
        }
        widget.set_tooltip_text(Some(TOOLTIP_ENABLED));
        glib::Propagation::Proceed
    });

    let enabled_click = enabled.clone();
    let actions_click = actions.clone();
    area.connect_button_press_event(move |widget, event| {
        if event.button() != 1 || event.event_type() != gdk::EventType::ButtonPress {
            return glib::Propagation::Proceed;
        }
        if !enabled_click.get() {
            return glib::Propagation::Proceed;
        }
        let (x, y) = event.position();
        let alloc = widget.allocation();
        match button_at(x, y, alloc.width() as f64, alloc.height() as f64, count) {
            Some(index) => match actions_click.get(index) {
                Some(action) => {
                    (action.run)();
                    glib::Propagation::Stop
                }
                None => glib::Propagation::Proceed,
            },
            None => glib::Propagation::Proceed,
        }
    });

    window.add(&area);
    Toolbox {
        window,
        enabled,
        area,
        width,
    }
}

fn toolbox_width(count: usize) -> i32 {
    let count = count.max(1) as f64;
    (SHADOW * 2.0 + BUTTON_PAD * 2.0 + BUTTON_SIDE * count + BUTTON_GAP * (count - 1.0)).round()
        as i32
}

impl Toolbox {
    pub fn set_enabled(&self, enabled: bool) {
        if self.enabled.get() == enabled {
            return;
        }
        self.enabled.set(enabled);
        self.area.set_tooltip_text(Some(if enabled {
            TOOLTIP_ENABLED
        } else {
            TOOLTIP_DISABLED
        }));
        self.area.queue_draw();
    }

    pub fn show_near(&self, anchor_x: i32, anchor_y: i32) {
        let (x, y) = place_near(anchor_x, anchor_y, self.width, TOOLBOX_H);
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

const HINT_IDLE: &str = "Clique no campo e pressione as teclas.";
const HINT_LISTEN: &str = "Pressione o atalho. Esc cancela.";
const HINT_SAVED: &str = "Atalho salvo.";
const HINT_DIR_IDLE: &str = "Enter ou sair do campo salva a pasta.";
const HINT_DIR_SAVED: &str = "Pasta salva.";
const HINT_LLM_IDLE: &str = "As consultas usam /v1/chat/completions.";
const HINT_LLM_SAVED: &str = "Configuração LLM salva.";

pub struct Settings {
    window: gtk::Window,
    field: gtk::Entry,
    copies_field: gtk::Entry,
    llm_base: gtk::Entry,
    llm_model: gtk::Entry,
    llm_key: gtk::Entry,
    query_prompts: Vec<gtk::TextView>,
    query_files: Vec<gtk::CheckButton>,
    listening: Rc<Cell<bool>>,
    saved: Rc<RefCell<String>>,
    saved_copies: Rc<RefCell<String>>,
    saved_llm: Rc<RefCell<(LlmConfig, Vec<Query>)>>,
}

impl Settings {
    pub fn present(&self) {
        self.window.show_all();
        self.window.present();
    }

    pub fn window(&self) -> &gtk::Window {
        &self.window
    }

    pub fn set_display(&self, display: &str) {
        *self.saved.borrow_mut() = display.to_string();
        if !self.listening.get() {
            self.field.set_text(display);
            self.field.select_region(0, 0);
        }
    }

    pub fn set_copies_dir(&self, display: &str) {
        *self.saved_copies.borrow_mut() = display.to_string();
        if !self.copies_field.has_focus() {
            self.copies_field.set_text(display);
        }
    }

    pub fn set_llm(&self, llm: &LlmConfig, queries: &[Query]) {
        *self.saved_llm.borrow_mut() = (llm.clone(), queries.to_vec());
        let focused = self.llm_base.has_focus()
            || self.llm_model.has_focus()
            || self.llm_key.has_focus()
            || self.query_prompts.iter().any(|view| view.has_focus())
            || self.query_files.iter().any(|check| check.has_focus());
        if focused {
            return;
        }
        self.llm_base.set_text(&llm.base_url);
        self.llm_model.set_text(&llm.model);
        self.llm_key.set_text(&llm.api_key);
        for (index, view) in self.query_prompts.iter().enumerate() {
            let query = queries.get(index);
            if let Some(buffer) = view.buffer() {
                buffer.set_text(query.map(|query| query.prompt.as_str()).unwrap_or(""));
            }
            self.query_files[index].set_active(
                query
                    .map(|query| query.destination == QueryDestination::File)
                    .unwrap_or(false),
            );
        }
    }
}

pub struct SettingsInitial<'a> {
    pub hotkey: &'a str,
    pub copies_dir: &'a str,
    pub llm: &'a LlmConfig,
    pub queries: &'a [Query],
}

type ListenFn = Rc<dyn Fn(bool)>;
type SaveHotkeyFn = Rc<dyn Fn(&Hotkey) -> Result<(), String>>;
type SaveCopiesDirFn = Rc<dyn Fn(&str) -> Result<String, String>>;
type SaveLlmFn = Rc<dyn Fn(&LlmConfig, &[Query]) -> Result<(), String>>;

pub struct SettingsHandlers {
    pub on_listen: ListenFn,
    pub on_save_hotkey: SaveHotkeyFn,
    pub on_save_copies_dir: SaveCopiesDirFn,
    pub on_save_llm: SaveLlmFn,
}

pub fn build_settings(initial: SettingsInitial<'_>, handlers: SettingsHandlers) -> Settings {
    let SettingsInitial {
        hotkey: initial_hotkey,
        copies_dir: initial_copies_dir,
        llm: initial_llm,
        queries: initial_queries,
    } = initial;
    let SettingsHandlers {
        on_listen,
        on_save_hotkey,
        on_save_copies_dir,
        on_save_llm,
    } = handlers;
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("Configurações");
    window.set_resizable(false);
    window.set_position(gtk::WindowPosition::Center);
    window.set_type_hint(gdk::WindowTypeHint::Dialog);
    window.set_accept_focus(true);
    window.set_focus_on_map(true);
    window.set_default_size(520, 640);
    window.style_context().add_class("flamenote-settings");

    let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
    content.set_margin_top(16);
    content.set_margin_bottom(16);
    content.set_margin_start(16);
    content.set_margin_end(16);

    let title = gtk::Label::new(Some("Configurações"));
    title.set_xalign(0.0);
    title.style_context().add_class("title");

    let notebook = gtk::Notebook::new();
    notebook.set_show_border(false);

    let general = gtk::Box::new(gtk::Orientation::Vertical, 8);
    general.set_border_width(12);

    let section = gtk::Label::new(Some("Atalho"));
    section.set_xalign(0.0);
    section.set_margin_top(8);
    section.style_context().add_class("section");

    let field = gtk::Entry::new();
    field.set_text(initial_hotkey);
    field.set_editable(false);
    field.set_can_focus(true);
    field.set_has_frame(false);
    field.set_width_chars(22);
    field.set_placeholder_text(Some("Pressione as teclas…"));
    field.set_tooltip_text(Some("Clique e pressione o novo atalho"));
    field.style_context().add_class("capture");

    let hint = gtk::Label::new(Some(HINT_IDLE));
    hint.set_xalign(0.0);
    hint.set_line_wrap(true);
    hint.set_max_width_chars(52);
    hint.style_context().add_class("hint");

    let dir_section = gtk::Label::new(Some("Pasta de destino"));
    dir_section.set_xalign(0.0);
    dir_section.set_margin_top(8);
    dir_section.style_context().add_class("section");

    let copies_field = gtk::Entry::new();
    copies_field.set_text(initial_copies_dir);
    copies_field.set_editable(true);
    copies_field.set_width_chars(34);
    copies_field.set_placeholder_text(Some("~/copias"));
    copies_field.set_tooltip_text(Some("Pasta onde o texto selecionado é salvo como arquivo"));
    copies_field.style_context().add_class("path");

    let dir_hint = gtk::Label::new(Some(HINT_DIR_IDLE));
    dir_hint.set_xalign(0.0);
    dir_hint.set_line_wrap(true);
    dir_hint.set_max_width_chars(52);
    dir_hint.style_context().add_class("hint");

    general.add(&section);
    general.add(&field);
    general.add(&hint);
    general.add(&dir_section);
    general.add(&copies_field);
    general.add(&dir_hint);

    let llm_page = gtk::Box::new(gtk::Orientation::Vertical, 8);
    llm_page.set_border_width(12);

    let llm_section = gtk::Label::new(Some("Servidor LLM (compatível com OpenAI)"));
    llm_section.set_xalign(0.0);
    llm_section.set_margin_top(8);
    llm_section.style_context().add_class("section");

    let base_label = field_label("Base URL");
    let llm_base = gtk::Entry::new();
    llm_base.set_text(&initial_llm.base_url);
    llm_base.set_width_chars(40);
    llm_base.set_placeholder_text(Some("http://localhost:11434"));
    llm_base.style_context().add_class("path");

    let model_label = field_label("Modelo");
    let llm_model = gtk::Entry::new();
    llm_model.set_text(&initial_llm.model);
    llm_model.set_width_chars(40);
    llm_model.set_placeholder_text(Some("qwen3:14b"));
    llm_model.style_context().add_class("path");

    let key_label = field_label("Chave de API");
    let llm_key = gtk::Entry::new();
    llm_key.set_text(&initial_llm.api_key);
    llm_key.set_width_chars(40);
    llm_key.set_visibility(false);
    llm_key.set_input_purpose(gtk::InputPurpose::Password);
    llm_key.set_placeholder_text(Some("vazia em servidores locais"));
    llm_key.style_context().add_class("path");

    llm_page.add(&llm_section);
    llm_page.add(&base_label);
    llm_page.add(&llm_base);
    llm_page.add(&model_label);
    llm_page.add(&llm_model);
    llm_page.add(&key_label);
    llm_page.add(&llm_key);

    let query_section = gtk::Label::new(Some("Consultas de pré-prompt"));
    query_section.set_xalign(0.0);
    query_section.set_margin_top(12);
    query_section.style_context().add_class("section");
    llm_page.add(&query_section);

    let mut query_prompts = Vec::new();
    let mut query_files = Vec::new();
    for index in 0..MAX_QUERIES {
        let query = initial_queries.get(index);
        llm_page.add(&field_label(&format!("Consulta {}", index + 1)));

        let view = gtk::TextView::new();
        view.set_wrap_mode(gtk::WrapMode::WordChar);
        view.set_accepts_tab(false);
        view.set_left_margin(8);
        view.set_right_margin(8);
        view.set_top_margin(6);
        view.set_bottom_margin(6);
        if let Some(buffer) = view.buffer() {
            buffer.set_text(query.map(|query| query.prompt.as_str()).unwrap_or(""));
        }
        let scroll = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
        scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
        scroll.set_shadow_type(gtk::ShadowType::None);
        scroll.set_size_request(-1, 64);
        scroll.style_context().add_class("prompt");
        scroll.add(&view);
        llm_page.add(&scroll);

        let check = gtk::CheckButton::with_label("Salvar em arquivo");
        check.set_active(
            query
                .map(|query| query.destination == QueryDestination::File)
                .unwrap_or(false),
        );
        llm_page.add(&check);

        query_prompts.push(view);
        query_files.push(check);
    }

    let destination_hint = gtk::Label::new(Some(
        "Desmarcado: a resposta é copiada para a área de transferência.",
    ));
    destination_hint.set_xalign(0.0);
    destination_hint.set_line_wrap(true);
    destination_hint.set_max_width_chars(52);
    destination_hint.style_context().add_class("hint");
    llm_page.add(&destination_hint);

    let save_llm = gtk::Button::with_label("Salvar configuração LLM");
    let llm_hint = gtk::Label::new(Some(HINT_LLM_IDLE));
    llm_hint.set_xalign(0.0);
    llm_hint.set_line_wrap(true);
    llm_hint.set_max_width_chars(52);
    llm_hint.style_context().add_class("hint");
    llm_page.add(&save_llm);
    llm_page.add(&llm_hint);

    notebook.append_page(&general, Some(&gtk::Label::new(Some("Geral"))));
    notebook.append_page(&llm_page, Some(&gtk::Label::new(Some("LLM"))));
    content.add(&title);
    content.add(&notebook);
    window.add(&content);

    let listening = Rc::new(Cell::new(false));
    let saved = Rc::new(RefCell::new(initial_hotkey.to_string()));
    let saved_copies = Rc::new(RefCell::new(initial_copies_dir.to_string()));
    let saved_llm = Rc::new(RefCell::new((
        initial_llm.clone(),
        initial_queries.to_vec(),
    )));

    let listening_click = listening.clone();
    let field_click = field.clone();
    let hint_click = hint.clone();
    let on_listen_click = on_listen.clone();
    field.connect_button_press_event(move |_, event| {
        if event.button() != 1 {
            return glib::Propagation::Proceed;
        }
        if !listening_click.get() {
            listening_click.set(true);
            field_click.set_text("");
            field_click.style_context().add_class("listening");
            set_hint(&hint_click, HINT_LISTEN, false);
            on_listen_click(true);
        }
        field_click.grab_focus();
        glib::Propagation::Proceed
    });

    let cancel = {
        let listening = listening.clone();
        let saved = saved.clone();
        let field = field.clone();
        let hint = hint.clone();
        let on_listen = on_listen.clone();
        Rc::new(move || {
            if !listening.get() {
                return;
            }
            listening.set(false);
            field.set_text(&saved.borrow());
            field.select_region(0, 0);
            field.style_context().remove_class("listening");
            set_hint(&hint, HINT_IDLE, false);
            on_listen(false);
        }) as Rc<dyn Fn()>
    };

    let listening_key = listening.clone();
    let saved_key = saved.clone();
    let field_key = field.clone();
    let hint_key = hint.clone();
    let cancel_key = cancel.clone();
    field.connect_key_press_event(move |_, event| {
        if !listening_key.get() {
            return glib::Propagation::Proceed;
        }
        let key = event.keyval();
        let Some(name) = key.name() else {
            return glib::Propagation::Stop;
        };
        let name = name.to_string();
        if event.is_modifier() || config::is_modifier_key(&name) {
            return glib::Propagation::Stop;
        }
        if name == "Escape" {
            cancel_key();
            return glib::Propagation::Stop;
        }
        let state = event.state();
        let mods = CaptureMods {
            super_key: state.contains(gdk::ModifierType::SUPER_MASK)
                || state.contains(gdk::ModifierType::MOD4_MASK)
                || state.contains(gdk::ModifierType::META_MASK),
            ctrl: state.contains(gdk::ModifierType::CONTROL_MASK),
            alt: state.contains(gdk::ModifierType::MOD1_MASK),
            shift: state.contains(gdk::ModifierType::SHIFT_MASK),
            mod3: state.contains(gdk::ModifierType::MOD3_MASK),
            mod5: state.contains(gdk::ModifierType::MOD5_MASK),
        };
        match config::hotkey_from_capture(mods, &name) {
            Ok(hotkey) => match on_save_hotkey(&hotkey) {
                Ok(()) => {
                    listening_key.set(false);
                    *saved_key.borrow_mut() = hotkey.display.clone();
                    field_key.set_text(&hotkey.display);
                    field_key.select_region(0, 0);
                    field_key.style_context().remove_class("listening");
                    set_hint(&hint_key, HINT_SAVED, false);
                }
                Err(message) => set_hint(&hint_key, &message, true),
            },
            Err(err) => set_hint(&hint_key, &err.0, true),
        }
        glib::Propagation::Stop
    });

    let cancel_focus = cancel.clone();
    let field_focus = field.clone();
    field.connect_focus_out_event(move |_, _| {
        let cancel = cancel_focus.clone();
        let field = field_focus.clone();
        glib::idle_add_local(move || {
            if !field.has_focus() {
                cancel();
            }
            glib::ControlFlow::Break
        });
        glib::Propagation::Proceed
    });

    let commit_copies = {
        let copies_field = copies_field.clone();
        let dir_hint = dir_hint.clone();
        let saved_copies = saved_copies.clone();
        let on_save = on_save_copies_dir.clone();
        Rc::new(move || {
            let current = copies_field.text().to_string();
            let trimmed = current.trim();
            if trimmed == saved_copies.borrow().trim() {
                if current != *saved_copies.borrow() {
                    copies_field.set_text(&saved_copies.borrow());
                }
                return;
            }
            match on_save(trimmed) {
                Ok(display) => {
                    *saved_copies.borrow_mut() = display.clone();
                    copies_field.set_text(&display);
                    set_hint(&dir_hint, HINT_DIR_SAVED, false);
                }
                Err(message) => set_hint(&dir_hint, &message, true),
            }
        }) as Rc<dyn Fn()>
    };

    let commit_activate = commit_copies.clone();
    copies_field.connect_activate(move |_| {
        commit_activate();
    });

    let commit_focus = commit_copies.clone();
    copies_field.connect_focus_out_event(move |_, _| {
        let commit = commit_focus.clone();
        glib::idle_add_local(move || {
            commit();
            glib::ControlFlow::Break
        });
        glib::Propagation::Proceed
    });

    let save_llm_click = {
        let llm_base = llm_base.clone();
        let llm_model = llm_model.clone();
        let llm_key = llm_key.clone();
        let prompts = query_prompts.clone();
        let files = query_files.clone();
        let hint = llm_hint.clone();
        let saved = saved_llm.clone();
        let on_save = on_save_llm.clone();
        Rc::new(move || {
            let llm = LlmConfig {
                base_url: llm_base.text().trim().to_string(),
                model: llm_model.text().trim().to_string(),
                api_key: llm_key.text().trim().to_string(),
            };
            let mut queries = Vec::new();
            for (view, check) in prompts.iter().zip(files.iter()) {
                let Some(buffer) = view.buffer() else {
                    continue;
                };
                let start = buffer.start_iter();
                let end = buffer.end_iter();
                let prompt = buffer
                    .text(&start, &end, false)
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                if prompt.is_empty() {
                    continue;
                }
                queries.push(Query {
                    prompt,
                    destination: if check.is_active() {
                        QueryDestination::File
                    } else {
                        QueryDestination::Clipboard
                    },
                });
            }
            if !queries.is_empty() && (llm.base_url.is_empty() || llm.model.is_empty()) {
                set_hint(
                    &hint,
                    "Defina a base URL e o modelo para usar as consultas.",
                    true,
                );
                return;
            }
            match on_save(&llm, &queries) {
                Ok(()) => {
                    *saved.borrow_mut() = (llm, queries);
                    set_hint(&hint, HINT_LLM_SAVED, false);
                }
                Err(message) => set_hint(&hint, &message, true),
            }
        }) as Rc<dyn Fn()>
    };
    save_llm.connect_clicked(move |_| save_llm_click());

    let listening_destroy = listening.clone();
    window.connect_destroy(move |_| {
        if listening_destroy.get() {
            listening_destroy.set(false);
            on_listen(false);
        }
    });

    Settings {
        window,
        field,
        copies_field,
        llm_base,
        llm_model,
        llm_key,
        query_prompts,
        query_files,
        listening,
        saved,
        saved_copies,
        saved_llm,
    }
}

fn field_label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.style_context().add_class("field");
    label
}

#[cfg(test)]
mod tests {
    use super::*;
    use glib::translate::ToGlibPtr;

    #[test]
    fn toolbox_width_follows_button_count() {
        assert_eq!(toolbox_width(1), 96);
        assert_eq!(toolbox_width(2), 146);
        assert_eq!(toolbox_width(5), 296);
    }

    #[test]
    fn button_at_maps_each_column() {
        let width = toolbox_width(5) as f64;
        assert_eq!(button_at(30.0, 30.0, width, 96.0, 5), Some(0));
        assert_eq!(button_at(100.0, 30.0, width, 96.0, 5), Some(1));
        assert_eq!(button_at(70.0, 30.0, width, 96.0, 5), None);
        assert_eq!(button_at(260.0, 30.0, width, 96.0, 5), Some(4));
        assert_eq!(button_at(290.0, 30.0, width, 96.0, 5), None);
    }

    #[test]
    fn settings_window_captures_a_chord() {
        if std::env::var_os("DISPLAY").is_none() {
            return;
        }
        gtk::init().expect("gtk");
        // O harness usa uma thread por teste, e o GTK só inicializa numa.
        let root = gdk::Screen::default()
            .and_then(|screen| screen.root_window())
            .expect("raiz");
        let trigger = crate::menu_trigger(&root, 40, 70, 3, 123_456);
        assert_eq!(trigger.button(), Some(3));
        assert_eq!(trigger.time(), 123_456);
        assert_eq!(trigger.root_coords(), Some((40.0, 70.0)));
        assert!(trigger.device().is_some());
        assert!(trigger.window().is_some());
        install_style();
        let saved = Rc::new(RefCell::new(String::new()));
        let saved_cb = saved.clone();
        let saved_copies = Rc::new(RefCell::new(String::new()));
        let saved_copies_cb = saved_copies.clone();
        let llm = LlmConfig {
            base_url: "http://localhost:11434".into(),
            model: "qwen3:14b".into(),
            api_key: String::new(),
        };
        let queries = vec![Query {
            prompt: "Resuma:".into(),
            destination: QueryDestination::File,
        }];
        let settings = build_settings(
            SettingsInitial {
                hotkey: "Super+Shift+N",
                copies_dir: "~/copias",
                llm: &llm,
                queries: &queries,
            },
            SettingsHandlers {
                on_listen: Rc::new(|_| {}),
                on_save_hotkey: Rc::new(move |hotkey| {
                    *saved_cb.borrow_mut() = hotkey.display.clone();
                    Ok(())
                }),
                on_save_copies_dir: Rc::new(move |dir| {
                    let display = dir.to_string();
                    *saved_copies_cb.borrow_mut() = display.clone();
                    Ok(display)
                }),
                on_save_llm: Rc::new(|_, _| Ok(())),
            },
        );
        assert_eq!(settings.field.text(), "Super+Shift+N");
        assert_eq!(settings.copies_field.text(), "~/copias");
        assert_eq!(settings.llm_base.text(), "http://localhost:11434");
        assert_eq!(settings.llm_model.text(), "qwen3:14b");
        let buffer = settings.query_prompts[0].buffer().expect("buffer");
        assert_eq!(
            buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .unwrap_or_default(),
            "Resuma:"
        );
        assert!(settings.query_files[0].is_active());
        let window = settings.window.clone();
        window.show_all();
        let ctx = glib::MainContext::default();
        for _ in 0..30 {
            let _ = ctx.iteration(false);
        }
        let field = settings.field.clone();
        field.grab_focus();
        send_button(&field);
        for _ in 0..5 {
            let _ = ctx.iteration(false);
        }
        send_key(
            &field,
            "n",
            gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::MOD1_MASK,
        );
        for _ in 0..5 {
            let _ = ctx.iteration(false);
        }
        assert_eq!(saved.borrow().as_str(), "Ctrl+Alt+N");
        assert_eq!(settings.field.text(), "Ctrl+Alt+N");
        window.close();
        for _ in 0..5 {
            let _ = ctx.iteration(false);
        }
    }

    fn send_button(field: &gtk::Entry) {
        let event = gdk::Event::new(gdk::EventType::ButtonPress);
        unsafe {
            let raw: *mut gdk::ffi::GdkEvent = event.to_glib_none().0;
            let ptr = raw as *mut gdk::ffi::GdkEventButton;
            (*ptr).button = 1;
            (*ptr).send_event = 1;
            if let Some(win) = field.window() {
                (*ptr).window = win.to_glib_none().0;
            }
        }
        field.event(&event);
    }

    fn send_key(field: &gtk::Entry, name: &str, state: gdk::ModifierType) {
        let event = gdk::Event::new(gdk::EventType::KeyPress);
        unsafe {
            let raw: *mut gdk::ffi::GdkEvent = event.to_glib_none().0;
            let ptr = raw as *mut gdk::ffi::GdkEventKey;
            (*ptr).keyval = *gdk::keys::Key::from_name(name);
            (*ptr).state = state.bits();
            (*ptr).length = 1;
            (*ptr).send_event = 1;
            if let Some(win) = field.window() {
                (*ptr).window = win.to_glib_none().0;
            }
        }
        field.event(&event);
    }
}

fn set_hint(hint: &gtk::Label, text: &str, error: bool) {
    hint.set_text(text);
    if error {
        hint.style_context().add_class("error");
    } else {
        hint.style_context().remove_class("error");
    }
}

fn clamp(value: i32, min: i32, max: i32) -> i32 {
    if max < min {
        min
    } else {
        value.clamp(min, max)
    }
}

fn button_rects(w: f64, h: f64, count: usize) -> Vec<(f64, f64, f64, f64)> {
    let count = count.max(1);
    let side = (h - SHADOW * 2.0 - BUTTON_PAD * 2.0).min(
        (w - SHADOW * 2.0 - BUTTON_PAD * 2.0 - BUTTON_GAP * (count as f64 - 1.0)) / count as f64,
    );
    let bx = SHADOW + BUTTON_PAD;
    let by = SHADOW + BUTTON_PAD;
    (0..count)
        .map(|index| (bx + index as f64 * (side + BUTTON_GAP), by, side, side))
        .collect()
}

fn button_at(x: f64, y: f64, w: f64, h: f64, count: usize) -> Option<usize> {
    button_rects(w, h, count)
        .iter()
        .position(|(bx, by, bs, _)| x >= *bx && y >= *by && x <= bx + bs && y <= by + bs)
}

fn paint_toolbox(
    cr: &cairo::Context,
    w: f64,
    h: f64,
    enabled: bool,
    hover: Option<usize>,
    actions: &[ToolAction],
) {
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

    for (index, (bx, by, bs, _)) in button_rects(w, h, actions.len()).into_iter().enumerate() {
        rounded(cr, bx, by, bs, bs, 12.0);
        if enabled && hover == Some(index) {
            cr.set_source_rgba(0.271, 0.278, 0.353, 1.0);
        } else if enabled {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.03);
        } else {
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.12);
        }
        let _ = cr.fill();
        match actions.get(index).map(|action| action.icon) {
            Some(ToolIcon::Note) => paint_tool_icon(cr, bx, by, bs, enabled),
            Some(ToolIcon::File) => paint_save_icon(cr, bx, by, bs, enabled),
            Some(ToolIcon::Llm) => paint_llm_icon(cr, bx, by, bs, enabled),
            None => {}
        }
    }
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

fn paint_save_icon(cr: &cairo::Context, x: f64, y: f64, side: f64, enabled: bool) {
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
    let px = x + side * 0.23;
    let py = y + side * 0.21;
    let ps = side * 0.54;

    rounded(cr, px, py, ps, ps, 4.0);
    cr.set_source_rgb(paper.0, paper.1, paper.2);
    let _ = cr.fill();

    cr.set_source_rgb(ink.0, ink.1, ink.2);
    cr.rectangle(px + ps * 0.29, py + ps * 0.10, ps * 0.42, ps * 0.20);
    let _ = cr.fill();

    rounded(
        cr,
        px + ps * 0.20,
        py + ps * 0.54,
        ps * 0.60,
        ps * 0.32,
        2.0,
    );
    let _ = cr.fill();
}

fn paint_llm_icon(cr: &cairo::Context, x: f64, y: f64, side: f64, enabled: bool) {
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
    let bx = x + side * 0.20;
    let by = y + side * 0.20;
    let bw = side * 0.60;
    let bh = side * 0.42;

    rounded(cr, bx, by, bw, bh, side * 0.12);
    cr.set_source_rgb(paper.0, paper.1, paper.2);
    let _ = cr.fill();

    cr.move_to(bx + bw * 0.26, by + bh * 0.86);
    cr.line_to(bx + bw * 0.18, by + bh + side * 0.13);
    cr.line_to(bx + bw * 0.46, by + bh * 0.86);
    cr.close_path();
    let _ = cr.fill();

    cr.set_source_rgb(ink.0, ink.1, ink.2);
    let dot = (side * 0.036).max(1.2);
    for index in 0..3 {
        cr.arc(
            bx + bw * (0.27 + 0.23 * index as f64),
            by + bh * 0.5,
            dot,
            0.0,
            std::f64::consts::TAU,
        );
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
