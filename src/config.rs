//! Configuración en ~/.config/colibri/config.toml (se recarga sola al guardar).

use eframe::egui::{Color32, Key, Modifiers};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub font_size: f32,
    /// Ruta a un .ttf/.otf/.ttc monoespaciado. Vacío = Menlo.
    pub font_path: String,
    pub font_bold_path: String,
    pub line_height: f32,
    /// Margen alrededor del texto, en puntos.
    pub padding: f32,
    /// Opacidad del fondo de la terminal: 1.0 = sólido, menos = se ve lo de detrás.
    pub opacity: f32,
    /// Desenfoca lo que hay detrás de la ventana (solo se nota con opacity < 1).
    pub blur: bool,
    /// Forma del cursor: "block", "bar" o "underline".
    pub cursor: String,
    pub scrollback: usize,
    /// Vacío = $SHELL
    pub shell: String,
    /// Notificación de macOS cuando un agente o comando largo termina en segundo plano.
    pub notify: bool,
    /// Al abrir, recupera las pestañas que tenías (en sus carpetas) al cerrar.
    pub restore_tabs: bool,
    /// ↑ en el prompt abre la lista de comandos usados antes.
    pub history_popup: bool,
    /// Qué apartados del panel lateral están visibles al abrir.
    pub show_agents: bool,
    pub show_ports: bool,
    pub show_processes: bool,
    pub theme: Theme,
    pub keys: Keys,
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Theme {
    pub background: String,
    pub foreground: String,
    pub cursor: String,
    pub selection: String,
    pub accent: String,
    pub tab_bar: String,
    pub tab_active: String,
    pub panel: String,
    pub muted: String,
    pub border: String,
    pub done: String,
    /// 16 colores ANSI: 8 normales + 8 brillantes.
    pub ansi: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Keys {
    pub new_tab: String,
    pub close_tab: String,
    pub next_tab: String,
    pub prev_tab: String,
    /// Cambia la pestaña activa de posición.
    pub move_tab_left: String,
    pub move_tab_right: String,
    /// Salta a la pestaña del agente que terminó más recientemente.
    pub jump_finished: String,
    /// Muestra u oculta los dos apartados a la vez.
    pub toggle_panel: String,
    pub toggle_agents: String,
    pub toggle_ports: String,
    pub toggle_processes: String,
    pub font_bigger: String,
    pub font_smaller: String,
    pub font_reset: String,
    pub open_config: String,
    pub palette: String,
    pub rename_tab: String,
    pub preferences: String,
    pub clear: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            font_size: 13.0,
            font_path: String::new(),
            font_bold_path: String::new(),
            line_height: 1.15,
            padding: 8.0,
            opacity: 1.0,
            blur: false,
            cursor: "block".into(),
            scrollback: 10_000,
            shell: String::new(),
            notify: true,
            restore_tabs: true,
            history_popup: true,
            show_agents: false,
            show_ports: false,
            show_processes: false,
            theme: Theme::default(),
            keys: Keys::default(),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        let ansi = [
            "#1c2128", "#f47067", "#8ddb8c", "#e0b25b", "#6cb6ff", "#dcbdfb", "#76e3ea", "#c9d1d9",
            "#636e7b", "#ff938a", "#a5e8a4", "#f0c674", "#96d0ff", "#e8d2ff", "#a5f0f5", "#ffffff",
        ];
        Self {
            background: "#0b0e14".into(),
            foreground: "#d7dce4".into(),
            cursor: "#d7dce4".into(),
            selection: "#2b4468".into(),
            accent: "#6cb6ff".into(),
            tab_bar: "#07090d".into(),
            tab_active: "#0b0e14".into(),
            panel: "#090c11".into(),
            muted: "#7d8794".into(),
            border: "#1a1f29".into(),
            done: "#57c96b".into(),
            ansi: ansi.iter().map(|s| s.to_string()).collect(),
        }
    }
}

impl Default for Keys {
    fn default() -> Self {
        Self {
            new_tab: "cmd+t".into(),
            close_tab: "cmd+w".into(),
            next_tab: "cmd+alt+right".into(),
            prev_tab: "cmd+alt+left".into(),
            move_tab_left: "cmd+shift+left".into(),
            move_tab_right: "cmd+shift+right".into(),
            jump_finished: "cmd+j".into(),
            toggle_panel: "cmd+b".into(),
            toggle_agents: "cmd+shift+a".into(),
            toggle_ports: "cmd+shift+p".into(),
            toggle_processes: "cmd+shift+m".into(),
            font_bigger: "cmd+=".into(),
            font_smaller: "cmd+-".into(),
            font_reset: "cmd+0".into(),
            open_config: "cmd+shift+,".into(),
            palette: "cmd+p".into(),
            rename_tab: "cmd+shift+r".into(),
            preferences: "cmd+,".into(),
            clear: "cmd+k".into(),
        }
    }
}

const HEADER: &str = "\
# Colibrí — configuración. Los cambios se aplican al guardar.
# Atajos: combina cmd, ctrl, shift, alt con una tecla: \"cmd+shift+j\", \"cmd+alt+right\", \"ctrl+tab\".
# Fijo: cmd+1..9 va a la pestaña N.
# Colores en formato #rrggbb.

";

pub fn path() -> PathBuf {
    // COLIBRI_CONFIG permite probar con otro archivo sin tocar el del usuario.
    if let Ok(p) = std::env::var("COLIBRI_CONFIG") {
        return PathBuf::from(p);
    }
    dirs::home_dir()
        .unwrap_or_default()
        .join(".config/colibri/config.toml")
}

pub fn load() -> Config {
    let p = path();
    match std::fs::read_to_string(&p) {
        Ok(text) => match toml::from_str::<Config>(&text) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("colibri: config inválida ({e}); usando valores por defecto");
                Config::default()
            }
        },
        Err(_) => {
            let c = Config::default();
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(body) = toml::to_string_pretty(&c) {
                let _ = std::fs::write(&p, format!("{HEADER}{body}"));
            }
            c
        }
    }
}

pub fn save(c: &Config) {
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(body) = toml::to_string_pretty(c) {
        let _ = std::fs::write(&p, format!("{HEADER}{body}"));
    }
}

/// Temas incluidos: (nombre visible, tema).
pub fn presets() -> Vec<(&'static str, Theme)> {
    let base = Theme::default();
    let midnight = Theme {
        background: "#000000".into(),
        tab_bar: "#000000".into(),
        tab_active: "#14171d".into(),
        panel: "#050608".into(),
        border: "#1c2028".into(),
        ..base.clone()
    };
    let light_ansi = [
        "#24292f", "#cf222e", "#116329", "#7d4e00", "#0969da", "#8250df", "#1b7c83", "#6e7781",
        "#57606a", "#a40e26", "#1a7f37", "#633c01", "#218bff", "#a475f9", "#3192aa", "#8c959f",
    ];
    let light = Theme {
        background: "#fafafa".into(),
        foreground: "#24292f".into(),
        cursor: "#24292f".into(),
        selection: "#cfe3ff".into(),
        accent: "#0969da".into(),
        tab_bar: "#e9ebee".into(),
        tab_active: "#fafafa".into(),
        panel: "#f1f2f4".into(),
        muted: "#6e7781".into(),
        border: "#d5d9de".into(),
        done: "#1a7f37".into(),
        ansi: light_ansi.iter().map(|s| s.to_string()).collect(),
    };
    let one_dark_ansi = [
        "#3f4451", "#e06c75", "#98c379", "#e5c07b", "#61afef", "#c678dd", "#56b6c2", "#abb2bf",
        "#5c6370", "#ef7d87", "#a9d48a", "#d19a66", "#74bcf7", "#d48fe8", "#67c7d3", "#ffffff",
    ];
    let one_dark = Theme {
        background: "#282c34".into(),
        foreground: "#abb2bf".into(),
        cursor: "#528bff".into(),
        selection: "#3e4451".into(),
        accent: "#61afef".into(),
        tab_bar: "#21252b".into(),
        tab_active: "#282c34".into(),
        panel: "#21252b".into(),
        muted: "#7f848e".into(),
        border: "#3b4048".into(),
        done: "#98c379".into(),
        ansi: one_dark_ansi.iter().map(|s| s.to_string()).collect(),
    };
    vec![
        ("Colibrí", base),
        ("One Dark", one_dark),
        ("Medianoche", midnight),
        ("Claro", light),
    ]
}

/// Fuentes monoespaciadas disponibles: (nombre, ruta normal, ruta negrita). La primera es Menlo.
pub fn fonts() -> Vec<(String, String, String)> {
    let mut found = vec![("Menlo".to_string(), String::new(), String::new())];
    let mut dirs_to_scan = vec![std::path::PathBuf::from("/Library/Fonts")];
    if let Some(home) = dirs::home_dir() {
        dirs_to_scan.insert(0, home.join("Library/Fonts"));
    }
    for dir in dirs_to_scan {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut names: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        names.sort();
        for file in names {
            let lower = file.to_lowercase();
            let is_font = lower.ends_with(".ttf") || lower.ends_with(".otf");
            let mono = [
                "mono", "code", "nerd", "hack", "consol", "courier", "iosevka", "menlo", "meslo",
                "cascadia",
            ]
            .iter()
            .any(|k| lower.contains(k));
            // Las fuentes variables traen todos los pesos en un solo archivo.
            let variable = lower.contains("variablefont") || lower.contains("[wght]");
            if !is_font
                || !mono
                || lower.contains("italic")
                || !(variable || lower.contains("regular"))
            {
                continue;
            }
            let stem = file.rsplit_once('.').map(|(s, _)| s).unwrap_or(&file);
            let stem = stem.split("-VariableFont").next().unwrap_or(stem);
            let stem = stem.split('[').next().unwrap_or(stem);
            let name = spaced(
                &stem
                    .replace("-Regular", "")
                    .replace("Regular", "")
                    .replace(['-', '_'], " "),
            );
            let path = dir.join(&file).display().to_string();
            let bold = if variable {
                path.clone()
            } else {
                let bold = dir.join(file.replace("Regular", "Bold"));
                if bold.is_file() {
                    bold.display().to_string()
                } else {
                    String::new()
                }
            };
            if found.iter().any(|(n, _, _)| *n == name) {
                continue;
            }
            found.push((name, path, bold));
            if found.len() >= 40 {
                return found;
            }
        }
    }
    found
}

/// "CascadiaCode" → "Cascadia Code".
fn spaced(name: &str) -> String {
    let mut out = String::new();
    let mut prev_lower = false;
    for c in name.trim().chars() {
        if c.is_uppercase() && prev_lower {
            out.push(' ');
        }
        prev_lower = c.is_lowercase();
        out.push(c);
    }
    out
}

/// "cmd+shift+p" → "⌘⇧P".
pub fn pretty_shortcut(s: &str) -> String {
    let mut mods = String::new();
    let mut key = String::new();
    for part in s.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        match part.to_lowercase().as_str() {
            "ctrl" | "control" => mods.push('⌃'),
            "alt" | "opt" | "option" => mods.push('⌥'),
            "shift" => mods.push('⇧'),
            "cmd" | "command" | "super" => mods.push('⌘'),
            "tab" => key = "⇥".into(),
            "left" | "arrowleft" => key = "←".into(),
            "right" | "arrowright" => key = "→".into(),
            "up" | "arrowup" => key = "↑".into(),
            "down" | "arrowdown" => key = "↓".into(),
            "enter" => key = "↩".into(),
            other => key = other.to_uppercase(),
        }
    }
    format!("{mods}{key}")
}

pub fn mtime() -> Option<std::time::SystemTime> {
    std::fs::metadata(path()).and_then(|m| m.modified()).ok()
}

pub fn hex(s: &str, fallback: Color32) -> Color32 {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 {
        return fallback;
    }
    match u32::from_str_radix(h, 16) {
        Ok(v) => Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8),
        Err(_) => fallback,
    }
}

/// Colores ya resueltos, listos para pintar.
#[derive(Clone)]
pub struct Palette {
    pub bg: Color32,
    pub fg: Color32,
    pub cursor: Color32,
    pub selection: Color32,
    pub accent: Color32,
    pub tab_bar: Color32,
    pub tab_active: Color32,
    pub panel: Color32,
    pub muted: Color32,
    pub border: Color32,
    pub done: Color32,
    pub ansi: [Color32; 16],
}

impl Palette {
    pub fn new(t: &Theme) -> Self {
        let d = Theme::default();
        let mut ansi = [Color32::GRAY; 16];
        for (i, slot) in ansi.iter_mut().enumerate() {
            let def = hex(&d.ansi[i], Color32::GRAY);
            *slot = t.ansi.get(i).map(|s| hex(s, def)).unwrap_or(def);
        }
        Self {
            bg: hex(&t.background, Color32::BLACK),
            fg: hex(&t.foreground, Color32::WHITE),
            cursor: hex(&t.cursor, Color32::WHITE),
            selection: hex(&t.selection, Color32::DARK_BLUE),
            accent: hex(&t.accent, Color32::LIGHT_BLUE),
            tab_bar: hex(&t.tab_bar, Color32::BLACK),
            tab_active: hex(&t.tab_active, Color32::BLACK),
            panel: hex(&t.panel, Color32::BLACK),
            muted: hex(&t.muted, Color32::GRAY),
            border: hex(&t.border, Color32::DARK_GRAY),
            done: hex(&t.done, Color32::GREEN),
            ansi,
        }
    }

    /// Color de la paleta xterm de 256 entradas.
    pub fn indexed(&self, i: u8) -> Color32 {
        match i {
            0..=15 => self.ansi[i as usize],
            16..=231 => {
                let i = i - 16;
                let lvl = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
                Color32::from_rgb(lvl(i / 36), lvl((i / 6) % 6), lvl(i % 6))
            }
            _ => {
                let v = 8 + (i - 232) * 10;
                Color32::from_rgb(v, v, v)
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub struct Shortcut {
    pub cmd: bool,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: Key,
}

impl Shortcut {
    pub fn parse(s: &str) -> Option<Self> {
        let mut sc = Shortcut {
            cmd: false,
            ctrl: false,
            shift: false,
            alt: false,
            key: Key::Escape,
        };
        let mut found = false;
        // "cmd++" no se soporta; usa "cmd+=".
        for part in s.split('+').map(str::trim).filter(|p| !p.is_empty()) {
            match part.to_lowercase().as_str() {
                "cmd" | "command" | "super" => sc.cmd = true,
                "ctrl" | "control" => sc.ctrl = true,
                "shift" => sc.shift = true,
                "alt" | "opt" | "option" => sc.alt = true,
                other => {
                    let name = match other {
                        "left" => "ArrowLeft".to_string(),
                        "right" => "ArrowRight".to_string(),
                        "up" => "ArrowUp".to_string(),
                        "down" => "ArrowDown".to_string(),
                        _ if other.chars().count() == 1 => other.to_uppercase(),
                        _ => part.to_string(),
                    };
                    let key = Key::from_name(&name).or_else(|| {
                        let mut c = part.chars();
                        let cap: String = c
                            .next()
                            .map(|f| f.to_uppercase().chain(c).collect())
                            .unwrap_or_default();
                        Key::from_name(&cap)
                    })?;
                    sc.key = key;
                    found = true;
                }
            }
        }
        found.then_some(sc)
    }

    pub fn matches(&self, key: Key, physical: Option<Key>, m: &Modifiers) -> bool {
        (key == self.key || physical == Some(self.key))
            && m.mac_cmd == self.cmd
            && m.ctrl == self.ctrl
            && m.shift == self.shift
            && m.alt == self.alt
    }
}
