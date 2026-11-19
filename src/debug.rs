//! Guion de depuración: COLIBRI_SCRIPT=archivo ejecuta pasos y guarda capturas del propio render.
//! Pasos (uno por línea): `type <texto>` (\n = Enter), `wait <ms>`, `tab`, `select <n>`,
//! `move <de> <a>`, `agent next|prev`, `name <nombre>`, `shot <ruta.bmp>`, `procs`, `ports`, `hover`, `killport <puerto>`,
//! `open palette|prefs|history|branches|dirs`, `key up|down|enter|esc|tab`,
//! `press <x> <y>`, `moveto <x> <y>`, `release <x> <y>`, `rclick <x> <y>`, `pref <id> <+1|-1>`, `update`, `quit`.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub enum Step {
    Type(String),
    Wait(u64),
    Tab,
    Select(usize),
    Shot(String),
    /// Muestra u oculta el apartado de procesos.
    Procs,
    /// Muestra u oculta el apartado de puertos.
    Ports,
    /// Dibuja los botones de las filas como si el puntero estuviera encima.
    Hover,
    /// Detiene el proceso que escucha en ese puerto (mismo camino que el botón).
    KillPort(u16),
    /// Abre la paleta (`open palette`), las preferencias (`open prefs`) o el historial
    /// de comandos (`open history`).
    Open(String),
    /// Cambia una preferencia: `pref <id> <+1|-1>`.
    Pref(String, i32),
    /// Pone nombre a la pestaña activa.
    Name(String),
    /// Pulsa una tecla como si viniera del teclado: `key up|down|enter|esc|tab`.
    Key(String),
    /// Puntero simulado, en puntos de la ventana: `press`, `moveto` o `release` + `<x> <y>`;
    /// `rclick` es un clic derecho completo.
    Pointer(String, f32, f32),
    /// Mueve una pestaña de una posición a otra (lo mismo que arrastrarla).
    Move(usize, usize),
    /// Va al agente siguiente (`agent next`) o anterior (`agent prev`).
    Agent(bool),
    /// Muestra u oculta el botón «Actualizar» sin que haya una compilación nueva.
    Update,
    Quit,
}

pub struct Script {
    steps: VecDeque<Step>,
    next_at: Instant,
}

impl Script {
    pub fn from_env() -> Option<Self> {
        let path = std::env::var("COLIBRI_SCRIPT").ok()?;
        let text = std::fs::read_to_string(path).ok()?;
        let steps = text
            .lines()
            .filter_map(|line| {
                let (cmd, arg) = line.trim().split_once(' ').unwrap_or((line.trim(), ""));
                Some(match cmd {
                    "type" => Step::Type(arg.replace("\\n", "\r")),
                    "wait" => Step::Wait(arg.parse().ok()?),
                    "tab" => Step::Tab,
                    "select" => Step::Select(arg.parse().ok()?),
                    "shot" => Step::Shot(arg.to_string()),
                    "procs" => Step::Procs,
                    "ports" => Step::Ports,
                    "hover" => Step::Hover,
                    "killport" => Step::KillPort(arg.parse().ok()?),
                    "open" => Step::Open(arg.to_string()),
                    "pref" => {
                        let (id, dir) = arg.split_once(' ')?;
                        Step::Pref(id.to_string(), dir.parse().ok()?)
                    }
                    "name" => Step::Name(arg.to_string()),
                    "move" => {
                        let (from, to) = arg.split_once(' ')?;
                        Step::Move(from.parse().ok()?, to.parse().ok()?)
                    }
                    "key" => Step::Key(arg.to_string()),
                    "agent" => Step::Agent(arg != "prev"),
                    "press" | "moveto" | "release" | "rclick" => {
                        let (x, y) = arg.split_once(' ')?;
                        Step::Pointer(cmd.to_string(), x.parse().ok()?, y.parse().ok()?)
                    }
                    "update" => Step::Update,
                    "quit" => Step::Quit,
                    _ => return None,
                })
            })
            .collect();
        Some(Self {
            steps,
            next_at: Instant::now() + Duration::from_millis(800),
        })
    }

    pub fn next(&mut self) -> Option<Step> {
        if Instant::now() < self.next_at {
            return None;
        }
        let step = self.steps.pop_front()?;
        let pause = if let Step::Wait(ms) = &step { *ms } else { 60 };
        self.next_at = Instant::now() + Duration::from_millis(pause);
        Some(step)
    }

    pub fn done(&self) -> bool {
        self.steps.is_empty()
    }
}

pub fn save_bmp(path: &str, image: &eframe::egui::ColorImage) {
    let [w, h] = image.size;
    let row = (w * 3 + 3) & !3;
    let size = 54 + row * h;
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(size as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(h as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    for y in (0..h).rev() {
        for x in 0..w {
            let c = image.pixels[y * w + x];
            out.extend_from_slice(&[c.b(), c.g(), c.r()]);
        }
        out.resize(out.len() + (row - w * 3), 0);
    }
    let _ = std::fs::write(path, out);
}
