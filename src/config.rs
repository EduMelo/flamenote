//! Atalho global e leitura de `config.toml`.

use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const DEFAULT_HOTKEY: &str = "Super+Shift+N";

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

#[derive(Clone, Debug)]
pub struct Config {
    pub path: PathBuf,
    pub hotkey: Hotkey,
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

#[derive(Deserialize)]
struct FileConfig {
    hotkey: Option<String>,
}

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
    let hotkey = match fs::read_to_string(path) {
        Ok(text) => match toml::from_str::<FileConfig>(&text) {
            Ok(file) => match file.hotkey {
                Some(raw) => match parse_hotkey(&raw) {
                    Ok(hotkey) => hotkey,
                    Err(err) => {
                        eprintln!(
                            "flamenote: {err}. Usando {DEFAULT_HOTKEY}. Arquivo: {}",
                            path.display()
                        );
                        parse_hotkey(DEFAULT_HOTKEY).expect("atalho padrão")
                    }
                },
                None => parse_hotkey(DEFAULT_HOTKEY).expect("atalho padrão"),
            },
            Err(err) => {
                eprintln!(
                    "flamenote: config inválida ({err}). Usando {DEFAULT_HOTKEY}. Arquivo: {}",
                    path.display()
                );
                parse_hotkey(DEFAULT_HOTKEY).expect("atalho padrão")
            }
        },
        Err(err) => {
            eprintln!(
                "flamenote: não li {} ({err}). Usando {DEFAULT_HOTKEY}.",
                path.display()
            );
            parse_hotkey(DEFAULT_HOTKEY).expect("atalho padrão")
        }
    };
    Config {
        path: path.to_path_buf(),
        hotkey,
        modified,
    }
}

fn write_default(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let body = format!(
        "# Atalho global do flamenote.\n\
         # Modificadores: Ctrl, Alt, Shift, Super.\n\
         # Exemplos: \"Super+Shift+N\", \"Ctrl+Alt+N\", \"F9\"\n\
         hotkey = \"{DEFAULT_HOTKEY}\"\n"
    );
    fs::write(path, body)
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
}
