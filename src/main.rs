//! Flamenote fica na bandeja. O atalho arma a seleção: o cursor vira uma
//! cruz e a caixa de ferramentas só aparece junto do texto escolhido.
//! As ferramentas criam um bilhete, gravam o texto num arquivo ou enviam o
//! texto a até três consultas configuradas num servidor compatível com OpenAI.

mod config;
mod copies;
mod llm;
mod ui;
mod x11svc;

use config::{LlmConfig, NoteConfig, Query, QueryDestination};
use gio::prelude::ApplicationExt;
use gtk::prelude::*;
use std::cell::RefCell;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use x11svc::{AppEvent, Command};

struct LlmOutcome {
    query: usize,
    destination: QueryDestination,
    result: Result<String, String>,
}

struct App {
    gtk: gtk::Application,
    to_x11: std::sync::mpsc::Sender<Command>,
    toolbox: Option<ui::Toolbox>,
    selection: Option<String>,
    notes: Rc<RefCell<Vec<gtk::Window>>>,
    note_seq: i32,
    config_path: std::path::PathBuf,
    hotkey_display: String,
    copies_dir: PathBuf,
    note: NoteConfig,
    llm: LlmConfig,
    queries: Vec<Query>,
    llm_tx: glib::Sender<LlmOutcome>,
    settings: Option<ui::Settings>,
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
    let (llm_tx, llm_rx) = glib::MainContext::channel(glib::Priority::DEFAULT);
    let to_x11 = x11svc::spawn(cfg.clone(), to_gtk);

    let app = Rc::new(RefCell::new(App {
        gtk: gtk_app,
        to_x11,
        toolbox: None,
        selection: None,
        notes: Rc::new(RefCell::new(Vec::new())),
        note_seq: 0,
        config_path: cfg.path.clone(),
        hotkey_display: cfg.hotkey.display,
        copies_dir: cfg.copies_dir.clone(),
        note: cfg.note.clone(),
        llm: cfg.llm.clone(),
        queries: cfg.queries.clone(),
        llm_tx,
        settings: None,
        _hold: hold,
        _lock: Some(lock),
    }));

    let app_events = app.clone();
    from_x11.attach(None, move |event| {
        on_event(&app_events, event);
        glib::ControlFlow::Continue
    });
    let app_llm = app.clone();
    llm_rx.attach(None, move |outcome| {
        on_llm_outcome(&app_llm, outcome);
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
        AppEvent::TrayMenu { x, y, button, time } => popup_menu(app, x, y, button, time),
        AppEvent::HotkeyChanged(display) => {
            let mut this = app.borrow_mut();
            this.hotkey_display = display.clone();
            if let Some(settings) = &this.settings {
                settings.set_display(&display);
            }
        }
        AppEvent::CopiesDirChanged(dir) => {
            let mut this = app.borrow_mut();
            this.copies_dir = dir.clone();
            if let Some(settings) = &this.settings {
                settings.set_copies_dir(&config::display_path(&dir));
            }
        }
        AppEvent::NoteSizeChanged(note) => {
            app.borrow_mut().note = note;
        }
        AppEvent::LlmChanged { llm, queries } => {
            let mut this = app.borrow_mut();
            this.llm = llm.clone();
            this.queries = queries.clone();
            if let Some(settings) = &this.settings {
                settings.set_llm(&llm, &queries);
            }
            this.toolbox = None;
        }
    }
}

fn show_toolbox(app: &Rc<RefCell<App>>, x: i32, y: i32, text: String) {
    {
        let mut this = app.borrow_mut();
        this.selection = Some(text);
        if this.toolbox.is_none() {
            let app_note = app.clone();
            let app_file = app.clone();
            let tx = this.llm_tx.clone();
            let queries = this.queries.clone();
            let mut actions = vec![
                ui::ToolAction::new(ui::ToolIcon::Note, "Criar um bilhete", move || {
                    capture(&app_note)
                }),
                ui::ToolAction::new(ui::ToolIcon::File, "Salvar como arquivo", move || {
                    save_selection_file(&app_file)
                }),
            ];
            for (index, query) in queries.into_iter().enumerate() {
                let app_query = app.clone();
                let tx_query = tx.clone();
                let tooltip = query_tooltip(index, &query.prompt);
                actions.push(ui::ToolAction::new(ui::ToolIcon::Llm, tooltip, move || {
                    run_query(&app_query, &tx_query, index, &query)
                }));
            }
            this.toolbox = Some(ui::build_toolbox(actions));
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
    let (text, seq, notes, note_size) = {
        let mut this = app.borrow_mut();
        let Some(text) = this.selection.clone() else {
            return;
        };
        let seq = this.note_seq;
        this.note_seq += 1;
        (text, seq, this.notes.clone(), this.note.clone())
    };
    let note = ui::spawn_note(&text, seq, &note_size);
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

fn save_selection_file(app: &Rc<RefCell<App>>) {
    let (text, dir, gtk_app) = {
        let this = app.borrow();
        let Some(text) = this.selection.clone() else {
            return;
        };
        (text, this.copies_dir.clone(), this.gtk.clone())
    };

    match copies::save(&text, &dir) {
        Ok(path) => {
            copy_file_to_clipboard(&path);
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("arquivo");
            notify(&gtk_app, None, &format!("Arquivo '{name}' salvo"), None);
        }
        Err(err) => {
            eprintln!("flamenote: não salvei em {}: {err}", dir.display());
            notify(
                &gtk_app,
                None,
                "Não salvei o arquivo",
                Some(&err.to_string()),
            );
        }
    }

    let _ = app.borrow().to_x11.send(Command::CloseToolbox);
    app.borrow_mut().hide_toolbox();
}

fn run_query(app: &Rc<RefCell<App>>, tx: &glib::Sender<LlmOutcome>, index: usize, query: &Query) {
    let (selection, llm, gtk_app) = {
        let this = app.borrow();
        let Some(selection) = this.selection.clone() else {
            return;
        };
        (selection, this.llm.clone(), this.gtk.clone())
    };

    let _ = app.borrow().to_x11.send(Command::CloseToolbox);
    app.borrow_mut().hide_toolbox();
    let notification_id = format!("flamenote-llm-{}", index + 1);
    notify(
        &gtk_app,
        Some(&notification_id),
        &format!("Consulta {} em andamento", index + 1),
        Some(&short_prompt(&query.prompt)),
    );

    let prompt = query.prompt.clone();
    let destination = query.destination;
    let tx = tx.clone();
    std::thread::spawn(move || {
        let result = llm::complete(&llm, &prompt, &selection);
        let _ = tx.send(LlmOutcome {
            query: index,
            destination,
            result,
        });
    });
}

fn on_llm_outcome(app: &Rc<RefCell<App>>, outcome: LlmOutcome) {
    let (gtk_app, copies_dir) = {
        let this = app.borrow();
        (this.gtk.clone(), this.copies_dir.clone())
    };
    let query = outcome.query + 1;
    let notification_id = format!("flamenote-llm-{query}");
    match outcome.result {
        Ok(text) => match outcome.destination {
            QueryDestination::File => match copies::save(&text, &copies_dir) {
                Ok(path) => {
                    copy_file_to_clipboard(&path);
                    let name = path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("arquivo");
                    notify(
                        &gtk_app,
                        Some(&notification_id),
                        &format!("Consulta {query}: arquivo '{name}' salvo"),
                        None,
                    );
                }
                Err(err) => {
                    eprintln!("flamenote: não salvei em {}: {err}", copies_dir.display());
                    notify(
                        &gtk_app,
                        Some(&notification_id),
                        &format!("Consulta {query}: não salvei o arquivo"),
                        Some(&err.to_string()),
                    );
                }
            },
            QueryDestination::Clipboard => {
                copy_text_to_clipboard(&text);
                notify(
                    &gtk_app,
                    Some(&notification_id),
                    &format!("Consulta {query} copiada"),
                    Some("Resposta na área de transferência."),
                );
            }
        },
        Err(message) => notify(
            &gtk_app,
            Some(&notification_id),
            &format!("Consulta {query} falhou"),
            Some(&message),
        ),
    }
}

fn notify(gtk_app: &gtk::Application, id: Option<&str>, title: &str, body: Option<&str>) {
    let notification = gio::Notification::new(title);
    if let Some(body) = body {
        notification.set_body(Some(body));
    }
    gtk_app.send_notification(id, &notification);
}

fn short_prompt(prompt: &str) -> String {
    let prompt = prompt.replace('\n', " ");
    let mut short: String = prompt.chars().take(60).collect();
    if prompt.chars().count() > 60 {
        short.push('…');
    }
    short
}

fn query_tooltip(index: usize, prompt: &str) -> String {
    format!("Consulta {}: {}", index + 1, short_prompt(prompt))
}

fn copy_text_to_clipboard(text: &str) {
    gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD).set_text(text);
}

/// Copia o arquivo como item colável, como o `x-special/gnome-copied-files`
/// do script original.
fn copy_file_to_clipboard(path: &Path) {
    let uri = gio::File::for_path(path).uri().to_string();
    let gnome = gdk::Atom::intern("x-special/gnome-copied-files");
    let uri_list = gdk::Atom::intern("text/uri-list");
    let utf8 = gdk::Atom::intern("UTF8_STRING");
    let targets = [
        gtk::TargetEntry::new("x-special/gnome-copied-files", gtk::TargetFlags::empty(), 0),
        gtk::TargetEntry::new("text/uri-list", gtk::TargetFlags::empty(), 1),
        gtk::TargetEntry::new("UTF8_STRING", gtk::TargetFlags::empty(), 2),
    ];
    let clipboard = gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD);
    clipboard.set_with_data(&targets, move |_, data, info| {
        let (atom, payload) = match info {
            0 => (gnome, format!("copy\n{uri}\n")),
            1 => (uri_list, format!("{uri}\r\n")),
            _ => (utf8, uri.clone()),
        };
        data.set(&atom, 8, payload.as_bytes());
    });
}

impl App {
    fn hide_toolbox(&mut self) {
        if let Some(toolbox) = &self.toolbox {
            toolbox.hide();
        }
    }
}

fn popup_menu(app: &Rc<RefCell<App>>, x: i32, y: i32, button: u32, time: u32) {
    let (to_x11, gtk_app) = {
        let this = app.borrow();
        (this.to_x11.clone(), this.gtk.clone())
    };

    let menu = gtk::Menu::new();
    let open = gtk::MenuItem::with_label("Selecionar texto");
    open.connect_activate(move |_| {
        let _ = to_x11.send(Command::Arm);
    });
    let app_settings = app.clone();
    let settings = gtk::MenuItem::with_label("Configurações");
    settings.connect_activate(move |_| open_settings(&app_settings));
    let quit = gtk::MenuItem::with_label("Sair");
    quit.connect_activate(move |_| {
        gtk_app.quit();
    });
    menu.append(&open);
    menu.append(&settings);
    menu.append(&gtk::SeparatorMenuItem::new());
    menu.append(&quit);
    menu.show_all();

    let Some(screen) = gdk::Screen::default() else {
        return;
    };
    let Some(root) = screen.root_window() else {
        return;
    };
    let rect = gdk::Rectangle::new(x, y, 1, 1);
    let trigger = menu_trigger(&root, x, y, button, time);
    menu.popup_at_rect(
        &root,
        &rect,
        gdk::Gravity::NorthWest,
        gdk::Gravity::NorthWest,
        Some(&trigger),
    );
}

/// O clique do ícone chega pela thread do X11, sem evento do GDK. Sem este
/// botão e este instante o GTK avisa "no trigger event for menu popup" e o
/// menu não segura o ponteiro.
pub(crate) fn menu_trigger(
    root: &gdk::Window,
    x: i32,
    y: i32,
    button: u32,
    time: u32,
) -> gdk::Event {
    use glib::translate::{ToGlibPtr, ToGlibPtrMut};

    let mut event = gdk::Event::new(gdk::EventType::ButtonPress);
    if let Some(pointer) = root
        .display()
        .default_seat()
        .and_then(|seat| seat.pointer())
    {
        event.set_device(Some(&pointer));
        event.set_source_device(Some(&pointer));
    }
    unsafe {
        let raw: *mut gdk::ffi::GdkEvent = event.to_glib_none_mut().0;
        let ev = raw as *mut gdk::ffi::GdkEventButton;
        let window: *mut gdk::ffi::GdkWindow = root.to_glib_none().0;
        // gdk_event_free libera a janela. A referência extra fica com o evento.
        (*ev).window = glib::gobject_ffi::g_object_ref(window.cast()) as *mut gdk::ffi::GdkWindow;
        (*ev).button = button;
        (*ev).time = time;
        (*ev).x = f64::from(x);
        (*ev).y = f64::from(y);
        (*ev).x_root = f64::from(x);
        (*ev).y_root = f64::from(y);
        (*ev).send_event = 1;
    }
    event
}

fn open_settings(app: &Rc<RefCell<App>>) {
    if let Some(settings) = app.borrow().settings.as_ref() {
        settings.present();
        return;
    }
    let (path, display, copies_display, llm_initial, queries_initial, to_x11, gtk_app) = {
        let this = app.borrow();
        (
            this.config_path.clone(),
            this.hotkey_display.clone(),
            config::display_path(&this.copies_dir),
            this.llm.clone(),
            this.queries.clone(),
            this.to_x11.clone(),
            this.gtk.clone(),
        )
    };
    let to_pause = to_x11.clone();
    let on_listen = Rc::new(move |listen: bool| {
        let command = if listen {
            Command::PauseHotkey
        } else {
            Command::ResumeHotkey
        };
        let _ = to_pause.send(command);
    });

    let path_hotkey = path.clone();
    let to_x11_hotkey = to_x11.clone();
    let app_save = app.clone();
    let on_save = Rc::new(move |hotkey: &config::Hotkey| {
        let (copies_dir, note, llm, queries) = {
            let this = app_save.borrow();
            (
                this.copies_dir.clone(),
                this.note.clone(),
                this.llm.clone(),
                this.queries.clone(),
            )
        };
        config::save_config(&path_hotkey, hotkey, &copies_dir, &note, &llm, &queries)
            .map_err(|err| format!("Não salvei o atalho: {err}"))?;
        app_save.borrow_mut().hotkey_display = hotkey.display.clone();
        let _ = to_x11_hotkey.send(Command::ReloadConfig);
        Ok(())
    });

    let path_dir = path.clone();
    let app_save_dir = app.clone();
    let on_save_dir = Rc::new(move |raw: &str| {
        let dir = config::expand_path(raw);
        if dir.exists() && !dir.is_dir() {
            return Err(format!("{} não é uma pasta", dir.display()));
        }
        let (hotkey, note, llm, queries) = {
            let this = app_save_dir.borrow();
            let hotkey = config::parse_hotkey(&this.hotkey_display).unwrap_or_else(|_| {
                config::parse_hotkey(config::DEFAULT_HOTKEY).expect("atalho padrão")
            });
            (
                hotkey,
                this.note.clone(),
                this.llm.clone(),
                this.queries.clone(),
            )
        };
        config::save_config(&path_dir, &hotkey, &dir, &note, &llm, &queries)
            .map_err(|err| format!("Não salvei a pasta: {err}"))?;
        app_save_dir.borrow_mut().copies_dir = dir.clone();
        Ok(config::display_path(&dir))
    });

    let path_llm = path.clone();
    let app_save_llm = app.clone();
    let on_save_llm = Rc::new(move |llm: &LlmConfig, queries: &[Query]| {
        let llm = LlmConfig {
            base_url: llm.base_url.trim().to_string(),
            model: llm.model.trim().to_string(),
            api_key: llm.api_key.trim().to_string(),
        };
        let queries: Vec<Query> = queries.iter().take(config::MAX_QUERIES).cloned().collect();
        if !llm.base_url.is_empty()
            && !llm.base_url.starts_with("http://")
            && !llm.base_url.starts_with("https://")
        {
            return Err("A base URL deve começar com http:// ou https://.".into());
        }
        if !queries.is_empty() && (llm.base_url.is_empty() || llm.model.is_empty()) {
            return Err("Defina a base URL e o modelo para usar as consultas.".into());
        }
        let (hotkey, copies_dir, note) = {
            let this = app_save_llm.borrow();
            let hotkey = config::parse_hotkey(&this.hotkey_display).unwrap_or_else(|_| {
                config::parse_hotkey(config::DEFAULT_HOTKEY).expect("atalho padrão")
            });
            (hotkey, this.copies_dir.clone(), this.note.clone())
        };
        config::save_config(&path_llm, &hotkey, &copies_dir, &note, &llm, &queries)
            .map_err(|err| format!("Não salvei a configuração LLM: {err}"))?;
        let mut this = app_save_llm.borrow_mut();
        this.llm = llm;
        this.queries = queries;
        this.toolbox = None;
        Ok(())
    });

    let settings = ui::build_settings(
        ui::SettingsInitial {
            hotkey: &display,
            copies_dir: &copies_display,
            llm: &llm_initial,
            queries: &queries_initial,
        },
        ui::SettingsHandlers {
            on_listen,
            on_save_hotkey: on_save,
            on_save_copies_dir: on_save_dir,
            on_save_llm,
        },
    );
    let window = settings.window().clone();
    gtk_app.add_window(&window);
    app.borrow_mut().settings = Some(settings);
    let app_closed = app.clone();
    window.connect_destroy(move |_| {
        app_closed.borrow_mut().settings = None;
    });
    window.show_all();
    window.present();
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
