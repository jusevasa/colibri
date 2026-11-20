//! Guion de depuración: COLIBRI_SCRIPT=archivo ejecuta pasos y guarda capturas del propio render.
//! Pasos (uno por línea): `type <texto>` (\n = Enter), `wait <ms>`, `tab`, `select <n>`,
//! `move <de> <a>`, `agent next|prev`, `name <nombre>`, `shot <ruta.bmp>`, `procs`, `ports`, `hover`, `killport <puerto>`,
//! `open palette|prefs|history|branches|dirs`, `key up|down|enter|esc|tab|paste`,
//! `press <x> <y>`, `moveto <x> <y>`, `release <x> <y>`, `rclick <x> <y>`, `pref <id> <+1|-1>`, `cmd down|up`, `drop <ruta>`, `minimize`, `update`, `quit`.

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
    /// Texto a medio componer, como el que manda el sistema con una tecla muerta.
    Preedit(String),
    /// Pulsa una tecla como si viniera del teclado: `key up|down|enter|esc|tab|paste`
    /// (`paste` hace lo mismo que ⌘V con lo que haya en el portapapeles).
    Key(String),
    /// Puntero simulado, en puntos de la ventana: `press`, `moveto` o `release` + `<x> <y>`;
    /// `rclick` es un clic derecho completo.
    Pointer(String, f32, f32),
    /// Mueve una pestaña de una posición a otra (lo mismo que arrastrarla).
    Move(usize, usize),
    /// Va al agente siguiente (`agent next`) o anterior (`agent prev`).
    Agent(bool),
    /// Mantiene ⌘ pulsada (`cmd down`) o la suelta (`cmd up`): para pasar por un enlace
    /// o hacer clic en él con el puntero simulado.
    Cmd(bool),
    /// Suelta un archivo sobre la ventana, como al arrastrarlo desde Finder.
    Drop(String),
    /// Minimiza la ventana (para medir cuánto se pinta sin verse).
    Minimize,
    /// Sube (positivo) o baja (negativo) por el historial de la pestaña activa.
    Scroll(i32),
    /// Muestra u oculta el botón «Actualizar» sin que haya una compilación nueva.
    Update,
    /// Ventanas: `window new` abre una, `window close` cierra la del frente y
    /// `window <n>` trae al frente la n-ésima (por orden de apertura, desde 0).
    Window(String),
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
                    // `\s` = espacio: las líneas se recortan, así que es la forma de acabar en uno.
                    "type" => Step::Type(arg.replace("\\n", "\r").replace("\\s", " ")),
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
                    "preedit" => Step::Preedit(arg.to_string()),
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
                    "cmd" => Step::Cmd(arg != "up"),
                    "drop" => Step::Drop(arg.to_string()),
                    "minimize" => Step::Minimize,
                    "scroll" => Step::Scroll(arg.parse().ok()?),
                    "update" => Step::Update,
                    "window" => Step::Window(arg.to_string()),
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

    /// Cuánto falta para el siguiente paso.
    pub fn until_next(&self) -> Duration {
        self.next_at.saturating_duration_since(Instant::now()) + Duration::from_millis(5)
    }

    pub fn done(&self) -> bool {
        self.steps.is_empty()
    }
}

/// Tiempo de CPU por fase de cada cuadro, para medir con `COLIBRI_FPS` (en µs acumulados
/// durante el segundo en curso): interfaz entera, solo la rejilla (`view::show`), teselado
/// y pintor. También los vértices que llegan al pintor.
pub mod timing {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    pub static PASS: AtomicU64 = AtomicU64::new(0);
    pub static LOGIC: AtomicU64 = AtomicU64::new(0);
    pub static UI: AtomicU64 = AtomicU64::new(0);
    pub static GRID: AtomicU64 = AtomicU64::new(0);
    pub static TESS: AtomicU64 = AtomicU64::new(0);
    pub static PAINT: AtomicU64 = AtomicU64::new(0);
    pub static VERTICES: AtomicU64 = AtomicU64::new(0);
    /// Rectángulos de la rejilla con instancias (`grid.rs`), que no cuentan como vértices.
    pub static INSTANCES: AtomicU64 = AtomicU64::new(0);

    pub fn add(slot: &AtomicU64, d: Duration) {
        slot.fetch_add(d.as_micros() as u64, Ordering::Relaxed);
    }

    pub(super) fn take(slot: &AtomicU64) -> u64 {
        slot.swap(0, Ordering::Relaxed)
    }
}

/// Con `COLIBRI_FPS` definida, escribe en stderr cuántos cuadros se pintaron cada segundo
/// (solo los segundos en que hubo alguno). Sirve para ver si algo repinta de más.
pub fn count_frame() {
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicU64, Ordering};
    static ON: OnceLock<bool> = OnceLock::new();
    static START: OnceLock<Instant> = OnceLock::new();
    static SECOND: AtomicU64 = AtomicU64::new(0);
    static FRAMES: AtomicU64 = AtomicU64::new(0);
    if !*ON.get_or_init(|| std::env::var_os("COLIBRI_FPS").is_some()) {
        return;
    }
    let second = START.get_or_init(Instant::now).elapsed().as_secs();
    if SECOND.swap(second, Ordering::Relaxed) != second {
        let (shown, skipped, wait) = crate::metal::stats::take();
        let frames = FRAMES.swap(0, Ordering::Relaxed);
        let per = |slot| timing::take(slot) as f64 / 1000.0 / frames.max(1) as f64;
        let (ui, grid, tess, paint) = (
            per(&timing::UI),
            per(&timing::GRID),
            per(&timing::TESS),
            per(&timing::PAINT),
        );
        let verts = timing::take(&timing::VERTICES) / frames.max(1);
        let insts = timing::take(&timing::INSTANCES) / frames.max(1);
        let (pass, logic) = (per(&timing::PASS), per(&timing::LOGIC));
        eprintln!(
            "cuadros/s: {frames} (presentados {shown}, saltados {skipped}, espera máx. {:.1} ms) \
             ms/cuadro: pasada {pass:.2} lógica {logic:.2} interfaz {ui:.2} (rejilla {grid:.2}) teselado {tess:.2} pintor {paint:.2} \
             vértices {verts} instancias {insts}",
            wait as f64 / 1000.0
        );
    }
    FRAMES.fetch_add(1, Ordering::Relaxed);
}

pub fn save_bmp(path: &str, image: &egui::ColorImage) {
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
