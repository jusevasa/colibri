//! Historial de comandos para la lista que abre ↑ en el prompt. Se lee del archivo del
//! propio shell (zsh, bash o fish), solo en local, y se le suma lo tecleado en esta sesión.

use std::collections::HashSet;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

/// Basta con el final del archivo: ahí están los comandos recientes.
const TAIL_BYTES: u64 = 1_048_576;
const MAX_ENTRIES: usize = 3000;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Zsh,
    Bash,
    Fish,
}

/// Comandos del más reciente al más antiguo, sin repetidos. `typed` (lo tecleado en
/// Colibrí, del más antiguo al más reciente) va primero: zsh puede no haberlo guardado aún.
pub fn load(shell: &str, typed: &[String]) -> Vec<String> {
    let mut all: Vec<String> = Vec::new();
    if let Some((path, kind)) = source(shell) {
        if let Some(bytes) = tail(&path) {
            all = parse(&bytes, kind);
        }
    }
    all.extend(typed.iter().cloned());
    let mut seen = HashSet::new();
    all.into_iter()
        .rev()
        .filter(|c| seen.insert(c.clone()))
        .take(MAX_ENTRIES)
        .collect()
}

fn source(shell: &str) -> Option<(PathBuf, Kind)> {
    let name = shell.rsplit('/').next().unwrap_or(shell);
    let kind = match name {
        "bash" | "sh" => Kind::Bash,
        "fish" => Kind::Fish,
        _ => Kind::Zsh,
    };
    if let Some(p) = std::env::var_os("HISTFILE").filter(|p| !p.is_empty()) {
        return Some((PathBuf::from(p), kind));
    }
    let home = dirs::home_dir()?;
    Some(match kind {
        Kind::Zsh => {
            let dir = std::env::var_os("ZDOTDIR")
                .map(PathBuf::from)
                .unwrap_or(home);
            (dir.join(".zsh_history"), kind)
        }
        Kind::Bash => (home.join(".bash_history"), kind),
        Kind::Fish => (home.join(".local/share/fish/fish_history"), kind),
    })
}

fn tail(path: &PathBuf) -> Option<Vec<u8>> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let cut = len > TAIL_BYTES;
    if cut {
        file.seek(SeekFrom::Start(len - TAIL_BYTES)).ok()?;
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    if cut {
        // El corte cae a mitad de una línea: se descarta ese trozo.
        let first = bytes.iter().position(|b| *b == b'\n').map_or(0, |i| i + 1);
        bytes.drain(..first);
    }
    Some(bytes)
}

/// Comandos en el orden del archivo (el más antiguo primero).
fn parse(bytes: &[u8], kind: Kind) -> Vec<String> {
    let text = match kind {
        Kind::Zsh => String::from_utf8_lossy(&unmetafy(bytes)).into_owned(),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    };
    let mut out = Vec::new();
    match kind {
        Kind::Zsh => {
            let mut lines = text.lines();
            while let Some(line) = lines.next() {
                // Un comando de varias líneas se guarda con «\» al final de cada una.
                let mut cmd = line.to_string();
                while cmd.ends_with('\\') {
                    let Some(next) = lines.next() else { break };
                    cmd.pop();
                    cmd.push('\n');
                    cmd.push_str(next);
                }
                out.push(strip_zsh_stamp(&cmd).to_string());
            }
        }
        Kind::Bash => {
            let stamp = |l: &str| {
                l.len() > 1 && l.starts_with('#') && l[1..].bytes().all(|b| b.is_ascii_digit())
            };
            out.extend(text.lines().filter(|l| !stamp(l)).map(str::to_string));
        }
        Kind::Fish => {
            for line in text.lines() {
                if let Some(cmd) = line.strip_prefix("- cmd: ") {
                    out.push(cmd.replace("\\n", "\n").replace("\\\\", "\\"));
                }
            }
        }
    }
    out.retain(|c| !c.trim().is_empty());
    out
}

/// Quita el prefijo «: 1700000000:0;» del formato extendido de zsh.
fn strip_zsh_stamp(line: &str) -> &str {
    let Some(rest) = line.strip_prefix(": ") else {
        return line;
    };
    match rest.split_once(';') {
        Some((stamp, cmd)) if stamp.bytes().all(|b| b.is_ascii_digit() || b == b':') => cmd,
        _ => line,
    }
}

/// zsh guarda ciertos bytes (los de tildes y ñ entre ellos) como 0x83 + (byte ^ 0x20).
fn unmetafy(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut it = bytes.iter();
    while let Some(&b) = it.next() {
        if b == 0x83 {
            if let Some(&next) = it.next() {
                out.push(next ^ 0x20);
            }
        } else {
            out.push(b);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zsh_extended_and_multiline() {
        let text = ": 1700000000:0;git status\nls -la\n: 1700000005:2;echo uno \\\n  dos\n";
        assert_eq!(
            parse(text.as_bytes(), Kind::Zsh),
            vec!["git status", "ls -la", "echo uno \n  dos"]
        );
    }

    #[test]
    fn zsh_metafied_bytes() {
        // «ñ» es c3 b1; zsh escribe b1 como 83 91.
        let bytes = [b'e', b'c', b'h', b'o', b' ', 0xc3, 0x83, 0x91, b'\n'];
        assert_eq!(parse(&bytes, Kind::Zsh), vec!["echo ñ"]);
    }

    #[test]
    fn bash_skips_timestamps() {
        assert_eq!(
            parse(b"#1700000000\nmake test\n# nota\n", Kind::Bash),
            vec!["make test", "# nota"]
        );
    }

    #[test]
    fn fish_commands() {
        let text = "- cmd: cargo build\n  when: 1700000000\n- cmd: echo a\\nb\n";
        assert_eq!(
            parse(text.as_bytes(), Kind::Fish),
            vec!["cargo build", "echo a\nb"]
        );
    }
}
