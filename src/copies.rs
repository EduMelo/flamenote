//! Grava a seleção como arquivo texto, no espírito de `copiar_selecao.sh`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const MAX_NAME_CHARS: usize = 30;
const FALLBACK_NAME: &str = "sem_nome";

pub fn save(text: &str, dir: &Path) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.txt", sanitize_name(text)));
    let mut body = String::with_capacity(text.len() + 1);
    body.push_str(text);
    body.push('\n');
    fs::write(&path, body)?;
    Ok(path)
}

pub fn sanitize_name(text: &str) -> String {
    let head: String = text.chars().take(MAX_NAME_CHARS).collect();
    let name: String = head
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | ' '))
        .map(|ch| if ch == ' ' { '_' } else { ch })
        .collect();
    if name.is_empty() {
        FALLBACK_NAME.to_string()
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_like_the_script() {
        assert_eq!(sanitize_name("olá mundo / teste"), "ol_mundo__teste");
        assert_eq!(sanitize_name("///"), FALLBACK_NAME);
        assert_eq!(sanitize_name("  "), "__");
        assert_eq!(sanitize_name("a-b_c"), "a-b_c");
    }

    #[test]
    fn takes_at_most_thirty_characters_of_the_selection() {
        let name = sanitize_name(&"x".repeat(80));
        assert_eq!(name.chars().count(), MAX_NAME_CHARS);
    }

    #[test]
    fn writes_the_selection_with_a_trailing_newline() {
        let dir = std::env::temp_dir().join(format!("flamenote-copies-{}", std::process::id()));
        let path = save("primeira linha\nsegunda", &dir).unwrap();
        assert_eq!(path, dir.join("primeira_linhasegunda.txt"));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "primeira linha\nsegunda\n"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
