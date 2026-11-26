//! Rama y número de archivos con cambios de la carpeta de la pestaña activa, para la barra
//! de estado. La rama se lee de `.git/HEAD`; los cambios salen de `git status`, que corre en
//! un hilo aparte y solo cuando hubo actividad en la pestaña desde la última consulta.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Default, PartialEq)]
pub struct Info {
    pub branch: String,
    /// Archivos modificados, nuevos o borrados. None = aún no se sabe.
    pub changes: Option<usize>,
}

/// Entre dos `git status` seguidos: en repos grandes no es gratis.
const MIN_GAP: Duration = Duration::from_secs(3);

pub struct Watch {
    /// Carpeta consultada y lo que se encontró (None = no es un repo).
    result: Arc<Mutex<(PathBuf, Option<Info>)>>,
    running: Arc<AtomicBool>,
    /// Carpeta, actividad y rama de la pestaña en la última consulta lanzada.
    asked: Option<(PathBuf, u64, Option<String>)>,
    asked_at: Instant,
    /// Rama leída de `.git/HEAD` para esa carpeta y actividad. Se lee aparte de `git status`
    /// (es un archivo, no cuesta) para que un `git checkout` se vea al momento.
    head: Option<(PathBuf, u64, Option<String>)>,
}

impl Watch {
    pub fn new() -> Self {
        Self {
            result: Arc::new(Mutex::new((PathBuf::new(), None))),
            running: Arc::new(AtomicBool::new(false)),
            asked: None,
            asked_at: Instant::now() - MIN_GAP,
            head: None,
        }
    }

    /// Lo que se sabe de `cwd`. `activity` cambia cuando la pestaña recibe salida: es la
    /// señal de que puede haber cambiado algo (un commit, un checkout) y hay que mirar otra vez.
    pub fn info(&mut self, ctx: &egui::Context, cwd: &Path, activity: u64) -> Option<Info> {
        // La rama se relee con cada actividad, sin esperar a `git status` ni a `MIN_GAP`.
        if self
            .head
            .as_ref()
            .is_none_or(|(dir, seen, _)| dir != cwd || *seen != activity)
        {
            self.head = Some((cwd.to_path_buf(), activity, branch(cwd)));
        }
        let head = self.head.as_ref().and_then(|(_, _, b)| b.clone());
        let stale = self
            .asked
            .as_ref()
            .is_none_or(|(dir, seen, _)| dir != cwd || *seen != activity);
        // Carpeta nueva o rama nueva (un checkout cambia los archivos): consultar ya.
        let urgent = self
            .asked
            .as_ref()
            .is_none_or(|(dir, _, b)| dir != cwd || *b != head);
        if (stale || urgent) && !self.running.load(Ordering::Relaxed) {
            let wait = MIN_GAP.saturating_sub(self.asked_at.elapsed());
            if urgent || wait.is_zero() {
                self.asked = Some((cwd.to_path_buf(), activity, head.clone()));
                self.asked_at = Instant::now();
                self.running.store(true, Ordering::Relaxed);
                let (result, running, ctx, dir) = (
                    self.result.clone(),
                    self.running.clone(),
                    ctx.clone(),
                    cwd.to_path_buf(),
                );
                std::thread::spawn(move || {
                    let info = branch(&dir).map(|branch| Info {
                        changes: changes(&dir),
                        branch,
                    });
                    *result.lock().unwrap() = (dir, info);
                    running.store(false, Ordering::Relaxed);
                    ctx.request_repaint();
                });
            } else {
                ctx.request_repaint_after(wait);
            }
        }
        // Si había un `git status` en curso con la rama anterior, su `request_repaint` al
        // acabar trae de vuelta aquí y `urgent` lanza el nuevo.
        let result = self.result.lock().unwrap();
        // Los cambios del último `git status` de esta carpeta (aunque sea de la rama anterior,
        // hasta que llegue el nuevo) y la rama recién leída.
        let changes = (result.0 == cwd)
            .then(|| result.1.as_ref().and_then(|i| i.changes))
            .flatten();
        head.map(|branch| Info { branch, changes })
    }
}

/// Rama actual del repo que contiene `dir`, o el commit abreviado si HEAD está suelto.
pub fn branch(dir: &Path) -> Option<String> {
    let dot_git = dir
        .ancestors()
        .map(|d| d.join(".git"))
        .find(|p| p.exists())?;
    // En un worktree o submódulo `.git` es un archivo que apunta a la carpeta real.
    let git_dir = if dot_git.is_file() {
        let text = std::fs::read_to_string(&dot_git).ok()?;
        let target = text.trim().strip_prefix("gitdir:")?.trim();
        dot_git.parent()?.join(target)
    } else {
        dot_git
    };
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();
    Some(match head.strip_prefix("ref: refs/heads/") {
        Some(name) => name.to_string(),
        None => head.chars().take(7).collect(),
    })
}

/// Ramas locales del repo de `dir`, de la que tuvo el commit más reciente a la más antigua.
pub fn branches(dir: &Path) -> Vec<String> {
    let out = std::process::Command::new("git")
        .arg("--no-optional-locks")
        .arg("-C")
        .arg(dir)
        .args([
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(refname:short)",
            "refs/heads",
        ])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();
    match out {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::to_string)
            .filter(|l| !l.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

/// Una rama del repositorio, con su último commit: fecha (unix) y primera línea del mensaje.
/// En las remotas, `name` lleva el remoto delante (`origin/x`).
#[derive(Clone, PartialEq, Debug)]
pub struct Ref {
    pub name: String,
    pub remote: bool,
    pub time: u64,
    pub subject: String,
}

/// Ramas locales y remotas del repo de `dir`, de la que tuvo el commit más reciente a la más
/// antigua. Solo lee el repositorio local (no hay `fetch`).
pub fn refs(dir: &Path) -> Vec<Ref> {
    git_output(
        dir,
        &[
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(refname)%00%(committerdate:unix)%00%(subject)",
            "refs/heads",
            "refs/remotes",
        ],
    )
    .map(|out| parse_refs(&out))
    .unwrap_or_default()
}

/// Las líneas de `for-each-ref` con el formato de `refs`: `refs/heads/x` o
/// `refs/remotes/origin/x`, NUL, fecha, NUL, mensaje. Las `…/HEAD` de los remotos no son
/// ramas y se saltan.
pub fn parse_refs(out: &str) -> Vec<Ref> {
    out.lines()
        .filter_map(|line| {
            let mut parts = line.split('\0');
            let full = parts.next()?;
            let time = parts.next()?.trim().parse().unwrap_or(0);
            let subject = parts.next().unwrap_or("").to_string();
            let (name, remote) = match full.strip_prefix("refs/heads/") {
                Some(name) => (name, false),
                None => (full.strip_prefix("refs/remotes/")?, true),
            };
            if name.is_empty() || (remote && name.ends_with("/HEAD")) {
                return None;
            }
            Some(Ref {
                name: name.to_string(),
                remote,
                time,
                subject,
            })
        })
        .collect()
}

/// Alias de git (`git config alias.co checkout`) que ve el repo de `dir`: nombre y valor.
pub fn aliases(dir: &Path) -> Vec<(String, String)> {
    git_output(dir, &["config", "--get-regexp", r"^alias\."])
        .map(|out| parse_aliases(&out))
        .unwrap_or_default()
}

/// Las líneas de `git config --get-regexp ^alias\.`: `alias.co checkout`.
pub fn parse_aliases(out: &str) -> Vec<(String, String)> {
    out.lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(' ')?;
            Some((key.strip_prefix("alias.")?.to_string(), value.to_string()))
        })
        .collect()
}

/// Lo que escribe una orden de git en `dir`, si acaba bien. Sin tocar el índice ni leer del
/// teclado.
fn git_output(dir: &Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git")
        .arg("--no-optional-locks")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn changes(dir: &Path) -> Option<usize> {
    // --no-optional-locks: no tocar el índice, para no estorbar a un git que esté corriendo.
    let out = std::process::Command::new("git")
        .arg("--no-optional-locks")
        .arg("-C")
        .arg(dir)
        .args(["status", "--porcelain"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status.success().then(|| {
        out.stdout
            .split(|b| *b == b'\n')
            .filter(|l| !l.is_empty())
            .count()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramas_de_for_each_ref() {
        let out = "refs/heads/main\x00300\x00inicio\n\
                   refs/remotes/origin/HEAD\x00300\x00inicio\n\
                   refs/remotes/origin/feat-c\x00250\x00trabajo en c\n";
        let r = parse_refs(out);
        assert_eq!(r.len(), 2);
        assert_eq!((r[0].name.as_str(), r[0].remote), ("main", false));
        assert_eq!(
            (r[1].name.as_str(), r[1].remote, r[1].time),
            ("origin/feat-c", true, 250)
        );
        assert_eq!(r[1].subject, "trabajo en c");
        assert_eq!(
            parse_aliases("alias.co checkout\nalias.st status -sb\n")[1],
            ("st".to_string(), "status -sb".to_string())
        );
    }

    #[test]
    fn branch_from_head() {
        let dir = std::env::temp_dir().join(format!("colibri-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        std::fs::write(dir.join(".git/HEAD"), "ref: refs/heads/feat/x\n").unwrap();
        assert_eq!(branch(&dir.join("a/b")).as_deref(), Some("feat/x"));
        std::fs::write(dir.join(".git/HEAD"), "0123456789abcdef\n").unwrap();
        assert_eq!(branch(&dir).as_deref(), Some("0123456"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
