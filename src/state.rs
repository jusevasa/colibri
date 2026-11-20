//! Pestañas abiertas, guardadas en ~/.config/colibri/session.toml para restaurarlas al abrir.
//! Se guarda dónde estaba cada pestaña, no los procesos: esos mueren al cerrar la app.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Default, PartialEq, Clone)]
#[serde(default)]
pub struct Saved {
    /// Formato antiguo, de cuando había una sola ventana: se lee pero ya no se escribe.
    #[serde(skip_serializing_if = "is_zero")]
    pub active: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tabs: Vec<Tab>,
    /// Las ventanas, de la que estaba al frente a la última.
    pub windows: Vec<Window>,
}

#[derive(Serialize, Deserialize, Default, PartialEq, Clone)]
#[serde(default)]
pub struct Window {
    pub active: usize,
    pub tabs: Vec<Tab>,
    /// Posición y tamaño en puntos: x e y de la esquina de la ventana, ancho y alto del
    /// contenido. Vacío = donde la ponga el sistema.
    pub frame: Option<[f32; 4]>,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

impl Saved {
    /// Las ventanas guardadas; un archivo del formato antiguo es una sola ventana.
    pub fn windows(&self) -> Vec<Window> {
        if self.windows.is_empty() && !self.tabs.is_empty() {
            return vec![Window {
                active: self.active,
                tabs: self.tabs.clone(),
                frame: None,
            }];
        }
        self.windows.clone()
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sesion_antigua_es_una_ventana() {
        let old = "active = 1\n[[tabs]]\ncwd = \"/tmp\"\n[[tabs]]\ncwd = \"/\"\n";
        let saved: Saved = toml::from_str(old).unwrap();
        let windows = saved.windows();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].active, 1);
        assert_eq!(windows[0].tabs.len(), 2);
        assert_eq!(windows[0].frame, None);
    }

    #[test]
    fn sesion_nueva_ida_y_vuelta() {
        let saved = Saved {
            windows: vec![
                Window {
                    active: 0,
                    tabs: vec![Tab {
                        cwd: "/tmp".into(),
                        ..Default::default()
                    }],
                    frame: Some([10.0, 20.0, 800.0, 600.0]),
                },
                Window::default(),
            ],
            ..Default::default()
        };
        let text = toml::to_string(&saved).unwrap();
        assert!(!text.contains("active = 0\n[[tabs]]"));
        let back: Saved = toml::from_str(&text).unwrap();
        assert!(back == saved);
        assert_eq!(back.windows().len(), 2);
    }
}
