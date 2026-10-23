//! Flamenote fica na bandeja. O atalho arma a seleção: o cursor vira uma
//! cruz e a caixa de ferramentas só aparece junto do texto escolhido.
//! A única ferramenta desta versão cola esse texto num bilhete branco,
//! arrastável, que fecha com um clique duplo.

mod config;
mod ui;
mod x11svc;

use gtk::prelude::*;
use std::cell::RefCell;
use std::os::fd::AsRawFd;
use std::rc::Rc;
use x11svc::{AppEvent, Command};

struct App {
    gtk: gtk::Application,
    to_x11: std::sync::mpsc::Sender<Command>,
    toolbox: Option<ui::Toolbox>,
    selection: Option<String>,
    notes: Rc<RefCell<Vec<gtk::Window>>>,
    note_seq: i32,
    hotkey_display: String,
    /// Mantém o processo vivo sem janelas. O guard libera o hold ao cair.
    _hold: gio::ApplicationHoldGuard,
    _lock: Option<std::fs::File>,
}

fn main() {
    if std::env::var_os("DISPLAY").is_none() {
        eprintln!("flamenote precisa de uma sessão X11. DISPLAY não está definido.");
        std::process::exit(1);
    }

    let gtk_app = gtk::Application::new(
        Some("com.edumelo.flamenote"),
        gio::ApplicationFlags::empty(),
    );
    let state: Rc<RefCell<Option<Rc<RefCell<App>>>>> = Rc::new(RefCell::new(None));
    let state_for_shutdown = state.clone();

    gtk_app.connect_activate(move |gtk_app| {
        if state.borrow().is_some() {
            if let Some(app) = state.borrow().as_ref() {
                let _ = app.borrow().to_x11.send(Command::Arm);
            }
            return;
        }
        let app = start(gtk_app.clone());
        *state.borrow_mut() = Some(app);
    });

    gtk_app.connect_shutdown(move |_| {
        if let Some(app) = state_for_shutdown.borrow().as_ref() {
            let _ = app.borrow().to_x11.send(Command::Shutdown);
        }
    });

    gtk_app.run();
}

fn start(gtk_app: gtk::Application) -> Rc<RefCell<App>> {
    gdk::set_program_class("Flamenote");
    ui::install_style();
    let cfg = config::load();
    let lock = instance_lock();
    let hold = gtk_app.hold();
    eprintln!("flamenote na bandeja. Atalho: {}", cfg.hotkey.display);
    eprintln!("Configuração: {}", cfg.path.display());

    let (to_gtk, from_x11) = glib::MainContext::channel(glib::Priority::DEFAULT);
    let to_x11 = x11svc::spawn(cfg.clone(), to_gtk);

    let app = Rc::new(RefCell::new(App {
        gtk: gtk_app,
        to_x11,
        toolbox: None,
        selection: None,
        notes: Rc::new(RefCell::new(Vec::new())),
        note_seq: 0,
        hotkey_display: cfg.hotkey.display,
        _hold: hold,
        _lock: Some(lock),
    }));

    let app_events = app.clone();
    from_x11.attach(None, move |event| {
        on_event(&app_events, event);
        glib::ControlFlow::Continue
    });
    app
}

fn on_event(app: &Rc<RefCell<App>>, event: AppEvent) {
    match event {
        AppEvent::ShowToolbox { x, y, text } => show_toolbox(app, x, y, text),
        AppEvent::HideToolbox => app.borrow_mut().hide_toolbox(),
        AppEvent::Selection(text) => {
            let mut this = app.borrow_mut();
            this.selection = text;
            if let Some(toolbox) = &this.toolbox {
                toolbox.set_enabled(this.selection.is_some());
            }
        }
        AppEvent::TrayMenu { x, y } => popup_menu(app, x, y),
        AppEvent::HotkeyChanged(display) => {
            app.borrow_mut().hotkey_display = display;
        }
    }
}

fn show_toolbox(app: &Rc<RefCell<App>>, x: i32, y: i32, text: String) {
    {
        let mut this = app.borrow_mut();
        this.selection = Some(text);
        if this.toolbox.is_none() {
            let app_click = app.clone();
            let toolbox = ui::build_toolbox(Rc::new(move || capture(&app_click)));
            this.toolbox = Some(toolbox);
        }
        if let Some(toolbox) = &this.toolbox {
            toolbox.set_enabled(true);
        }
    }
    if let Some(toolbox) = app.borrow().toolbox.as_ref() {
        toolbox.show_near(x, y);
    }
}

fn capture(app: &Rc<RefCell<App>>) {
    let (text, seq, notes) = {
        let mut this = app.borrow_mut();
        let Some(text) = this.selection.clone() else {
            return;
        };
        let seq = this.note_seq;
        this.note_seq += 1;
        (text, seq, this.notes.clone())
    };
    let note = ui::spawn_note(&text, seq);
    let notes_drop = notes.clone();
    note.connect_destroy(move |window| {
        notes_drop
            .borrow_mut()
            .retain(|existing| existing != window);
    });
    notes.borrow_mut().push(note);
    let _ = app.borrow().to_x11.send(Command::CloseToolbox);
    app.borrow_mut().hide_toolbox();
}

impl App {
    fn hide_toolbox(&mut self) {
        if let Some(toolbox) = &self.toolbox {
            toolbox.hide();
        }
    }
}

fn popup_menu(app: &Rc<RefCell<App>>, x: i32, y: i32) {
    let (to_x11, gtk_app, hotkey) = {
        let this = app.borrow();
        (
            this.to_x11.clone(),
            this.gtk.clone(),
            this.hotkey_display.clone(),
        )
    };

    let menu = gtk::Menu::new();
    let open = gtk::MenuItem::with_label("Selecionar texto");
    open.connect_activate(move |_| {
        let _ = to_x11.send(Command::Arm);
    });
    let current = gtk::MenuItem::with_label(&format!("Atalho: {hotkey}"));
    current.set_sensitive(false);
    let quit = gtk::MenuItem::with_label("Sair");
    quit.connect_activate(move |_| {
        gtk_app.quit();
    });
    menu.append(&open);
    menu.append(&current);
    menu.append(&quit);
    menu.show_all();

    let Some(screen) = gdk::Screen::default() else {
        return;
    };
    let Some(root) = screen.root_window() else {
        return;
    };
    let rect = gdk::Rectangle::new(x, y, 1, 1);
    menu.popup_at_rect(
        &root,
        &rect,
        gdk::Gravity::NorthWest,
        gdk::Gravity::NorthWest,
        None::<&gdk::Event>,
    );
}

fn instance_lock() -> std::fs::File {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("flamenote-{}", unsafe { libc::getuid() }))
        });
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("flamenote.lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&path)
        .unwrap_or_else(|err| {
            eprintln!("flamenote: não travei {}: {err}", path.display());
            std::process::exit(1);
        });
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc != 0 {
        eprintln!("flamenote já está em execução.");
        std::process::exit(0);
    }
    file
}
