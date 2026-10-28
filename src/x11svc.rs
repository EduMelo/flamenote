//! Bandeja XEmbed, atalho global e leitura da seleção primária.
//!
//! A i3bar fala o systray clássico do X11, não o StatusNotifier do DBus.
//! O texto "selecionado na janela em foco" é a seleção PRIMARY quando o dono
//! dela é a janela ativa, um parente na árvore, o mesmo processo ou a mesma
//! conexão X11 (o Chromium guarda a seleção numa janela sem PID).
//!
//! O modo de seleção não usa XGrabPointer: esse grab ficaria com os cliques e
//! a outra janela não receberia o arrasto. O cursor do servidor é escondido
//! (XFixes) e uma cruz preta de borda branca, sem área de entrada, segue o ponteiro.

use crate::config::{self, Config, Hotkey};
use std::collections::HashSet;
use std::os::fd::AsRawFd;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::protocol::shape::{ConnectionExt as _, SK, SO};
use x11rb::protocol::xfixes::ConnectionExt as _;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ClientMessageData, ClientMessageEvent, ConfigureNotifyEvent,
    ConfigureWindowAux, ConnectionExt as _, CreateGCAux, CreateWindowAux, EventMask,
    GetPropertyReply, ImageFormat, ImageOrder, KeyPressEvent, ModMask, PropMode, Property,
    Rectangle, StackMode, Window, WindowClass, CLIENT_MESSAGE_EVENT,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use xkbcommon::xkb::{keysym_from_name, KEYSYM_CASE_INSENSITIVE, KEYSYM_NO_FLAGS};

const ICON_PX: u16 = 22;
const QUERY_EVERY: Duration = Duration::from_millis(180);
const ARM_QUERY: Duration = Duration::from_millis(70);
const SEL_TIMEOUT: Duration = Duration::from_millis(280);
const CROSS: u16 = 25;
const CROSS_HOT: i32 = 12;
const BUTTON1: u16 = 1 << 8;
/// Leituras vazias depois do arrasto antes de desistir. O PRIMARY às vezes
/// chega alguns milissegundos depois do botão subir.
const EMPTY_RELEASE_TRIES: u8 = 4;
const DOCK_RETRY: Duration = Duration::from_secs(1);
const CONFIG_POLL: Duration = Duration::from_secs(2);

const LOCK: u16 = 1 << 1;
const MOD2: u16 = 1 << 4;
const MOD5: u16 = 1 << 7;

const XEMBED_MAPPED: u32 = 1;
const XEMBED_EMBEDDED_NOTIFY: u32 = 0;
const SYSTEM_TRAY_REQUEST_DOCK: u32 = 0;

pub enum AppEvent {
    ShowToolbox { x: i32, y: i32, text: String },
    HideToolbox,
    Selection(Option<String>),
    TrayMenu { x: i32, y: i32 },
    HotkeyChanged(String),
}

pub enum Command {
    /// Entra no modo de seleção. A caixa só aparece depois do texto.
    Arm,
    CloseToolbox,
    Shutdown,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SampleKind {
    Normal,
    Baseline,
    AfterRelease,
    Stale,
}

type XResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

struct Atoms {
    primary: Atom,
    utf8: Atom,
    incr: Atom,
    prop: Atom,
    net_active: Atom,
    net_wm_pid: Atom,
    net_wm_name: Atom,
    tray: Atom,
    tray_opcode: Atom,
    xembed: Atom,
    xembed_info: Atom,
}

enum SelState {
    Idle {
        last_at: Instant,
    },
    AwaitNotify {
        deadline: Instant,
        tried_string: bool,
    },
    AwaitIncr {
        deadline: Instant,
        chunks: Vec<u8>,
    },
}

struct X11 {
    conn: RustConnection,
    root: Window,
    visual: u32,
    depth: u8,
    bpp: u8,
    lsb: bool,
    icon: Window,
    gc: u32,
    icon_w: u16,
    icon_h: u16,
    atoms: Atoms,
    hotkey: Hotkey,
    config: Config,
    hotkey_codes: Vec<u8>,
    escape_codes: Vec<u8>,
    toolbox_open: bool,
    armed: bool,
    cursor_hidden: bool,
    crosshair: Window,
    crosshair_at: Option<(i32, i32)>,
    pointer: (i32, i32),
    anchor: Option<(i32, i32)>,
    press_at: Option<(i32, i32)>,
    baseline_ready: bool,
    baseline: Option<String>,
    button_down: bool,
    dragged: bool,
    gesture_started: bool,
    reveal_on_next: bool,
    release_sample: bool,
    empty_release_reads: u8,
    inflight_kind: SampleKind,
    docked: bool,
    next_dock: Instant,
    tray_warned: bool,
    held: HashSet<u8>,
    sel: SelState,
    last_sent: Option<Option<String>>,
    to_gtk: glib::Sender<AppEvent>,
    commands: Receiver<Command>,
    quit: bool,
    config_check: Instant,
}

pub fn spawn(config: Config, to_gtk: glib::Sender<AppEvent>) -> Sender<Command> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("flamenote-x11".into())
        .spawn(move || {
            if let Err(err) = run(config, to_gtk, rx) {
                eprintln!("flamenote: erro no X11: {err}");
            }
        })
        .expect("não criei a thread do X11");
    tx
}

fn run(config: Config, to_gtk: glib::Sender<AppEvent>, commands: Receiver<Command>) -> XResult<()> {
    let mut svc = X11::open(config, to_gtk, commands)?;
    svc.grab_hotkey()?;
    svc.dock()?;
    eprintln!("flamenote: atalho {}", svc.hotkey.display);

    let fd = svc.conn.stream().as_raw_fd();
    while !svc.quit {
        svc.drain_commands()?;
        if svc.quit {
            break;
        }
        svc.maybe_reload_config()?;
        svc.maybe_retry_dock()?;
        svc.track_crosshair()?;
        svc.maybe_begin_selection()?;
        svc.conn.flush()?;

        let mut pollfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let wait_ms = if svc.armed { 8 } else { 50 };
        let rc = unsafe { libc::poll(&mut pollfd, 1, wait_ms) };
        if rc < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err.into());
        }
        if rc > 0 {
            while let Some(event) = svc.conn.poll_for_event()? {
                svc.handle_event(event)?;
            }
        }
        svc.maybe_timeout_selection();
    }
    let _ = svc.conn.flush();
    Ok(())
}

impl X11 {
    fn open(
        config: Config,
        to_gtk: glib::Sender<AppEvent>,
        commands: Receiver<Command>,
    ) -> XResult<Self> {
        let (conn, screen_num) = RustConnection::connect(None)?;
        let setup = conn.setup();
        let screen = setup
            .roots
            .get(screen_num)
            .ok_or("tela X11 inexistente")?
            .clone();
        let root = screen.root;
        let depth = screen.root_depth;
        let visual = screen.root_visual;
        let lsb = setup.image_byte_order == ImageOrder::LSB_FIRST;
        let bpp = setup
            .pixmap_formats
            .iter()
            .find(|fmt| fmt.depth == depth)
            .map(|fmt| fmt.bits_per_pixel)
            .unwrap_or(32);
        let min_keycode = setup.min_keycode;
        let max_keycode = setup.max_keycode;

        let atoms = Atoms {
            primary: AtomEnum::PRIMARY.into(),
            utf8: intern(&conn, "UTF8_STRING")?,
            incr: intern(&conn, "INCR")?,
            prop: intern(&conn, "FLAMENOTE_SELECTION")?,
            net_active: intern(&conn, "_NET_ACTIVE_WINDOW")?,
            net_wm_pid: intern(&conn, "_NET_WM_PID")?,
            net_wm_name: intern(&conn, "_NET_WM_NAME")?,
            tray: intern(&conn, &format!("_NET_SYSTEM_TRAY_S{screen_num}"))?,
            tray_opcode: intern(&conn, "_NET_SYSTEM_TRAY_OPCODE")?,
            xembed: intern(&conn, "_XEMBED")?,
            xembed_info: intern(&conn, "_XEMBED_INFO")?,
        };

        let icon = conn.generate_id()?;
        conn.create_window(
            depth,
            icon,
            root,
            0,
            0,
            ICON_PX,
            ICON_PX,
            0,
            WindowClass::INPUT_OUTPUT,
            visual,
            &CreateWindowAux::new()
                .background_pixel(0x001e_1e2e)
                .event_mask(
                    EventMask::EXPOSURE
                        | EventMask::BUTTON_PRESS
                        | EventMask::STRUCTURE_NOTIFY
                        | EventMask::PROPERTY_CHANGE,
                ),
        )?;
        let gc = conn.generate_id()?;
        conn.create_gc(gc, icon, &CreateGCAux::new())?;
        conn.change_property8(
            PropMode::REPLACE,
            icon,
            AtomEnum::WM_NAME,
            AtomEnum::STRING,
            b"flamenote",
        )?;
        conn.change_property8(
            PropMode::REPLACE,
            icon,
            atoms.net_wm_name,
            atoms.utf8,
            b"flamenote",
        )?;
        conn.change_property8(
            PropMode::REPLACE,
            icon,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            b"flamenote\0Flamenote\0",
        )?;
        conn.change_property32(
            PropMode::REPLACE,
            icon,
            atoms.net_wm_pid,
            AtomEnum::CARDINAL,
            &[std::process::id()],
        )?;
        conn.change_property32(
            PropMode::REPLACE,
            icon,
            atoms.xembed_info,
            atoms.xembed_info,
            &[0, XEMBED_MAPPED],
        )?;
        conn.flush()?;

        let mut svc = Self {
            conn,
            root,
            visual,
            depth,
            bpp,
            lsb,
            icon,
            gc,
            icon_w: ICON_PX,
            icon_h: ICON_PX,
            atoms,
            hotkey: config.hotkey.clone(),
            config,
            hotkey_codes: Vec::new(),
            escape_codes: Vec::new(),
            toolbox_open: false,
            armed: false,
            cursor_hidden: false,
            crosshair: x11rb::NONE,
            crosshair_at: None,
            pointer: (0, 0),
            anchor: None,
            press_at: None,
            baseline_ready: false,
            baseline: None,
            button_down: false,
            dragged: false,
            gesture_started: false,
            reveal_on_next: false,
            release_sample: false,
            empty_release_reads: 0,
            inflight_kind: SampleKind::Normal,
            docked: false,
            next_dock: Instant::now(),
            tray_warned: false,
            held: HashSet::new(),
            sel: SelState::Idle {
                last_at: Instant::now(),
            },
            last_sent: None,
            to_gtk,
            commands,
            quit: false,
            config_check: Instant::now(),
        };
        svc.resolve_keycodes(min_keycode, max_keycode)?;
        svc.redraw()?;
        svc.conn.shape_query_version()?.reply()?;
        let fixes = svc.conn.xfixes_query_version(5, 0)?.reply()?;
        if fixes.major_version < 4 {
            eprintln!("flamenote: XFixes antigo demais para trocar o cursor.");
        }
        Ok(svc)
    }

    fn resolve_keycodes(&mut self, min_keycode: u8, max_keycode: u8) -> XResult<()> {
        let count = max_keycode.saturating_sub(min_keycode).saturating_add(1);
        let map = self
            .conn
            .get_keyboard_mapping(min_keycode, count)?
            .reply()?;
        let per = map.keysyms_per_keycode as usize;
        let hot_syms = keysyms_for_name(&self.hotkey.key);
        let esc_syms = keysyms_for_name("Escape");
        if hot_syms.is_empty() {
            eprintln!(
                "flamenote: não reconheci a tecla {:?}. Exemplos: N, F9, Print.",
                self.hotkey.key
            );
        }
        self.hotkey_codes.clear();
        self.escape_codes.clear();
        if per == 0 {
            return Ok(());
        }
        for (index, chunk) in map.keysyms.chunks(per).enumerate() {
            let code = min_keycode.saturating_add(index as u8);
            let matches = |wanted: &[u32]| chunk.iter().any(|sym| wanted.contains(&(*sym as u32)));
            if matches(&hot_syms) {
                self.hotkey_codes.push(code);
            }
            if matches(&esc_syms) {
                self.escape_codes.push(code);
            }
        }
        Ok(())
    }

    fn grab_hotkey(&mut self) -> XResult<()> {
        self.ungrab_codes(&self.hotkey_codes.clone(), self.hotkey.modifiers)?;
        if self.hotkey_codes.is_empty() {
            return Ok(());
        }
        let mut grabbed = false;
        for &code in &self.hotkey_codes.clone() {
            for mods in modifier_variants(self.hotkey.modifiers) {
                match self.conn.grab_key(
                    false,
                    self.root,
                    ModMask::from(mods),
                    code,
                    x11rb::protocol::xproto::GrabMode::ASYNC,
                    x11rb::protocol::xproto::GrabMode::ASYNC,
                ) {
                    Ok(cookie) => {
                        if cookie.check().is_ok() {
                            grabbed = true;
                        }
                    }
                    Err(err) => eprintln!("flamenote: grab do atalho falhou: {err}"),
                }
            }
        }
        if !grabbed {
            eprintln!(
                "flamenote: não consegui reservar {}. Outro programa já usa essa tecla.",
                self.hotkey.display
            );
        }
        Ok(())
    }

    fn grab_escape(&mut self) -> XResult<()> {
        for &code in &self.escape_codes.clone() {
            for mods in modifier_variants(0) {
                if let Ok(cookie) = self.conn.grab_key(
                    false,
                    self.root,
                    ModMask::from(mods),
                    code,
                    x11rb::protocol::xproto::GrabMode::ASYNC,
                    x11rb::protocol::xproto::GrabMode::ASYNC,
                ) {
                    let _ = cookie.check();
                }
            }
        }
        Ok(())
    }

    fn ungrab_escape(&mut self) -> XResult<()> {
        self.ungrab_codes(&self.escape_codes.clone(), 0)
    }

    fn ungrab_codes(&mut self, codes: &[u8], base: u16) -> XResult<()> {
        for &code in codes {
            for mods in modifier_variants(base) {
                if let Ok(cookie) = self.conn.ungrab_key(code, self.root, ModMask::from(mods)) {
                    let _ = cookie.check();
                }
            }
        }
        Ok(())
    }

    fn dock(&mut self) -> XResult<()> {
        let owner = self
            .conn
            .get_selection_owner(self.atoms.tray)?
            .reply()?
            .owner;
        self.next_dock = Instant::now() + DOCK_RETRY;
        if owner == 0 {
            if !self.tray_warned {
                self.tray_warned = true;
                eprintln!("flamenote: bandeja ainda não apareceu. O atalho funciona mesmo assim.");
            }
            return Ok(());
        }
        let event = ClientMessageEvent {
            response_type: CLIENT_MESSAGE_EVENT,
            format: 32,
            sequence: 0,
            window: owner,
            type_: self.atoms.tray_opcode,
            data: ClientMessageData::from([
                x11rb::CURRENT_TIME,
                SYSTEM_TRAY_REQUEST_DOCK,
                self.icon,
                0,
                0,
            ]),
        };
        self.conn
            .send_event(false, owner, EventMask::NO_EVENT, event)?;
        self.conn.flush()?;
        Ok(())
    }

    fn maybe_retry_dock(&mut self) -> XResult<()> {
        if self.docked || Instant::now() < self.next_dock {
            return Ok(());
        }
        self.dock()
    }

    fn redraw(&self) -> XResult<()> {
        if self.bpp != 32 || self.icon_w == 0 || self.icon_h == 0 {
            return Ok(());
        }
        let data = render_icon(self.icon_w, self.icon_h, self.lsb);
        self.conn.put_image(
            ImageFormat::Z_PIXMAP,
            self.icon,
            self.gc,
            self.icon_w,
            self.icon_h,
            0,
            0,
            0,
            self.depth,
            &data,
        )?;
        Ok(())
    }

    fn drain_commands(&mut self) -> XResult<()> {
        loop {
            match self.commands.try_recv() {
                Ok(Command::Arm) => self.arm()?,
                Ok(Command::CloseToolbox) => self.close_toolbox(false)?,
                Ok(Command::Shutdown) => self.quit = true,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.quit = true;
                    break;
                }
            }
        }
        Ok(())
    }

    fn toggle(&mut self) -> XResult<()> {
        if self.armed {
            self.disarm()
        } else if self.toolbox_open {
            self.close_toolbox(true)
        } else {
            self.arm()
        }
    }

    fn arm(&mut self) -> XResult<()> {
        if self.armed {
            return Ok(());
        }
        if self.toolbox_open {
            self.close_toolbox(true)?;
        }
        self.armed = true;
        self.reset_gesture();
        self.baseline = None;
        self.baseline_ready = false;
        self.inflight_kind = SampleKind::Normal;
        self.last_sent = None;
        self.sel = SelState::Idle {
            last_at: Instant::now() - QUERY_EVERY,
        };
        self.grab_escape()?;
        self.show_crosshair()?;
        debug("modo seleção");
        Ok(())
    }

    /// Esc no modo de seleção: o cursor volta e nada mais acontece.
    fn disarm(&mut self) -> XResult<()> {
        self.hide_crosshair()?;
        self.armed = false;
        self.reset_gesture();
        self.baseline = None;
        self.baseline_ready = false;
        self.inflight_kind = SampleKind::Normal;
        self.ungrab_escape()?;
        self.sel = SelState::Idle {
            last_at: Instant::now(),
        };
        debug("seleção cancelada");
        Ok(())
    }

    fn close_toolbox(&mut self, notify: bool) -> XResult<()> {
        if !self.toolbox_open {
            return Ok(());
        }
        self.toolbox_open = false;
        if !self.armed {
            self.ungrab_escape()?;
        }
        self.sel = SelState::Idle {
            last_at: Instant::now(),
        };
        if notify {
            let _ = self.to_gtk.send(AppEvent::HideToolbox);
        }
        Ok(())
    }

    fn reveal(&mut self, x: i32, y: i32, text: String) -> XResult<()> {
        self.hide_crosshair()?;
        self.armed = false;
        self.reset_gesture();
        self.baseline = None;
        self.baseline_ready = false;
        self.inflight_kind = SampleKind::Normal;
        self.toolbox_open = true;
        self.last_sent = Some(Some(text.clone()));
        self.sel = SelState::Idle {
            last_at: Instant::now(),
        };
        debug(&format!("caixa junto da seleção em {x},{y}"));
        let _ = self.to_gtk.send(AppEvent::ShowToolbox { x, y, text });
        Ok(())
    }

    fn reset_gesture(&mut self) {
        self.press_at = None;
        self.anchor = None;
        self.button_down = false;
        self.dragged = false;
        self.gesture_started = false;
        self.reveal_on_next = false;
        self.release_sample = false;
        self.empty_release_reads = 0;
    }

    fn show_crosshair(&mut self) -> XResult<()> {
        self.ensure_crosshair()?;
        let (x, y, _) = self.pointer_state()?;
        self.pointer = (x, y);
        self.move_crosshair(x, y)?;
        self.conn.map_window(self.crosshair)?;
        self.conn.configure_window(
            self.crosshair,
            &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
        )?;
        self.conn
            .clear_area(false, self.crosshair, 0, 0, 0, 0)?
            .check()?;
        if !self.cursor_hidden {
            self.conn.xfixes_hide_cursor(self.root)?.check()?;
            self.cursor_hidden = true;
        }
        self.conn.flush()?;
        Ok(())
    }

    fn hide_crosshair(&mut self) -> XResult<()> {
        if self.cursor_hidden {
            self.conn.xfixes_show_cursor(self.root)?.check()?;
            self.cursor_hidden = false;
        }
        if self.crosshair != x11rb::NONE {
            self.conn.unmap_window(self.crosshair)?;
        }
        self.crosshair_at = None;
        self.conn.flush()?;
        Ok(())
    }

    fn ensure_crosshair(&mut self) -> XResult<()> {
        if self.crosshair != x11rb::NONE {
            return Ok(());
        }
        if self.bpp != 32 {
            return Err("profundidade do X11 sem 32 bits por pixel; não desenhei a cruz".into());
        }
        let id = self.conn.generate_id()?;
        // Fundo da janela, não um put_image antes do mapa. No mapa o servidor
        // pinta o background; com pixel 0 a cruz ficava só preta e a borda
        // branca sumia. O pixmap permanece: é a referência do fundo.
        let pixmap = self.conn.generate_id()?;
        self.conn
            .create_pixmap(self.depth, pixmap, self.root, CROSS, CROSS)?
            .check()?;
        let data = render_crosshair(self.lsb);
        self.conn
            .put_image(
                ImageFormat::Z_PIXMAP,
                pixmap,
                self.gc,
                CROSS,
                CROSS,
                0,
                0,
                0,
                self.depth,
                &data,
            )?
            .check()?;
        self.conn.create_window(
            self.depth,
            id,
            self.root,
            0,
            0,
            CROSS,
            CROSS,
            0,
            WindowClass::INPUT_OUTPUT,
            self.visual,
            &CreateWindowAux::new()
                .override_redirect(1u32)
                .background_pixmap(pixmap)
                .event_mask(EventMask::NO_EVENT),
        )?;
        self.conn.change_property8(
            PropMode::REPLACE,
            id,
            AtomEnum::WM_NAME,
            AtomEnum::STRING,
            b"flamenote-cursor",
        )?;
        let mid = (CROSS / 2) as i16;
        let bars = [
            Rectangle {
                x: 0,
                y: mid - 1,
                width: CROSS,
                height: 3,
            },
            Rectangle {
                x: mid - 1,
                y: 0,
                width: 3,
                height: CROSS,
            },
        ];
        self.conn
            .shape_rectangles(
                SO::SET,
                SK::BOUNDING,
                x11rb::protocol::xproto::ClipOrdering::UNSORTED,
                id,
                0,
                0,
                &bars,
            )?
            .check()?;
        let hole = [Rectangle {
            x: mid - 1,
            y: mid - 1,
            width: 3,
            height: 3,
        }];
        self.conn
            .shape_rectangles(
                SO::SUBTRACT,
                SK::BOUNDING,
                x11rb::protocol::xproto::ClipOrdering::UNSORTED,
                id,
                0,
                0,
                &hole,
            )?
            .check()?;
        self.shape_input_empty(id)?;
        self.crosshair = id;
        Ok(())
    }

    fn shape_input_empty(&mut self, window: Window) -> XResult<()> {
        let pixmap = self.conn.generate_id()?;
        let gc = self.conn.generate_id()?;
        self.conn
            .create_pixmap(1, pixmap, self.root, CROSS, CROSS)?;
        self.conn
            .create_gc(gc, pixmap, &CreateGCAux::new().foreground(0))?;
        self.conn.poly_fill_rectangle(
            pixmap,
            gc,
            &[Rectangle {
                x: 0,
                y: 0,
                width: CROSS,
                height: CROSS,
            }],
        )?;
        self.conn
            .shape_mask(SO::SET, SK::INPUT, window, 0, 0, pixmap)?
            .check()?;
        self.conn.free_gc(gc)?;
        self.conn.free_pixmap(pixmap)?;
        Ok(())
    }

    fn move_crosshair(&mut self, x: i32, y: i32) -> XResult<()> {
        let at = (x - CROSS_HOT, y - CROSS_HOT);
        if self.crosshair_at == Some(at) {
            return Ok(());
        }
        self.crosshair_at = Some(at);
        self.conn.configure_window(
            self.crosshair,
            &ConfigureWindowAux::new()
                .x(at.0)
                .y(at.1)
                .stack_mode(StackMode::ABOVE),
        )?;
        Ok(())
    }

    fn pointer_state(&self) -> XResult<(i32, i32, bool)> {
        let reply = self.conn.query_pointer(self.root)?.reply()?;
        let down = key_state_bits(reply.mask) & BUTTON1 != 0;
        Ok((reply.root_x as i32, reply.root_y as i32, down))
    }

    fn track_crosshair(&mut self) -> XResult<()> {
        if !self.armed {
            return Ok(());
        }
        let (x, y, down) = self.pointer_state()?;
        self.pointer = (x, y);
        self.move_crosshair(x, y)?;
        if down && !self.button_down {
            self.button_down = true;
            self.gesture_started = true;
            self.press_at = Some((x, y));
            self.dragged = false;
            self.reveal_on_next = false;
            self.empty_release_reads = 0;
            if !self.baseline_ready && self.inflight_kind == SampleKind::Baseline {
                self.inflight_kind = SampleKind::Stale;
            }
        } else if down {
            if let Some((px, py)) = self.press_at {
                let dx = x - px;
                let dy = y - py;
                if dx * dx + dy * dy > 9 {
                    self.dragged = true;
                }
            }
        } else if self.button_down {
            self.button_down = false;
            self.anchor = Some((x, y));
            if self.dragged {
                self.reveal_on_next = true;
                self.empty_release_reads = 0;
            }
            self.release_sample = true;
            if matches!(
                self.sel,
                SelState::AwaitNotify { .. } | SelState::AwaitIncr { .. }
            ) {
                self.inflight_kind = SampleKind::Stale;
            } else if let SelState::Idle { last_at } = &mut self.sel {
                *last_at = Instant::now() - QUERY_EVERY;
            }
        }
        Ok(())
    }

    fn maybe_reload_config(&mut self) -> XResult<()> {
        if self.config_check.elapsed() < CONFIG_POLL {
            return Ok(());
        }
        self.config_check = Instant::now();
        let next = config::reload_if_changed(&self.config);
        if next.hotkey == self.config.hotkey {
            self.config = next;
            return Ok(());
        }
        self.ungrab_codes(&self.hotkey_codes.clone(), self.hotkey.modifiers)?;
        self.hotkey = next.hotkey.clone();
        self.config = next;
        let (min, max) = {
            let setup = self.conn.setup();
            (setup.min_keycode, setup.max_keycode)
        };
        self.resolve_keycodes(min, max)?;
        self.grab_hotkey()?;
        let _ = self
            .to_gtk
            .send(AppEvent::HotkeyChanged(self.hotkey.display.clone()));
        Ok(())
    }

    fn handle_event(&mut self, event: Event) -> XResult<()> {
        match event {
            Event::Error(err) => {
                debug(&format!("x11: {err:?}"));
            }
            Event::Expose(ev) if ev.window == self.icon && ev.count == 0 => {
                self.redraw()?;
            }
            Event::ConfigureNotify(cfg) if cfg.window == self.icon => {
                self.resize_icon(cfg)?;
            }
            Event::ReparentNotify(ev) if ev.window == self.icon => {
                self.docked = ev.parent != self.root && ev.parent != 0;
                if self.docked {
                    self.tray_warned = false;
                } else {
                    self.next_dock = Instant::now() + Duration::from_millis(400);
                }
            }
            Event::ButtonPress(ev) if ev.event == self.icon => match ev.detail {
                1 => self.toggle()?,
                2 | 3 => {
                    let _ = self.to_gtk.send(AppEvent::TrayMenu {
                        x: ev.root_x as i32,
                        y: ev.root_y as i32,
                    });
                }
                _ => {}
            },
            Event::KeyRelease(ev) => {
                if let Some(next) = self.conn.poll_for_event()? {
                    if let Event::KeyPress(press) = &next {
                        if press.detail == ev.detail && press.time == ev.time {
                            return Ok(());
                        }
                    }
                    self.held.remove(&ev.detail);
                    self.handle_event(next)?;
                } else {
                    self.held.remove(&ev.detail);
                }
            }
            Event::KeyPress(ev) => {
                if self.held.insert(ev.detail) {
                    self.on_key_press(&ev)?;
                }
            }
            Event::SelectionNotify(ev) if ev.requestor == self.icon => {
                self.on_selection_notify(ev.property)?;
            }
            Event::PropertyNotify(ev) if ev.window == self.icon && ev.atom == self.atoms.prop => {
                if matches!(self.sel, SelState::AwaitIncr { .. }) && ev.state == Property::NEW_VALUE
                {
                    self.on_incr_chunk()?;
                }
            }
            Event::ClientMessage(ev) if ev.window == self.icon && ev.type_ == self.atoms.xembed => {
                let data = ev.data.as_data32();
                if data[1] == XEMBED_EMBEDDED_NOTIFY {
                    self.docked = true;
                }
            }
            Event::MappingNotify(_) => {
                let (min, max) = {
                    let setup = self.conn.setup();
                    (setup.min_keycode, setup.max_keycode)
                };
                self.ungrab_codes(&self.hotkey_codes.clone(), self.hotkey.modifiers)?;
                if self.toolbox_open || self.armed {
                    self.ungrab_escape()?;
                }
                self.resolve_keycodes(min, max)?;
                self.grab_hotkey()?;
                if self.toolbox_open || self.armed {
                    self.grab_escape()?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn resize_icon(&mut self, cfg: ConfigureNotifyEvent) -> XResult<()> {
        let w = cfg.width.max(1);
        let h = cfg.height.max(1);
        if w != self.icon_w || h != self.icon_h {
            self.icon_w = w;
            self.icon_h = h;
            self.redraw()?;
        }
        Ok(())
    }

    fn on_key_press(&mut self, ev: &KeyPressEvent) -> XResult<()> {
        let state = key_state_bits(ev.state);
        debug(&format!(
            "tecla {} estado {state:#x} hotkey {}",
            ev.detail,
            self.hotkey_codes.contains(&ev.detail)
        ));
        if self.escape_codes.contains(&ev.detail) && plain_escape(state) {
            if self.armed {
                self.disarm()?;
            } else if self.toolbox_open {
                self.close_toolbox(true)?;
            }
            return Ok(());
        }
        if self.hotkey_codes.contains(&ev.detail) && chord_matches(state, self.hotkey.modifiers) {
            self.toggle()?;
        }
        Ok(())
    }

    fn maybe_begin_selection(&mut self) -> XResult<()> {
        if !self.toolbox_open && !self.armed {
            return Ok(());
        }
        let SelState::Idle { last_at } = self.sel else {
            return Ok(());
        };
        let every = if self.armed { ARM_QUERY } else { QUERY_EVERY };
        if last_at.elapsed() < every {
            return Ok(());
        }
        let focused = self.active_window()?;
        let owner = self
            .conn
            .get_selection_owner(self.atoms.primary)?
            .reply()?
            .owner;
        let belongs =
            selection_belongs_to_focus(&self.conn, owner, focused, self.atoms.net_wm_pid)?;
        if focused == 0 || !belongs {
            if self.armed {
                self.armed_without_owner();
                return Ok(());
            }
            if self.last_sent.as_ref() != Some(&None) {
                debug(&format!(
                    "ferramenta off: foco {focused:#x} dono {owner:#x}"
                ));
            }
            self.publish_selection(None);
            self.sel = SelState::Idle {
                last_at: Instant::now(),
            };
            return Ok(());
        }
        self.classify_sample();
        self.request_selection(self.atoms.utf8)?;
        self.sel = SelState::AwaitNotify {
            deadline: Instant::now() + SEL_TIMEOUT,
            tried_string: false,
        };
        Ok(())
    }

    fn armed_without_owner(&mut self) {
        if !self.baseline_ready && !self.reveal_on_next && !self.button_down {
            self.baseline = None;
            self.baseline_ready = true;
            self.gesture_started = false;
        }
        self.sel = SelState::Idle {
            last_at: Instant::now(),
        };
    }

    fn classify_sample(&mut self) {
        if self.button_down {
            self.inflight_kind = SampleKind::Normal;
        } else if self.armed
            && !self.baseline_ready
            && !self.gesture_started
            && !self.reveal_on_next
        {
            self.inflight_kind = SampleKind::Baseline;
        } else if self.release_sample {
            self.release_sample = false;
            self.inflight_kind = SampleKind::AfterRelease;
        } else {
            self.inflight_kind = SampleKind::Normal;
        }
    }

    fn request_selection(&self, target: Atom) -> XResult<()> {
        self.conn.convert_selection(
            self.icon,
            self.atoms.primary,
            target,
            self.atoms.prop,
            x11rb::CURRENT_TIME,
        )?;
        self.conn.flush()?;
        Ok(())
    }

    fn on_selection_notify(&mut self, property: Atom) -> XResult<()> {
        let SelState::AwaitNotify { tried_string, .. } = self.sel else {
            return Ok(());
        };
        if property == 0 || property == x11rb::NONE {
            if !tried_string {
                self.request_selection(AtomEnum::STRING.into())?;
                self.sel = SelState::AwaitNotify {
                    deadline: Instant::now() + SEL_TIMEOUT,
                    tried_string: true,
                };
            } else {
                self.finish_selection(None)?;
            }
            return Ok(());
        }
        let reply = self
            .conn
            .get_property(
                true,
                self.icon,
                self.atoms.prop,
                AtomEnum::ANY,
                0,
                256 * 1024,
            )?
            .reply()?;
        if reply.type_ == self.atoms.incr {
            self.sel = SelState::AwaitIncr {
                deadline: Instant::now() + SEL_TIMEOUT,
                chunks: Vec::new(),
            };
            return Ok(());
        }
        self.finish_selection(decode_property(&reply))?;
        Ok(())
    }

    fn on_incr_chunk(&mut self) -> XResult<()> {
        let reply = self
            .conn
            .get_property(
                true,
                self.icon,
                self.atoms.prop,
                AtomEnum::ANY,
                0,
                256 * 1024,
            )?
            .reply()?;
        let SelState::AwaitIncr { chunks, .. } = &mut self.sel else {
            return Ok(());
        };
        if reply.value.is_empty() || reply.value_len == 0 {
            let text = String::from_utf8_lossy(chunks).into_owned();
            self.finish_selection(Some(text))?;
        } else {
            chunks.extend_from_slice(&reply.value);
            if let SelState::AwaitIncr { deadline, .. } = &mut self.sel {
                *deadline = Instant::now() + SEL_TIMEOUT;
            }
        }
        Ok(())
    }

    fn finish_selection(&mut self, text: Option<String>) -> XResult<()> {
        let text = text.and_then(meaningful);
        let kind = std::mem::replace(&mut self.inflight_kind, SampleKind::Normal);
        self.sel = SelState::Idle {
            last_at: Instant::now(),
        };
        if self.armed {
            self.absorb_armed_sample(text, kind)?;
            if self.armed && self.release_sample {
                if let SelState::Idle { last_at } = &mut self.sel {
                    *last_at = Instant::now() - QUERY_EVERY;
                }
            }
            return Ok(());
        }
        self.publish_selection(text);
        Ok(())
    }

    fn absorb_armed_sample(&mut self, text: Option<String>, kind: SampleKind) -> XResult<()> {
        if kind == SampleKind::Stale || self.button_down {
            return Ok(());
        }
        if self.reveal_on_next && kind == SampleKind::AfterRelease {
            if text.is_none() {
                self.empty_release_reads = self.empty_release_reads.saturating_add(1);
                if self.empty_release_reads < EMPTY_RELEASE_TRIES {
                    self.release_sample = true;
                    return Ok(());
                }
                self.reveal_on_next = false;
                self.dragged = false;
                self.gesture_started = false;
                self.empty_release_reads = 0;
                self.baseline_ready = true;
                return Ok(());
            }
            self.empty_release_reads = 0;
            self.reveal_on_next = false;
            self.dragged = false;
            self.gesture_started = false;
            self.baseline_ready = true;
            self.baseline = text.clone();
            if let Some(text) = text {
                let (x, y) = self.anchor.unwrap_or(self.pointer);
                self.reveal(x, y, text)?;
            }
            return Ok(());
        }
        if !self.baseline_ready {
            // Clique sem arrasto, antes da foto inicial: o texto atual vira a
            // base e a caixa continua fechada.
            if !self.gesture_started || kind == SampleKind::AfterRelease {
                self.baseline = text;
                self.baseline_ready = true;
                self.gesture_started = false;
                self.dragged = false;
            }
            return Ok(());
        }
        if kind == SampleKind::AfterRelease {
            self.gesture_started = false;
            self.dragged = false;
        }
        if text != self.baseline {
            match text {
                Some(text) => {
                    let (x, y) = self.pointer;
                    self.reveal(x, y, text)?;
                }
                None => self.baseline = None,
            }
        }
        Ok(())
    }

    fn maybe_timeout_selection(&mut self) {
        let timed_out = match self.sel {
            SelState::AwaitNotify { deadline, .. } | SelState::AwaitIncr { deadline, .. } => {
                Instant::now() >= deadline
            }
            SelState::Idle { .. } => false,
        };
        if !timed_out {
            return;
        }
        if self.inflight_kind == SampleKind::AfterRelease {
            self.release_sample = true;
        }
        self.inflight_kind = SampleKind::Normal;
        let retry = self.release_sample;
        self.sel = SelState::Idle {
            last_at: if retry {
                Instant::now() - QUERY_EVERY
            } else {
                Instant::now()
            },
        };
    }

    fn publish_selection(&mut self, text: Option<String>) {
        if self.last_sent.as_ref() == Some(&text) {
            return;
        }
        if let Some(text) = &text {
            debug(&format!("seleção de {} caracteres", text.chars().count()));
        }
        self.last_sent = Some(text.clone());
        let _ = self.to_gtk.send(AppEvent::Selection(text));
    }

    fn active_window(&self) -> XResult<Window> {
        let reply = self
            .conn
            .get_property(
                false,
                self.root,
                self.atoms.net_active,
                AtomEnum::WINDOW,
                0,
                1,
            )?
            .reply()?;
        Ok(reply.value32().and_then(|mut it| it.next()).unwrap_or(0))
    }
}

fn selection_belongs_to_focus(
    conn: &RustConnection,
    owner: Window,
    focused: Window,
    pid_atom: Atom,
) -> XResult<bool> {
    if owner == 0 || focused == 0 {
        return Ok(false);
    }
    if windows_related(conn, owner, focused)? {
        return Ok(true);
    }
    match (
        window_pid(conn, owner, pid_atom)?,
        window_pid(conn, focused, pid_atom)?,
    ) {
        (Some(a), Some(b)) if a != 0 && a == b => return Ok(true),
        _ => {}
    }
    let mask = conn.setup().resource_id_mask;
    Ok(same_x_client(owner, focused, mask))
}

fn same_x_client(a: Window, b: Window, resource_mask: u32) -> bool {
    if resource_mask == 0 || resource_mask == u32::MAX {
        return false;
    }
    let client_bits = !resource_mask;
    let owner_client = a & client_bits;
    owner_client != 0 && owner_client == (b & client_bits)
}

fn windows_related(conn: &RustConnection, a: Window, b: Window) -> XResult<bool> {
    if a == b {
        return Ok(true);
    }
    Ok(is_ancestor(conn, a, b)? || is_ancestor(conn, b, a)?)
}

fn is_ancestor(conn: &RustConnection, ancestor: Window, mut window: Window) -> XResult<bool> {
    for _ in 0..24 {
        let parent = conn.query_tree(window)?.reply()?.parent;
        if parent == ancestor {
            return Ok(true);
        }
        if parent == 0 || parent == window {
            return Ok(false);
        }
        window = parent;
    }
    Ok(false)
}

fn window_pid(conn: &RustConnection, window: Window, pid_atom: Atom) -> XResult<Option<u32>> {
    let reply = conn
        .get_property(false, window, pid_atom, AtomEnum::CARDINAL, 0, 1)?
        .reply()?;
    Ok(reply.value32().and_then(|mut it| it.next()))
}

fn decode_property(reply: &GetPropertyReply) -> Option<String> {
    if reply.value.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&reply.value).into_owned())
}

fn meaningful(text: String) -> Option<String> {
    if text.chars().any(|ch| !ch.is_whitespace()) {
        Some(text)
    } else {
        None
    }
}

fn modifier_variants(base: u16) -> [u16; 8] {
    let noise = [
        0,
        LOCK,
        MOD2,
        LOCK | MOD2,
        MOD5,
        MOD5 | LOCK,
        MOD5 | MOD2,
        MOD5 | LOCK | MOD2,
    ];
    noise.map(|extra| base | extra)
}

fn plain_escape(state: u16) -> bool {
    let noise = LOCK | MOD2 | MOD5;
    (state & !noise) & 0x00ff == 0
}

fn chord_matches(state: u16, required: u16) -> bool {
    let mut noise = LOCK | MOD2;
    if required & MOD5 == 0 {
        noise |= MOD5;
    }
    (state & !noise) & 0x00ff == required & 0x00ff
}

fn key_state_bits(state: impl IntoKeyBits) -> u16 {
    state.into_key_bits()
}

trait IntoKeyBits {
    fn into_key_bits(self) -> u16;
}

impl IntoKeyBits for u16 {
    fn into_key_bits(self) -> u16 {
        self
    }
}

impl IntoKeyBits for x11rb::protocol::xproto::KeyButMask {
    fn into_key_bits(self) -> u16 {
        u16::from(self)
    }
}

fn keysyms_for_name(name: &str) -> Vec<u32> {
    let mut found = Vec::new();
    for flags in [KEYSYM_NO_FLAGS, KEYSYM_CASE_INSENSITIVE] {
        let raw = keysym_from_name(name, flags).raw();
        if raw != 0 && !found.contains(&raw) {
            found.push(raw);
        }
    }
    if let Some(ch) = name.chars().next().filter(|_| name.chars().count() == 1) {
        let other = if ch.is_ascii_lowercase() {
            ch.to_ascii_uppercase()
        } else {
            ch.to_ascii_lowercase()
        };
        let raw = keysym_from_name(&other.to_string(), KEYSYM_NO_FLAGS).raw();
        if raw != 0 && !found.contains(&raw) {
            found.push(raw);
        }
    }
    found
}

fn intern(conn: &RustConnection, name: &str) -> XResult<Atom> {
    Ok(conn.intern_atom(false, name.as_bytes())?.reply()?.atom)
}

/// Cruz de 3 px: traço preto de 1 px com borda branca de 1 px.
/// O branco fecha as pontas e o vão central, para o preto não encostar
/// no que estiver atrás. O miolo 3×3 fica vazio.
fn render_crosshair(lsb: bool) -> Vec<u8> {
    let n = CROSS as i32;
    let mid = n / 2;
    let mut data = vec![0u8; (n * n * 4) as usize];
    for y in 0..n {
        for x in 0..n {
            let on_bar = (x - mid).abs() <= 1 || (y - mid).abs() <= 1;
            let in_hole = (x - mid).abs() <= 1 && (y - mid).abs() <= 1;
            if !on_bar || in_hole {
                continue;
            }
            let spine = x == mid || y == mid;
            let cap = x == 0
                || y == 0
                || x == n - 1
                || y == n - 1
                || (x - mid).abs() == 2
                || (y - mid).abs() == 2;
            let rgb = if spine && !cap {
                (0, 0, 0)
            } else {
                (255, 255, 255)
            };
            write_pixel(&mut data, n, x, y, rgb, lsb);
        }
    }
    data
}

fn render_icon(width: u16, height: u16, lsb: bool) -> Vec<u8> {
    let w = width as i32;
    let h = height as i32;
    let mut data = vec![0u8; (w * h * 4) as usize];
    let x0 = (w as f32 * 0.16) as i32;
    let y0 = (h as f32 * 0.14) as i32;
    let x1 = w - x0;
    let y1 = h - (h as f32 * 0.14) as i32;
    let rad = ((x1 - x0).min(y1 - y0) / 4).max(2);
    for y in 0..h {
        for x in 0..w {
            let mut rgb = (0x1e, 0x1e, 0x2e);
            if in_round(x, y, x0, y0, x1, y1, rad) {
                rgb = (0xf4, 0xf2, 0xf6);
                let line_h = ((y1 - y0) / 11).max(1);
                let lx0 = x0 + rad.max(2);
                let lx1 = x1 - rad.max(2);
                for (index, frac) in [0.36_f32, 0.52, 0.68].into_iter().enumerate() {
                    let ly = y0 + ((y1 - y0) as f32 * frac) as i32;
                    if y >= ly && y < ly + line_h && x >= lx0 && x < lx1 {
                        rgb = if index == 1 {
                            (0xfa, 0xb3, 0x87)
                        } else {
                            (0x6c, 0x70, 0x86)
                        };
                    }
                }
            }
            write_pixel(&mut data, w, x, y, rgb, lsb);
        }
    }
    data
}

fn in_round(x: i32, y: i32, x0: i32, y0: i32, x1: i32, y1: i32, rad: i32) -> bool {
    if x < x0 || y < y0 || x >= x1 || y >= y1 {
        return false;
    }
    let r = rad.min((x1 - x0) / 2).min((y1 - y0) / 2).max(1);
    let (cx, cy) = if x < x0 + r && y < y0 + r {
        (x0 + r, y0 + r)
    } else if x >= x1 - r && y < y0 + r {
        (x1 - r - 1, y0 + r)
    } else if x < x0 + r && y >= y1 - r {
        (x0 + r, y1 - r - 1)
    } else if x >= x1 - r && y >= y1 - r {
        (x1 - r - 1, y1 - r - 1)
    } else {
        return true;
    };
    let dx = x - cx;
    let dy = y - cy;
    dx * dx + dy * dy <= r * r
}

fn write_pixel(data: &mut [u8], width: i32, x: i32, y: i32, rgb: (u8, u8, u8), lsb: bool) {
    let i = ((y * width + x) * 4) as usize;
    if lsb {
        data[i] = rgb.2;
        data[i + 1] = rgb.1;
        data[i + 2] = rgb.0;
        data[i + 3] = 0xff;
    } else {
        data[i] = 0xff;
        data[i + 1] = rgb.0;
        data[i + 2] = rgb.1;
        data[i + 3] = rgb.2;
    }
}

fn debug(msg: &str) {
    if std::env::var_os("FLAMENOTE_DEBUG").is_some() {
        eprintln!("flamenote: {msg}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgba(data: &[u8], x: i32, y: i32, lsb: bool) -> [u8; 4] {
        let n = CROSS as i32;
        let i = ((y * n + x) * 4) as usize;
        let b = &data[i..i + 4];
        if lsb {
            [b[2], b[1], b[0], b[3]]
        } else {
            [b[1], b[2], b[3], b[0]]
        }
    }

    #[test]
    fn crosshair_black_stroke_is_enclosed_by_white() {
        for lsb in [false, true] {
            let data = render_crosshair(lsb);
            let n = CROSS as i32;
            let mid = n / 2;
            let mut black = 0;
            let mut white = 0;
            for y in 0..n {
                for x in 0..n {
                    let px = rgba(&data, x, y, lsb);
                    let on_bar = (x - mid).abs() <= 1 || (y - mid).abs() <= 1;
                    let in_hole = (x - mid).abs() <= 1 && (y - mid).abs() <= 1;
                    if !on_bar || in_hole {
                        assert_eq!(px, [0, 0, 0, 0], "fora da cruz em {x},{y}");
                        continue;
                    }
                    if px == [0, 0, 0, 255] {
                        black += 1;
                        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                            let nx = x + dx;
                            let ny = y + dy;
                            assert!(
                                (0..n).contains(&nx) && (0..n).contains(&ny),
                                "traço preto na borda da janela em {x},{y}"
                            );
                            let neighbor = rgba(&data, nx, ny, lsb);
                            assert!(
                                neighbor == [255, 255, 255, 255] || neighbor == [0, 0, 0, 255],
                                "preto em {x},{y} encosta em {neighbor:?} em {nx},{ny}"
                            );
                        }
                    } else {
                        assert_eq!(px, [255, 255, 255, 255], "pixel da cruz em {x},{y}");
                        white += 1;
                    }
                }
            }
            assert!(black > 0 && white > black, "preto {black}, branco {white}");
        }
    }
}

impl Drop for X11 {
    fn drop(&mut self) {
        if !self.cursor_hidden {
            return;
        }
        let _ = self.conn.xfixes_show_cursor(self.root);
        let _ = self.conn.flush();
    }
}
