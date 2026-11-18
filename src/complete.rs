//! Sugerencias de archivos y carpetas para lo que se está tecleando en el prompt (Tab).
//! Solo mira el disco: no ejecuta nada ni le pregunta al shell.

use std::path::{Path, PathBuf};

#[derive(Clone, PartialEq, Debug)]
pub struct Item {
    pub name: String,
    pub is_dir: bool,
}

/// Lo que hay que completar en una línea del prompt.
#[derive(Clone, PartialEq, Debug)]
pub struct Target {
    /// La última palabra tal como está escrita ("src/ma", "mi\ car").
    pub token: String,
    /// Parte de la palabra que es carpeta, tal como está escrita ("src/"); se conserva.
    pub typed_dir: String,
    /// Lo que debe aparecer en el nombre ("ma"), ya sin barras de escape.
    pub prefix: String,
    /// `cd` y similares: solo interesan las carpetas.
    pub dirs_only: bool,
}

/// Decide qué completar. None = mejor que lo haga el shell: aún se escribe el comando, o la
/// palabra lleva comillas, variables o comodines que aquí no se interpretan.
pub fn target(line: &str) -> Option<Target> {
    let command = line.split_whitespace().next()?;
    // Corta en el último espacio sin escapar.
    let mut start = None;
    let mut escaped = false;
    for (i, c) in line.char_indices() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == ' ' {
            start = Some(i + 1);
        }
    }
    let token = &line[start?..];
    if token.contains(['"', '\'', '$', '*', '?', '`', '|', ';', '&', '<', '>', '(']) {
        return None;
    }
    let cut = token.rfind('/').map_or(0, |i| i + 1);
    Some(Target {
        token: token.to_string(),
        typed_dir: token[..cut].to_string(),
        prefix: unescape(&token[cut..]),
        dirs_only: matches!(command, "cd" | "pushd" | "rmdir" | "z"),
    })
}

/// Entradas de la carpeta que contienen el prefijo, sin distinguir mayúsculas. Primero las
/// que empiezan por él y, dentro de eso, las carpetas. Las ocultas solo si el prefijo
/// empieza por punto.
pub fn candidates(cwd: &Path, t: &Target) -> Vec<Item> {
    let dir = unescape(&t.typed_dir);
    let base: PathBuf = if let Some(rest) = dir.strip_prefix("~/") {
        match dirs::home_dir() {
            Some(home) => home.join(rest),
            None => return Vec::new(),
        }
    } else if dir.starts_with('/') {
        PathBuf::from(&dir)
    } else {
        cwd.join(&dir)
    };
    let Ok(entries) = std::fs::read_dir(&base) else {
        return Vec::new();
    };
    let needle = t.prefix.to_lowercase();
    let mut items: Vec<Item> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') && !needle.starts_with('.') {
                return None;
            }
            if !name.to_lowercase().contains(&needle) {
                return None;
            }
            // Sigue los enlaces: un enlace a una carpeta cuenta como carpeta.
            let is_dir = e.path().is_dir();
            (is_dir || !t.dirs_only).then_some(Item { name, is_dir })
        })
        .collect();
    items.sort_by_cached_key(|i| {
        let lower = i.name.to_lowercase();
        (!lower.starts_with(&needle), !i.is_dir, lower)
    });
    items.truncate(300);
    items
}

/// Texto que sustituye a la palabra: la carpeta tal como se escribió + el nombre con sus
/// caracteres especiales escapados (+ `/` si es carpeta, para seguir completando dentro).
pub fn replacement(t: &Target, item: &Item) -> String {
    let mut out = t.typed_dir.clone();
    for c in item.name.chars() {
        if " \\'\"$&;()<>|*?[]!#`{}~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    if item.is_dir {
        out.push('/');
    }
    out
}

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            out.extend(chars.next());
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_from_line() {
        // Todavía se escribe el comando: lo completa el shell.
        assert_eq!(target("gi"), None);
        assert_eq!(target(""), None);
        let t = target("cd integ").unwrap();
        assert_eq!(
            (t.token.as_str(), t.prefix.as_str(), t.dirs_only),
            ("integ", "integ", true)
        );
        let t = target("cat src/ma").unwrap();
        assert_eq!(
            (t.typed_dir.as_str(), t.prefix.as_str(), t.dirs_only),
            ("src/", "ma", false)
        );
        // Espacio escapado: sigue siendo la misma palabra.
        let t = target("ls mi\\ car").unwrap();
        assert_eq!(
            (t.token.as_str(), t.prefix.as_str()),
            ("mi\\ car", "mi car")
        );
        // Tras un espacio, palabra vacía: se ofrece todo.
        assert_eq!(target("cd ").unwrap().prefix, "");
        assert_eq!(target("echo \"ho"), None);
        assert_eq!(target("ls $HOME/x"), None);
    }

    #[test]
    fn candidates_and_replacement() {
        let dir = std::env::temp_dir().join(format!("colibri-complete-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("integrations-logs")).unwrap();
        std::fs::create_dir_all(dir.join("delivery integrations")).unwrap();
        std::fs::create_dir_all(dir.join(".oculta-integ")).unwrap();
        std::fs::write(dir.join("integ.txt"), "").unwrap();

        let t = target("cd integ").unwrap();
        let names: Vec<String> = candidates(&dir, &t).into_iter().map(|i| i.name).collect();
        // Solo carpetas; primero la que empieza por el prefijo; la oculta no sale.
        assert_eq!(names, ["integrations-logs", "delivery integrations"]);

        let t = target("cat integ").unwrap();
        let all = candidates(&dir, &t);
        assert_eq!(all.len(), 3);
        assert_eq!(all[1].name, "integ.txt");
        assert_eq!(replacement(&t, &all[0]), "integrations-logs/");
        assert_eq!(replacement(&t, &all[2]), "delivery\\ integrations/");

        let t = target("ls integrations-logs/").unwrap();
        assert!(candidates(&dir, &t).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
