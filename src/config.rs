//! Atalho global e leitura de `config.toml`.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const DEFAULT_HOTKEY: &str = "Super+Shift+N";
pub const DEFAULT_COPIES_DIR: &str = "~/copias";
pub const MAX_QUERIES: usize = 3;

const SHIFT: u16 = 1 << 0;
const CONTROL: u16 = 1 << 2;
const MOD1: u16 = 1 << 3;
const MOD3: u16 = 1 << 5;
const MOD4: u16 = 1 << 6;
const MOD5: u16 = 1 << 7;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hotkey {
    /// Máscara de modificadores do X11, sem Lock/NumLock.
    pub modifiers: u16,
    /// Nome do keysym aceito pelo xkbcommon (`n`, `F9`, `Print`).
    pub key: String,
    /// Forma canônica para mostrar na bandeja (`Super+Shift+N`).
    pub display: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmConfig {
    pub base_url: String,
    pub model: String,
    pub api_key: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QueryDestination {
    File,
    #[default]
    Clipboard,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Query {
    pub prompt: String,
    pub destination: QueryDestination,
}

impl Default for Query {
    fn default() -> Self {
        Self {
            prompt: String::new(),
            destination: QueryDestination::Clipboard,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub path: PathBuf,
    pub hotkey: Hotkey,
    /// Pasta onde a ferramenta de arquivo grava as cópias, já expandida.
    pub copies_dir: PathBuf,
    pub llm: LlmConfig,
    pub queries: Vec<Query>,
    pub modified: Option<SystemTime>,
}

#[derive(Debug)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

#[derive(Default, Deserialize)]
#[serde(default)]
struct FileConfig {
    hotkey: Option<String>,
    copies_dir: Option<String>,
    llm: LlmConfig,
    queries: Vec<Query>,
}

#[derive(Serialize)]
struct FileConfigOut<'a> {
    hotkey: &'a str,
    copies_dir: &'a str,
    llm: &'a LlmConfig,
    queries: &'a [Query],
}

const CONFIG_HEADER: &str = "\
# Atalho global do flamenote.\n\
# Modificadores: Ctrl, Alt, Shift, Super.\n\
# Exemplos: \"Super+Shift+N\", \"Ctrl+Alt+N\", \"F9\"\n\
# copies_dir aceita caminho absoluto ou começando com ~.\n\
# Até três [[queries]]; destination = \"file\" ou \"clipboard\".\n";

pub fn config_path() -> PathBuf {
    if let Some(path) = std::env::var_os("FLAMENOTE_CONFIG") {
        return PathBuf::from(path);
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| home_dir().join(".config"));
    base.join("flamenote").join("config.toml")
}

pub fn load() -> Config {
    let path = config_path();
    if !path.exists() {
        if let Err(err) = write_default(&path) {
            eprintln!("flamenote: não criei {path:?}: {err}");
        }
    }
    read_path(&path)
}

/// Relê o arquivo se o mtime mudou. Mantém o atalho anterior se o texto for inválido.
pub fn reload_if_changed(current: &Config) -> Config {
    let modified = mtime(&current.path);
    if modified == current.modified {
        return current.clone();
    }
    let next = read_path(&current.path);
    if next.hotkey != current.hotkey {
        eprintln!("flamenote: atalho agora é {}", next.hotkey.display);
    }
    next
}

/// Lê o arquivo de novo, mesmo que o mtime não tenha mudado.
pub fn load_from(path: &Path) -> Config {
    read_path(path)
}

/// Modificadores vistos num key-press, sem Lock nem NumLock.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CaptureMods {
    pub super_key: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub mod3: bool,
    pub mod5: bool,
}

pub fn is_modifier_key(name: &str) -> bool {
    matches!(
        name,
        "Shift_L"
            | "Shift_R"
            | "Control_L"
            | "Control_R"
            | "Alt_L"
            | "Alt_R"
            | "Meta_L"
            | "Meta_R"
            | "Super_L"
            | "Super_R"
            | "Hyper_L"
            | "Hyper_R"
            | "ISO_Level3_Shift"
            | "ISO_Level5_Shift"
            | "Mode_switch"
            | "Caps_Lock"
            | "Shift_Lock"
            | "Num_Lock"
            | "Scroll_Lock"
            | "Multi_key"
    )
}

/// Monta o atalho a partir da tecla e dos modificadores segurados.
pub fn hotkey_from_capture(mods: CaptureMods, key_name: &str) -> Result<Hotkey, ConfigError> {
    let key_name = match key_name {
        "ISO_Left_Tab" => "Tab",
        other => other,
    };
    if key_name.is_empty() {
        return Err(ConfigError("nenhuma tecla".into()));
    }
    if is_modifier_key(key_name) {
        return Err(ConfigError(
            "solte os modificadores e pressione a tecla do atalho".into(),
        ));
    }
    let mut parts = Vec::new();
    if mods.super_key {
        parts.push("Super");
    }
    if mods.ctrl {
        parts.push("Ctrl");
    }
    if mods.alt {
        parts.push("Alt");
    }
    if mods.shift {
        parts.push("Shift");
    }
    if mods.mod3 {
        parts.push("Mod3");
    }
    if mods.mod5 {
        parts.push("Mod5");
    }
    parts.push(key_name);
    parse_hotkey(&parts.join("+"))
}

pub fn save_config(
    path: &Path,
    hotkey: &Hotkey,
    copies_dir: &Path,
    llm: &LlmConfig,
    queries: &[Query],
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let copies_dir = display_path(copies_dir);
    let queries = &queries[..queries.len().min(MAX_QUERIES)];
    let file = FileConfigOut {
        hotkey: &hotkey.display,
        copies_dir: &copies_dir,
        llm,
        queries,
    };
    let body =
        toml::to_string(&file).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    fs::write(path, format!("{CONFIG_HEADER}\n{body}"))?;
    restrict_permissions(path);
    Ok(())
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    if let Ok(metadata) = fs::metadata(path) {
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o600);
        let _ = fs::set_permissions(path, permissions);
    }
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) {}

pub fn default_copies_dir() -> PathBuf {
    expand_path(DEFAULT_COPIES_DIR)
}

/// Expande `~`, `~/…` e resolve caminhos relativos a partir da pasta pessoal.
pub fn expand_path(raw: &str) -> PathBuf {
    let raw = raw.trim();
    if raw.is_empty() || raw == "~" {
        return if raw.is_empty() {
            default_copies_dir()
        } else {
            home_dir()
        };
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return home_dir().join(rest);
    }
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        home_dir().join(path)
    }
}

/// Forma curta para mostrar no arquivo e na janela de configuração.
pub fn display_path(path: &Path) -> String {
    match path.strip_prefix(home_dir()) {
        Ok(rest) if !rest.as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => path.display().to_string(),
    }
}

pub fn parse_hotkey(raw: &str) -> Result<Hotkey, ConfigError> {
    let mut modifiers = 0u16;
    let mut key: Option<String> = None;
    let mut display_mods: Vec<String> = Vec::new();

    for part in raw.split('+') {
        let token = part.trim();
        if token.is_empty() {
            continue;
        }
        match token.to_ascii_lowercase().as_str() {
            "shift" => {
                modifiers |= SHIFT;
                display_mods.push("Shift".to_string());
            }
            "ctrl" | "control" => {
                modifiers |= CONTROL;
                display_mods.push("Ctrl".to_string());
            }
            "alt" | "mod1" | "opt" | "option" => {
                modifiers |= MOD1;
                display_mods.push("Alt".to_string());
            }
            "mod3" => {
                modifiers |= MOD3;
                display_mods.push("Mod3".to_string());
            }
            "super" | "mod4" | "win" | "meta" | "cmd" => {
                modifiers |= MOD4;
                display_mods.push("Super".to_string());
            }
            "mod5" => {
                modifiers |= MOD5;
                display_mods.push("Mod5".to_string());
            }
            _ => {
                if key.is_some() {
                    return Err(ConfigError(format!("atalho {raw:?} tem mais de uma tecla")));
                }
                key = Some(normalize_key_name(token));
            }
        }
    }

    let Some(key) = key else {
        return Err(ConfigError(format!("atalho {raw:?} não tem tecla")));
    };
    if key.eq_ignore_ascii_case("escape") || key == "Esc" {
        return Err(ConfigError(
            "Esc fecha a caixa de ferramentas e não pode ser o atalho".into(),
        ));
    }

    display_mods.push(display_key(&key));
    let display = display_mods.join("+");

    Ok(Hotkey {
        modifiers,
        key,
        display,
    })
}

fn read_path(path: &Path) -> Config {
    let modified = mtime(path);
    let (hotkey, copies_dir, llm, queries) = match fs::read_to_string(path) {
        Ok(text) => match toml::from_str::<FileConfig>(&text) {
            Ok(file) => {
                let hotkey = file
                    .hotkey
                    .as_deref()
                    .map(|raw| match parse_hotkey(raw) {
                        Ok(hotkey) => hotkey,
                        Err(err) => {
                            eprintln!(
                                "flamenote: {err}. Usando {DEFAULT_HOTKEY}. Arquivo: {}",
                                path.display()
                            );
                            parse_hotkey(DEFAULT_HOTKEY).expect("atalho padrão")
                        }
                    })
                    .unwrap_or_else(|| parse_hotkey(DEFAULT_HOTKEY).expect("atalho padrão"));
                let copies_dir = file
                    .copies_dir
                    .as_deref()
                    .map(expand_path)
                    .filter(|dir| !dir.as_os_str().is_empty())
                    .unwrap_or_else(default_copies_dir);
                let llm = LlmConfig {
                    base_url: file.llm.base_url.trim().to_string(),
                    model: file.llm.model.trim().to_string(),
                    api_key: file.llm.api_key.trim().to_string(),
                };
                let queries = file
                    .queries
                    .into_iter()
                    .filter(|query| !query.prompt.trim().is_empty())
                    .take(MAX_QUERIES)
                    .map(|mut query| {
                        query.prompt = query.prompt.trim().to_string();
                        query
                    })
                    .collect();
                (hotkey, copies_dir, llm, queries)
            }
            Err(err) => {
                eprintln!(
                    "flamenote: config inválida ({err}). Usando {DEFAULT_HOTKEY}. Arquivo: {}",
                    path.display()
                );
                (
                    parse_hotkey(DEFAULT_HOTKEY).expect("atalho padrão"),
                    default_copies_dir(),
                    LlmConfig::default(),
                    Vec::new(),
                )
            }
        },
        Err(err) => {
            eprintln!(
                "flamenote: não li {} ({err}). Usando {DEFAULT_HOTKEY}.",
                path.display()
            );
            (
                parse_hotkey(DEFAULT_HOTKEY).expect("atalho padrão"),
                default_copies_dir(),
                LlmConfig::default(),
                Vec::new(),
            )
        }
    };
    Config {
        path: path.to_path_buf(),
        hotkey,
        copies_dir,
        llm,
        queries,
        modified,
    }
}

fn write_default(path: &Path) -> std::io::Result<()> {
    let hotkey = parse_hotkey(DEFAULT_HOTKEY).expect("atalho padrão");
    save_config(
        path,
        &hotkey,
        &default_copies_dir(),
        &LlmConfig::default(),
        &[],
    )
}

fn mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|meta| meta.modified()).ok()
}

fn normalize_key_name(token: &str) -> String {
    if token.chars().count() == 1 {
        return token.to_ascii_lowercase();
    }
    // F1..F35 e nomes do xkb: primeira letra maiúscula, o resto como veio
    // se o usuário escreveu "print" ou "escape" ou "return".
    let lower = token.to_ascii_lowercase();
    match lower.as_str() {
        "esc" => "Escape".to_string(),
        "print" | "printscreen" | "prtsc" => "Print".to_string(),
        "return" | "enter" => "Return".to_string(),
        "space" | "espaço" | "espaco" => "space".to_string(),
        "tab" => "Tab".to_string(),
        "backspace" => "BackSpace".to_string(),
        "delete" | "del" => "Delete".to_string(),
        "insert" | "ins" => "Insert".to_string(),
        "home" => "Home".to_string(),
        "end" => "End".to_string(),
        "pageup" | "pgup" => "Page_Up".to_string(),
        "pagedown" | "pgdn" => "Page_Down".to_string(),
        "left" => "Left".to_string(),
        "right" => "Right".to_string(),
        "up" => "Up".to_string(),
        "down" => "Down".to_string(),
        other if other.starts_with('f') && other[1..].parse::<u8>().is_ok() => {
            let n: u8 = other[1..].parse().unwrap_or(0);
            format!("F{n}")
        }
        _ => token.to_string(),
    }
}

fn display_key(key: &str) -> String {
    if key.chars().count() == 1 {
        return key.to_ascii_uppercase();
    }
    key.to_string()
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_hotkey() {
        let hotkey = parse_hotkey("Super+Shift+N").unwrap();
        assert_eq!(hotkey.modifiers, MOD4 | SHIFT);
        assert_eq!(hotkey.key, "n");
        assert_eq!(hotkey.display, "Super+Shift+N");
    }

    #[test]
    fn modifiers_can_come_in_any_order() {
        let hotkey = parse_hotkey("n + ctrl + alt").unwrap();
        assert_eq!(hotkey.modifiers, CONTROL | MOD1);
        assert_eq!(hotkey.key, "n");
        assert_eq!(hotkey.display, "Ctrl+Alt+N");
    }

    #[test]
    fn parses_function_key_without_modifiers() {
        let hotkey = parse_hotkey("f9").unwrap();
        assert_eq!(hotkey.modifiers, 0);
        assert_eq!(hotkey.key, "F9");
        assert_eq!(hotkey.display, "F9");
    }

    #[test]
    fn rejects_escape_as_hotkey() {
        assert!(parse_hotkey("Escape").is_err());
        assert!(parse_hotkey("Ctrl+Esc").is_err());
    }

    #[test]
    fn rejects_two_keys() {
        assert!(parse_hotkey("Super+A+B").is_err());
    }

    #[test]
    fn capture_builds_super_shift() {
        let hotkey = hotkey_from_capture(
            CaptureMods {
                super_key: true,
                shift: true,
                ..CaptureMods::default()
            },
            "n",
        )
        .unwrap();
        assert_eq!(hotkey.display, "Super+Shift+N");
        assert_eq!(hotkey.key, "n");
        assert_eq!(hotkey.modifiers, MOD4 | SHIFT);
    }

    #[test]
    fn capture_maps_shift_tab_and_rejects_escape() {
        let tab = hotkey_from_capture(
            CaptureMods {
                shift: true,
                ..CaptureMods::default()
            },
            "ISO_Left_Tab",
        )
        .unwrap();
        assert_eq!(tab.display, "Shift+Tab");
        assert_eq!(tab.key, "Tab");
        assert!(hotkey_from_capture(CaptureMods::default(), "Escape").is_err());
        assert!(is_modifier_key("Super_L"));
        assert!(!is_modifier_key("F9"));
    }

    #[test]
    fn save_config_roundtrip() {
        let dir = std::env::temp_dir().join(format!("flamenote-save-{}", std::process::id()));
        let path = dir.join("config.toml");
        let hotkey = parse_hotkey("Ctrl+Alt+N").unwrap();
        let copies = dir.join("copias");
        let llm = LlmConfig {
            base_url: "http://localhost:11434".into(),
            model: "qwen3:14b".into(),
            api_key: String::new(),
        };
        let queries = vec![
            Query {
                prompt: "Resuma:".into(),
                destination: QueryDestination::File,
            },
            Query {
                prompt: "Traduza:".into(),
                destination: QueryDestination::Clipboard,
            },
        ];
        save_config(&path, &hotkey, &copies, &llm, &queries).unwrap();
        let loaded = load_from(&path);
        assert_eq!(loaded.hotkey, hotkey);
        assert_eq!(loaded.copies_dir, copies);
        assert_eq!(loaded.llm, llm);
        assert_eq!(loaded.queries, queries);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn queries_are_trimmed_and_limited_to_three() {
        let dir = std::env::temp_dir().join(format!("flamenote-queries-{}", std::process::id()));
        let path = dir.join("config.toml");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            &path,
            r#"
hotkey = "Super+Shift+N"
copies_dir = "~/copias"

[llm]
base_url = " http://localhost:11434 "
model = " qwen3:14b "
api_key = ""

[[queries]]
prompt = "  primeira  "
destination = "file"

[[queries]]
prompt = "segunda"

[[queries]]
prompt = "   "

[[queries]]
prompt = "terceira"
destination = "clipboard"

[[queries]]
prompt = "quarta"
"#,
        )
        .unwrap();
        let loaded = load_from(&path);
        assert_eq!(loaded.llm.base_url, "http://localhost:11434");
        assert_eq!(loaded.llm.model, "qwen3:14b");
        assert_eq!(loaded.queries.len(), 3);
        assert_eq!(loaded.queries[0].prompt, "primeira");
        assert_eq!(loaded.queries[0].destination, QueryDestination::File);
        assert_eq!(loaded.queries[1].prompt, "segunda");
        assert_eq!(loaded.queries[1].destination, QueryDestination::Clipboard);
        assert_eq!(loaded.queries[2].prompt, "terceira");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_copies_dir_uses_default() {
        let dir = std::env::temp_dir().join(format!("flamenote-default-{}", std::process::id()));
        let path = dir.join("config.toml");
        fs::create_dir_all(&dir).unwrap();
        fs::write(&path, "hotkey = \"Super+Shift+N\"\n").unwrap();
        let loaded = load_from(&path);
        assert_eq!(loaded.copies_dir, default_copies_dir());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn paths_expand_tilde_and_relative() {
        assert_eq!(expand_path("~/copias"), home_dir().join("copias"));
        assert_eq!(expand_path("copias"), home_dir().join("copias"));
        assert_eq!(expand_path("/tmp/copias"), PathBuf::from("/tmp/copias"));
        assert_eq!(display_path(&home_dir().join("copias")), DEFAULT_COPIES_DIR);
        assert_eq!(display_path(Path::new("/tmp/copias")), "/tmp/copias");
    }
}
