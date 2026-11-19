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
    /// Carpeta y actividad de la pestaña en la última consulta lanzada.
    asked: Option<(PathBuf, u64)>,
    asked_at: Instant,
}

impl Watch {
    pub fn new() -> Self {
        Self {
            result: Arc::new(Mutex::new((PathBuf::new(), None))),
            running: Arc::new(AtomicBool::new(false)),
            asked: None,
            asked_at: Instant::now() - MIN_GAP,
        }
    }

    /// Lo que se sabe de `cwd`. `activity` cambia cuando la pestaña recibe salida: es la
    /// señal de que puede haber cambiado algo (un commit, un checkout) y hay que mirar otra vez.
    pub fn info(&mut self, ctx: &egui::Context, cwd: &Path, activity: u64) -> Option<Info> {
        let stale = self
            .asked
            .as_ref()
            .is_none_or(|(dir, seen)| dir != cwd || *seen != activity);
        let new_dir = self.asked.as_ref().is_none_or(|(dir, _)| dir != cwd);
        if stale && !self.running.load(Ordering::Relaxed) {
            let wait = MIN_GAP.saturating_sub(self.asked_at.elapsed());
            if new_dir || wait.is_zero() {
                self.asked = Some((cwd.to_path_buf(), activity));
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
        let result = self.result.lock().unwrap();
        if result.0 == cwd {
            result.1.clone()
        } else {
            // Carpeta recién cambiada: la rama es una lectura de archivo, se puede dar ya.
            branch(cwd).map(|branch| Info {
                branch,
                changes: None,
            })
        }
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
