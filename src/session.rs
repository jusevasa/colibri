//! Una pestaña: el emulador (alacritty_terminal), su PTY y el estado del agente que corre dentro.

use crate::config::{Config, Palette};
use crate::proc;
use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::{Dimensions, Grid, Row, Scroll};
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::{Cell, Flags, Hyperlink};
use alacritty_terminal::term::{Config as TermConfig, Term};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::Rgb;
use std::borrow::Cow;
use std::collections::HashMap;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Marcas (carácter de ancho cero en la primera celda de cada dato) de una línea de cajas.
pub const CHIP_DIR: char = '\u{E0D0}';
pub const CHIP_BRANCH: char = '\u{E0D1}';
pub const CHIP_CHANGES: char = '\u{E0D2}';
/// Prompt de una máquina remota (ssh), sellado en su propia fila: «usuario@máquina», un
/// hueco de relleno al principio y la última celda del prompt (ahí empieza el comando).
pub const CHIP_HOST: char = '\u{E0D3}';
const CHIP_PAD: char = '\u{E0D4}';
const CHIP_END: char = '\u{E0D5}';

/// Trozo de un comando que se colorea al escribir en una máquina remota.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tok {
    Command,
    Flag,
    Str,
    /// Una ruta que existe, en el comando de un prompt local (`Session::path_tokens`).
    Path,
}

/// Colorea un comando por encima: la orden (y la que siga a `|`, `&&`, `;`), las opciones
/// y las cadenas entre comillas. Devuelve tramos [inicio, fin) en caracteres.
pub fn tokenize(text: &[char]) -> Vec<(usize, usize, Tok)> {
    let mut out = Vec::new();
    let mut i = 0;
    let mut command = true;
    while i < text.len() {
        let c = text[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        if c == '\'' || c == '"' {
            i += 1;
            while i < text.len() && text[i] != c {
                i += if text[i] == '\\' && c == '"' { 2 } else { 1 };
            }
            i = (i + 1).min(text.len());
            out.push((start, i, Tok::Str));
            command = false;
            continue;
        }
        while i < text.len() && !text[i].is_whitespace() && !"'\"".contains(text[i]) {
            i += 1;
        }
        let word: String = text[start..i].iter().collect();
        if matches!(word.as_str(), "|" | "||" | "&&" | ";" | "&") {
            command = true;
        } else if command {
            // «VAR=valor orden» y «sudo orden»: la orden es lo siguiente.
            let prefix = word.contains('=') || matches!(word.as_str(), "sudo" | "env" | "time");
            out.push((start, i, Tok::Command));
            command = prefix;
        } else if word.starts_with('-') && word.len() > 1 {
            out.push((start, i, Tok::Flag));
        }
    }
    out
}

/// Lo que decide las rutas coloreadas de una pestaña (ver `Session::paths_memo`).
type PathsKey = (u64, usize, (usize, usize), Point, bool);

/// Un trozo de comando a colorear en pantalla: fila, columna inicial, final exclusiva y qué es.
pub type TokSpan = (i32, usize, usize, Tok);

/// El comando de un prompt sacado de la rejilla: la carpeta de su marca, el texto, dónde
/// está cada carácter (fila, columna) y hasta dónde mirar (el cursor, en el que se escribe).
type PromptCommand = (PathBuf, Vec<char>, Vec<(i32, usize)>, Option<usize>);

/// Un tramo [from, to) de un texto sacado de la rejilla, en celdas de pantalla: uno por fila
/// (fila, columna inicial, final exclusiva). `at` dice dónde está cada carácter del texto.
fn split_rows(at: &[(i32, usize)], from: usize, to: usize) -> Vec<(i32, usize, usize)> {
    let mut out = Vec::new();
    let mut start = from;
    for i in from..=to.min(at.len()) {
        if i == to.min(at.len()) || at[i].0 != at[start].0 {
            if i > start {
                out.push((at[start].0, at[start].1, at[i - 1].1 + 1));
            }
            start = i;
        }
    }
    out
}

/// Palabras de un comando que pueden ser rutas: tramo [inicio, fin) en caracteres, la
/// palabra ya sin escapes ni comillas y si basta con que sea el principio de algo que existe
/// (la palabra en la que está el cursor). Quien llama mira el disco con `path_exists`.
///
/// No cuentan la orden (la primera palabra de cada orden, salvo que parezca una ruta: `./x`,
/// `~/x`, `carpeta/`), las opciones, las asignaciones (`A=1`), los redirectores ni las
/// palabras con variables, comodines o sustituciones. Con `upto` (el cursor del prompt que
/// se escribe) no se mira lo que hay detrás: puede ser la sugerencia en gris de
/// zsh-autosuggestions, que está en las celdas como si se hubiera tecleado.
pub fn path_words(text: &[char], upto: Option<usize>) -> Vec<(usize, usize, String, bool)> {
    let limit = upto.unwrap_or(text.len()).min(text.len());
    let mut out = Vec::new();
    let mut first = true;
    let mut i = 0;
    while i < text.len() {
        if text[i].is_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        // Hasta el siguiente espacio fuera de comillas y sin escapar.
        let mut quote = None;
        while i < text.len() {
            let c = text[i];
            match quote {
                Some(q) if c == q => quote = None,
                Some('"') if c == '\\' => i += 1,
                Some(_) => {}
                None if c == '\\' => i += 1,
                None if c == '\'' || c == '"' => quote = Some(c),
                None if c.is_whitespace() => break,
                None => {}
            }
            i += 1;
        }
        let end = i.min(text.len());
        if start >= limit {
            break;
        }
        let cut = end.min(limit);
        let typing = upto.is_some() && end >= limit;
        let raw: String = text[start..end].iter().collect();
        if matches!(raw.as_str(), "|" | "||" | "&&" | ";" | "&") {
            first = true;
            continue;
        }
        if matches!(raw.as_str(), ">" | ">>" | "<" | "2>" | "2>>" | "&>") || raw.starts_with('-') {
            first = false;
            continue;
        }
        let Some(word) = shell_word(&text[start..cut]) else {
            // Asignación delante de la orden: lo siguiente sigue siendo la orden.
            first = first && raw.contains('=');
            continue;
        };
        let was_first = std::mem::replace(&mut first, false);
        if word.is_empty() || (was_first && !(word.contains('/') || word.starts_with(['.', '~']))) {
            continue;
        }
        out.push((start, cut, word, typing));
        if typing {
            break;
        }
    }
    out
}

/// Una palabra del shell sin escapes ni comillas; `None` si lleva lo que el shell expandiría
/// (variables, comodines, sustituciones) o un `=` (asignación, `--opción=valor`).
fn shell_word(chars: &[char]) -> Option<String> {
    let mut out = String::new();
    let mut quote = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match quote {
            Some(q) if c == q => quote = None,
            Some('\'') => out.push(c),
            Some(_) if c == '\\' => {
                i += 1;
                out.extend(chars.get(i));
            }
            Some(_) if c == '$' || c == '`' => return None,
            Some(_) => out.push(c),
            None if c == '\\' => {
                i += 1;
                out.extend(chars.get(i));
            }
            None if c == '\'' || c == '"' => quote = Some(c),
            None if "$*?`({[=<>|;&".contains(c) => return None,
            None => out.push(c),
        }
        i += 1;
    }
    Some(out)
}

/// `word` (ya sin escapes) es una ruta que existe, relativa a `dir` si no es absoluta ni
/// empieza por `~`. Con `prefix` basta con que sea el principio del nombre de una entrada.
pub fn path_exists(dir: &Path, word: &str, prefix: bool) -> bool {
    let home = || dirs::home_dir().unwrap_or_default();
    let path = if word == "~" {
        home()
    } else if let Some(rest) = word.strip_prefix("~/") {
        home().join(rest)
    } else if word.starts_with('/') {
        PathBuf::from(word)
    } else {
        dir.join(word)
    };
    if !prefix || word == "~" || word.ends_with('/') {
        return path.exists();
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => {
            let name = name.to_string_lossy();
            std::fs::read_dir(parent).is_ok_and(|entries| {
                entries
                    .flatten()
                    .any(|e| e.file_name().to_string_lossy().starts_with(&*name))
            })
        }
        _ => path.exists(),
    }
}

/// Un prompt remoto reconocido en una línea: la máquina y la carpeta que nombra (si las
/// nombra) y cuántos caracteres ocupa (hasta donde empieza el comando).
#[derive(PartialEq, Debug)]
pub struct RemotePrompt {
    pub host: Option<String>,
    pub dir: Option<String>,
    pub len: usize,
}

const PROMPT_SYMBOLS: &str = "$#%>❯➜λ";

/// Reconoce los prompts habituales de un servidor:
/// - `usuario@máquina:carpeta$ ` y `[usuario@máquina carpeta]# ` (bash), con `(entorno) `;
/// - `➜  carpeta ` de oh-my-zsh, con su `git:(rama) ✗ ` opcional;
/// - en general, una o dos palabras y un símbolo de prompt: `~/app $ `, `máquina% `.
pub fn remote_prompt(text: &[char]) -> Option<RemotePrompt> {
    user_host_prompt(text)
        .or_else(|| arrow_prompt(text))
        .or_else(|| plain_prompt(text))
}

fn word(text: &[char], from: usize, to: usize) -> String {
    text[from..to].iter().collect()
}

fn user_host_prompt(text: &[char]) -> Option<RemotePrompt> {
    let name = |c: char| c.is_alphanumeric() || "._-".contains(c);
    let mut i = 0;
    while text.get(i) == Some(&'(') {
        let close = text[i..].iter().position(|c| *c == ')')? + i;
        if text.get(close + 1) != Some(&' ') {
            return None;
        }
        i = close + 2;
    }
    if text.get(i) == Some(&'[') {
        i += 1;
    }
    let start = i;
    while text.get(i).is_some_and(|c| name(*c)) {
        i += 1;
    }
    if i == start || text.get(i) != Some(&'@') {
        return None;
    }
    let at = i;
    i += 1;
    while text.get(i).is_some_and(|c| name(*c)) {
        i += 1;
    }
    if i == at + 1 || !matches!(text.get(i), Some(':' | ' ')) {
        return None;
    }
    let host = (start, i);
    i += 1;
    let dir_start = i;
    while text.get(i).is_some_and(|c| !c.is_whitespace() && *c != ']') {
        i += 1;
    }
    let mut dir_end = i;
    let symbol = |c: Option<&char>| c.is_some_and(|c| "$#%>".contains(*c));
    if text.get(i) == Some(&']') {
        i += 1;
        if text.get(i) == Some(&' ') {
            i += 1;
        }
        if !symbol(text.get(i)) {
            return None;
        }
        i += 1;
    } else if dir_end > dir_start && symbol(text.get(dir_end - 1)) {
        // `carpeta$`: el símbolo va pegado a la carpeta.
        dir_end -= 1;
    } else if text.get(i) == Some(&' ') && symbol(text.get(i + 1)) {
        i += 2;
    } else {
        return None;
    }
    if dir_end == dir_start || text.get(i) != Some(&' ') {
        return None;
    }
    Some(RemotePrompt {
        host: Some(word(text, host.0, host.1)),
        dir: Some(word(text, dir_start, dir_end)),
        len: i + 1,
    })
}

/// `➜  carpeta ` (tema robbyrussell de oh-my-zsh), seguido o no de `git:(rama) ` y `✗ `.
fn arrow_prompt(text: &[char]) -> Option<RemotePrompt> {
    if !matches!(text.first(), Some('➜' | '❯')) {
        return None;
    }
    let mut i = 1;
    while text.get(i) == Some(&' ') {
        i += 1;
    }
    let start = i;
    while text.get(i).is_some_and(|c| !c.is_whitespace()) {
        i += 1;
    }
    if start == 1 || i == start || text.get(i) != Some(&' ') {
        return None;
    }
    let dir = word(text, start, i);
    i += 1;
    let rest: String = text[i..].iter().collect();
    if rest.starts_with("git:(") {
        let close = rest.find(") ")?;
        i += rest[..close + 2].chars().count();
        if text.get(i) == Some(&'✗') && text.get(i + 1) == Some(&' ') {
            i += 2;
        }
    }
    Some(RemotePrompt {
        host: None,
        dir: Some(dir),
        len: i,
    })
}

/// Una o dos palabras y un símbolo de prompt seguido de espacio.
fn plain_prompt(text: &[char]) -> Option<RemotePrompt> {
    let end = (0..text.len().min(80)).find(|&i| {
        PROMPT_SYMBOLS.contains(text[i])
            && text.get(i + 1) == Some(&' ')
            && (i == 0 || text[i - 1] == ' ' || "$#%>".contains(text[i]))
    })?;
    let before: String = text[..end].iter().collect();
    let words: Vec<&str> = before.split_whitespace().collect();
    // Ni frases ni porcentajes («50% », «Uso: 41.3% de…»).
    let number = before.trim_end().ends_with(|c: char| c.is_ascii_digit());
    let odd = |w: &&str| w.contains(['=', '|', '"', '\'', ':']);
    if words.len() > 2 || number || words.iter().any(odd) {
        return None;
    }
    let dir = words
        .iter()
        .find(|w| w.starts_with('~') || w.starts_with('/'))
        .map(|w| w.to_string());
    Some(RemotePrompt {
        host: None,
        dir,
        len: end + 2,
    })
}

/// «usuario@máquina» y carpeta del título de la ventana, que casi todos los servidores
/// ponen a `usuario@máquina: carpeta` (bash de Debian/Ubuntu, oh-my-zsh…).
pub fn title_host(title: &str) -> Option<(String, Option<String>)> {
    let title = title.trim();
    let (who, dir) = match title.split_once(':') {
        Some((who, dir)) => (who.trim(), Some(dir.trim()).filter(|d| !d.is_empty())),
        None => (title, None),
    };
    let (user, host) = who.split_once('@')?;
    let name =
        |s: &str| !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || "._-".contains(c));
    (name(user) && name(host)).then(|| (who.to_string(), dir.map(str::to_string)))
}

/// A qué máquina se conecta un `ssh …` (o `sshpass … ssh …`), por sus argumentos.
pub fn ssh_target(args: &[String]) -> Option<String> {
    let from = args
        .iter()
        .rposition(|a| a.rsplit('/').next() == Some("ssh"))?
        + 1;
    let mut skip = false;
    for arg in &args[from..] {
        if skip {
            skip = false;
        } else if let Some(flag) = arg.strip_prefix('-') {
            // Opciones que llevan valor aparte: `-p 22`, `-i clave`…
            skip = flag.len() == 1 && "bcDEeFIiJLlmOopQRSWw".contains(flag);
        } else {
            return Some(arg.clone());
        }
    }
    None
}

/// Cómo acabó el comando anterior a un prompt marcado: código de salida y milisegundos.
fn tag_result(link: &Hyperlink) -> Option<(i32, u64)> {
    let mut parts = link.uri().split(';').skip(1);
    let status = parts.next()?.strip_prefix("s=")?.parse().ok()?;
    let ms = parts.next()?.strip_prefix("t=")?.parse().ok()?;
    Some((status, ms))
}

/// Una línea de cajas visible: fila en pantalla, celdas que ocupa su texto, dónde empieza
/// lo siguiente a su derecha y los datos (marca + texto) de cada caja.
#[derive(Clone, PartialEq)]
pub struct ChipRow {
    pub row: i32,
    pub end: usize,
    pub next: usize,
    pub parts: Vec<(char, String)>,
    /// Celdas que ocupa el símbolo del prompt («❯ ») al inicio de la fila siguiente; 0 si
    /// esa fila no empieza por un símbolo suelto.
    pub symbol: usize,
    /// Prompt remoto: las cajas van en la misma fila que el comando, que empieza en `end`.
    pub inline: bool,
    /// Cómo acabó el comando anterior a este prompt (código, milisegundos), si se sabe.
    pub result: Option<(i32, u64)>,
    /// Tramos del comando remoto a colorear: (fila, columna inicial, final, qué es).
    pub tokens: Vec<(i32, usize, usize, Tok)>,
}

/// Esquema del hiperenlace con el que la integración de zsh (`assets/shell/zshenv.zsh`)
/// envuelve el prompt: sus celdas quedan marcadas en la rejilla con la carpeta en la que
/// se pintó, codificada byte a byte como `%XX`.
const PROMPT_URI: &str = "colibri-prompt:";

/// La marca de prompt de una celda, si la tiene.
fn prompt_tag(cell: &Cell) -> Option<Hyperlink> {
    cell.hyperlink()
        .filter(|link| link.uri().starts_with(PROMPT_URI))
}

/// La carpeta que lleva una marca de prompt.
fn tag_dir(link: &Hyperlink) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    // Tras la carpeta puede venir «;s=…;t=…» (ver `tag_result`).
    let uri = &link.uri()[PROMPT_URI.len()..];
    let raw = uri.split(';').next().unwrap_or(uri).as_bytes();
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let hex = (raw[i] == b'%')
            .then(|| raw.get(i + 1..i + 3))
            .flatten()
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match hex {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(raw[i]);
                i += 1;
            }
        }
    }
    PathBuf::from(std::ffi::OsString::from_vec(out))
}

/// Las dos rutas nombran la misma carpeta (el shell da la ruta lógica, con enlaces
/// simbólicos; el sistema, la física).
fn same_dir(a: &Path, b: &Path) -> bool {
    a == b
        || matches!(
            (std::fs::canonicalize(a), std::fs::canonicalize(b)),
            (Ok(a), Ok(b)) if a == b
        )
}

/// Última marca de prompt leída, ya resuelta: el prompt se resella en cada cuadro y la
/// marca casi nunca cambia, así que ni se vuelve a decodificar ni se vuelve a comparar
/// con la carpeta del sondeo (que son dos llamadas al sistema si hay enlaces simbólicos).
struct TagCache {
    uri: String,
    dir: PathBuf,
    /// Carpeta del sondeo con la que se comparó, y si era la misma.
    cwd: PathBuf,
    same: bool,
}

/// Celdas de una línea de cajas: carácter y, en la primera de cada dato, su marca.
type ChipCells = Vec<(char, Option<char>)>;

/// Lo que debe decir una línea de cajas de `cols` celdas: «carpeta rama cambios».
fn chip_cells(dir: &Path, branch: Option<&str>, changes: Option<usize>, cols: usize) -> ChipCells {
    let extras: Vec<(char, String)> = [
        branch.map(|b| (CHIP_BRANCH, b.to_string())),
        branch.and(changes).map(|n| (CHIP_CHANGES, n.to_string())),
    ]
    .into_iter()
    .flatten()
    .collect();
    let extras_len: usize = extras.iter().map(|(_, t)| t.chars().count() + 1).sum();
    let mut path: Vec<char> = proc::pretty_path(dir).chars().collect();
    let (max_path, extras) = if cols >= extras_len + 6 {
        (cols - extras_len, extras)
    } else {
        (cols, Vec::new())
    };
    if path.len() > max_path && max_path > 1 {
        // Se recorta por el principio: lo que importa es dónde acaba la ruta.
        path = std::iter::once('…')
            .chain(path[path.len() - (max_path - 1)..].iter().copied())
            .collect();
    }
    let mut want = ChipCells::new();
    for (kind, text) in
        std::iter::once((CHIP_DIR, path.into_iter().collect::<String>())).chain(extras)
    {
        if !want.is_empty() {
            want.push((' ', None));
        }
        for (i, c) in text.chars().enumerate() {
            want.push((c, (i == 0).then_some(kind)));
        }
    }
    want.truncate(cols);
    want
}

/// Escribe una línea de cajas en la rejilla (si no dice ya eso). Lo que el shell tuviera
/// a la derecha (el reloj de powerlevel10k) se borra: la línea es solo de las cajas. La
/// línea conserva la marca de prompt.
fn write_chips(
    grid: &mut Grid<Cell>,
    line: Line,
    want: &[(char, Option<char>)],
    tag: Option<Hyperlink>,
) {
    let cols = grid.columns();
    let at = |col: usize| want.get(col).copied().unwrap_or((' ', None));
    let same = (0..cols).all(|col| {
        let cell = &grid[line][Column(col)];
        (if cell.c == '\0' { ' ' } else { cell.c }, chip_kind(cell)) == at(col)
    });
    if same {
        return;
    }
    for col in 0..cols {
        let (c, kind) = at(col);
        let mut cell = Cell {
            c,
            ..Default::default()
        };
        if let Some(kind) = kind {
            cell.push_zerowidth(kind);
        }
        cell.set_hyperlink(tag.clone());
        grid[line][Column(col)] = cell;
    }
}

/// Dos celdas son del mismo prompt: llevan la marca del mismo hiperenlace (cada vez que
/// el shell pinta el prompt es un hiperenlace nuevo, aunque la carpeta sea la misma).
fn same_prompt(a: &Option<Hyperlink>, b: &Option<Hyperlink>) -> bool {
    matches!((a, b), (Some(a), Some(b)) if a.id() == b.id())
}

/// Carpeta con el `.zshenv` de la integración, escrito (o actualizado) en el momento.
fn zsh_integration_dir() -> Option<PathBuf> {
    let dir = dirs::cache_dir()?.join("colibri").join("zsh");
    let file = dir.join(".zshenv");
    let body = include_str!("../assets/shell/zshenv.zsh");
    if std::fs::read_to_string(&file).ok().as_deref() != Some(body) {
        std::fs::create_dir_all(&dir).ok()?;
        std::fs::write(&file, body).ok()?;
    }
    Some(dir)
}

fn is_blank(c: char) -> bool {
    c == ' ' || c == '\0'
}

fn chip_kind(cell: &Cell) -> Option<char> {
    cell.zerowidth()?
        .iter()
        .copied()
        .find(|c| (CHIP_DIR..=CHIP_END).contains(c))
}

/// Dónde acaba el texto de la izquierda de una fila (antes de un hueco de 3 o más
/// espacios) y dónde empieza lo que haya después; `cols` si no hay nada más.
fn text_extent(row: &Row<Cell>, cols: usize) -> (usize, usize) {
    let (mut end, mut gap) = (0, 0);
    for col in 0..cols {
        if is_blank(row[Column(col)].c) {
            gap += 1;
        } else {
            if gap >= 3 && end > 0 {
                return (end, col);
            }
            gap = 0;
            end = col + 1;
        }
    }
    (end, cols)
}

/// Ritmo de repintado por la salida de los programas. Pintar un cuadro es barato, pero un
/// agente que anima su pantalla manda decenas de trozos de salida por segundo y cada uno
/// pedía un cuadro: con varias pestañas trabajando la ventana se repintaba ~100 veces por
/// segundo, casi todas sin nada nuevo que ver. Ahora:
/// - la pestaña visible se repinta como mucho cada `OUTPUT_FRAME`;
/// - salvo justo después de una tecla (`ECHO_WINDOW`): el eco de lo que escribes va al
///   momento, que es donde se nota la latencia;
/// - una pestaña que no se ve solo necesita ponerse al día cada `BACKGROUND_FRAME` (su
///   indicador de «trabajando» y su título).
const OUTPUT_FRAME: u64 = 33;
const ECHO_WINDOW: u64 = 150;
const BACKGROUND_FRAME: u64 = 250;

static LAST_INPUT: AtomicU64 = AtomicU64::new(0);

/// Lo que el hilo de la pestaña necesita saber para decidir cuándo pedir un cuadro. Es de
/// cada pestaña (no global) porque con varias ventanas puede haber varias a la vista.
struct Pace {
    /// La pestaña es la activa de una ventana que se ve (ni minimizada ni tapada).
    shown: AtomicBool,
    /// Cuándo se pintó por última vez la ventana que la muestra (`now_ms`).
    last_frame: AtomicU64,
    /// El contexto de la ventana donde está la pestaña, a quien se piden los cuadros.
    ctx: Mutex<egui::Context>,
}

fn now_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

#[derive(Clone)]
pub struct Listener {
    tx: Sender<Event>,
    pace: Arc<Pace>,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        // Lo que solo cambia lo que se ve puede esperar; lo demás (respuestas que el
        // programa está esperando, portapapeles, fin del proceso) se atiende ya.
        let visual = matches!(
            event,
            Event::Wakeup
                | Event::Title(_)
                | Event::ResetTitle
                | Event::MouseCursorDirty
                | Event::CursorBlinkingChange
        );
        let _ = self.tx.send(event);
        let pace = &self.pace;
        let ctx = pace.ctx.lock().unwrap_or_else(|e| e.into_inner());
        if !visual {
            return ctx.request_repaint();
        }
        if !pace.shown.load(Ordering::Relaxed) {
            return ctx.request_repaint_after(Duration::from_millis(BACKGROUND_FRAME));
        }
        let now = now_ms();
        let since_frame = now.saturating_sub(pace.last_frame.load(Ordering::Relaxed));
        let typing = now.saturating_sub(LAST_INPUT.load(Ordering::Relaxed)) < ECHO_WINDOW;
        // Con espera (aunque sea de 1 ms) egui pinta un solo cuadro; `request_repaint()`
        // pinta dos seguidos, y eso doblaba los cuadros de cualquier salida continua y del
        // eco de cada tecla.
        if typing {
            return ctx.request_repaint_after(Duration::from_millis(1));
        }
        let wait = OUTPUT_FRAME.saturating_sub(since_frame).max(1);
        ctx.request_repaint_after(Duration::from_millis(wait));
    }
}

#[derive(Clone, Copy)]
pub struct TermSize {
    pub cols: usize,
    pub lines: usize,
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.lines
    }
    fn screen_lines(&self) -> usize {
        self.lines
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Agent {
    Claude,
    Codex,
    Gemini,
    Aider,
    OpenCode,
    Other,
}

impl Agent {
    pub fn label(self) -> &'static str {
        match self {
            Agent::Claude => "Claude Code",
            Agent::Codex => "Codex",
            Agent::Gemini => "Gemini",
            Agent::Aider => "Aider",
            Agent::OpenCode => "OpenCode",
            Agent::Other => crate::i18n::t().term.other_agent,
        }
    }
    pub fn color(self) -> egui::Color32 {
        match self {
            Agent::Claude => egui::Color32::from_rgb(0xd9, 0x77, 0x57),
            Agent::Codex => egui::Color32::from_rgb(0xe6, 0xe6, 0xe6),
            Agent::Gemini => egui::Color32::from_rgb(0x7a, 0xa2, 0xf7),
            _ => egui::Color32::from_rgb(0xb3, 0x9d, 0xdb),
        }
    }

    fn from_word(w: &str) -> Option<Agent> {
        let base = w.rsplit('/').next().unwrap_or(w).to_lowercase();
        let base = base.trim_end_matches(".js").trim_end_matches(".mjs");
        Some(match base {
            "claude" | "claude-code" => Agent::Claude,
            "codex" => Agent::Codex,
            "gemini" => Agent::Gemini,
            "aider" => Agent::Aider,
            "opencode" => Agent::OpenCode,
            "amp" | "cursor-agent" | "copilot" | "goose" | "crush" | "droid" | "qwen" => {
                Agent::Other
            }
            _ => return None,
        })
    }

    fn detect(pid: i32, name: &str) -> Option<Agent> {
        if let Some(a) = Agent::from_word(name) {
            return Some(a);
        }
        // El nombre del proceso engaña: Claude Code se llama como su versión ("2.1.327")
        // y otros CLIs corren sobre node/python. argv dice la verdad.
        let args = proc::args(pid);
        for a in args.iter().take(4) {
            if let Some(agent) = Agent::from_word(a) {
                return Some(agent);
            }
            if a.contains("claude-code") || a.contains("/claude/versions/") {
                return Some(Agent::Claude);
            }
        }
        None
    }
}

/// La actividad que ve git (`git::Watch`, para las cajas del prompt): la de la pestaña
/// mientras el shell está en primer plano. Con otro programa delante (un agente) no hay
/// prompt donde se vean las cajas y se congela; así no se relee `.git/HEAD` en cada cuadro
/// ni corre `git status` cada 3 s mientras el agente escribe. Al volver el shell, el salto
/// hace que se consulte otra vez (el agente pudo cambiar de rama o hacer commits).
fn git_activity(frozen: &mut u64, activity: u64, shell_in_front: bool) -> u64 {
    if shell_in_front {
        *frozen = activity;
    }
    *frozen
}

/// Qué hacer con la orden pendiente de una pestaña nueva (`Session::run_when_ready`).
#[derive(Debug, PartialEq)]
enum PendingStep {
    Run,
    /// Volver a mirar dentro de tanto.
    Wait(Duration),
    /// Abandonarla.
    Drop,
}

/// `age`: desde que se pidió; `typed`: tecleaste en la pestaña desde entonces; `ready`: el
/// shell ya escribió algo y está en su prompt; `quiet`: desde la última salida. Se ejecuta
/// tras 300 ms sin salida (el prompt ya pintado); se abandona si tecleaste (`insert_command`
/// borraría lo tuyo) o si en 10 s no hubo prompt (no se queda sondeando ni arranca el
/// agente mucho después, al salir de lo que hubiera encima).
fn pending_step(age: Duration, typed: bool, ready: bool, quiet: Duration) -> PendingStep {
    const QUIET: Duration = Duration::from_millis(300);
    const GIVE_UP: Duration = Duration::from_secs(10);
    if typed || age > GIVE_UP {
        PendingStep::Drop
    } else if !ready {
        PendingStep::Wait(QUIET)
    } else if quiet < QUIET {
        PendingStep::Wait(QUIET - quiet)
    } else {
        PendingStep::Run
    }
}

/// Tras salida nueva (o moverse por el historial), cuánto se espera para comprimirlo.
const COMPACT_EVERY: Duration = Duration::from_secs(2);

/// Filas comprimidas por tanda (unos 2–4 µs cada una con el terminal bloqueado, sumando
/// liberar la fila) y espera hasta la siguiente si quedó trabajo: ~2600 filas/s sin
/// retener el terminal más de ~1–2 ms seguidos.
const COMPACT_BUDGET: usize = 400;
const COMPACT_SOON: Duration = Duration::from_millis(150);

/// Líneas de historial justo encima de la pantalla que se dejan sin comprimir: las que
/// se releen a menudo (`in_command` mira hasta 200 por encima del cursor, los enlaces
/// siguen líneas partidas, la selección).
const COMPACT_KEEP: usize = 256;

pub struct Session {
    pub id: u64,
    pub term: Arc<FairMutex<Term<Listener>>>,
    /// Lo que comparte con su `Listener` para el ritmo de repintado.
    pace: Arc<Pace>,
    sender: EventLoopSender,
    rx: Receiver<Event>,
    master_fd: i32,
    shell_pid: i32,
    pub size: TermSize,
    pub cell_px: (u16, u16),
    pub exited: bool,

    /// Nombre que le pusiste a la pestaña; vacío = automático.
    pub name: String,
    /// Color que le pusiste a la pestaña: índice en `TAB_COLORS`.
    pub color: Option<u8>,
    /// Título que puso el programa (OSC 0/2).
    pub title: String,
    pub cwd: PathBuf,
    pub fg_pid: i32,
    pub fg_name: String,
    /// Nombre del proceso en primer plano tal cual lo da el sistema, para notar un `exec`.
    fg_comm: String,
    /// Hay un ssh (o similar) entre el shell y el proceso en primer plano.
    remote: bool,
    /// A dónde apunta ese ssh, por sus argumentos (por si el servidor no dice su nombre).
    remote_target: Option<String>,
    pub agent: Option<Agent>,
    /// La sesión de Claude Code de la pestaña (`~/.claude/sessions/<pid>.json`), la última
    /// que se vio: al restaurar se retoma esa y no «la más reciente de la carpeta».
    pub claude: Option<crate::claude::Running>,

    /// Hay algo corriendo: un agente trabajando o un comando en primer plano.
    pub busy: bool,
    busy_since: Instant,
    /// Desde cuándo está en su estado actual (para el panel de agentes): al abrir la
    /// pestaña, al cambiar el agente y en cada cambio de `busy`.
    pub state_since: Instant,
    last_input: Instant,
    last_active_output: Instant,
    streak_start: Instant,
    /// Cuándo terminó la última tarea (para el panel y el salto).
    pub finished_at: Option<Instant>,
    /// Terminó mientras no la estabas viendo.
    pub unseen: bool,
    dirty: bool,
    last_poll: Instant,
    /// Cuándo toca comprimir el historial (`compact_history`); `None` si no hay nada.
    compact_at: Option<Instant>,
    /// `activity` y desplazamiento por el historial vistos en la última compresión.
    compact_seen: (u64, usize),
    pub scroll_acc: f32,
    /// Celda del cursor en pantalla en el último cuadro (para anclar el historial).
    pub cursor_px: Option<egui::Rect>,
    /// Líneas de cajas en pantalla: su hueco y los datos de cada caja (se fija al pintar).
    pub chips_px: Vec<(egui::Rect, Vec<(char, String)>)>,
    /// Filas en pantalla que se pintan corridas a la izquierda (sin el símbolo del prompt)
    /// y cuántas celdas; el ratón tiene que deshacer ese corrimiento.
    pub row_shifts: Vec<(i32, i32)>,
    /// Lo que ya se sabe de qué palabras son rutas que existen (`path_tokens`): por carpeta,
    /// palabra y si bastaba con el principio, el resultado y cuándo se miró.
    paths_seen: HashMap<(PathBuf, String, bool), (bool, Instant)>,
    /// El último resultado de `path_tokens`, con lo que lo decide (salida, desplazamiento,
    /// tamaño, cursor y si el shell espera) y cuándo se calculó: mientras nada de eso cambie
    /// y lo mirado en el disco siga valiendo, se reutiliza sin recorrer la pantalla.
    paths_memo: Option<(PathsKey, Instant, Vec<TokSpan>)>,
    /// La rejilla del cuadro anterior, para rehacer solo las filas que cambian.
    pub grid: crate::view::GridCache,
    /// Texto a medio componer (tecla muerta: «˜» antes de la «ñ»; o un método de entrada),
    /// que aún no se ha enviado al programa. Se pinta en el cursor, subrayado.
    pub preedit: String,
    /// Cuadro en que llegó ese texto (para saber si una tecla es posterior a él).
    pub preedit_frame: u64,
    tag_cache: Option<TagCache>,
    /// Lo último que se selló en el prompt actual (marca y celdas), para volver a ponerlo
    /// si el shell repinta ese prompt cuando ya no lo es.
    last_chips: Option<(String, ChipCells)>,
    /// Comando sugerido en gris en el prompt vacío (al restaurar una pestaña donde corría
    /// un agente). No está escrito en el shell: se acepta con Tab/→ o se ejecuta con
    /// Enter; cualquier otra tecla lo descarta.
    pub suggestion: Option<String>,
    /// Orden que se ejecutará en cuanto el shell esté en su prompt (`run_when_ready`) y
    /// cuándo se pidió.
    pending: Option<(String, Instant)>,
    /// Cuándo escribió algo el programa por última vez (para saber si el shell ya pintó
    /// su prompt y se quedó quieto).
    last_output: Instant,
    /// Lo tecleado en el prompt desde el último Enter. None = se perdió la pista
    /// (hubo teclas de edición: flechas, tabulador…).
    line: Option<String>,
    /// Comandos tecleados en esta pestaña, del más antiguo al más reciente.
    pub typed: Vec<String>,
    /// Sube cada vez que el programa escribe algo: sirve para saber si hubo novedades.
    pub activity: u64,
    /// `activity` la última vez que el shell estaba en primer plano (`git_activity`).
    git_frozen: u64,
    /// Última celda informada al programa y botón que se mantiene pulsado.
    pub mouse_cell: Option<(usize, usize)>,
    pub mouse_down: Option<u8>,
}

impl Session {
    pub fn new(
        id: u64,
        cfg: &Config,
        cwd: Option<PathBuf>,
        ctx: &egui::Context,
    ) -> std::io::Result<Self> {
        let size = TermSize {
            cols: 80,
            lines: 24,
        };
        let (tx, rx) = channel();
        let pace = Arc::new(Pace {
            shown: AtomicBool::new(false),
            last_frame: AtomicU64::new(0),
            ctx: Mutex::new(ctx.clone()),
        });
        let listener = Listener {
            tx,
            pace: pace.clone(),
        };

        let shell = if !cfg.shell.is_empty() {
            cfg.shell.clone()
        } else {
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into())
        };
        let mut env = HashMap::new();
        env.insert("TERM".to_string(), "xterm-256color".to_string());
        env.insert("COLORTERM".to_string(), "truecolor".to_string());
        env.insert("TERM_PROGRAM".to_string(), "Colibri".to_string());
        env.insert(
            "TERM_PROGRAM_VERSION".to_string(),
            env!("CARGO_PKG_VERSION").to_string(),
        );
        // Claude Code (y lo que use `supports-hyperlinks`) solo emite hiperenlaces OSC 8 en
        // las terminales que conoce; así los emite aquí, y ⌘+clic los abre enteros aunque
        // la dirección ocupe varias líneas.
        env.insert("FORCE_HYPERLINK".to_string(), "1".to_string());
        if std::env::var_os("LANG").is_none() {
            env.insert("LANG".to_string(), "en_US.UTF-8".to_string());
        }
        // Integración con zsh: se arranca con ZDOTDIR en una carpeta propia cuyo `.zshenv`
        // restaura el ZDOTDIR del usuario, carga su `.zshenv` y añade el gancho que marca
        // el prompt. Con otros shells no hay marca y el prompt se reconoce por su texto.
        if cfg.shell_integration
            && shell.rsplit('/').next() == Some("zsh")
            && let Some(dir) = zsh_integration_dir()
        {
            // Color con el que zsh pinta la selección dentro del comando.
            let sel = cfg.theme.selection.trim();
            if sel.len() == 7
                && sel.starts_with('#')
                && sel[1..].bytes().all(|b| b.is_ascii_hexdigit())
            {
                env.insert("COLIBRI_SELECTION".to_string(), sel.to_string());
            }
            if let Ok(orig) = std::env::var("ZDOTDIR") {
                env.insert("COLIBRI_ZDOTDIR".to_string(), orig);
            }
            env.insert("ZDOTDIR".to_string(), dir.to_string_lossy().into_owned());
        }
        let cwd = cwd.filter(|p| p.is_dir()).or_else(dirs::home_dir);
        let options = tty::Options {
            shell: Some(tty::Shell::new(shell, vec!["-l".into()])),
            working_directory: cwd.clone(),
            drain_on_exit: false,
            env,
        };
        let window_size = WindowSize {
            num_lines: 24,
            num_cols: 80,
            cell_width: 8,
            cell_height: 16,
        };
        let pty = tty::new(&options, window_size, id)?;
        let master_fd = proc::dup_cloexec(pty.file().as_raw_fd());
        let shell_pid = pty.child().id() as i32;

        let term_cfg = TermConfig {
            scrolling_history: cfg.scrollback,
            ..Default::default()
        };
        let term = Arc::new(FairMutex::new(Term::new(term_cfg, &size, listener.clone())));
        let event_loop = EventLoop::new(term.clone(), listener, pty, false, false)?;
        let sender = event_loop.channel();
        event_loop.spawn();

        let now = Instant::now();
        let old = now - Duration::from_secs(60);
        Ok(Self {
            id,
            term,
            pace,
            sender,
            rx,
            master_fd,
            shell_pid,
            size,
            cell_px: (8, 16),
            exited: false,
            name: String::new(),
            color: None,
            title: String::new(),
            cwd: cwd.unwrap_or_default(),
            fg_pid: shell_pid,
            fg_name: String::new(),
            fg_comm: String::new(),
            remote: false,
            remote_target: None,
            agent: None,
            claude: None,
            busy: false,
            busy_since: now,
            state_since: now,
            last_input: old,
            last_active_output: old,
            streak_start: old,
            finished_at: None,
            unseen: false,
            dirty: true,
            last_poll: old,
            compact_at: None,
            compact_seen: (0, 0),
            scroll_acc: 0.0,
            cursor_px: None,
            chips_px: Vec::new(),
            row_shifts: Vec::new(),
            paths_seen: HashMap::new(),
            paths_memo: None,
            grid: Default::default(),
            preedit: String::new(),
            preedit_frame: 0,
            tag_cache: None,
            last_chips: None,
            suggestion: None,
            pending: None,
            last_output: old,
            line: Some(String::new()),
            typed: Vec::new(),
            activity: 0,
            git_frozen: 0,
            mouse_cell: None,
            mouse_down: None,
        })
    }

    fn send(&self, bytes: Vec<u8>) {
        if !bytes.is_empty() {
            let _ = self.sender.send(Msg::Input(Cow::Owned(bytes)));
        }
    }

    /// Entrada del usuario (teclado o pegar).
    pub fn input(&mut self, bytes: Vec<u8>) {
        self.suggestion = None;
        self.track(&bytes);
        self.last_input = Instant::now();
        LAST_INPUT.store(now_ms(), Ordering::Relaxed);
        self.term.lock().scroll_display(Scroll::Bottom);
        self.send(bytes);
    }

    /// Reconstruye el comando que se teclea en el prompt, para ofrecerlo en el historial
    /// aunque el shell aún no lo haya guardado. Lo que se le escribe a otro programa
    /// (contraseñas de sudo, mensajes a un agente) no se registra.
    fn track(&mut self, bytes: &[u8]) {
        if self.fg_pid != self.shell_pid {
            self.line = Some(String::new());
            return;
        }
        let Ok(text) = std::str::from_utf8(bytes) else {
            self.line = None;
            return;
        };
        for c in text.chars() {
            match c {
                '\r' | '\n' => {
                    // Como en zsh: lo que empieza por espacio no se guarda.
                    if let Some(line) = self.line.take().filter(|l| !l.starts_with(' ')) {
                        let line = line.trim();
                        if !line.is_empty() {
                            self.typed.push(line.to_string());
                            if self.typed.len() > 500 {
                                self.typed.remove(0);
                            }
                        }
                    }
                    self.line = Some(String::new());
                }
                '\u{3}' | '\u{15}' => self.line = Some(String::new()),
                '\u{7f}' | '\u{8}' => {
                    if let Some(line) = &mut self.line {
                        line.pop();
                    }
                }
                c if c.is_control() => self.line = None,
                c => {
                    if let Some(line) = &mut self.line {
                        line.push(c);
                    }
                }
            }
        }
    }

    /// La sugerencia, si ahora mismo se puede mostrar: prompt del shell y nada escrito.
    /// Si la pestaña está a la vista (la activa de una ventana que se ve). Lo pone la
    /// interfaz en cada pasada; mientras no, su salida solo pide un cuadro cada
    /// `BACKGROUND_FRAME`.
    pub fn set_shown(&self, shown: bool) {
        self.pace.shown.store(shown, Ordering::Relaxed);
    }

    /// Se acaba de pintar un cuadro de la ventana que muestra esta pestaña.
    pub fn frame_painted(&self) {
        self.pace.last_frame.store(now_ms(), Ordering::Relaxed);
    }

    /// La pestaña pasa a otra ventana: sus cuadros se piden a ese contexto.
    pub fn set_context(&self, ctx: &egui::Context) {
        *self.pace.ctx.lock().unwrap_or_else(|e| e.into_inner()) = ctx.clone();
    }

    pub fn suggested(&self) -> Option<&str> {
        let text = self.suggestion.as_deref()?;
        (self.line.as_deref() == Some("") && self.at_prompt()).then_some(text)
    }

    /// Escribe la sugerencia en el prompt y, con `run`, la ejecuta.
    pub fn accept_suggestion(&mut self, run: bool) {
        if let Some(cmd) = self.suggestion.take() {
            self.insert_command(&cmd);
            if run {
                self.input(b"\r".to_vec());
            }
        }
    }

    /// Lo tecleado en el prompt hasta ahora, si se le sigue la pista.
    pub fn prompt_line(&self) -> Option<&str> {
        self.line.as_deref()
    }

    /// Si se perdió la pista de lo tecleado (un Tab que completó zsh, flechas, el
    /// historial de ^R…), la recupera leyendo de la rejilla lo que hay entre el prompt
    /// marcado y el cursor. Solo con la integración del shell.
    pub fn resync_line(&mut self) {
        if self.line.is_some() {
            return;
        }
        let Some((cells, cursor)) = self.input_cells() else {
            return;
        };
        let term = self.term.lock();
        let grid = term.grid();
        let mut text = String::new();
        for p in cells.iter().take_while(|p| **p < cursor) {
            let cell = &grid[p.line][p.column];
            if chip_kind(cell).is_some() {
                return;
            }
            text.push(if cell.c == '\0' { ' ' } else { cell.c });
            text.extend(cell.zerowidth().into_iter().flatten());
        }
        drop(term);
        self.line = Some(text);
    }

    /// Cambia la última palabra del prompt (`chars` caracteres) por `text`, como si se
    /// borrara y se tecleara: así `track` sigue cuadrando con lo que ve el shell.
    pub fn replace_word(&mut self, chars: usize, text: &str) {
        let mut bytes = vec![0x7f; chars];
        bytes.extend_from_slice(text.as_bytes());
        self.input(bytes);
    }

    /// El programa de la pestaña está pidiendo una contraseña (ver `proc::password_mode`).
    /// Se mira en el momento, sin guardar nada: es un `tcgetattr`.
    pub fn password_prompt(&self) -> bool {
        proc::password_mode(self.master_fd)
    }

    /// El shell espera un comando: no corre nada encima ni hay un programa a pantalla completa.
    pub fn at_prompt(&self) -> bool {
        use alacritty_terminal::term::TermMode;
        self.fg_pid == self.shell_pid
            && !self
                .term
                .lock()
                .mode()
                .intersects(TermMode::ALT_SCREEN | TermMode::MOUSE_MODE)
    }

    /// Convierte la línea del prompt que nombra la carpeta (la de encima de donde se escribe,
    /// en prompts de dos líneas como el de powerlevel10k) en una «línea de cajas»: reescribe
    /// sus celdas con carpeta, rama y cambios, cada dato marcado en su primera celda. Al
    /// quedar en la rejilla, las cajas persisten: suben con el historial y cada prompt
    /// conserva lo que valía cuando se usó. Mientras el prompt es el actual se va
    /// actualizando (y se vuelve a sellar si el shell lo repinta).
    pub fn stamp_prompt(&mut self, branch: Option<&str>, changes: Option<usize>) {
        use alacritty_terminal::term::TermMode;
        let mut term = self.term.lock();
        if term
            .mode()
            .intersects(TermMode::ALT_SCREEN | TermMode::MOUSE_MODE)
        {
            return;
        }
        let grid = term.grid_mut();
        let cols = grid.columns();
        // Un comando largo ocupa varias filas: se sube hasta donde empieza.
        let mut line = grid.cursor.point.line.0;
        while line > 0
            && grid[Line(line - 1)][Column(cols - 1)]
                .flags
                .contains(Flags::WRAPLINE)
        {
            line -= 1;
        }
        // Al pulsar Enter zsh puede repintar el prompt (powerlevel10k lo hace si cambió la
        // hora), y eso pisa las cajas cuando ese prompt ya ha dejado de ser el actual. El
        // repintado vuelve a traer la marca, así que se resellan los prompts marcados de
        // la pantalla que no sean línea de cajas: con lo último que se selló si es el mismo
        // sitio, o solo con su carpeta.
        let rows = grid.screen_lines() as i32;
        for r in 0..rows - 1 {
            let head = &grid[Line(r)][Column(0)];
            let tag = prompt_tag(head);
            if r + 1 == line || tag.is_none() || chip_kind(head) == Some(CHIP_DIR) {
                continue;
            }
            // Solo la penúltima fila de un prompt de varias líneas.
            let below = prompt_tag(&grid[Line(r + 1)][Column(0)]);
            let after = (r + 2 < rows).then(|| prompt_tag(&grid[Line(r + 2)][Column(0)]));
            if !same_prompt(&tag, &below) || after.is_some_and(|a| same_prompt(&tag, &a)) {
                continue;
            }
            let Some(link) = &tag else { continue };
            let want = match &self.last_chips {
                Some((uri, cells)) if uri == link.uri() => {
                    cells.iter().copied().take(cols).collect()
                }
                _ => chip_cells(&tag_dir(link), None, None, cols),
            };
            write_chips(grid, Line(r), &want, tag);
        }
        // Dentro de una sesión ssh no hay integración: el prompt es el de la máquina remota
        // y se reconoce por su texto (`remote_prompt`). La máquina y la carpeta salen, por
        // este orden, del título que publica el servidor, del propio prompt y del comando
        // `ssh`. Se guardan como caracteres de ancho cero en la primera celda (no caben en
        // las del prompt, que puede ser un simple «➜ ~»), y la última celda marca dónde
        // empieza el comando.
        if self.remote && prompt_tag(&grid[Line(line)][Column(0)]).is_none() {
            let row = &grid[Line(line)];
            if chip_kind(&row[Column(0)]) == Some(CHIP_PAD) {
                return;
            }
            let text: Vec<char> = (0..cols.min(200)).map(|col| row[Column(col)].c).collect();
            let Some(found) = remote_prompt(&text) else {
                return;
            };
            let titled = title_host(self.clean_title());
            let dir = titled
                .as_ref()
                .and_then(|(_, dir)| dir.clone())
                .or(found.dir);
            let host = titled
                .map(|(host, _)| host)
                .or(found.host)
                .or_else(|| self.remote_target.clone());
            // El cursor tiene que estar ya en el comando: si no, esa línea no es un prompt.
            let at = grid.cursor.point;
            let typing = at.line.0 > line || at.column.0 + 1 >= found.len;
            if host.is_none() && dir.is_none() || found.len < 2 || found.len > cols || !typing {
                return;
            }
            // Las celdas conservan su texto (sigue ahí para copiarlo); solo se marcan.
            let first = &mut grid[Line(line)][Column(0)];
            first.push_zerowidth(CHIP_PAD);
            for (kind, text) in [(CHIP_HOST, &host), (CHIP_DIR, &dir)] {
                if let Some(text) = text {
                    first.push_zerowidth(kind);
                    text.chars().for_each(|c| first.push_zerowidth(c));
                }
            }
            grid[Line(line)][Column(found.len - 1)].push_zerowidth(CHIP_END);
            return;
        }
        if line < 1 {
            return;
        }
        // Con la integración del shell el prompt viene marcado: se sabe sin adivinar que
        // el cursor está en un prompt (no hace falta esperar al sondeo de procesos), qué
        // filas ocupa y en qué carpeta se pintó (la del sondeo llega un poco después de un
        // `cd`). La fila de encima solo es del prompt si también lleva la marca; si no, el
        // prompt es de una línea y se deja como está.
        let tag = prompt_tag(&grid[Line(line)][Column(0)]);
        // Dentro de una sesión ssh no hay integración: el prompt es el de la máquina
        // remota y se reconoce por su texto (ver `remote_prompt`).
        if let Some(link) = &tag {
            if !same_prompt(&tag, &prompt_tag(&grid[Line(line - 1)][Column(0)])) {
                return;
            }
            let cache = match self.tag_cache.take() {
                Some(c) if c.uri == link.uri() && c.cwd == self.cwd => c,
                old => {
                    let dir = match old {
                        Some(c) if c.uri == link.uri() => c.dir,
                        _ => tag_dir(link),
                    };
                    TagCache {
                        uri: link.uri().to_string(),
                        same: same_dir(&dir, &self.cwd),
                        cwd: self.cwd.clone(),
                        dir,
                    }
                }
            };
            self.tag_cache = Some(cache);
        }
        let cached = self.tag_cache.as_ref().filter(|_| tag.is_some());
        let dir: &Path = cached.map_or(&self.cwd, |c| &c.dir);
        if tag.is_none() {
            // Sin marca (otro shell, integración apagada) se reconoce por el texto, y solo
            // si el shell está en primer plano. La línea donde se escribe empieza por el
            // símbolo del prompt; si está vacía es que el shell está imprimiendo algo (un
            // bucle, una función), no esperando.
            if self.fg_pid != self.shell_pid || is_blank(grid[Line(line)][Column(0)].c) {
                return;
            }
            let needle = if dirs::home_dir().as_deref() == Some(dir) {
                "~".to_string()
            } else {
                format!("/{}", proc::base_name(dir))
            };
            if needle == "/" {
                return;
            }
            let row = &grid[Line(line - 1)];
            if chip_kind(&row[Column(0)]) != Some(CHIP_DIR) {
                let (end, _) = text_extent(row, cols);
                let text: String = (0..end).map(|col| row[Column(col)].c).collect();
                if !(text.starts_with('~') || text.starts_with('/')) || !text.contains(&needle) {
                    return;
                }
            }
        }
        let line = Line(line - 1);
        // La rama y los cambios son los de la carpeta del sondeo: hasta que este alcance a
        // la del prompt solo se pone la carpeta.
        let (branch, changes) = if cached.is_none_or(|c| c.same) {
            (branch, changes)
        } else {
            (None, None)
        };

        let want = chip_cells(dir, branch, changes, cols);
        if self
            .last_chips
            .as_ref()
            .is_none_or(|(_, cells)| *cells != want)
            && let Some(link) = &tag
        {
            self.last_chips = Some((link.uri().to_string(), want.clone()));
        }
        write_chips(grid, line, &want, tag);
    }

    /// El programa en primer plano es una conexión a otra máquina (o corre dentro de una).
    pub fn remote(&self) -> bool {
        self.remote
    }

    /// Celdas de lo que se está escribiendo en el prompt, una por carácter y en orden, y
    /// dónde está el cursor. Solo con la integración del shell (el prompt marcado dice
    /// dónde acaba el prompt y empieza el comando) y con el shell esperando; lo usan la
    /// selección y el clic dentro del comando, que se le piden a zle con teclas.
    /// El cursor está en el comando que se escribe en un prompt marcado, aunque el comando
    /// ocupe varias líneas (un comando pegado o sacado del historial con saltos de línea):
    /// la fila marcada más cercana por encima del cursor es su prompt. Basta para mandar
    /// a zle las teclas de selección; para contar celdas hace falta `input_cells`.
    pub fn in_command(&self) -> bool {
        use alacritty_terminal::term::TermMode;
        if self.fg_pid != self.shell_pid {
            return false;
        }
        let term = self.term.lock();
        if term
            .mode()
            .intersects(TermMode::ALT_SCREEN | TermMode::MOUSE_MODE)
        {
            return false;
        }
        let grid = term.grid();
        let cursor = grid.cursor.point.line.0;
        let top = (cursor - 200).max(-(grid.history_size() as i32));
        (top..=cursor)
            .rev()
            .any(|line| prompt_tag(&grid[Line(line)][Column(0)]).is_some())
    }

    pub fn input_cells(&self) -> Option<(Vec<Point>, Point)> {
        use alacritty_terminal::term::TermMode;
        if self.fg_pid != self.shell_pid {
            return None;
        }
        let term = self.term.lock();
        if term
            .mode()
            .intersects(TermMode::ALT_SCREEN | TermMode::MOUSE_MODE)
        {
            return None;
        }
        let grid = term.grid();
        let cols = grid.columns();
        let wraps = |line: i32| {
            grid[Line(line)][Column(cols - 1)]
                .flags
                .contains(Flags::WRAPLINE)
        };
        let cursor = grid.cursor.point;
        let mut first = cursor.line.0;
        while first > 0 && wraps(first - 1) {
            first -= 1;
        }
        let prompt = (0..cols)
            .take_while(|&col| prompt_tag(&grid[Line(first)][Column(col)]).is_some())
            .count();
        if prompt == 0 {
            return None;
        }
        let mut last = cursor.line.0;
        while last + 1 < grid.screen_lines() as i32 && wraps(last) {
            last += 1;
        }
        let mut cells = Vec::new();
        for line in first..=last {
            for col in if line == first { prompt } else { 0 }..cols {
                let cell = &grid[Line(line)][Column(col)];
                if !cell
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                {
                    cells.push(Point::new(Line(line), Column(col)));
                }
            }
        }
        // Lo escrito acaba en la última celda con algo; los espacios de antes del cursor
        // sí cuentan (se han tecleado).
        while cells
            .last()
            .is_some_and(|p| *p >= cursor && is_blank(grid[p.line][p.column].c))
        {
            cells.pop();
        }
        Some((cells, cursor))
    }

    /// Rutas que existen en el comando de cada prompt marcado que se ve (el que se escribe y
    /// los del historial): (fila de la pantalla, columna inicial, final exclusiva, `Tok::Path`).
    /// Las relativas se miran contra la carpeta de la marca de su prompt. En el que se
    /// escribe solo hasta el cursor (ver `path_words`). Nada en una sesión ssh (no es este
    /// disco) ni sin la integración (sin marca no se sabe dónde empieza el comando).
    pub fn path_tokens(&mut self) -> Vec<TokSpan> {
        /// Cuánto vale lo que se miró en el disco.
        const FRESH: Duration = Duration::from_secs(2);
        if self.remote {
            return Vec::new();
        }
        let shell_waits = self.fg_pid == self.shell_pid;
        let key = {
            let term = self.term.lock();
            let grid = term.grid();
            (
                self.activity,
                grid.display_offset(),
                (grid.columns(), grid.screen_lines()),
                grid.cursor.point,
                shell_waits,
            )
        };
        if let Some((seen, at, tokens)) = &self.paths_memo
            && *seen == key
            && at.elapsed() < FRESH
        {
            return tokens.clone();
        }
        // Primero se recogen los comandos (con el terminal bloqueado) y luego se mira el disco.
        let mut commands: Vec<PromptCommand> = Vec::new();
        {
            let term = self.term.lock();
            let grid = term.grid();
            let cols = grid.columns();
            let offset = grid.display_offset() as i32;
            let screen = grid.screen_lines() as i32;
            let cursor = grid.cursor.point;
            let wraps = |line: i32| {
                grid[Line(line)][Column(cols - 1)]
                    .flags
                    .contains(Flags::WRAPLINE)
            };
            for row in 0..screen {
                let line = row - offset;
                let cells = &grid[Line(line)];
                let Some(tag) = prompt_tag(&cells[Column(0)]) else {
                    continue;
                };
                // Línea de cajas, o la de arriba de un prompt de dos líneas: el comando va
                // en la siguiente.
                let next_same = row + 1 < screen
                    && same_prompt(
                        &Some(tag.clone()),
                        &prompt_tag(&grid[Line(line + 1)][Column(0)]),
                    );
                if chip_kind(&cells[Column(0)]).is_some() || next_same {
                    continue;
                }
                let start = (0..cols)
                    .take_while(|&col| prompt_tag(&cells[Column(col)]).is_some())
                    .count();
                let mut text = Vec::new();
                let mut at = Vec::new();
                let mut upto = None;
                let mut r = row;
                loop {
                    let l = r - offset;
                    for col in if r == row { start } else { 0 }..cols {
                        let cell = &grid[Line(l)][Column(col)];
                        if cell
                            .flags
                            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                        {
                            continue;
                        }
                        if shell_waits
                            && upto.is_none()
                            && Point::new(Line(l), Column(col)) >= cursor
                        {
                            upto = Some(text.len());
                        }
                        text.push(if cell.c == '\0' { ' ' } else { cell.c });
                        at.push((r, col));
                    }
                    if !wraps(l) || r + 1 >= screen {
                        break;
                    }
                    r += 1;
                }
                // Solo el prompt en el que está el cursor se corta en él.
                let here = shell_waits && (line..=r - offset).contains(&cursor.line.0);
                commands.push((tag_dir(&tag), text, at, upto.filter(|_| here)));
            }
        }
        if self.paths_seen.len() > 4096 {
            self.paths_seen.clear();
        }
        let now = Instant::now();
        let mut out = Vec::new();
        for (dir, text, at, upto) in commands {
            for (from, to, word, prefix) in path_words(&text, upto) {
                let key = (dir.clone(), word, prefix);
                let exists = match self.paths_seen.get(&key) {
                    Some(&(found, when)) if now.duration_since(when) < FRESH => found,
                    _ => {
                        let found = path_exists(&key.0, &key.1, prefix);
                        self.paths_seen.insert(key, (found, now));
                        found
                    }
                };
                if exists {
                    out.extend(
                        split_rows(&at, from, to)
                            .into_iter()
                            .map(|(row, a, b)| (row, a, b, Tok::Path)),
                    );
                }
            }
        }
        self.paths_memo = Some((key, now, out.clone()));
        out
    }

    /// Líneas de cajas visibles ahora mismo.
    pub fn chip_rows(&self) -> Vec<ChipRow> {
        let term = self.term.lock();
        let grid = term.grid();
        let cols = grid.columns();
        let offset = grid.display_offset() as i32;
        let screen = grid.screen_lines() as i32;
        let mut out = Vec::new();
        for row in 0..screen {
            let cells = &grid[Line(row - offset)];
            let inline = match chip_kind(&cells[Column(0)]) {
                Some(CHIP_DIR) => false,
                Some(CHIP_PAD) => true,
                _ => continue,
            };
            let (end, next) = if inline {
                // El prompt remoto acaba en la celda marcada; detrás va el comando.
                let end = (0..cols)
                    .find(|&col| chip_kind(&cells[Column(col)]) == Some(CHIP_END))
                    .map_or(cols, |col| col + 1);
                (end, end)
            } else {
                text_extent(cells, cols)
            };
            let mut parts: Vec<(char, String)> = Vec::new();
            if inline {
                // Los datos van en la primera celda, como caracteres de ancho cero.
                for c in cells[Column(0)].zerowidth().into_iter().flatten() {
                    if [CHIP_HOST, CHIP_DIR].contains(c) {
                        parts.push((*c, String::new()));
                    } else if let Some((_, text)) = parts.last_mut() {
                        text.push(*c);
                    }
                }
            } else {
                for col in 0..end {
                    let cell = &cells[Column(col)];
                    if let Some(kind) = chip_kind(cell) {
                        parts.push((kind, String::new()));
                    }
                    if let Some((_, text)) = parts.last_mut() {
                        text.push(cell.c);
                    }
                }
            }
            for (_, text) in &mut parts {
                *text = text.trim().to_string();
            }
            // La fila de debajo es donde se escribe: suele empezar por «❯ », «$ », «% »…
            let symbol = if !inline && row + 1 < screen && cols > 2 {
                let below = &grid[Line(row + 1 - offset)];
                // Con la marca de la integración se sabe exactamente cuánto ocupa.
                let tagged = (0..cols)
                    .take_while(|&col| prompt_tag(&below[Column(col)]).is_some())
                    .count();
                let (c0, c1) = (below[Column(0)].c, below[Column(1)].c);
                let lone = !is_blank(c0) && !c0.is_alphanumeric() && !"/~.-_".contains(c0);
                if tagged > 0 {
                    tagged
                } else if lone && is_blank(c1) {
                    2
                } else {
                    0
                }
            } else {
                0
            };
            // El comando remoto, que puede seguir en las filas de debajo si no cabe.
            let mut tokens = Vec::new();
            if inline {
                let mut at: Vec<(i32, usize)> = Vec::new();
                let mut text: Vec<char> = Vec::new();
                let mut r = row;
                loop {
                    let line = &grid[Line(r - offset)];
                    for col in if r == row { end } else { 0 }..cols {
                        at.push((r, col));
                        text.push(line[Column(col)].c);
                    }
                    let wraps = line[Column(cols - 1)].flags.contains(Flags::WRAPLINE);
                    if !wraps || r + 1 >= screen {
                        break;
                    }
                    r += 1;
                }
                for (from, to, tok) in tokenize(&text) {
                    tokens.extend(
                        split_rows(&at, from, to)
                            .into_iter()
                            .map(|(row, a, b)| (row, a, b, tok)),
                    );
                }
            }
            out.push(ChipRow {
                row,
                end,
                next,
                parts,
                symbol,
                inline,
                result: prompt_tag(&cells[Column(0)]).as_ref().and_then(tag_result),
                tokens,
            });
        }
        out
    }

    /// Deja `cmd` escrito en el prompt, en lugar de lo que hubiera, sin ejecutarlo.
    pub fn insert_command(&mut self, cmd: &str) {
        self.input(vec![0x15]);
        // Una sola línea se teclea tal cual: pegada, zsh la deja resaltada, y el historial
        // la va cambiando con cada flecha. Con saltos de línea hay que pegarla.
        if cmd.chars().any(char::is_control) {
            self.paste(cmd);
        } else {
            self.input(cmd.as_bytes().to_vec());
        }
        self.line = Some(cmd.to_string());
    }

    /// Ejecuta `cmd` en cuanto el shell esté listo: en su prompt, tras haberlo pintado y
    /// sin escribir nada durante un momento. Escribir antes de que el shell arranque
    /// descuadraba el prompt.
    pub fn run_when_ready(&mut self, cmd: String) {
        self.pending = Some((cmd, Instant::now()));
    }

    /// Escribe y ejecuta la orden pendiente si el shell ya está listo; si aún no, pide un
    /// cuadro para volver a mirar.
    fn run_pending(&mut self, ctx: &egui::Context, now: Instant) {
        let Some((_, queued)) = &self.pending else {
            return;
        };
        let step = pending_step(
            now.duration_since(*queued),
            self.last_input > *queued,
            self.activity > 0 && self.at_prompt(),
            now.duration_since(self.last_output),
        );
        match step {
            PendingStep::Wait(d) => ctx.request_repaint_after(d),
            PendingStep::Drop => self.pending = None,
            PendingStep::Run => {
                if let Some((cmd, _)) = self.pending.take() {
                    self.insert_command(&cmd);
                    self.input(b"\r".to_vec());
                }
            }
        }
    }

    /// Tiempo desde la última tecla (el cursor no parpadea mientras se escribe).
    pub fn since_input(&self) -> Duration {
        self.last_input.elapsed()
    }

    /// Informes de ratón: van al programa sin mover el scroll.
    pub fn report(&mut self, bytes: Vec<u8>) {
        self.last_input = Instant::now();
        self.send(bytes);
    }

    pub fn paste(&mut self, text: &str) {
        let bracketed = self
            .term
            .lock()
            .mode()
            .contains(alacritty_terminal::term::TermMode::BRACKETED_PASTE);
        if bracketed {
            let clean = text.replace('\x1b', "");
            self.input(format!("\x1b[200~{clean}\x1b[201~").into_bytes());
        } else {
            self.input(text.replace("\r\n", "\r").replace('\n', "\r").into_bytes());
        }
    }

    pub fn resize(&mut self, cols: usize, lines: usize, cell_w: f32, cell_h: f32) {
        let (cols, lines) = (cols.max(2), lines.max(1));
        if cols == self.size.cols && lines == self.size.lines {
            return;
        }
        self.size = TermSize { cols, lines };
        self.cell_px = (
            cell_w.round().max(1.0) as u16,
            cell_h.round().max(1.0) as u16,
        );
        self.term.lock().resize(self.size);
        let _ = self.sender.send(Msg::Resize(self.window_size()));
    }

    fn window_size(&self) -> WindowSize {
        WindowSize {
            num_lines: self.size.lines as u16,
            num_cols: self.size.cols as u16,
            cell_width: self.cell_px.0,
            cell_height: self.cell_px.1,
        }
    }

    /// Procesa lo que mandó el emulador desde el último cuadro.
    pub fn drain(&mut self, ctx: &egui::Context, pal: &Palette) {
        let now = Instant::now();
        while let Ok(event) = self.rx.try_recv() {
            self.dirty = true;
            match event {
                Event::Wakeup => {
                    self.activity += 1;
                    self.last_output = now;
                    // Salida que no es eco de lo que tecleas = actividad real.
                    if now.duration_since(self.last_input) > Duration::from_millis(350) {
                        if now.duration_since(self.last_active_output) > Duration::from_millis(1500)
                        {
                            self.streak_start = now;
                        }
                        self.last_active_output = now;
                    }
                }
                Event::Title(t) => self.title = t,
                Event::ResetTitle => self.title.clear(),
                Event::PtyWrite(s) => self.send(s.into_bytes()),
                Event::ClipboardStore(_, s) => ctx.copy_text(s),
                Event::ColorRequest(index, format) => {
                    let known = self.term.lock().colors()[index];
                    let rgb = known.unwrap_or_else(|| {
                        let c = match index {
                            0..=255 => pal.indexed(index as u8),
                            257 => pal.bg,
                            258 => pal.cursor,
                            _ => pal.fg,
                        };
                        Rgb {
                            r: c.r(),
                            g: c.g(),
                            b: c.b(),
                        }
                    });
                    self.send(format(rgb).into_bytes());
                }
                Event::TextAreaSizeRequest(format) => {
                    self.send(format(self.window_size()).into_bytes())
                }
                Event::Exit | Event::ChildExit(_) => self.exited = true,
                // ClipboardLoad se ignora a propósito: ningún programa lee tu portapapeles.
                _ => {}
            }
        }
    }

    /// Actualiza proceso en primer plano y estado. Devuelve true si algo acaba de terminar
    /// sin que lo estuvieras viendo.
    /// Comprime el historial lejos de la pantalla (ver `vendor/alacritty_terminal`, buscar
    /// «Colibrí:»): cada línea se guardaba con todas sus columnas a 24 bytes por celda
    /// (~4,5 KB con 190 columnas) y comprimida ocupa unos pocos bytes por letra. Solo
    /// trabaja con lo que salió de la pantalla desde la última vez, así que en reposo es
    /// recorrer el historial sin tocar nada.
    fn compact_history(&mut self, ctx: &egui::Context, now: Instant) {
        static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *OFF.get_or_init(|| std::env::var_os("COLIBRI_NO_COMPACT").is_some()) {
            return;
        }
        let offset = self.term.lock().grid().display_offset();
        if (self.activity, offset) != self.compact_seen && self.compact_at.is_none() {
            // Salida nueva, o se movió por el historial (lo leído queda descomprimido).
            self.compact_at = Some(now + COMPACT_EVERY);
        }
        match self.compact_at {
            None => return,
            Some(at) if at > now => {
                // Aún no toca: se vuelve a pedir la pasada (egui la descarta si ya hay una
                // antes, y esta pasada pudo ser esa).
                ctx.request_repaint_after(at - now);
                return;
            }
            Some(_) => {}
        }
        self.compact_seen = (self.activity, offset);
        let mut term = self.term.lock();
        let started = Instant::now();
        let packed = term.compact_history(COMPACT_KEEP, COMPACT_BUDGET);
        let took = started.elapsed();
        drop(term);
        if std::env::var_os("COLIBRI_FPS").is_some() {
            let (plain, packed_now, cached) = self.term.lock().grid().history_stats();
            eprintln!(
                "historial: {packed} filas comprimidas en {:.2} ms (normales {plain}, \
                 comprimidas {packed_now}, en caché {cached})",
                took.as_secs_f64() * 1000.0
            );
        }
        self.compact_at = if packed >= COMPACT_BUDGET {
            // Quedó trabajo: otra tanda en breve, sin retener el terminal de un tirón.
            ctx.request_repaint_after(COMPACT_SOON);
            Some(now + COMPACT_SOON)
        } else {
            None
        };
    }

    pub fn poll(&mut self, ctx: &egui::Context, watching: bool) -> bool {
        let now = Instant::now();
        self.compact_history(ctx, now);
        self.run_pending(ctx, now);
        let before = (self.fg_pid, self.busy);
        let mut moved = false;
        let interval = if self.dirty { 200 } else { 1000 };
        let due = now.duration_since(self.last_poll) >= Duration::from_millis(interval);
        if due && (self.dirty || self.busy) {
            self.last_poll = now;
            self.dirty = false;
            let fg = proc::foreground_pid(self.master_fd).unwrap_or(self.shell_pid);
            // Se mira también el nombre: si el sondeo cae entre el `fork` del shell y el
            // `exec` del programa, el pid nuevo aún se llama «zsh»; tras el `exec` el pid es
            // el mismo, y sin esto la pestaña se quedaba en «zsh», sin agente y «Corriendo…».
            let comm = proc::name(fg);
            if fg != self.fg_pid || comm != self.fg_comm || self.fg_name.is_empty() {
                self.fg_pid = fg;
                self.fg_comm = comm.clone();
                self.fg_name = comm;
                if self
                    .fg_name
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_digit())
                {
                    if let Some(arg0) = proc::args(fg).first() {
                        self.fg_name = arg0.rsplit('/').next().unwrap_or(arg0).to_string();
                    }
                }
                // ¿Sesión remota? Se mira el proceso en primer plano y sus padres hasta el
                // shell: `sshpass … ssh`, o lo que el propio ssh lance en local.
                self.remote = false;
                self.remote_target = None;
                let (mut pid, mut hops) = (fg, 0);
                while pid != self.shell_pid && pid > 1 && hops < 6 {
                    let name = proc::name(pid);
                    if matches!(
                        name.as_str(),
                        "ssh" | "sshpass" | "autossh" | "slogin" | "mosh-client"
                    ) {
                        self.remote = true;
                        self.remote_target = ssh_target(&proc::args(pid));
                        break;
                    }
                    let Some(parent) = proc::parent(pid) else {
                        break;
                    };
                    (pid, hops) = (parent, hops + 1);
                }
                let agent = if fg == self.shell_pid {
                    None
                } else {
                    Agent::detect(fg, &self.fg_name)
                };
                if agent != self.agent {
                    self.state_since = now;
                }
                self.agent = agent;
            }
            if let Some(dir) = proc::cwd(fg).or_else(|| proc::cwd(self.shell_pid)) {
                moved = dir != self.cwd;
                self.cwd = dir;
            }
            // Un archivo pequeño, como mucho una vez por segundo y solo con actividad.
            if matches!(self.agent, Some(Agent::Claude))
                && let Some(r) = crate::claude::of_pid(fg)
            {
                self.claude = Some(r);
            }
        }

        let busy = match self.agent {
            Some(_) => match self.title_hint() {
                Some(hint) => hint,
                None => {
                    now.duration_since(self.last_active_output) < Duration::from_millis(2000)
                        && self.last_active_output.duration_since(self.streak_start)
                            > Duration::from_millis(1000)
                }
            },
            None => self.fg_pid != self.shell_pid,
        };

        let mut notify = false;
        if busy && !self.busy {
            self.busy_since = now;
            self.state_since = now;
            // Un «terminó» que aún no viste sobrevive a un parpadeo de actividad: si se
            // borrara aquí y la ráfaga nueva fuera corta, la pestaña quedaría marcada como
            // terminada pero sin fecha, y ⌘J no la encontraría.
            if !self.unseen {
                self.finished_at = None;
            }
        } else if !busy && self.busy {
            self.state_since = now;
            let ran = now.duration_since(self.busy_since);
            let min = if self.agent.is_some() { 2 } else { 8 };
            if ran >= Duration::from_secs(min) {
                self.finished_at = Some(now);
                if !watching {
                    self.unseen = true;
                    notify = true;
                }
            }
        }
        self.busy = busy;
        // Esto corre en `logic()`, y lo que cambia aquí (carpeta, proceso, «trabajando»)
        // se dibuja en `ui()`: sin pedir otro cuadro, el cambio no se vería hasta la
        // siguiente tecla. Era el caso de `cd`: el sondeo llega después de que el shell
        // pinte el prompt, y las cajas no se sellaban porque la carpeta aún era la vieja.
        if moved || before != (self.fg_pid, self.busy) {
            ctx.request_repaint();
        }
        notify
    }

    /// Algunos agentes (Claude Code) anuncian su estado en el título:
    /// spinner braille = trabajando, ✳ = esperando.
    fn title_hint(&self) -> Option<bool> {
        let first = self.title.chars().next()?;
        if ('\u{2800}'..='\u{28FF}').contains(&first) {
            Some(true)
        } else if first == '✳' {
            Some(false)
        } else {
            None
        }
    }

    /// Título del programa sin el símbolo de estado inicial.
    pub fn clean_title(&self) -> &str {
        let t = self.title.trim();
        match t.chars().next() {
            Some(c) if !c.is_alphanumeric() && c != '~' && c != '/' => t[c.len_utf8()..].trim(),
            _ => t,
        }
    }

    pub fn shell_pid(&self) -> i32 {
        self.shell_pid
    }

    pub fn project(&self) -> String {
        proc::base_name(&self.cwd)
    }

    /// Texto de la pestaña: siempre dice dónde estás.
    pub fn tab_label(&self) -> String {
        if !self.name.is_empty() {
            return self.name.clone();
        }
        match self.agent {
            // Claude Code pone el nombre de la sesión en el título: «carpeta · sesión».
            Some(agent) => {
                let project = self.project();
                let title = self.clean_title();
                if agent == Agent::Claude
                    && !title.is_empty()
                    && title != agent.label()
                    && title != project
                {
                    format!("{project} · {title}")
                } else {
                    project
                }
            }
            None if self.fg_pid != self.shell_pid && !self.fg_name.is_empty() => {
                format!("{} · {}", self.fg_name, self.project())
            }
            None => proc::pretty_path(&self.cwd),
        }
    }

    pub fn wants_ticks(&self) -> bool {
        self.busy || self.dirty
    }

    /// La actividad que debe ver `git::Watch` (ver `git_activity`).
    pub fn git_activity(&mut self) -> u64 {
        git_activity(
            &mut self.git_frozen,
            self.activity,
            self.fg_pid == self.shell_pid,
        )
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.sender.send(Msg::Shutdown);
        if self.master_fd >= 0 {
            unsafe { libc::close(self.master_fd) };
        }
    }
}

#[cfg(test)]
mod tests {
    /// El eco de una tecla debe costar una sola pasada de egui: con `request_repaint()` egui
    /// pinta dos seguidas y la segunda ya no tiene nada nuevo que enseñar.
    #[test]
    fn eco_de_una_tecla_una_sola_pasada() {
        use super::{LAST_INPUT, Listener, Pace, now_ms};
        use alacritty_terminal::event::{Event, EventListener};
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
        use std::sync::{Arc, Mutex};
        let ctx = egui::Context::default();
        let (tx, _rx) = std::sync::mpsc::channel();
        let listener = Listener {
            tx,
            pace: Arc::new(Pace {
                shown: AtomicBool::new(true),
                last_frame: AtomicU64::new(0),
                ctx: Mutex::new(ctx.clone()),
            }),
        };
        // Una pasada, como el lanzador: epaint exige vaciar las texturas que entrega.
        let pass = || {
            ctx.run_ui(egui::RawInput::default(), |_| {})
                .textures_delta
                .clear()
        };
        // Que egui no tenga nada pendiente antes del eco.
        for _ in 0..5 {
            pass();
        }
        assert!(!ctx.has_requested_repaint());

        // Se acaba de pulsar una tecla y llega su eco.
        LAST_INPUT.store(now_ms(), Ordering::Relaxed);
        listener.send_event(Event::Wakeup);
        let mut passes = 0;
        while ctx.has_requested_repaint() && passes < 5 {
            passes += 1;
            pass();
        }
        assert_eq!(passes, 1, "pasadas de egui por el eco de una tecla");
    }

    #[test]
    fn git_no_se_consulta_con_un_agente_delante() {
        use super::git_activity;
        let mut frozen = 0;
        // Con el shell delante, git sigue a la salida de la pestaña.
        assert_eq!(git_activity(&mut frozen, 5, true), 5);
        assert_eq!(git_activity(&mut frozen, 6, true), 6);
        // Con un agente delante, su salida no es motivo para mirar git.
        assert_eq!(git_activity(&mut frozen, 40, false), 6);
        assert_eq!(git_activity(&mut frozen, 90, false), 6);
        // Vuelve el shell: se mira otra vez.
        assert_eq!(git_activity(&mut frozen, 91, true), 91);
    }

    #[test]
    fn orden_pendiente_cuando_toca() {
        use super::{PendingStep, pending_step};
        use std::time::Duration;
        let ms = Duration::from_millis;
        // Shell listo y quieto 300 ms: se ejecuta.
        assert_eq!(
            pending_step(ms(500), false, true, ms(300)),
            PendingStep::Run
        );
        // Aún pintando el prompt: espera lo que falta.
        assert_eq!(
            pending_step(ms(500), false, true, ms(100)),
            PendingStep::Wait(ms(200))
        );
        // Sin prompt todavía (o algo corriendo encima): vuelve a mirar en 300 ms.
        assert_eq!(
            pending_step(ms(500), false, false, ms(900)),
            PendingStep::Wait(ms(300))
        );
        // Si tecleaste en la pestaña, la orden ya no se escribe (borraría lo tuyo).
        assert_eq!(
            pending_step(ms(500), true, true, ms(900)),
            PendingStep::Drop
        );
        // Si el shell no llega a su prompt en 10 s, se abandona: ni sondeo sin fin ni el
        // agente arrancando mucho después, al salir de lo que hubiera encima.
        assert_eq!(
            pending_step(ms(10_001), false, false, ms(900)),
            PendingStep::Drop
        );
        assert_eq!(
            pending_step(ms(10_001), false, true, ms(900)),
            PendingStep::Drop
        );
    }

    use super::*;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    fn words(s: &str, upto: Option<usize>) -> Vec<(usize, usize, String, bool)> {
        path_words(&chars(s), upto)
    }

    #[test]
    fn palabras_de_ruta() {
        assert_eq!(words("cd src", None), [(3, 6, "src".into(), false)]);
        assert_eq!(
            words("ls -la src/main.rs", None),
            [(7, 18, "src/main.rs".into(), false)]
        );
        assert_eq!(
            words("cat mi\\ archivo.txt", None),
            [(4, 19, "mi archivo.txt".into(), false)]
        );
        assert_eq!(
            words("cat 'mi archivo'", None),
            [(4, 16, "mi archivo".into(), false)]
        );
        assert_eq!(
            words("devops-scripts/ x", None)[0],
            (0, 15, "devops-scripts/".into(), false)
        );
        // «git» es la orden; «status» se mira (y no existirá).
        assert_eq!(words("git status", None), [(4, 10, "status".into(), false)]);
        assert!(words("echo $HOME/x *.rs --file=a", None).is_empty());
        assert_eq!(
            words("make && ./run.sh", None),
            [(8, 16, "./run.sh".into(), false)]
        );
        assert_eq!(
            words("sort > out.txt", None),
            [(7, 14, "out.txt".into(), false)]
        );
        assert_eq!(words("A=1 ls src", None), [(7, 10, "src".into(), false)]);
    }

    #[test]
    fn tramos_partidos_por_filas() {
        let at = [(5, 78), (5, 79), (6, 0), (6, 1)];
        assert_eq!(split_rows(&at, 0, 4), [(5, 78, 80), (6, 0, 2)]);
        assert_eq!(split_rows(&at, 1, 2), [(5, 79, 80)]);
    }

    #[test]
    fn palabras_de_ruta_hasta_el_cursor() {
        // «cd s» tecleado; «rc && cargo build» es la sugerencia gris de zsh-autosuggestions.
        assert_eq!(
            words("cd src && cargo build", Some(4)),
            [(3, 4, "s".into(), true)]
        );
        assert_eq!(words("cd src ", Some(7)), [(3, 6, "src".into(), false)]);
        assert!(words("cd ", Some(3)).is_empty());
    }

    #[test]
    fn existe_relativa_a_la_carpeta_dada() {
        let dir = std::env::temp_dir().join(format!("colibri-rutas-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "").unwrap();
        assert!(path_exists(&dir, "src", false));
        assert!(path_exists(&dir, "src/main.rs", false));
        assert!(path_exists(&dir, "sr", true));
        assert!(!path_exists(&dir, "sr", false));
        assert!(path_exists(&dir, "src/", true));
        assert!(path_exists(&dir, "src/ma", true));
        assert!(!path_exists(&dir.join("borrada"), "src", false));
        assert!(path_exists(Path::new("/"), "~", false));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remote_prompts() {
        let p = remote_prompt(&chars("muy@aid-muysabaneta:~$ docker ps")).unwrap();
        assert_eq!(p.host.as_deref(), Some("muy@aid-muysabaneta"));
        assert_eq!((p.dir.as_deref(), p.len), (Some("~"), 23));
        let p = remote_prompt(&chars("[root@db01 /var/log]# ")).unwrap();
        assert_eq!(p.host.as_deref(), Some("root@db01"));
        assert_eq!((p.dir.as_deref(), p.len), (Some("/var/log"), 22));
        let p = remote_prompt(&chars("(venv) ana@web.local:~/app$ ")).unwrap();
        assert_eq!(p.dir.as_deref(), Some("~/app"));
        // oh-my-zsh: sin máquina en el prompt.
        let p = remote_prompt(&chars("➜  ~ docker ps")).unwrap();
        assert_eq!((p.host, p.dir.as_deref(), p.len), (None, Some("~"), 5));
        let p = remote_prompt(&chars("➜  app git:(main) ✗ ls")).unwrap();
        assert_eq!((p.dir.as_deref(), p.len), (Some("app"), 20));
        // Genéricos.
        let p = remote_prompt(&chars("~/app $ ls")).unwrap();
        assert_eq!((p.dir.as_deref(), p.len), (Some("~/app"), 8));
        assert_eq!(remote_prompt(&chars("servidor% ")).unwrap().len, 10);
        // Salida normal no es un prompt.
        assert_eq!(remote_prompt(&chars("Welcome to Ubuntu 22.04")), None);
        assert_eq!(
            remote_prompt(&chars(
                "Last login: Thu Nov 19 14:41:17 2026 from 127.0.0.1"
            )),
            None
        );
        assert_eq!(
            remote_prompt(&chars("  System load:  0.26   Processes: 185")),
            None
        );
        assert_eq!(remote_prompt(&chars("Descargando 50% hecho")), None);
        assert_eq!(remote_prompt(&chars("  Swap usage:   2% ")), None);
    }

    #[test]
    fn host_from_title_and_args() {
        assert_eq!(
            title_host("muy@aid-muysabaneta: ~/app"),
            Some(("muy@aid-muysabaneta".into(), Some("~/app".into())))
        );
        assert_eq!(
            title_host("muy@aid:~"),
            Some(("muy@aid".into(), Some("~".into())))
        );
        assert_eq!(title_host("ssh muy@tunnel.muy.com.co -p 1054"), None);
        assert_eq!(title_host("vim notas.txt"), None);
        let args = |s: &str| s.split(' ').map(str::to_string).collect::<Vec<_>>();
        assert_eq!(
            ssh_target(&args("sshpass -p clave ssh muy@tunnel.muy.com.co -p 1054")).as_deref(),
            Some("muy@tunnel.muy.com.co")
        );
        assert_eq!(
            ssh_target(&args("/usr/bin/ssh -p 22 -i k web01 uptime")).as_deref(),
            Some("web01")
        );
    }

    #[test]
    fn command_tokens() {
        let toks = tokenize(&chars("docker logs --follow 'mi app' | grep -i error"));
        assert_eq!(
            toks,
            vec![
                (0, 6, Tok::Command),
                (12, 20, Tok::Flag),
                (21, 29, Tok::Str),
                (32, 36, Tok::Command),
                (37, 39, Tok::Flag),
            ]
        );
        assert_eq!(
            tokenize(&chars("sudo systemctl restart x"))[1],
            (5, 14, Tok::Command)
        );
    }

    #[test]
    fn prompt_result_from_tag() {
        let link = Hyperlink::new(
            None::<String>,
            "colibri-prompt:/tmp/a%20b;s=255;t=1095".into(),
        );
        assert_eq!(tag_result(&link), Some((255, 1095)));
        assert_eq!(tag_dir(&link), PathBuf::from("/tmp/a b"));
        let plain = Hyperlink::new(None::<String>, "colibri-prompt:/tmp".into());
        assert_eq!(tag_result(&plain), None);
    }
}
