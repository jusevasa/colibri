//! Lanzador de la aplicación: ventana de winit + egui, sin eframe.
//!
//! Hace lo que hacía `eframe::run_native` para una sola ventana: crea la ventana, reparte
//! los eventos de winit a egui, corre `logic()` + `ui()` y entrega las mallas al pintor de
//! Metal (`metal.rs`). El reparto de repintados está portado del lanzador nativo de eframe
//! 0.37 (MIT/Apache) para que `request_repaint()` siga dando dos cuadros y
//! `request_repaint_after()` uno, y para que en reposo no se pinte nada.
//!
//! Diferencia de fondo con eframe: el pintor nunca espera un drawable. Si no hay ninguno
//! libre devuelve `Outcome::Skipped` y avisa después con `wake`; mientras tanto aquí no se
//! corren más cuadros (ver `Running::waiting`).

use crate::debug;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use egui::{ViewportCommand, ViewportEvent, ViewportId, ViewportInfo};
use egui_winit::ActionRequested;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowId};

use crate::metal::{Outcome, Painter};

/// Lo que la aplicación le da al lanzador (el equivalente de `eframe::App`).
pub trait App {
    /// Color de fondo de la ventana (RGBA premultiplicado). Por defecto transparente.
    fn clear_color(&self) -> [f32; 4] {
        [0.0; 4]
    }

    /// Permite tocar la entrada cruda antes de que egui la vea.
    fn raw_input_hook(&mut self, _ctx: &egui::Context, _raw: &mut egui::RawInput) {}

    /// Trabajo que no pinta; corre en cada pasada, también con la ventana oculta.
    fn logic(&mut self, ctx: &egui::Context);

    /// La interfaz; solo corre si la ventana se ve.
    fn ui(&mut self, ui: &mut egui::Ui);
}

/// Con la ventana oculta (minimizada o tapada) un repintado pedido «ya» espera esto: sin
/// pintar no hay nada que frene las pasadas y un `request_repaint()` por pasada ocuparía
/// un núcleo entero. Los repintados con un plazo mayor (el sondeo de 250 ms) no cambian.
const HIDDEN_REPAINT_INTERVAL: Duration = Duration::from_millis(100);

/// Si winit no entrega el `RedrawRequested` pedido en este tiempo, se corre solo
/// `logic()` para que la aplicación no se quede parada (como hace eframe).
const MISSED_REDRAW_TIMEOUT: Duration = Duration::from_millis(250);

/// Tope de la espera por un drawable libre: si `wake` no llega, se vuelve a intentar.
const DRAWABLE_TIMEOUT: Duration = Duration::from_millis(100);

/// Eventos propios que llegan al bucle desde cualquier hilo.
#[derive(Debug)]
enum UserEvent {
    /// egui pide un repintado (`Context::request_repaint*`).
    Repaint {
        when: Instant,
        /// Número de pasada en que se pidió, para descartar las peticiones ya servidas.
        cumulative_pass_nr: u64,
    },
    /// El pintor vuelve a tener un drawable libre tras saltarse un cuadro.
    Wake,
}

/// Qué corre una pasada.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// `logic()` y, si la ventana se ve, `ui()` y el pintado.
    Full,
    /// Solo `logic()`: ni pasada de egui ni pintado.
    LogicOnly,
}

/// Abre la ventana y corre la aplicación hasta que se cierre. Hilo principal.
///
/// `create` se llama una vez, con el contexto de egui ya listo y antes del primer cuadro.
pub fn run(
    viewport: egui::ViewportBuilder,
    create: impl FnOnce(&egui::Context) -> Box<dyn App>,
) -> Result<(), String> {
    // Crear el bucle crea la NSApplication, pero no termina de lanzarla hasta `run_app`:
    // quien observe `NSApplicationWillFinishLaunching` antes de llamar aquí llega a tiempo.
    // Los valores por defecto de winit (menú estándar, política de activación normal y
    // activarse al arrancar) son los mismos que usaba eframe.
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .map_err(|e| format!("no se pudo crear el bucle de eventos: {e}"))?;
    let mut runner = Runner {
        viewport,
        create: Some(create),
        proxy: event_loop.create_proxy(),
        running: None,
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

/// El manejador de winit: nada hasta `resumed`, luego la ventana en marcha.
struct Runner<F> {
    viewport: egui::ViewportBuilder,
    create: Option<F>,
    proxy: EventLoopProxy<UserEvent>,
    running: Option<Running>,
    error: Option<String>,
}

/// La ventana con todo lo que cuelga de ella.
///
/// El orden de los campos es el orden en que se sueltan: primero la aplicación (cierra
/// PTY e hilos), luego el pintor y al final la ventana.
struct Running {
    app: Box<dyn App>,
    painter: Painter,
    state: egui_winit::State,
    ctx: egui::Context,
    info: ViewportInfo,
    /// Peticiones de `ViewportCommand` que se sirven al pintar (capturas, cortar/pegar).
    actions: Vec<ActionRequested>,
    /// Entrada recibida que egui aún no ha visto porque solo se corrió `logic()`.
    pending_input: egui::RawInput,
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

    window: Window,
}

impl<F: FnOnce(&egui::Context) -> Box<dyn App>> Runner<F> {
    /// Crea la ventana, el pintor, egui y la aplicación.
    fn init(&mut self, event_loop: &ActiveEventLoop) -> Result<Running, String> {
        let ctx = egui::Context::default();
        // Escritorio: las ventanas hijas no se incrustan (aquí no hay ninguna).
        ctx.set_embed_viewports(false);

        let mut viewport = self.viewport.clone();
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
        // Oculta hasta que haya un cuadro pintado, para que no se vea vacía al arrancar.
        let viewport = viewport.with_visible(false);

        let window = egui_winit::create_window(&ctx, event_loop, &viewport)
            .map_err(|e| format!("no se pudo crear la ventana: {e}"))?;

        let wake_sent = Arc::new(AtomicBool::new(false));
        let wake: Arc<dyn Fn() + Send + Sync> = {
            let proxy = self.proxy.clone();
            let sent = wake_sent.clone();
            Arc::new(move || {
                if !sent.swap(true, Ordering::AcqRel) {
                    let _ = proxy.send_event(UserEvent::Wake);
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
            Some(painter.max_texture_side()),
        );
        // ⌘C/⌘X/⌘V llegan como teclas: los traduce la aplicación (`clip::translate`).
        state.set_clipboard_shortcuts(false);

        let create = self
            .create
            .take()
            .ok_or_else(|| "la aplicación ya se había creado".to_owned())?;
        let app = create(&ctx);

        Ok(Running {
            app,
            painter,
            state,
            ctx,
            info,
            actions: Vec::new(),
            pending_input: egui::RawInput::default(),
            beginning: Instant::now(),
            first_frame: true,
            close: false,
            // El primer cuadro, ya.
            next_repaint: Some(Instant::now()),
            redraw_deadline: None,
            skipped: false,
            waiting: None,
            wake_sent,
            window,
        })
    }
}

impl<F> Runner<F> {
    /// Suelta la aplicación y la ventana (dentro del bucle, para que se cierren bien).
    fn destroy(&mut self) {
        if let Some(running) = self.running.take() {
            running.ctx.on_exit();
            drop(running);
        }
    }

    /// Tras cada evento: cierra si toca y, si no, decide qué repintar y hasta cuándo dormir.
    fn after_event(&mut self, event_loop: &ActiveEventLoop) {
        self.close_if_asked(event_loop);

        let now = Instant::now();
        let mut direct = None;
        if let Some(run) = &mut self.running {
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
                run.pass(event_loop, mode);
            }
        }
        if direct.is_some() {
            self.close_if_asked(event_loop);
        }

        // Siempre un flujo que duerme (nunca `Poll`): en reposo no se pinta nada.
        let wake_at = self.running.as_ref().and_then(|run| {
            // Mientras se espera un drawable, lo pendiente no corre antes de `waiting`.
            let repaint = match (run.next_repaint, run.waiting) {
                (Some(at), Some(until)) => Some(at.max(until)),
                (at, _) => at,
            };
            [repaint, run.redraw_deadline].into_iter().flatten().min()
        });
        event_loop.set_control_flow(match wake_at {
            Some(at) => ControlFlow::WaitUntil(at),
            None => ControlFlow::Wait,
        });
    }

    fn close_if_asked(&mut self, event_loop: &ActiveEventLoop) {
        if self.running.as_ref().is_some_and(|run| run.close) {
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

    fn repaint_now(&mut self) {
        self.next_repaint = Some(Instant::now());
    }

    /// Una pasada: `logic()` y, si la ventana se ve (y `mode` lo permite), `ui()` y pintar.
    fn pass(&mut self, event_loop: &ActiveEventLoop, mode: Mode) {
        let started = Instant::now();
        self.pass_inner(event_loop, mode);
        debug::timing::add(&debug::timing::PASS, started.elapsed());
    }

    fn pass_inner(&mut self, event_loop: &ActiveEventLoop, mode: Mode) {
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
        self.app.raw_input_hook(&self.ctx, &mut input);

        let close_requested = input.viewport().close_requested();

        if mode == Mode::LogicOnly || !visible {
            // Sin pasada de egui: el estado de la interfaz queda como estaba para cuando
            // la ventana vuelva a verse.
            let app = &mut self.app;
            let output = self.ctx.run_logic(&input, |ctx| app.logic(ctx));
            // Ninguna pasada consumió la entrada: se guarda para la siguiente.
            self.pending_input = input;

            self.info.events.clear();
            self.state.handle_platform_output_with_event_loop(
                &self.window,
                event_loop,
                output.platform_output,
            );
            let commands = output.viewport_commands.into_values().flatten().collect();
            self.finish(commands, close_requested);
            return;
        }

        let app = &mut self.app;
        let egui::FullOutput {
            platform_output,
            mut textures_delta,
            shapes,
            pixels_per_point,
            viewport_output,
        } = self.ctx.run_ui(input, |ui| {
            let t = Instant::now();
            app.logic(ui.ctx());
            debug::timing::add(&debug::timing::LOGIC, t.elapsed());
            let t = Instant::now();
            app.ui(ui);
            debug::timing::add(&debug::timing::UI, t.elapsed());
        });

        self.info.events.clear();
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
            self.app.clear_color(),
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

impl<F: FnOnce(&egui::Context) -> Box<dyn App>> ApplicationHandler<UserEvent> for Runner<F> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.running.is_none() && self.create.is_some() {
            match self.init(event_loop) {
                Ok(running) => self.running = Some(running),
                Err(error) => {
                    self.error = Some(error);
                    event_loop.exit();
                    return;
                }
            }
        }
        self.after_event(event_loop);
    }

    fn new_events(&mut self, event_loop: &ActiveEventLoop, _cause: StartCause) {
        // Aquí se despierta cuando vence un `WaitUntil`.
        self.after_event(event_loop);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        if let Some(run) = &mut self.running {
            match event {
                UserEvent::Repaint {
                    when,
                    cumulative_pass_nr,
                } => {
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
                UserEvent::Wake => {
                    run.wake_sent.store(false, Ordering::Release);
                    if run.skipped {
                        // Vuelve a haber drawable: se pinta lo que se quedó sin ver.
                        run.skipped = false;
                        run.waiting = None;
                        run.repaint_now();
                    }
                }
            }
        }
        self.after_event(event_loop);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        if let Some(run) = &mut self.running {
            if matches!(event, WindowEvent::RedrawRequested) {
                run.redraw_deadline = None;
                run.pass(event_loop, Mode::Full);
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
        if let DeviceEvent::MouseMotion { delta } = event
            && let Some(run) = &mut self.running
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
