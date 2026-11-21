//! Lanzador de la aplicación: ventana de winit + egui, sin eframe.
//!
//! Hace lo que hacía `eframe::run_native`, con varias ventanas: crea cada ventana, reparte
//! los eventos de winit a egui, corre `logic()` + `ui()` y entrega las mallas al pintor de
//! Metal (`metal.rs`). Cada ventana tiene su propio `egui::Context`, su `egui_winit::State`
//! y su `Painter` (como cada `WindowContext` de Alacritty); la aplicación es una sola y
//! recibe en cada llamada de qué ventana se trata. El reparto de repintados está portado del lanzador nativo de eframe
//! 0.37 (MIT/Apache) para que `request_repaint()` siga dando dos cuadros y
//! `request_repaint_after()` uno, y para que en reposo no se pinte nada.
//!
//! Diferencia de fondo con eframe: el pintor nunca espera un drawable. Si no hay ninguno
//! libre devuelve `Outcome::Skipped` y avisa después con `wake`; mientras tanto aquí no se
//! corren más cuadros (ver `Running::waiting`). Y se mira antes de empezar
//! (`Painter::ready`): sin drawable, o con el de pantalla aún por liberarse, la pasada solo
//! corre `logic()`.

use crate::debug;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use egui::{ViewportCommand, ViewportEvent, ViewportId, ViewportInfo};
use egui_winit::ActionRequested;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::Window;
pub use winit::window::WindowId;

use crate::metal::{Outcome, Painter, Ready};

/// Lo que la aplicación le da al lanzador (el equivalente de `eframe::App`).
pub trait App {
    /// Color de fondo de la ventana (RGBA premultiplicado). Por defecto transparente.
    fn clear_color(&self) -> [f32; 4] {
        [0.0; 4]
    }

    /// Se acaba de crear una ventana (aún oculta, antes de su primer cuadro).
    fn window_opened(&mut self, win: WindowId, ctx: &egui::Context);

    /// La ventana se cerró: la aplicación suelta lo suyo (pestañas, procesos).
    fn window_closed(&mut self, win: WindowId);

    /// Las ventanas nuevas que quiere abrir la aplicación (se mira al arrancar y tras cada
    /// evento), en orden: cada una con su posición y tamaño (`[x, y, ancho, alto]` en
    /// puntos: la esquina de la ventana y el tamaño del contenido) o donde toque.
    fn new_windows(&mut self) -> Vec<Option<[f32; 4]>> {
        Vec::new()
    }

    /// No se pudo abrir una de las ventanas pedidas en `new_windows` (la siguiente de las que
    /// faltan, en el mismo orden): la aplicación recupera lo que iba dentro.
    fn window_failed(&mut self) {}

    /// Permite tocar la entrada cruda antes de que egui la vea.
    fn raw_input_hook(&mut self, _win: WindowId, _ctx: &egui::Context, _raw: &mut egui::RawInput) {}

    /// Trabajo que no pinta; corre en cada pasada, también con la ventana oculta.
    fn logic(&mut self, win: WindowId, ctx: &egui::Context);

    /// La interfaz; solo corre si la ventana se ve.
    fn ui(&mut self, win: WindowId, ui: &mut egui::Ui);
}

/// El bucle de eventos, para despertar ventanas desde cualquier hilo (`wake_all`).
static PROXY: OnceLock<Mutex<EventLoopProxy<UserEvent>>> = OnceLock::new();

/// Pide un repintado a todas las ventanas. Es lo que usan los hilos de fondo y los
/// manejadores de macOS (atajo global, notificaciones, menú, Finder) que antes tenían
/// guardado el único `egui::Context`. Desde cualquier hilo.
pub fn wake_all() {
    if let Some(proxy) = PROXY.get() {
        let _ = proxy
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .send_event(UserEvent::RepaintAll);
    }
}

/// El `NSWindow` de cada ventana (su dirección), para saber cuál es la que macOS dice que
/// hay bajo el puntero (`window_with_number`). Se da de alta al crearla y de baja al
/// cerrarla; mientras tanto la ventana de winit la mantiene viva.
static NS_WINDOWS: Mutex<Vec<(WindowId, usize)>> = Mutex::new(Vec::new());

/// La ventana de Colibrí con ese número de ventana de macOS (`windowNumber`). El número se
/// consulta ahora y no al crearla: una ventana aún sin mostrar no tiene. Hilo principal.
pub fn window_with_number(number: isize) -> Option<WindowId> {
    let windows = NS_WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
    windows.iter().find_map(|&(id, ns)| {
        let ns = ns as *mut objc2::runtime::AnyObject;
        let n: isize = unsafe { objc2::msg_send![ns, windowNumber] };
        (n == number).then_some(id)
    })
}

/// El `NSWindow` de una ventana de winit, como dirección (o nada si no es de AppKit).
fn ns_window_of(window: &Window) -> Option<usize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return None;
    };
    let view = handle.ns_view.as_ptr().cast::<objc2::runtime::AnyObject>();
    // SAFETY: winit garantiza que es su `NSView`, viva mientras la ventana.
    let ns: *mut objc2::runtime::AnyObject = unsafe { objc2::msg_send![view, window] };
    (!ns.is_null()).then_some(ns as usize)
}

/// Con la ventana oculta (minimizada o tapada) un repintado pedido «ya» espera esto: sin
/// pintar no hay nada que frene las pasadas y un `request_repaint()` por pasada ocuparía
/// un núcleo entero. Los repintados con un plazo mayor (el sondeo de 250 ms) no cambian.
const HIDDEN_REPAINT_INTERVAL: Duration = Duration::from_millis(100);

/// Si winit no entrega el `RedrawRequested` pedido en este tiempo, se corre solo
/// `logic()` para que la aplicación no se quede parada (como hace eframe).
const MISSED_REDRAW_TIMEOUT: Duration = Duration::from_millis(250);

/// Oculta durante este tiempo, la ventana suelta sus drawables (`Painter::shrink`). La
/// espera evita parpadeos y reservas inútiles al ir y volver rápido con ⌘Tab.
const SHRINK_AFTER: Duration = Duration::from_secs(2);

/// Lado máximo de textura que se le anuncia a egui. epaint crea el atlas de glifos con
/// ese ancho y lo dobla en alto al llenarse: con 16384 (lo que admite Metal) cada fila
/// costaba 64 KB y el atlas saltaba de 2 a 4 MB, en CPU y otra vez en GPU, por unas
/// decenas de glifos. Con 2048 crece en pasos de 8 KB por fila.
const ATLAS_SIDE: usize = 2048;

/// Tope de la espera por un drawable libre: si `wake` no llega, se vuelve a intentar.
const DRAWABLE_TIMEOUT: Duration = Duration::from_millis(100);

/// Con la ventana a la vista y sin cuadros nuevos durante este tiempo, el pintor suelta el
/// drawable de reserva (`Painter::release_reserve`): ~14 MB en una Retina de 1180×760 pt.
/// La espera evita soltarlo y volver a crearlo entre tecla y tecla.
const RESERVE_IDLE: Duration = Duration::from_secs(5);

/// Cuándo toca soltar el drawable de reserva: `RESERVE_IDLE` después del último cuadro
/// presentado, una vez por tanda (`released`) y nunca antes del primer cuadro.
fn reserve_due(presented_at: Option<Instant>, released: bool) -> Option<Instant> {
    presented_at
        .filter(|_| !released)
        .map(|at| at + RESERVE_IDLE)
}

/// Eventos propios que llegan al bucle desde cualquier hilo.
#[derive(Debug)]
enum UserEvent {
    /// egui pide un repintado (`Context::request_repaint*`).
    Repaint {
        win: WindowId,
        when: Instant,
        /// Número de pasada en que se pidió, para descartar las peticiones ya servidas.
        cumulative_pass_nr: u64,
    },
    /// El pintor de esa ventana vuelve a tener un drawable libre tras saltarse un cuadro.
    Wake(WindowId),
    /// Un hilo de fondo pide repintar todas las ventanas (`wake_all`).
    RepaintAll,
}

/// Qué corre una pasada.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// `logic()` y, si la ventana se ve, `ui()` y el pintado.
    Full,
    /// Solo `logic()`: ni pasada de egui ni pintado.
    LogicOnly,
}

/// Lo de la salida de egui que vale hasta la siguiente pasada de interfaz (ver `Running::kept`).
#[derive(Clone, Default)]
struct Kept {
    cursor_icon: egui::CursorIcon,
    cursor_image: Option<egui::output::CustomCursorImage>,
    ime: Option<egui::output::IMEOutput>,
}

impl Kept {
    fn from(output: &egui::PlatformOutput) -> Self {
        Self {
            cursor_icon: output.cursor_icon,
            cursor_image: output.cursor_image.clone(),
            // Interrumpir la composición es una orden de esa pasada, no un estado.
            ime: output.ime.map(|ime| egui::output::IMEOutput {
                should_interrupt_composition: false,
                ..ime
            }),
        }
    }

    /// Pone en la salida de una pasada de solo `logic()` lo de la última de interfaz, salvo
    /// lo que `logic()` haya pedido él mismo.
    fn restore(&self, output: &mut egui::PlatformOutput) {
        if output.cursor_icon == egui::CursorIcon::Default && output.cursor_image.is_none() {
            output.cursor_icon = self.cursor_icon;
            output.cursor_image = self.cursor_image.clone();
        }
        if output.ime.is_none() {
            output.ime = self.ime;
        }
    }
}

/// Abre la primera ventana y corre la aplicación hasta que se cierre la última. Hilo
/// principal.
///
/// `create` se llama una vez, antes de abrir la primera ventana; `viewport` es la plantilla
/// de todas las ventanas.
pub fn run(
    viewport: egui::ViewportBuilder,
    create: impl FnOnce() -> Box<dyn App>,
) -> Result<(), String> {
    // Crear el bucle crea la NSApplication, pero no termina de lanzarla hasta `run_app`:
    // quien observe `NSApplicationWillFinishLaunching` antes de llamar aquí llega a tiempo.
    // Los valores por defecto de winit (menú estándar, política de activación normal y
    // activarse al arrancar) son los mismos que usaba eframe.
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .map_err(|e| format!("no se pudo crear el bucle de eventos: {e}"))?;
    let _ = PROXY.set(Mutex::new(event_loop.create_proxy()));
    let mut runner = Runner {
        viewport,
        create: Some(create),
        app: None,
        proxy: event_loop.create_proxy(),
        windows: HashMap::new(),
        error: None,
    };
    event_loop
        .run_app(&mut runner)
        .map_err(|e| format!("el bucle de eventos falló: {e}"))?;
    // Por si el bucle volvió sin pasar por `exiting`.
    runner.destroy();
    match runner.error.take() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// El manejador de winit: nada hasta `resumed`, luego la aplicación y sus ventanas.
struct Runner<F> {
    viewport: egui::ViewportBuilder,
    create: Option<F>,
    app: Option<Box<dyn App>>,
    proxy: EventLoopProxy<UserEvent>,
    windows: HashMap<WindowId, Running>,
    error: Option<String>,
}

/// Una ventana con todo lo que cuelga de ella.
///
/// El orden de los campos es el orden en que se sueltan: el pintor y al final la ventana.
/// Lo de la aplicación (pestañas, PTY) se suelta antes, en `App::window_closed`.
struct Running {
    painter: Painter,
    state: egui_winit::State,
    ctx: egui::Context,
    info: ViewportInfo,
    /// Peticiones de `ViewportCommand` que se sirven al pintar (capturas, cortar/pegar).
    actions: Vec<ActionRequested>,
    /// Entrada recibida que egui aún no ha visto porque solo se corrió `logic()`.
    pending_input: egui::RawInput,
    /// Lo que la última pasada de egui dijo del puntero y de la entrada de texto. Una pasada
    /// de solo `logic()` no dibuja nada y su salida trae el puntero por defecto y la entrada
    /// de texto apagada: entregada tal cual, el puntero saltaba de la I a la flecha (y
    /// vuelta) con cada una, y la entrada de texto del sistema se apagaba y encendía.
    kept: Kept,
    beginning: Instant,
    /// La ventana nace oculta y se muestra tras el primer cuadro.
    first_frame: bool,
    /// Toca cerrar: la aplicación vio la petición y no la canceló.
    close: bool,

    /// Cuándo toca la siguiente pasada (lo más pronto que se haya pedido).
    next_repaint: Option<Instant>,
    /// Se pidió un `RedrawRequested`; si a esta hora no ha llegado, se corre `logic()`.
    redraw_deadline: Option<Instant>,
    /// El último cuadro se saltó por no haber drawable: lo que debía verse no se ve.
    skipped: bool,
    /// Tras saltarse un cuadro no se corren más pasadas hasta que el pintor avise
    /// (`wake`) o pase esta hora. Sin esto, una ráfaga de repintados sin drawable libre
    /// daría vueltas sin freno: nada espera al refresco de la pantalla.
    waiting: Option<Instant>,
    /// Hay un `UserEvent::Wake` en camino (para no encolar uno por cada aviso).
    wake_sent: Arc<AtomicBool>,
    /// Desde cuándo la ventana no se ve (para soltar los drawables al cabo de un rato).
    hidden_since: Option<Instant>,
    /// Los drawables ya se soltaron: la capa está vacía hasta el siguiente cuadro.
    shrunk: bool,
    /// Cuándo se presentó el último cuadro (para soltar el drawable de reserva en reposo).
    presented_at: Option<Instant>,
    /// El drawable de reserva ya se soltó (o se intentó) en esta tanda de reposo.
    reserve_released: bool,

    window: Window,
}

impl<F: FnOnce() -> Box<dyn App>> Runner<F> {
    /// Crea una ventana con su pintor y su egui, y se la presenta a la aplicación.
    fn open_window(
        &mut self,
        event_loop: &ActiveEventLoop,
        frame: Option<[f32; 4]>,
    ) -> Result<WindowId, String> {
        let ctx = egui::Context::default();
        // Escritorio: las ventanas hijas no se incrustan (aquí no hay ninguna).
        ctx.set_embed_viewports(false);

        let mut viewport = self.viewport.clone();
        if let Some([_, _, w, h]) = frame {
            viewport = viewport.with_inner_size([w, h]);
        }
        // Como eframe: una ventana no nace más grande que el mayor monitor.
        if let Some(size) = viewport.inner_size {
            let mut largest = egui::Vec2::ZERO;
            for monitor in event_loop.available_monitors() {
                let s = monitor
                    .size()
                    .to_logical::<f32>(ctx.zoom_factor() as f64 * monitor.scale_factor());
                largest = largest.max(egui::vec2(s.width, s.height));
            }
            if largest != egui::Vec2::ZERO {
                viewport = viewport.with_inner_size(size.min(largest));
            }
        }
        // Donde se guardó, si cae en alguna pantalla (pudo desconectarse); si no, en
        // cascada desde la que está al frente (como Terminal).
        let saved_at = frame.map(|[x, y, ..]| egui::pos2(x, y)).filter(|p| {
            event_loop.available_monitors().any(|m| {
                let scale = m.scale_factor();
                let at = m.position().to_logical::<f32>(scale);
                let size = m.size().to_logical::<f32>(scale);
                egui::Rect::from_min_size(
                    egui::pos2(at.x, at.y),
                    egui::vec2(size.width, size.height),
                )
                .contains(*p)
            })
        });
        if let Some(at) = saved_at {
            viewport = viewport.with_position(at);
        } else if let Some(front) = self.front()
            && let Ok(at) = front.window.outer_position()
        {
            let at = at.to_logical::<f32>(front.window.scale_factor());
            viewport = viewport.with_position([at.x + 28.0, at.y + 28.0]);
        }
        // Oculta hasta que haya un cuadro pintado, para que no se vea vacía al arrancar.
        let viewport = viewport.with_visible(false);

        let window = egui_winit::create_window(&ctx, event_loop, &viewport)
            .map_err(|e| format!("no se pudo crear la ventana: {e}"))?;
        let id = window.id();

        let wake_sent = Arc::new(AtomicBool::new(false));
        let wake: Arc<dyn Fn() + Send + Sync> = {
            let proxy = self.proxy.clone();
            let sent = wake_sent.clone();
            Arc::new(move || {
                if !sent.swap(true, Ordering::AcqRel) {
                    let _ = proxy.send_event(UserEvent::Wake(id));
                }
            })
        };
        let painter = Painter::new(&window, wake)?;

        let mut info = ViewportInfo::default();
        egui_winit::update_viewport_info(&mut info, &ctx, &window, true);
        {
            // Que la aplicación ya sepa la escala de la pantalla mientras se crea.
            let pixels_per_point = egui_winit::pixels_per_point(&ctx, &window);
            ctx.input_mut(|i| {
                i.raw.viewports.insert(ViewportId::ROOT, info.clone());
                i.pixels_per_point = pixels_per_point;
            });
        }

        // Fuentes del sistema para los caracteres que las de egui no tienen (CJK, árabe…),
        // lo que eframe hacía con su `system_fonts`.
        ctx.add_font_provider(Arc::new(
            egui_system_font_provider::SystemFontProvider::new(),
        ));

        {
            let proxy = self.proxy.clone();
            ctx.set_request_repaint_callback(move |info| {
                if info.viewport_id != ViewportId::ROOT {
                    return;
                }
                let _ = proxy.send_event(UserEvent::Repaint {
                    win: id,
                    when: Instant::now() + info.delay,
                    cumulative_pass_nr: info.current_cumulative_pass_nr,
                });
            });
        }

        let mut state = egui_winit::State::new(
            ctx.clone(),
            ViewportId::ROOT,
            event_loop,
            Some(window.scale_factor() as f32),
            event_loop.system_theme(),
            Some(painter.max_texture_side().min(ATLAS_SIDE)),
        );
        // ⌘C/⌘X/⌘V llegan como teclas: los traduce la aplicación (`clip::translate`).
        state.set_clipboard_shortcuts(false);

        // Ya no hay vuelta atrás (lo que podía fallar, el pintor, ya está): si se apuntara
        // antes y fallase, quedaría la dirección de un `NSWindow` ya liberado.
        if let Some(ns) = ns_window_of(&window) {
            NS_WINDOWS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push((id, ns));
        }

        if let Some(app) = &mut self.app {
            app.window_opened(id, &ctx);
        }

        self.windows.insert(
            id,
            Running {
                painter,
                state,
                ctx,
                info,
                actions: Vec::new(),
                pending_input: egui::RawInput::default(),
                kept: Kept::default(),
                beginning: Instant::now(),
                first_frame: true,
                close: false,
                // El primer cuadro, ya.
                next_repaint: Some(Instant::now()),
                redraw_deadline: None,
                skipped: false,
                waiting: None,
                wake_sent,
                hidden_since: None,
                shrunk: false,
                presented_at: None,
                reserve_released: false,
                window,
            },
        );
        Ok(id)
    }
}

impl<F> Runner<F> {
    /// La ventana con el foco o, si ninguna lo tiene, cualquiera.
    fn front(&self) -> Option<&Running> {
        self.windows
            .values()
            .find(|run| run.info.focused == Some(true))
            .or_else(|| self.windows.values().next())
    }

    /// Suelta la aplicación y las ventanas (dentro del bucle, para que se cierren bien).
    fn destroy(&mut self) {
        for run in self.windows.values() {
            run.ctx.on_exit();
        }
        // Primero la aplicación (cierra PTY e hilos), luego pintores y ventanas.
        self.app = None;
        self.windows.clear();
    }

    /// Tras cada evento: cierra lo que toque, abre lo que pida la aplicación y decide qué
    /// repintar y hasta cuándo dormir.
    fn after_event(&mut self, event_loop: &ActiveEventLoop)
    where
        F: FnOnce() -> Box<dyn App>,
    {
        self.close_if_asked(event_loop);
        if self.windows.is_empty() {
            return;
        }

        let wanted = self.app.as_mut().map(|app| app.new_windows());
        for frame in wanted.into_iter().flatten() {
            if let Err(error) = self.open_window(event_loop, frame) {
                eprintln!("{error}");
                if let Some(app) = &mut self.app {
                    app.window_failed();
                }
            }
        }

        let now = Instant::now();
        for run in self.windows.values_mut() {
            // A la vista pero quieta: fuera el drawable de reserva. Se apunta aunque el pintor
            // no lo suelte (un cuadro en vuelo), para no despertar en bucle.
            if run.reserve_due().is_some_and(|at| at <= now) {
                run.reserve_released = true;
                if run.painter.release_reserve() && std::env::var_os("COLIBRI_FPS").is_some() {
                    eprintln!("drawable de reserva soltado");
                }
            }
        }
        let mut ran = false;
        if let Some(app) = self.app.as_deref_mut() {
            for run in self.windows.values_mut() {
                let mut direct = None;
                if run.waiting.is_some_and(|until| until <= now) {
                    run.waiting = None;
                }
                if run.waiting.is_none() && run.next_repaint.is_some_and(|at| at <= now) {
                    run.next_repaint = None;
                    if run.first_frame || run.window.is_minimized() == Some(true) {
                        // Una ventana que no está en pantalla no necesita pasar por
                        // `RedrawRequested`: la pasada se corre aquí mismo.
                        direct = Some(Mode::Full);
                    } else {
                        run.window.request_redraw();
                        run.redraw_deadline
                            .get_or_insert(now + MISSED_REDRAW_TIMEOUT);
                    }
                }
                if direct.is_none() && run.redraw_deadline.is_some_and(|at| at <= now) {
                    run.redraw_deadline = None;
                    direct = Some(Mode::LogicOnly);
                }
                if let Some(mode) = direct {
                    run.pass(app, event_loop, mode);
                    ran = true;
                }
            }
        }
        if ran {
            self.close_if_asked(event_loop);
        }

        // Siempre un flujo que duerme (nunca `Poll`): en reposo no se pinta nada.
        let wake_at = self
            .windows
            .values()
            .filter_map(|run| {
                // Mientras se espera un drawable, lo pendiente no corre antes de `waiting`.
                let repaint = match (run.next_repaint, run.waiting) {
                    (Some(at), Some(until)) => Some(at.max(until)),
                    (at, _) => at,
                };
                [repaint, run.redraw_deadline, run.reserve_due()]
                    .into_iter()
                    .flatten()
                    .min()
            })
            .min();
        event_loop.set_control_flow(match wake_at {
            Some(at) => ControlFlow::WaitUntil(at),
            None => ControlFlow::Wait,
        });
    }

    /// Cierra las ventanas que lo pidieron; sin ninguna, la aplicación termina.
    fn close_if_asked(&mut self, event_loop: &ActiveEventLoop) {
        let closing: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|(_, run)| run.close)
            .map(|(id, _)| *id)
            .collect();
        for id in closing {
            if let Some(app) = &mut self.app {
                app.window_closed(id);
            }
            NS_WINDOWS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .retain(|(w, _)| *w != id);
            if let Some(run) = self.windows.remove(&id) {
                run.ctx.on_exit();
            }
        }
        if self.windows.is_empty() && self.app.is_some() {
            self.destroy();
            event_loop.exit();
        }
    }
}

impl Running {
    /// La ventana no se ve: minimizada o tapada del todo.
    fn hidden(&self) -> bool {
        !self.first_frame && self.info.visible() == Some(false)
    }

    /// Oculta más de `SHRINK_AFTER`: suelta los drawables. Si aún no toca, se programa
    /// una pasada para entonces.
    fn shrink_if_hidden_long(&mut self) {
        if self.shrunk {
            return;
        }
        let now = Instant::now();
        let since = *self.hidden_since.get_or_insert(now);
        let due = since + SHRINK_AFTER;
        if now < due {
            self.next_repaint = Some(self.next_repaint.map_or(due, |at| at.min(due)));
            return;
        }
        self.painter.shrink();
        self.shrunk = true;
    }

    fn repaint_now(&mut self) {
        self.next_repaint = Some(Instant::now());
    }

    /// Cuándo soltar el drawable de reserva. Una ventana oculta no: ya suelta todos
    /// (`shrink_if_hidden_long`).
    fn reserve_due(&self) -> Option<Instant> {
        if self.shrunk || self.hidden() {
            return None;
        }
        reserve_due(self.presented_at, self.reserve_released)
    }

    /// Una pasada: `logic()` y, si la ventana se ve (y `mode` lo permite), `ui()` y pintar.
    fn pass(&mut self, app: &mut dyn App, event_loop: &ActiveEventLoop, mode: Mode) {
        let started = Instant::now();
        self.pass_inner(app, event_loop, mode);
        debug::timing::add(&debug::timing::PASS, started.elapsed());
    }

    fn pass_inner(&mut self, app: &mut dyn App, event_loop: &ActiveEventLoop, mode: Mode) {
        let win = self.window.id();
        egui_winit::update_viewport_info(&mut self.info, &self.ctx, &self.window, false);
        // `update_viewport_info` no refresca esto en macOS; sin ello, una ventana
        // minimizada con `ViewportCommand::Minimized` y devuelta desde el Dock seguiría
        // contando como oculta.
        self.info.minimized = self.window.is_minimized().or(self.info.minimized);

        // Una ventana oculta no se pinta, salvo que alguien quiera los píxeles (captura).
        let wants_paint = self.actions.iter().any(ActionRequested::wants_paint);
        let visible = self.first_frame || wants_paint || self.info.visible().unwrap_or(true);

        let mut new_input = self.state.take_egui_input(&self.window);
        new_input.viewports = std::iter::once((ViewportId::ROOT, self.info.clone())).collect();
        let mut input = std::mem::take(&mut self.pending_input);
        input.append(new_input); // donde coinciden gana lo nuevo
        input.time = Some(self.beginning.elapsed().as_secs_f64());
        app.raw_input_hook(win, &self.ctx, &mut input);

        let close_requested = input.viewport().close_requested();

        if visible {
            self.hidden_since = None;
            self.shrunk = false;
        } else {
            self.shrink_if_hidden_long();
        }

        // Sin drawable libre, la pasada de egui (interfaz y teselado) se tiraría entera, y con
        // el drawable a punto de liberarse `nextDrawable` pararía el hilo principal: solo
        // `logic()`, como con la ventana oculta, y el cuadro se pinta cuando el pintor avise
        // (`wake`) o al refresco siguiente. Las teclas esperan a esa pasada (unos
        // milisegundos). Una captura pendiente sí pinta: sale de una textura aparte.
        let ready = if mode == Mode::Full && visible && !wants_paint && !self.first_frame {
            self.painter.ready()
        } else {
            Ready::Now
        };
        let deferred = match ready {
            Ready::Now => false,
            Ready::Wake => {
                self.skipped = true;
                self.waiting = Some(Instant::now() + DRAWABLE_TIMEOUT);
                true
            }
            Ready::After(left) => {
                // Aquí no llega ningún aviso: la pasada se programa para entonces.
                let at = Instant::now() + left;
                self.waiting = Some(at);
                self.next_repaint = Some(self.next_repaint.map_or(at, |n| n.min(at)));
                true
            }
        };

        if mode == Mode::LogicOnly || !visible || deferred {
            // Sin pasada de egui: el estado de la interfaz queda como estaba para cuando
            // la ventana vuelva a verse.
            let mut output = self.ctx.run_logic(&input, |ctx| app.logic(win, ctx));
            // Ninguna pasada consumió la entrada: se guarda para la siguiente.
            self.pending_input = input;

            self.info.events.clear();
            self.kept.restore(&mut output.platform_output);
            self.state.handle_platform_output_with_event_loop(
                &self.window,
                event_loop,
                output.platform_output,
            );
            let commands = output.viewport_commands.into_values().flatten().collect();
            self.finish(commands, close_requested);
            return;
        }

        let egui::FullOutput {
            platform_output,
            mut textures_delta,
            shapes,
            pixels_per_point,
            viewport_output,
        } = self.ctx.run_ui(input, |ui| {
            let t = Instant::now();
            app.logic(win, ui.ctx());
            debug::timing::add(&debug::timing::LOGIC, t.elapsed());
            let t = Instant::now();
            app.ui(win, ui);
            debug::timing::add(&debug::timing::UI, t.elapsed());
        });

        self.info.events.clear();
        self.kept = Kept::from(&platform_output);
        self.state.handle_platform_output_with_event_loop(
            &self.window,
            event_loop,
            platform_output,
        );

        let t = Instant::now();
        let primitives = self.ctx.tessellate(shapes, pixels_per_point);
        debug::timing::add(&debug::timing::TESS, t.elapsed());
        let vertices: usize = primitives
            .iter()
            .map(|p| match &p.primitive {
                egui::epaint::Primitive::Mesh(m) => m.vertices.len(),
                egui::epaint::Primitive::Callback(_) => 0,
            })
            .sum();
        debug::timing::VERTICES.fetch_add(vertices as u64, std::sync::atomic::Ordering::Relaxed);

        // Lo pedido en pasadas anteriores (los comandos de esta se procesan al final,
        // como en eframe: una captura sale en el cuadro siguiente al que la pidió).
        let mut shots = Vec::new();
        for action in self.actions.drain(..) {
            match action {
                ActionRequested::Screenshot(callback) => shots.push(callback),
                // Pintar este cuadro es todo lo que pedía.
                ActionRequested::PaintWhileHidden => {}
                ActionRequested::Cut => self.state.egui_input_mut().events.push(egui::Event::Cut),
                ActionRequested::Copy => self.state.egui_input_mut().events.push(egui::Event::Copy),
                ActionRequested::Paste => {
                    if let Some(text) = self.state.clipboard_text() {
                        let text = text.replace("\r\n", "\n");
                        if !text.is_empty() {
                            self.state
                                .egui_input_mut()
                                .events
                                .push(egui::Event::Paste(text));
                        }
                    } else if let Some(image) = self.state.clipboard_image() {
                        self.state
                            .egui_input_mut()
                            .events
                            .push(egui::Event::PasteImage(Arc::new(image)));
                    }
                }
            }
        }

        let size = self.window.inner_size();
        let t = Instant::now();
        let painted = self.painter.paint(
            [size.width, size.height],
            pixels_per_point,
            app.clear_color(),
            &primitives,
            &textures_delta,
            !shots.is_empty(),
        );
        debug::timing::add(&debug::timing::PAINT, t.elapsed());
        // El pintor ya aplicó los cambios de texturas (también si saltó el cuadro); epaint
        // exige vaciarlos antes de soltarlos.
        textures_delta.clear();
        if let Some(image) = painted.capture {
            let image = Arc::new(image);
            for shot in shots {
                shot.complete(image.clone());
            }
        }
        match painted.outcome {
            Outcome::Presented => {
                self.skipped = false;
                self.waiting = None;
                self.presented_at = Some(Instant::now());
                self.reserve_released = false;
            }
            // Lo que se ve ya es este cuadro; no cuenta como cuadro nuevo para soltar la reserva.
            Outcome::Unchanged => {
                self.skipped = false;
                self.waiting = None;
            }
            Outcome::Skipped => {
                // No se insiste: el pintor avisa con `wake` cuando haya drawable.
                self.skipped = true;
                self.waiting = Some(Instant::now() + DRAWABLE_TIMEOUT);
            }
        }

        if std::mem::take(&mut self.first_frame) {
            // Se muestra aunque el cuadro se haya saltado: la ventana es transparente y
            // sin mostrarla podría no haber nunca un drawable.
            self.window.set_visible(true);
        }

        let commands = viewport_output
            .into_values()
            .flat_map(|output| output.commands)
            .collect();
        self.finish(commands, close_requested);
    }

    /// Aplica los comandos de ventana de la pasada y resuelve si toca cerrar.
    fn finish(&mut self, commands: Vec<ViewportCommand>, close_requested: bool) {
        // La aplicación puede negarse a cerrar con `CancelClose` en la misma pasada.
        let canceled = commands.contains(&ViewportCommand::CancelClose);
        egui_winit::process_viewport_commands(
            &self.ctx,
            &mut self.info,
            commands,
            &self.window,
            &mut self.actions,
        );
        if close_requested && !canceled {
            self.close = true;
        }
    }

    fn on_window_event(&mut self, event: &WindowEvent) {
        match event {
            WindowEvent::Occluded(occluded) => self.info.occluded = Some(*occluded),
            WindowEvent::CloseRequested => {
                // La aplicación lo ve en la siguiente pasada (`close_requested()`) y, si
                // no lo cancela, se cierra. `request_repaint` da las dos pasadas.
                self.info.events.push(ViewportEvent::Close);
                self.ctx.request_repaint();
            }
            _ => {}
        }
        // Todo lo demás (teclado, composición de texto, ratón, archivos soltados, cambio
        // de escala o de tamaño) lo traduce egui-winit.
        let response = self.state.on_window_event(&self.window, event);
        if response.repaint {
            self.repaint_now();
        }
    }
}

impl<F: FnOnce() -> Box<dyn App>> ApplicationHandler<UserEvent> for Runner<F> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.app.is_none()
            && let Some(create) = self.create.take()
        {
            // La aplicación primero: ella dice cuántas ventanas abrir (las de la última
            // vez) y dónde.
            let mut app = create();
            let mut frames = app.new_windows();
            self.app = Some(app);
            if frames.is_empty() {
                frames.push(None);
            }
            for frame in frames {
                if let Err(error) = self.open_window(event_loop, frame) {
                    eprintln!("{error}");
                    if let Some(app) = &mut self.app {
                        app.window_failed();
                    }
                }
            }
            if self.windows.is_empty() {
                self.error = Some("no se pudo abrir ninguna ventana".into());
                event_loop.exit();
                return;
            }
        }
        self.after_event(event_loop);
    }

    fn new_events(&mut self, event_loop: &ActiveEventLoop, _cause: StartCause) {
        // Aquí se despierta cuando vence un `WaitUntil`.
        self.after_event(event_loop);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Repaint {
                win,
                when,
                cumulative_pass_nr,
            } => {
                if let Some(run) = self.windows.get_mut(&win) {
                    // Una petición anterior a la última pasada ya está servida.
                    let current = run.ctx.cumulative_pass_nr_for(ViewportId::ROOT);
                    if current == cumulative_pass_nr || current == cumulative_pass_nr + 1 {
                        let when = if run.hidden() {
                            when.max(Instant::now() + HIDDEN_REPAINT_INTERVAL)
                        } else {
                            when
                        };
                        run.next_repaint = Some(run.next_repaint.map_or(when, |at| at.min(when)));
                    }
                }
            }
            UserEvent::Wake(win) => {
                if let Some(run) = self.windows.get_mut(&win) {
                    run.wake_sent.store(false, Ordering::Release);
                    if run.skipped {
                        // Vuelve a haber drawable: se pinta lo que se quedó sin ver.
                        run.skipped = false;
                        run.waiting = None;
                        run.repaint_now();
                    }
                }
            }
            UserEvent::RepaintAll => {
                // Como el `ctx.request_repaint()` que hacían antes: dos pasadas, y las
                // ventanas ocultas a su ritmo pausado.
                for run in self.windows.values() {
                    run.ctx.request_repaint();
                }
            }
        }
        self.after_event(event_loop);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, win: WindowId, event: WindowEvent) {
        if let (Some(run), Some(app)) = (self.windows.get_mut(&win), self.app.as_deref_mut()) {
            if matches!(event, WindowEvent::RedrawRequested) {
                run.redraw_deadline = None;
                run.pass(app, event_loop, Mode::Full);
            } else {
                run.on_window_event(&event);
            }
        }
        self.after_event(event_loop);
    }

    fn device_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _device_id: DeviceId,
        event: DeviceEvent,
    ) {
        // El movimiento crudo del ratón es para la ventana con el foco.
        if let DeviceEvent::MouseMotion { delta } = event
            && let Some(run) = self
                .windows
                .values_mut()
                .find(|run| run.info.focused == Some(true))
            && run.state.on_mouse_motion(delta)
        {
            run.repaint_now();
        }
        self.after_event(event_loop);
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        // También se llega aquí con ⌘Q, y entonces `run_app` no vuelve.
        self.destroy();
    }
}

#[cfg(test)]
mod tests {
    use super::{Kept, RESERVE_IDLE, reserve_due};
    use std::time::Instant;

    /// Con un agente trabajando hay pasadas de solo `logic()` entre las de interfaz (sin
    /// drawable libre, refresco de 250 ms). Su salida no puede devolver la flecha ni apagar la
    /// entrada de texto que dejó la última pasada de interfaz.
    #[test]
    fn pasada_de_solo_logica_conserva_puntero_y_entrada_de_texto() {
        let ctx = egui::Context::default();
        let ime = egui::output::IMEOutput {
            purpose: egui::IMEPurpose::Normal,
            should_interrupt_composition: false,
            rect: egui::Rect::from_min_size(egui::pos2(1.0, 2.0), egui::vec2(3.0, 4.0)),
            cursor_rect: egui::Rect::from_min_size(egui::pos2(1.0, 2.0), egui::vec2(1.0, 4.0)),
        };
        let full = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
            ui.ctx().output_mut(|o| {
                o.ime = Some(egui::output::IMEOutput {
                    should_interrupt_composition: true,
                    ..ime
                })
            });
        });
        let kept = Kept::from(&full.platform_output);
        full.drop_without_applying_deltas();

        let mut logic = ctx.run_logic(&egui::RawInput::default(), |_| {});
        assert_eq!(
            logic.platform_output.cursor_icon,
            egui::CursorIcon::Default,
            "egui deja la salida de `run_logic` con el puntero por defecto"
        );
        assert!(logic.platform_output.ime.is_none());
        kept.restore(&mut logic.platform_output);
        assert_eq!(logic.platform_output.cursor_icon, egui::CursorIcon::Text);
        assert_eq!(logic.platform_output.ime, Some(ime));

        // Lo que pida `logic()` por su cuenta manda.
        let mut logic = ctx.run_logic(&egui::RawInput::default(), |ctx| {
            ctx.set_cursor_icon(egui::CursorIcon::Wait);
        });
        kept.restore(&mut logic.platform_output);
        assert_eq!(logic.platform_output.cursor_icon, egui::CursorIcon::Wait);
    }

    #[test]
    fn reserva_se_suelta_tras_un_rato_sin_pintar_y_una_vez() {
        let t = Instant::now();
        assert_eq!(
            reserve_due(None, false),
            None,
            "sin cuadros todavía no hay nada que soltar"
        );
        assert_eq!(reserve_due(Some(t), false), Some(t + RESERVE_IDLE));
        assert_eq!(
            reserve_due(Some(t), true),
            None,
            "ya se soltó en esta tanda"
        );
    }
}
