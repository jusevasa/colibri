//! Sugerencias de archivos y carpetas para lo que se está tecleando en el prompt (Tab).
//! Solo mira el disco: no ejecuta nada ni le pregunta al shell.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// Qué es una sugerencia.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Dir,
    File,
    /// Un comando (de la lista del shell), no algo del disco.
    Command,
    Branch,
    CurrentBranch,
    /// Rama de un remoto sin rama local con su nombre (git crea la local al cambiar a ella).
    RemoteBranch,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Item {
    pub name: String,
    pub kind: Kind,
    /// Ramas: fecha (unix) y mensaje de su último commit.
    pub commit: Option<(u64, String)>,
}

impl Item {
    fn new(name: String, kind: Kind) -> Item {
        Item {
            name,
            kind,
            commit: None,
        }
    }

    pub fn is_dir(&self) -> bool {
        self.kind == Kind::Dir
    }

    pub fn is_branch(&self) -> bool {
        matches!(
            self.kind,
            Kind::Branch | Kind::CurrentBranch | Kind::RemoteBranch
        )
    }
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
    /// Es la primera palabra: se ofrecen comandos y carpetas (zsh entra en ellas con
    /// `autocd`), o carpetas y archivos si ya parece una ruta.
    pub command: bool,
}

impl Target {
    /// La palabra entera como nombre, sin separar la carpeta: una rama `feat/x` no es la
    /// entrada `x` de la carpeta `feat/`.
    pub fn whole(mut self) -> Target {
        self.prefix = unescape(&self.token);
        self.typed_dir.clear();
        self
    }
}

/// Decide qué completar. None = mejor que lo haga el shell: la línea está vacía, o la
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
    let start = start.unwrap_or(0);
    let token = &line[start..];
    let first = line[..start].trim().is_empty();
    if first && token.is_empty() {
        return None;
    }
    if token.contains(['"', '\'', '$', '*', '?', '`', '|', ';', '&', '<', '>', '(']) {
        return None;
    }
    let cut = token.rfind('/').map_or(0, |i| i + 1);
    Some(Target {
        token: token.to_string(),
        typed_dir: token[..cut].to_string(),
        prefix: unescape(&token[cut..]),
        dirs_only: !first && matches!(command, "cd" | "pushd" | "rmdir" | "z"),
        command: first,
    })
}

/// Entradas de la carpeta que contienen el prefijo, sin distinguir mayúsculas. Primero las
/// que empiezan por él y, dentro de eso, las carpetas. Las ocultas solo si el prefijo
/// empieza por punto.
///
/// En la primera palabra sin barras: las carpetas y los comandos que empiezan por ella.
pub fn candidates(cwd: &Path, t: &Target) -> Vec<Item> {
    let path_like = !t.typed_dir.is_empty() || t.token.starts_with(['.', '~']);
    if !t.command || path_like {
        return entries(cwd, t, false);
    }
    let needle = t.prefix.to_lowercase();
    let mut items: Vec<Item> = entries(cwd, t, true)
        .into_iter()
        .filter(|i| i.name.to_lowercase().starts_with(&needle))
        .collect();
    items.extend(
        commands()
            .iter()
            .filter(|c| c.starts_with(&t.prefix))
            .map(|c| Item::new(c.clone(), Kind::Command)),
    );
    // Primero lo más corto: con «gi», `git` antes que `git-receive-pack`.
    items.sort_by_cached_key(|i| (i.name.len(), !i.is_dir(), i.name.to_lowercase()));
    items.dedup_by(|a, b| a.name == b.name && a.is_dir() == b.is_dir());
    items.truncate(300);
    items
}

fn entries(cwd: &Path, t: &Target, dirs_only: bool) -> Vec<Item> {
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
            let kind = if is_dir { Kind::Dir } else { Kind::File };
            (is_dir || !(dirs_only || t.dirs_only)).then_some(Item::new(name, kind))
        })
        .collect();
    items.sort_by_cached_key(|i| {
        let lower = i.name.to_lowercase();
        (!lower.starts_with(&needle), !i.is_dir(), lower)
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
    match item.kind {
        Kind::Dir => out.push('/'),
        Kind::File => {}
        // Una orden o una rama ya están completas: lo siguiente es otra palabra.
        _ => out.push(' '),
    }
    out
}

/// Las ramas que contienen `prefix` (sin distinguir mayúsculas) como sugerencias: las locales
/// y, con `remotes`, las de los remotos que no tienen rama local con su nombre, sin el
/// remoto delante. Primero las que empiezan por `prefix`; dentro, la actual y luego de la
/// del commit más reciente a la más antigua.
pub fn branch_items(
    refs: &[crate::git::Ref],
    current: Option<&str>,
    remotes: bool,
    prefix: &str,
) -> Vec<Item> {
    use std::cmp::Reverse;
    let needle = prefix.to_lowercase();
    let locals: std::collections::HashSet<&str> = refs
        .iter()
        .filter(|r| !r.remote)
        .map(|r| r.name.as_str())
        .collect();
    let mut by_time: Vec<&crate::git::Ref> = refs.iter().collect();
    by_time.sort_by_key(|r| Reverse(r.time));
    let mut seen = std::collections::HashSet::new();
    let mut items = Vec::new();
    for r in by_time {
        let (name, kind) = if r.remote {
            match r.name.split_once('/') {
                Some((_, name)) if remotes && !locals.contains(name) => (name, Kind::RemoteBranch),
                _ => continue,
            }
        } else if current == Some(r.name.as_str()) {
            (r.name.as_str(), Kind::CurrentBranch)
        } else {
            (r.name.as_str(), Kind::Branch)
        };
        if !name.to_lowercase().contains(&needle) || !seen.insert(name) {
            continue;
        }
        items.push(Item {
            name: name.to_string(),
            kind,
            commit: Some((r.time, r.subject.clone())),
        });
    }
    // Estable: dentro de cada grupo se conserva el orden por fecha.
    items.sort_by_key(|i| {
        (
            !i.name.to_lowercase().starts_with(&needle),
            i.kind != Kind::CurrentBranch,
        )
    });
    items.truncate(300);
    items
}

/// Lo que se lee de git para una lista de sugerencias: se carga la primera vez que hace
/// falta y se reutiliza mientras la lista sigue abierta (al seguir escribiendo no se vuelve
/// a llamar a git).
#[derive(Clone, Default, PartialEq, Debug)]
pub struct GitData {
    pub refs: Option<Vec<crate::git::Ref>>,
    pub current: Option<String>,
    pub aliases: Option<Vec<(String, String)>>,
}

impl GitData {
    fn load_refs(&mut self, cwd: &Path) {
        if self.refs.is_none() {
            self.refs = Some(crate::git::refs(cwd));
            self.current = crate::git::branch(cwd);
        }
    }

    fn alias(&mut self, cwd: &Path, name: &str) -> Option<String> {
        self.aliases
            .get_or_insert_with(|| crate::git::aliases(cwd))
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, value)| value.clone())
    }
}

/// Qué ofrecer con Tab para `line`: la palabra que se sustituye y las opciones. En una orden
/// de git que pide una rama, las ramas (si alguna coincide); donde pide algo que no es rama
/// ni ruta, nada. `None` o la lista vacía: el Tab es del shell.
pub fn offer(
    line: &str,
    cwd: &Path,
    shell_alias: &dyn Fn(&str) -> Option<String>,
    git: &mut GitData,
) -> Option<(Target, Vec<Item>)> {
    let t = target(line)?;
    if !t.command {
        match git_arg(line, shell_alias, &mut |name| git.alias(cwd, name)) {
            Some(GitArg::Shell) => return None,
            Some(GitArg::Branch { remotes }) => {
                git.load_refs(cwd);
                let whole = t.clone().whole();
                let items = branch_items(
                    git.refs.as_deref().unwrap_or_default(),
                    git.current.as_deref(),
                    remotes,
                    &whole.prefix,
                );
                if !items.is_empty() {
                    return Some((whole, items));
                }
            }
            Some(GitArg::Files) | None => {}
        }
    }
    let items = candidates(cwd, &t);
    Some((t, items))
}

/// Comandos que conoce el shell: los escribe la integración de zsh
/// (`assets/shell/zshenv.zsh`) en cada prompt en que cambia el PATH. Sin ella, los
/// ejecutables del PATH de Colibrí. Se relee solo si el archivo cambió.
fn commands() -> Arc<Vec<String>> {
    static CACHE: Cache<Vec<String>> = Mutex::new(None);
    cached(&CACHE, "commands", |text| {
        let mut list: Vec<String> = match text {
            Some(text) => String::from_utf8_lossy(&text)
                .lines()
                .map(str::to_string)
                .collect(),
            None => std::env::var_os("PATH")
                .map(|p| {
                    std::env::split_paths(&p)
                        .filter_map(|d| std::fs::read_dir(d).ok())
                        .flatten()
                        .flatten()
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default(),
        };
        list.retain(|c| !c.is_empty() && !c.starts_with(['.', '_']) && !c.contains(['/', ' ']));
        list.sort();
        list.dedup();
        list
    })
}

/// Alias del shell y a qué equivalen (`gco` → `git checkout`), tal como los escribe la
/// integración de zsh junto a `commands`. Vacío sin ella.
pub fn shell_aliases() -> Arc<HashMap<String, String>> {
    static CACHE: Cache<HashMap<String, String>> = Mutex::new(None);
    cached(&CACHE, "aliases", |bytes| {
        bytes.map(|b| parse_pairs(&b)).unwrap_or_default()
    })
}

/// Ruta de cada orden tal como la ve el shell (su `$commands`), de la integración de zsh.
/// Vacío sin ella.
pub fn command_paths() -> Arc<HashMap<String, String>> {
    static CACHE: Cache<HashMap<String, String>> = Mutex::new(None);
    cached(&CACHE, "paths", |bytes| {
        bytes.map(|b| parse_pairs(&b)).unwrap_or_default()
    })
}

type Cache<T> = Mutex<Option<(Option<SystemTime>, Arc<T>)>>;

/// Un archivo de `~/Library/Caches/colibri/zsh`, interpretado con `parse` (que recibe `None`
/// si no existe) y vuelto a leer solo cuando cambia su fecha de modificación.
fn cached<T>(cache: &Cache<T>, name: &str, parse: impl FnOnce(Option<Vec<u8>>) -> T) -> Arc<T> {
    let file = dirs::cache_dir().map(|d| d.join("colibri/zsh").join(name));
    let mtime = file
        .as_ref()
        .and_then(|f| std::fs::metadata(f).and_then(|m| m.modified()).ok());
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, value)) = cache.as_ref()
        && *at == mtime
    {
        return value.clone();
    }
    let value = Arc::new(parse(file.as_ref().and_then(|f| std::fs::read(f).ok())));
    *cache = Some((mtime, value.clone()));
    value
}

/// Pares nombre/valor separados (y terminados) por NUL, como los deja `print -rN`.
fn parse_pairs(bytes: &[u8]) -> HashMap<String, String> {
    let bytes = bytes.strip_suffix(&[0]).unwrap_or(bytes);
    let mut parts = bytes.split(|b| *b == 0);
    let mut out = HashMap::new();
    while let (Some(name), Some(value)) = (parts.next(), parts.next()) {
        if !name.is_empty() {
            out.insert(
                String::from_utf8_lossy(name).into_owned(),
                String::from_utf8_lossy(value).into_owned(),
            );
        }
    }
    out
}

/// Qué pide el argumento que se escribe dentro de una orden de git.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum GitArg {
    /// Una rama; `remotes`: también las remotas sin rama local (checkout y switch).
    Branch { remotes: bool },
    /// Algo que no es rama ni ruta (un nombre nuevo, un remoto): el Tab es del shell.
    Shell,
    /// Rutas (tras `--`): como fuera de git.
    Files,
}

/// La orden en la que está el cursor: lo que sigue al último `|`, `||`, `&&`, `&` o `;`.
pub fn last_command(line: &str) -> &str {
    let cut = line.rfind(['|', '&', ';']).map_or(0, |i| i + 1);
    &line[cut..]
}

/// Sustituye la primera palabra por su alias, hasta 3 veces (alias de alias). Para si la
/// expansión empieza por el mismo nombre (`ls='ls -G'`).
pub fn expand_alias(command: &str, alias: &dyn Fn(&str) -> Option<String>) -> String {
    let mut line = command.trim_start().to_string();
    for _ in 0..3 {
        let first = line.split([' ', '\t']).next().unwrap_or("").to_string();
        let Some(value) = alias(&first) else {
            break;
        };
        let again = value.split_whitespace().next() == Some(first.as_str());
        line = format!("{value}{}", &line[first.len()..]);
        if again {
            break;
        }
    }
    line
}

/// Palabras separadas por espacios sin escapar, tal como se escribieron. Si la línea acaba
/// en espacio, la última es vacía (la que se empieza a escribir).
fn split_words(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut escaped = false;
    for (i, c) in line.char_indices() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == ' ' || c == '\t' {
            if i > start {
                out.push(&line[start..i]);
            }
            start = i + 1;
        }
    }
    out.push(&line[start..]);
    out
}

/// Subórdenes de git que reciben ramas (o que deciden qué recibe su argumento).
const GIT_BRANCH_COMMANDS: [&str; 13] = [
    "checkout",
    "switch",
    "merge",
    "rebase",
    "diff",
    "log",
    "show",
    "cherry-pick",
    "reset",
    "branch",
    "push",
    "pull",
    "fetch",
];

/// Qué pide la última palabra de `line` si es un argumento de una orden de git que admite
/// ramas, siguiendo los alias del shell (`gco`) y los de git (`git co`). `None`: no es una
/// de esas órdenes, o lo que se escribe es la propia suborden.
pub fn git_arg(
    line: &str,
    shell_alias: &dyn Fn(&str) -> Option<String>,
    git_alias: &mut dyn FnMut(&str) -> Option<String>,
) -> Option<GitArg> {
    let expanded = expand_alias(last_command(line), shell_alias);
    let mut words = split_words(&expanded);
    words.pop()?;
    let mut it = words.into_iter();
    if it.next()? != "git" {
        return None;
    }
    // Opciones generales de git antes de la suborden: `-C <carpeta>`, `-c <clave=valor>`…
    let mut sub = None;
    while let Some(w) = it.next() {
        if w == "-C" || w == "-c" {
            it.next();
        } else if !w.starts_with('-') {
            sub = Some(w);
            break;
        }
    }
    let sub = sub?;
    let (sub, mut args): (String, Vec<String>) = if GIT_BRANCH_COMMANDS.contains(&sub) {
        (sub.to_string(), Vec::new())
    } else {
        let value = git_alias(sub)?;
        if value.starts_with('!') {
            return None;
        }
        let mut parts = value.split_whitespace().map(str::to_string);
        (parts.next()?, parts.collect())
    };
    args.extend(it.map(str::to_string));
    if args.iter().any(|a| a == "--") {
        return Some(GitArg::Files);
    }
    let positional = args.iter().filter(|a| !a.starts_with('-')).count();
    Some(match sub.as_str() {
        "checkout" | "switch" => {
            let new_name = args
                .last()
                .is_some_and(|a| matches!(a.as_str(), "-b" | "-B" | "-c" | "-C" | "--orphan"));
            if new_name {
                GitArg::Shell
            } else {
                GitArg::Branch { remotes: true }
            }
        }
        "merge" | "rebase" | "diff" | "log" | "show" | "cherry-pick" | "reset" => {
            GitArg::Branch { remotes: false }
        }
        "branch" => {
            let existing = args.iter().any(|a| {
                matches!(
                    a.as_str(),
                    "-d" | "-D" | "-m" | "-M" | "--delete" | "--move"
                )
            });
            if existing || positional > 0 {
                GitArg::Branch { remotes: false }
            } else {
                GitArg::Shell
            }
        }
        "push" | "pull" | "fetch" => {
            if positional == 0 {
                GitArg::Shell
            } else {
                GitArg::Branch { remotes: false }
            }
        }
        _ => return None,
    })
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
    fn contexto_de_git() {
        let sh = |n: &str| (n == "gco").then(|| "git checkout".to_string());
        let mut git = |n: &str| match n {
            "co" => Some("checkout".to_string()),
            "sync" => Some("!git pull".to_string()),
            _ => None,
        };
        let b = |remotes| Some(GitArg::Branch { remotes });
        assert_eq!(git_arg("gco f", &sh, &mut git), b(true));
        assert_eq!(git_arg("git checkout ", &sh, &mut git), b(true));
        assert_eq!(
            git_arg("git switch -c ", &sh, &mut git),
            Some(GitArg::Shell)
        );
        // Tras el nombre de la rama nueva va su punto de partida: otra rama.
        assert_eq!(git_arg("git checkout -b nueva ", &sh, &mut git), b(true));
        assert_eq!(git_arg("git merge ma", &sh, &mut git), b(false));
        assert_eq!(git_arg("git branch -D fe", &sh, &mut git), b(false));
        assert_eq!(git_arg("git branch ", &sh, &mut git), Some(GitArg::Shell));
        assert_eq!(git_arg("git branch nueva ", &sh, &mut git), b(false));
        assert_eq!(git_arg("git push ", &sh, &mut git), Some(GitArg::Shell));
        assert_eq!(git_arg("git push origin fe", &sh, &mut git), b(false));
        assert_eq!(
            git_arg("git checkout -- src/ma", &sh, &mut git),
            Some(GitArg::Files)
        );
        assert_eq!(git_arg("git co fe", &sh, &mut git), b(true));
        assert_eq!(git_arg("git -C ../otro checkout f", &sh, &mut git), b(true));
        assert_eq!(git_arg("make && gco f", &sh, &mut git), b(true));
        // Un alias de git que es una orden del shell no se sigue.
        assert_eq!(git_arg("git sync fe", &sh, &mut git), None);
        // Se está escribiendo la suborden.
        assert_eq!(git_arg("git ch", &sh, &mut git), None);
        assert_eq!(git_arg("git status ", &sh, &mut git), None);
        assert_eq!(git_arg("ls f", &sh, &mut git), None);
    }

    #[test]
    fn alias_que_empieza_por_si_mismo() {
        let a = |n: &str| (n == "ls").then(|| "ls -G".to_string());
        assert_eq!(expand_alias("ls x", &a), "ls -G x");
        assert_eq!(last_command("make && gco f"), " gco f");
        assert_eq!(last_command("gco f"), "gco f");
    }

    fn rama(name: &str, remote: bool, time: u64) -> crate::git::Ref {
        crate::git::Ref {
            name: name.into(),
            remote,
            time,
            subject: format!("msg {name}"),
        }
    }

    /// Locales main, feat-a, feat-b; remotas feat-a (ya local) y feat-c en dos remotos.
    fn refs() -> Vec<crate::git::Ref> {
        vec![
            rama("main", false, 300),
            rama("feat-a", false, 200),
            rama("feat-b", false, 100),
            rama("origin/feat-a", true, 200),
            rama("origin/feat-c", true, 250),
            rama("upstream/feat-c", true, 240),
        ]
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("colibri-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn ramas_filtradas_y_ordenadas() {
        let names = |v: Vec<Item>| v.into_iter().map(|i| (i.name, i.kind)).collect::<Vec<_>>();
        assert_eq!(
            names(branch_items(&refs(), Some("feat-a"), true, "F")),
            [
                ("feat-a".to_string(), Kind::CurrentBranch),
                ("feat-c".to_string(), Kind::RemoteBranch),
                ("feat-b".to_string(), Kind::Branch),
            ]
        );
        // «a» no empieza ninguna: todas las que la contienen, la actual primero y luego por
        // fecha.
        let n: Vec<String> = branch_items(&refs(), Some("feat-a"), false, "a")
            .into_iter()
            .map(|i| i.name)
            .collect();
        assert_eq!(n, ["feat-a", "main", "feat-b"]);
        // Dos remotos con la misma rama: una sola, la más reciente.
        let c = branch_items(&refs(), None, true, "feat-c");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].commit, Some((250, "msg origin/feat-c".to_string())));
    }

    #[test]
    fn oferta_de_ramas_y_vuelta_a_archivos() {
        let dir = temp_dir("oferta");
        std::fs::write(dir.join("zz.txt"), "").unwrap();
        let mut g = GitData {
            refs: Some(refs()),
            current: Some("main".into()),
            aliases: Some(vec![]),
        };
        let none = |_: &str| None;
        let (t, items) = offer("git checkout fe", &dir, &none, &mut g).unwrap();
        assert_eq!((t.token.as_str(), t.typed_dir.as_str()), ("fe", ""));
        assert!(!items.is_empty() && items.iter().all(Item::is_branch));
        let (_, items) = offer("git checkout zz", &dir, &none, &mut g).unwrap();
        assert_eq!(items[0].name, "zz.txt");
        assert!(offer("git switch -c ", &dir, &none, &mut g).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn oferta_sin_repo_da_archivos() {
        let dir = temp_dir("sin-repo");
        std::fs::write(dir.join("zz.txt"), "").unwrap();
        let mut g = GitData {
            refs: Some(vec![]),
            current: None,
            aliases: Some(vec![]),
        };
        let (_, items) = offer("git merge zz", &dir, &|_| None, &mut g).unwrap();
        assert_eq!(items[0].kind, Kind::File);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn oferta_rama_con_barra() {
        let dir = temp_dir("barra");
        let mut g = GitData {
            refs: Some(vec![rama("feat/x", false, 10)]),
            current: None,
            aliases: Some(vec![]),
        };
        let (t, items) = offer("git checkout feat/", &dir, &|_| None, &mut g).unwrap();
        assert_eq!(
            (t.typed_dir.as_str(), items[0].name.as_str()),
            ("", "feat/x")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reemplazo_de_rama_escapado() {
        let t = target("git checkout fi").unwrap().whole();
        let item = Item {
            name: "fix(ui)".into(),
            kind: Kind::Branch,
            commit: None,
        };
        assert_eq!(replacement(&t, &item), "fix\\(ui\\) ");
    }

    #[test]
    fn pares_separados_por_nul() {
        let m = parse_pairs(b"gco\0git checkout\0ll\0ls -l\0");
        assert_eq!(m.get("gco").map(String::as_str), Some("git checkout"));
        assert_eq!(m.len(), 2);
        assert!(parse_pairs(b"suelto\0").is_empty());
    }

    #[test]
    fn target_from_line() {
        // La primera palabra: comandos y carpetas.
        let t = target("gi").unwrap();
        assert!(t.command && !t.dirs_only);
        assert_eq!(t.prefix, "gi");
        assert_eq!(target(""), None);
        assert_eq!(target("  "), None);
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

        // Primera palabra: carpetas que empiezan por ella (autocd), no las que la contienen.
        let t = target("integ").unwrap();
        let found = candidates(&dir, &t);
        assert!(
            found
                .iter()
                .any(|i| i.name == "integrations-logs" && i.is_dir())
        );
        assert!(!found.iter().any(|i| i.name == "delivery integrations"));
        // …y dentro de una carpeta escrita como orden, su contenido.
        std::fs::create_dir_all(dir.join("integrations-logs/src")).unwrap();
        let t = target("integrations-logs/").unwrap();
        let names: Vec<String> = candidates(&dir, &t).into_iter().map(|i| i.name).collect();
        assert_eq!(names, ["src"]);
        let cmd = Item::new("git".into(), Kind::Command);
        assert_eq!(replacement(&target("gi").unwrap(), &cmd), "git ");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
