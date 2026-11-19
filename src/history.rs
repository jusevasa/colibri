//! Historial de comandos para la lista que abre ↑ en el prompt. Se lee del archivo del
//! propio shell (zsh, bash o fish), solo en local, y se le suma lo tecleado en esta sesión.

use std::collections::HashMap;
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

/// Un comando del historial y cuándo se usó (segundos Unix), si el shell lo apunta.
#[derive(Clone, PartialEq, Debug)]
pub struct Entry {
    pub cmd: String,
    pub at: Option<u64>,
}

/// Comandos del más reciente al más antiguo, sin repetidos. `typed` (lo tecleado en
/// Colibrí, del más antiguo al más reciente) va primero: zsh puede no haberlo guardado aún.
pub fn load(shell: &str, typed: &[String]) -> Vec<Entry> {
    let mut all: Vec<Entry> = Vec::new();
    if let Some((path, kind)) = source(shell) {
        if let Some(bytes) = tail(&path) {
            all = parse(&bytes, kind);
        }
    }
    all.extend(typed.iter().map(|cmd| Entry {
        cmd: cmd.clone(),
        at: None,
    }));
    // De un comando repetido queda su uso más reciente; si ese no trae fecha (lo tecleado
    // aquí), se le pone la del anterior que sí la tenga.
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut out: Vec<Entry> = Vec::new();
    for entry in all.into_iter().rev() {
        match index.get(&entry.cmd) {
            Some(&i) => {
                if out[i].at.is_none() {
                    out[i].at = entry.at;
                }
            }
            None if out.len() < MAX_ENTRIES => {
                index.insert(entry.cmd.clone(), out.len());
                out.push(entry);
            }
            None => {}
        }
    }
    out
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
fn parse(bytes: &[u8], kind: Kind) -> Vec<Entry> {
    let text = match kind {
        Kind::Zsh => String::from_utf8_lossy(&unmetafy(bytes)).into_owned(),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    };
    let mut out: Vec<Entry> = Vec::new();
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
                let (at, cmd) = split_zsh_stamp(&cmd);
                out.push(Entry {
                    cmd: cmd.to_string(),
                    at,
                });
            }
        }
        Kind::Bash => {
            let stamp = |l: &str| {
                l.len() > 1 && l.starts_with('#') && l[1..].bytes().all(|b| b.is_ascii_digit())
            };
            // Con HISTTIMEFORMAT, cada comando va precedido de una línea «#1700000000».
            let mut at = None;
            for line in text.lines() {
                if stamp(line) {
                    at = line[1..].parse().ok();
                } else {
                    out.push(Entry {
                        cmd: line.to_string(),
                        at: at.take(),
                    });
                }
            }
        }
        Kind::Fish => {
            for line in text.lines() {
                if let Some(cmd) = line.strip_prefix("- cmd: ") {
                    out.push(Entry {
                        cmd: cmd.replace("\\n", "\n").replace("\\\\", "\\"),
                        at: None,
                    });
                } else if let (Some(when), Some(last)) =
                    (line.trim().strip_prefix("when: "), out.last_mut())
                {
                    last.at = when.trim().parse().ok();
                }
            }
        }
    }
    out.retain(|e| !e.cmd.trim().is_empty());
    out
}

/// Separa el prefijo «: 1700000000:0;» del formato extendido de zsh: la fecha y el comando.
fn split_zsh_stamp(line: &str) -> (Option<u64>, &str) {
    let Some(rest) = line.strip_prefix(": ") else {
        return (None, line);
    };
    match rest.split_once(';') {
        Some((stamp, cmd)) if stamp.bytes().all(|b| b.is_ascii_digit() || b == b':') => {
            (stamp.split(':').next().and_then(|n| n.parse().ok()), cmd)
        }
        _ => (None, line),
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

    fn cmds(bytes: &[u8], kind: Kind) -> Vec<String> {
        parse(bytes, kind).into_iter().map(|e| e.cmd).collect()
    }

    #[test]
    fn zsh_extended_and_multiline() {
        let text = ": 1700000000:0;git status\nls -la\n: 1700000005:2;echo uno \\\n  dos\n";
        assert_eq!(
            cmds(text.as_bytes(), Kind::Zsh),
            vec!["git status", "ls -la", "echo uno \n  dos"]
        );
    }

    #[test]
    fn stamps() {
        let zsh = parse(b": 1700000000:0;git status\nls\n", Kind::Zsh);
        assert_eq!((zsh[0].at, zsh[1].at), (Some(1700000000), None));
        let bash = parse(b"#1700000001\nmake\nls\n", Kind::Bash);
        assert_eq!((bash[0].at, bash[1].at), (Some(1700000001), None));
        let fish = parse(b"- cmd: ls\n  when: 1700000002\n", Kind::Fish);
        assert_eq!(fish[0].at, Some(1700000002));
    }

    #[test]
    fn zsh_metafied_bytes() {
        // «ñ» es c3 b1; zsh escribe b1 como 83 91.
        let bytes = [b'e', b'c', b'h', b'o', b' ', 0xc3, 0x83, 0x91, b'\n'];
        assert_eq!(cmds(&bytes, Kind::Zsh), vec!["echo ñ"]);
    }

    #[test]
    fn bash_skips_timestamps() {
        assert_eq!(
            cmds(b"#1700000000\nmake test\n# nota\n", Kind::Bash),
            vec!["make test", "# nota"]
        );
    }

    #[test]
    fn fish_commands() {
        let text = "- cmd: cargo build\n  when: 1700000000\n- cmd: echo a\\nb\n";
        assert_eq!(
            cmds(text.as_bytes(), Kind::Fish),
            vec!["cargo build", "echo a\nb"]
        );
    }
}
