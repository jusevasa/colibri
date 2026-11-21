//! «Buscar actualizaciones» (preferencia `check_updates`, activada por defecto): la única
//! conexión a internet de Colibrí, y se puede apagar. Al encenderla (o al arrancar con ella
//! encendida) y luego cada 24 h, lee con el `curl` del sistema el cask del tap de Homebrew
//! (justo lo que instalaría `brew upgrade`) y, si su versión es más nueva que la que corre, la
//! publica para que la barra de pestañas enseñe el aviso. Es un GET sin nada propio: `curl -q`
//! no lee `~/.curlrc`, no se añaden cabeceras y solo se admite https.

use std::cmp::Ordering;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::switch::Switch;

/// El cask que escribe el flujo de publicación (`scripts/cask.sh`).
const CASK_URL: &str =
    "https://raw.githubusercontent.com/jusevasa/homebrew-colibri/main/Casks/colibri.rb";
/// Cada cuánto se vuelve a mirar con la preferencia encendida.
const PERIOD: Duration = Duration::from_secs(24 * 60 * 60);
/// Orden que el aviso escribe en una pestaña nueva si Colibrí se instaló con Homebrew.
pub const BREW_UPGRADE: &str = "brew upgrade --cask colibri";

/// Página de una versión publicada, para quien no instaló con Homebrew.
pub fn release_url(version: &str) -> String {
    format!("https://github.com/jusevasa/colibri/releases/tag/v{version}")
}

pub struct Updates {
    /// Versión publicada más nueva que la que corre, si la hay.
    available: Arc<Mutex<Option<String>>>,
    /// Lo enciende `logic()` con la preferencia: apagado, el hilo duerme sin despertar.
    pub switch: Arc<Switch>,
}

impl Updates {
    /// `network`: falso con el guion de depuración, que no debe conectarse a nada (el aviso se
    /// simula con `update available`); entonces no hay hilo.
    pub fn start(current: &'static str, network: bool) -> Self {
        let available = Arc::new(Mutex::new(None));
        let switch = Arc::new(Switch::default());
        if network {
            let (a, sw) = (available.clone(), switch.clone());
            let _ = std::thread::Builder::new()
                .name("colibri-updates".into())
                .spawn(move || {
                    loop {
                        // Apagado espera sin plazo a que lo enciendan; encendido, 24 h. Al
                        // encenderse despierta al momento, así que la primera vez mira ya.
                        sw.wait(PERIOD);
                        if !sw.is_on() {
                            continue;
                        }
                        // Sin conexión o con el servidor caído se queda lo que se sabía.
                        let Some(text) = fetch() else { continue };
                        let found = cask_version(&text).filter(|v| newer(v, current));
                        let mut slot = a.lock().unwrap_or_else(|e| e.into_inner());
                        if *slot != found {
                            *slot = found;
                            crate::shell::wake_all();
                        }
                    }
                });
        }
        Self { available, switch }
    }

    pub fn available(&self) -> Option<String> {
        self.available
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Solo para el guion: simula que hay (o no) una versión nueva.
    pub fn set_available(&self, version: Option<String>) {
        *self.available.lock().unwrap_or_else(|e| e.into_inner()) = version;
    }
}

/// Descarga el cask. `COLIBRI_UPDATES_URL` cambia la dirección (para probar con un servidor
/// local o un `file://`); con la de siempre solo se admite https.
fn fetch() -> Option<String> {
    match std::env::var("COLIBRI_UPDATES_URL") {
        Ok(url) => fetch_url(&url, false),
        Err(_) => fetch_url(CASK_URL, true),
    }
}

fn fetch_url(url: &str, https_only: bool) -> Option<String> {
    let mut cmd = Command::new("/usr/bin/curl");
    // -q: sin ~/.curlrc (podría añadir cabeceras o un proxy); --fail: un 404 no es un cask.
    cmd.args([
        "-q",
        "--fail",
        "--silent",
        "--location",
        "--max-redirs",
        "3",
    ])
    .args(["--max-time", "20"]);
    if https_only {
        cmd.args(["--proto", "=https", "--proto-redir", "=https"]);
    }
    let out = cmd.arg(url).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

/// La versión de un cask: su línea `version "…"`.
fn cask_version(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let v = line.trim().strip_prefix("version \"")?.strip_suffix('"')?;
        let valid = !v.is_empty()
            && v.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'));
        valid.then(|| v.to_string())
    })
}

/// ¿`candidate` es posterior a `current`? Con las reglas de semver: la versión final va
/// después de sus prerelease («1.2.3» > «1.2.3-beta.2» > «1.2.3-beta.1») y los
/// identificadores numéricos se comparan como números («beta.10» > «beta.9»). Una versión
/// que no se entiende nunca cuenta como posterior.
pub fn newer(candidate: &str, current: &str) -> bool {
    compare(candidate, current) == Some(Ordering::Greater)
}

fn compare(a: &str, b: &str) -> Option<Ordering> {
    let (core_a, pre_a) = parse(a)?;
    let (core_b, pre_b) = parse(b)?;
    Some(core_a.cmp(&core_b).then_with(|| {
        match (pre_a.is_empty(), pre_b.is_empty()) {
            (true, true) => Ordering::Equal,
            // Sin prerelease va después que con él: 1.0.0 > 1.0.0-rc.1.
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => compare_pre(&pre_a, &pre_b),
        }
    }))
}

/// «MAYOR.MENOR.PARCHE[-pre][+compilación]» → números y piezas del prerelease.
fn parse(v: &str) -> Option<([u64; 3], Vec<&str>)> {
    let v = v.split('+').next()?;
    let (core, pre) = match v.split_once('-') {
        Some((core, pre)) => (core, pre.split('.').collect()),
        None => (v, Vec::new()),
    };
    let mut nums = core.split('.').map(|n| n.parse::<u64>().ok());
    let core = [nums.next()??, nums.next()??, nums.next()??];
    if nums.next().is_some() || pre.iter().any(|p: &&str| p.is_empty()) {
        return None;
    }
    Some((core, pre))
}

fn compare_pre(a: &[&str], b: &[&str]) -> Ordering {
    for (x, y) in a.iter().zip(b) {
        let o = match (x.parse::<u64>(), y.parse::<u64>()) {
            (Ok(m), Ok(n)) => m.cmp(&n),
            // Un número va antes que un texto: beta.1 < beta.rc.
            (Ok(_), Err(_)) => Ordering::Less,
            (Err(_), Ok(_)) => Ordering::Greater,
            _ => x.cmp(y),
        };
        if o != Ordering::Equal {
            return o;
        }
    }
    a.len().cmp(&b.len())
}

/// ¿Esta copia la instaló Homebrew? Corre desde `/Applications` (o `~/Applications`) y hay
/// un Caskroom de colibri. Si no, el aviso abre la página de la versión en vez de `brew`.
pub fn installed_with_brew() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let home_apps = dirs::home_dir().map(|h| h.join("Applications"));
    let in_apps = exe
        .ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .and_then(Path::parent)
        .is_some_and(|dir| dir == Path::new("/Applications") || Some(dir) == home_apps.as_deref());
    in_apps
        && [
            "/opt/homebrew/Caskroom/colibri",
            "/usr/local/Caskroom/colibri",
        ]
        .iter()
        .any(|p| Path::new(p).is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_del_cask() {
        let cask = "cask \"colibri\" do\n  version \"0.2.0-beta.2\"\n  sha256 \"90c4\"\nend\n";
        assert_eq!(cask_version(cask).as_deref(), Some("0.2.0-beta.2"));
        assert_eq!(cask_version("sin versión"), None);
        assert_eq!(cask_version("  version \"\"\n"), None);
        // Nada que no sea una versión: el texto viene de internet.
        assert_eq!(cask_version("  version \"1.0; rm -rf ~\"\n"), None);
    }

    #[test]
    fn orden_de_versiones() {
        let ordered = [
            "0.2.0-beta.1",
            "0.2.0-beta.2",
            "0.2.0-beta.10",
            "0.2.0-rc.1",
            "0.2.0",
            "0.2.1",
            "0.10.0",
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0",
        ];
        for pair in ordered.windows(2) {
            assert!(newer(pair[1], pair[0]), "{} > {}", pair[1], pair[0]);
            assert!(!newer(pair[0], pair[1]), "{} no > {}", pair[0], pair[1]);
        }
    }

    #[test]
    fn iguales_y_raras_no_son_nuevas() {
        assert!(!newer("0.2.0-beta.2", "0.2.0-beta.2"));
        assert!(!newer("0.2.0+51", "0.2.0"));
        assert!(!newer("basura", "0.2.0"));
        assert!(!newer("1.2", "0.2.0"));
        assert!(!newer("1.2.3.4", "0.2.0"));
        assert!(!newer("1.0.0-", "0.2.0"));
        assert!(!newer("1.0.0-beta..1", "0.2.0"));
    }

    #[test]
    fn descarga_con_curl() {
        let dir = std::env::temp_dir().join(format!("colibri-updates-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cask = dir.join("colibri.rb");
        std::fs::write(&cask, "cask \"colibri\" do\n  version \"9.9.9\"\nend\n").unwrap();
        let url = format!("file://{}", cask.display());
        let text = fetch_url(&url, false).expect("curl lee el archivo");
        assert_eq!(cask_version(&text).as_deref(), Some("9.9.9"));
        // Con la dirección de siempre solo vale https: un file:// se rechaza.
        assert_eq!(fetch_url(&url, true), None);
        // Lo que no existe no es un cask.
        assert_eq!(fetch_url(&format!("{url}.no"), false), None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn la_version_que_corre_se_entiende() {
        assert!(parse(env!("CARGO_PKG_VERSION")).is_some());
    }
}
