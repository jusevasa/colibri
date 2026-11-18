//! Pestañas abiertas, guardadas en ~/.config/colibri/session.toml para restaurarlas al abrir.
//! Se guarda dónde estaba cada pestaña, no los procesos: esos mueren al cerrar la app.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Default, PartialEq, Clone)]
#[serde(default)]
pub struct Saved {
    pub active: usize,
    pub tabs: Vec<Tab>,
}

#[derive(Serialize, Deserialize, Default, PartialEq, Clone)]
#[serde(default)]
pub struct Tab {
    pub cwd: String,
    /// Nombre personalizado de la pestaña, o vacío.
    pub name: String,
    /// Color de la pestaña ("azul"), o vacío.
    pub color: String,
    /// Agente que corría en la pestaña ("claude"), o vacío.
    pub agent: String,
}

/// Con un guion de depuración activo no se toca la sesión real del usuario,
/// salvo que COLIBRI_STATE apunte a otro archivo.
pub fn path(debugging: bool) -> Option<PathBuf> {
    if let Ok(p) = std::env::var("COLIBRI_STATE") {
        return Some(PathBuf::from(p));
    }
    if debugging {
        return None;
    }
    Some(dirs::home_dir()?.join(".config/colibri/session.toml"))
}

pub fn load(path: &PathBuf) -> Saved {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| toml::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save(path: &PathBuf, saved: &Saved) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = toml::to_string(saved) {
        // Escribir aparte y renombrar: nunca queda un archivo a medias.
        let tmp = path.with_extension("toml.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}
