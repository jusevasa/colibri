//! Colibrí — una terminal rápida para correr varios agentes.
//! Sin telemetría: la app no abre ninguna conexión de red propia.

mod blur;
mod config;
mod debug;
mod history;
mod notify;
mod overlay;
mod ports;
mod proc;
mod procs;
mod session;
mod state;
mod view;

use config::{Config, Palette, Shortcut};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Pos2, Rect,
    Sense, Stroke, StrokeKind, Vec2, ViewportCommand,
};
use session::Session;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant, SystemTime};

const TAB_BAR_H: f32 = 38.0;
const TRAFFIC_LIGHTS_W: f32 = 78.0;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Colibrí")
            // Sin esto eframe pone su propio icono en el Dock al arrancar y tapa el del
            // bundle (assets/Colibri.icns). Con el icono vacío macOS usa el de Colibri.app.
            .with_icon(egui::IconData::default())
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([480.0, 300.0])
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false)
            .with_transparent(true),
        ..Default::default()
    };
    eframe::run_native(
        "Colibrí",
        options,
        Box::new(|cc| Ok(Box::new(App::new(&cc.egui_ctx)))),
    )
}

/// Todo lo que se puede hacer desde un atajo o desde la paleta de comandos.
#[derive(Clone, Copy, PartialEq)]
pub enum Cmd {
    NewTab,
    CloseTab,
    NextTab,
    PrevTab,
    MoveTabLeft,
    MoveTabRight,
    JumpFinished,
    TogglePanel,
    ToggleAgents,
    TogglePorts,
    ToggleProcs,
    FontBigger,
    FontSmaller,
    FontReset,
    Palette,
    Prefs,
    OpenConfig,
    Clear,
    RenameTab,
    Restart,
    History,
    GoTab(usize),
}

/// Atajos configurables ya resueltos, con su texto para mostrar ("⌘T").
struct Keys {
    bindings: Vec<(Cmd, Shortcut, String)>,
}

impl Keys {
    fn new(k: &config::Keys) -> Self {
        let table = [
            (Cmd::Palette, &k.palette),
            (Cmd::Prefs, &k.preferences),
            (Cmd::NewTab, &k.new_tab),
            (Cmd::CloseTab, &k.close_tab),
            (Cmd::NextTab, &k.next_tab),
            (Cmd::PrevTab, &k.prev_tab),
            (Cmd::MoveTabLeft, &k.move_tab_left),
            (Cmd::MoveTabRight, &k.move_tab_right),
            (Cmd::JumpFinished, &k.jump_finished),
            (Cmd::TogglePanel, &k.toggle_panel),
            (Cmd::ToggleProcs, &k.toggle_processes),
            (Cmd::ToggleAgents, &k.toggle_agents),
            (Cmd::TogglePorts, &k.toggle_ports),
            (Cmd::FontBigger, &k.font_bigger),
            (Cmd::FontSmaller, &k.font_smaller),
            (Cmd::FontReset, &k.font_reset),
            (Cmd::OpenConfig, &k.open_config),
            (Cmd::Clear, &k.clear),
            (Cmd::RenameTab, &k.rename_tab),
        ];
        let bindings = table
            .into_iter()
            .filter_map(|(cmd, text)| {
                Some((cmd, Shortcut::parse(text)?, config::pretty_shortcut(text)))
            })
            .collect();
        Self { bindings }
    }

    fn label(&self, cmd: Cmd) -> String {
        self.bindings
            .iter()
            .find(|(c, _, _)| *c == cmd)
            .map(|(_, _, l)| l.clone())
            .unwrap_or_default()
    }
}

struct App {
    cfg: Config,
    pal: Palette,
    keys: Keys,
    cfg_mtime: Option<SystemTime>,
    cfg_checked: Instant,
    sessions: Vec<Session>,
    active: usize,
    next_id: u64,
    show_agents: bool,
    show_ports: bool,
    show_procs: bool,
    monitor: procs::Monitor,
    proc_search: String,
    proc_sort_mem: bool,
    /// Proceso con la confirmación de «Detener» armada.
    confirm_stop: Option<(i32, Instant)>,
    /// Procesos a los que se pidió terminar y siguen en la lista.
    stopping: std::collections::HashMap<i32, Instant>,
    /// Los que ignoraron la petición: el siguiente intento los fuerza.
    stubborn: std::collections::HashSet<i32>,
    /// Solo para pruebas: dibuja los botones de fila sin puntero encima.
    force_hover: bool,
    /// Radio de desenfoque aplicado a la ventana, si ya se aplicó alguno.
    blur_applied: Option<i64>,
    search_focus: bool,
    zoom: f32,
    ports: ports::Ports,
    show_other_ports: bool,
    focused: bool,
    script: Option<debug::Script>,
    state_path: Option<std::path::PathBuf>,
    saved: state::Saved,
    state_checked: Instant,
    overlay: overlay::Overlay,
    font_choices: Vec<(String, String, String)>,
    /// Fecha del ejecutable al arrancar: si cambia, hay una compilación nueva.
    exe_started: Option<SystemTime>,
    exe_checked: Instant,
    update_ready: bool,
    /// Solo para pruebas: muestra el botón «Actualizar» aunque no haya nada nuevo.
    force_update: bool,
    /// Pestaña que se está arrastrando: su id y a qué distancia de su borde se agarró.
    drag_tab: Option<(u64, f32)>,
    /// Comandos que muestra el historial, del más reciente al más antiguo.
    history: Vec<String>,
    /// Solo para pruebas: teclas del guion que entran como si se pulsaran de verdad.
    script_keys: Vec<egui::Key>,
    /// Solo para pruebas: eventos de puntero del guion (`press`, `moveto`, `release`).
    script_pointer: Vec<egui::Event>,
    /// Si macOS puede mover la ventana ahora mismo (se apaga con el puntero sobre las pestañas).
    window_movable: Option<bool>,
}

impl App {
    fn new(ctx: &egui::Context) -> Self {
        let cfg = config::load();
        ctx.set_theme(egui::Theme::Dark);
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        install_fonts(ctx, &cfg);
        notify::init(ctx);
        let mut app = Self {
            pal: Palette::new(&cfg.theme),
            keys: Keys::new(&cfg.keys),
            cfg_mtime: config::mtime(),
            cfg_checked: Instant::now(),
            sessions: Vec::new(),
            active: 0,
            next_id: 1,
            show_agents: cfg.show_agents,
            show_ports: cfg.show_ports,
            show_procs: cfg.show_processes,
            monitor: procs::Monitor::start(ctx.clone()),
            proc_search: String::new(),
            proc_sort_mem: false,
            confirm_stop: None,
            stopping: Default::default(),
            stubborn: Default::default(),
            force_hover: false,
            blur_applied: None,
            search_focus: false,
            zoom: 0.0,
            ports: ports::Ports::start(ctx.clone()),
            show_other_ports: false,
            focused: true,
            script: debug::Script::from_env(),
            state_path: None,
            saved: state::Saved::default(),
            state_checked: Instant::now(),
            overlay: overlay::Overlay::None,
            font_choices: Vec::new(),
            exe_started: exe_mtime(),
            exe_checked: Instant::now(),
            update_ready: false,
            force_update: false,
            drag_tab: None,
            history: Vec::new(),
            script_keys: Vec::new(),
            script_pointer: Vec::new(),
            window_movable: None,
            cfg,
        };
        app.state_path = state::path(app.script.is_some());
        app.restore_tabs(ctx);
        if app.sessions.is_empty() {
            let start_dir = std::env::current_dir()
                .ok()
                .filter(|p| p.as_os_str() != "/");
            app.new_tab(ctx, start_dir);
        }
        app
    }

    /// Reabre las pestañas de la última vez, cada una en su carpeta. Donde corría Claude Code
    /// deja escrito `claude --continue` sin ejecutarlo: Enter retoma esa conversación.
    fn restore_tabs(&mut self, ctx: &egui::Context) {
        let Some(path) = self.state_path.clone().filter(|_| self.cfg.restore_tabs) else {
            return;
        };
        let saved = state::load(&path);
        for tab in &saved.tabs {
            let dir = std::path::PathBuf::from(&tab.cwd);
            if !dir.is_dir() {
                continue;
            }
            let before = self.sessions.len();
            self.new_tab(ctx, Some(dir));
            if self.sessions.len() > before {
                if let Some(s) = self.sessions.last_mut() {
                    s.name = tab.name.clone();
                }
            }
            if tab.agent == "claude" && self.sessions.len() > before {
                if let Some(s) = self.sessions.last_mut() {
                    s.input(b"claude --continue".to_vec());
                }
            }
        }
        if !self.sessions.is_empty() {
            self.active = saved.active.min(self.sessions.len() - 1);
        }
        self.saved = saved;
    }

    /// Guarda las pestañas cuando cambian (como mucho cada 2 s, y siempre al quedarse sin ninguna).
    fn save_tabs(&mut self) {
        let Some(path) = self.state_path.clone().filter(|_| self.cfg.restore_tabs) else {
            return;
        };
        if !self.sessions.is_empty() && self.state_checked.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.state_checked = Instant::now();
        let now = state::Saved {
            active: self.active,
            tabs: self
                .sessions
                .iter()
                .map(|s| state::Tab {
                    cwd: s.cwd.display().to_string(),
                    name: s.name.clone(),
                    agent: match s.agent {
                        Some(session::Agent::Claude) => "claude".into(),
                        _ => String::new(),
                    },
                })
                .collect(),
        };
        if now != self.saved {
            state::save(&path, &now);
            self.saved = now;
        }
    }

    /// Lee el historial del shell y le suma lo tecleado aquí (primero la pestaña activa).
    fn load_history(&mut self) {
        let shell = if self.cfg.shell.is_empty() {
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into())
        } else {
            self.cfg.shell.clone()
        };
        let mut typed: Vec<String> = Vec::new();
        for (i, s) in self.sessions.iter().enumerate() {
            if i != self.active {
                typed.extend(s.typed.iter().cloned());
            }
        }
        if let Some(s) = self.sessions.get(self.active) {
            typed.extend(s.typed.iter().cloned());
        }
        self.history = history::load(&shell, &typed);
    }

    /// ↑ en el prompt abre el historial; en cualquier otro caso la tecla sigue su camino.
    fn history_key(&mut self, ctx: &egui::Context) {
        if !self.cfg.history_popup
            || !self
                .sessions
                .get(self.active)
                .is_some_and(|s| s.at_prompt())
        {
            return;
        }
        let up = Shortcut {
            cmd: false,
            ctrl: false,
            shift: false,
            alt: false,
            key: egui::Key::ArrowUp,
        };
        if !take_shortcut(ctx, &up) {
            return;
        }
        self.load_history();
        if self.history.is_empty() {
            // Sin historial que mostrar, el shell recibe su flecha de siempre.
            if let Some(s) = self.sessions.get_mut(self.active) {
                let app = s
                    .term
                    .lock()
                    .mode()
                    .contains(alacritty_terminal::term::TermMode::APP_CURSOR);
                s.input(if app {
                    b"\x1bOA".to_vec()
                } else {
                    b"\x1b[A".to_vec()
                });
            }
            return;
        }
        self.overlay = overlay::Overlay::History {
            query: String::new(),
            sel: 0,
        };
        ctx.request_repaint();
    }

    /// Arranca la versión que hay ahora en disco y cierra esta. Las pestañas se guardan
    /// antes y la nueva las reabre; lo que estuviera corriendo en ellas se detiene.
    fn restart(&mut self) {
        self.state_checked = Instant::now() - Duration::from_secs(3);
        self.save_tabs();
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        // Dentro de Colibri.app se relanza el bundle, para conservar permisos y notificaciones.
        let bundle = exe
            .ancestors()
            .find(|p| p.extension().is_some_and(|e| e == "app"));
        let launched = match bundle {
            Some(app) => std::process::Command::new("/usr/bin/open")
                .arg("-n")
                .arg(app)
                .spawn(),
            None => std::process::Command::new(&exe).spawn(),
        };
        match launched {
            Ok(_) => std::process::exit(0),
            Err(e) => eprintln!("colibri: no se pudo reiniciar: {e}"),
        }
    }

    /// Hay una compilación nueva cuando el ejecutable cambió y lleva 2 s sin tocarse
    /// (bundle.sh lo copia y luego lo firma: no hay que arrancarlo a medias).
    fn check_update(&mut self, ctx: &egui::Context) {
        if self.exe_checked.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.exe_checked = Instant::now();
        let changed = exe_mtime().filter(|m| self.exe_started.is_some_and(|s| s != *m));
        let settled =
            changed.is_some_and(|m| m.elapsed().is_ok_and(|d| d > Duration::from_secs(2)));
        if changed.is_some() && !settled {
            ctx.request_repaint_after(Duration::from_secs(2));
        }
        self.update_ready = settled;
    }

    fn new_tab(&mut self, ctx: &egui::Context, cwd: Option<std::path::PathBuf>) {
        match Session::new(self.next_id, &self.cfg, cwd, ctx) {
            Ok(s) => {
                self.next_id += 1;
                self.sessions.push(s);
                self.active = self.sessions.len() - 1;
            }
            Err(e) => eprintln!("colibri: no se pudo abrir la terminal: {e}"),
        }
    }

    fn close_tab(&mut self, i: usize) {
        if i >= self.sessions.len() {
            return;
        }
        self.sessions.remove(i);
        if self.active > i || self.active >= self.sessions.len() {
            self.active = self.active.saturating_sub(1);
        }
    }

    /// Cambia una pestaña de posición; la activa sigue siendo la misma.
    fn move_tab(&mut self, from: usize, to: usize) {
        let n = self.sessions.len();
        if from >= n || to >= n || from == to {
            return;
        }
        let active_id = self.sessions.get(self.active).map(|s| s.id);
        let tab = self.sessions.remove(from);
        self.sessions.insert(to, tab);
        if let Some(i) = active_id.and_then(|id| self.sessions.iter().position(|s| s.id == id)) {
            self.active = i;
        }
    }

    fn select(&mut self, i: usize) {
        if i < self.sessions.len() {
            self.active = i;
            self.sessions[i].unseen = false;
        }
    }

    /// Va al agente (o comando largo) que terminó más recientemente; primero los no vistos.
    fn jump_finished(&mut self) {
        let best = |unseen_only: bool, sessions: &[Session], active: usize| {
            sessions
                .iter()
                .enumerate()
                .filter(|(i, s)| {
                    *i != active && s.finished_at.is_some() && !s.busy && (!unseen_only || s.unseen)
                })
                .max_by_key(|(_, s)| s.finished_at)
                .map(|(i, _)| i)
        };
        let target = best(true, &self.sessions, self.active)
            .or_else(|| best(false, &self.sessions, self.active));
        if let Some(i) = target {
            self.select(i);
        }
    }

    fn reload_config(&mut self, ctx: &egui::Context) {
        let new = config::load();
        if new.font_path != self.cfg.font_path || new.font_bold_path != self.cfg.font_bold_path {
            install_fonts(ctx, &new);
        }
        self.pal = Palette::new(&new.theme);
        self.keys = Keys::new(&new.keys);
        self.cfg = new;
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let hits: Vec<Cmd> = self
            .keys
            .bindings
            .iter()
            .filter(|(_, sc, _)| take_shortcut(ctx, sc))
            .map(|(cmd, _, _)| *cmd)
            .collect();
        for cmd in hits {
            self.run(ctx, cmd);
        }
        const DIGITS: [egui::Key; 9] = [
            egui::Key::Num1,
            egui::Key::Num2,
            egui::Key::Num3,
            egui::Key::Num4,
            egui::Key::Num5,
            egui::Key::Num6,
            egui::Key::Num7,
            egui::Key::Num8,
            egui::Key::Num9,
        ];
        for (i, key) in DIGITS.iter().enumerate() {
            let sc = Shortcut {
                cmd: true,
                ctrl: false,
                shift: false,
                alt: false,
                key: *key,
            };
            if take_shortcut(ctx, &sc) {
                // Como en los navegadores: cmd+9 va a la última.
                let n = self.sessions.len();
                self.run(
                    ctx,
                    Cmd::GoTab(if i == 8 { n.saturating_sub(1) } else { i }),
                );
            }
        }
    }

    pub fn run(&mut self, ctx: &egui::Context, cmd: Cmd) {
        let n = self.sessions.len();
        match cmd {
            Cmd::NewTab => {
                let cwd = self.sessions.get(self.active).map(|s| s.cwd.clone());
                self.new_tab(ctx, cwd);
            }
            Cmd::CloseTab => self.close_tab(self.active),
            Cmd::NextTab if n > 0 => self.select((self.active + 1) % n),
            Cmd::PrevTab if n > 0 => self.select((self.active + n - 1) % n),
            Cmd::NextTab | Cmd::PrevTab => {}
            Cmd::MoveTabLeft if self.active > 0 => self.move_tab(self.active, self.active - 1),
            Cmd::MoveTabRight => self.move_tab(self.active, self.active + 1),
            Cmd::MoveTabLeft => {}
            Cmd::GoTab(i) => self.select(i),
            Cmd::JumpFinished => self.jump_finished(),
            Cmd::TogglePanel => {
                let open = !(self.show_agents || self.show_ports || self.show_procs);
                self.show_agents = open;
                self.show_ports = open;
                self.show_procs = false;
            }
            Cmd::ToggleProcs => self.show_procs = !self.show_procs,
            Cmd::ToggleAgents => self.show_agents = !self.show_agents,
            Cmd::TogglePorts => self.show_ports = !self.show_ports,
            Cmd::FontBigger => self.zoom = (self.zoom + 1.0).min(24.0),
            Cmd::FontSmaller => self.zoom = (self.zoom - 1.0).max(6.0 - self.cfg.font_size),
            Cmd::FontReset => self.zoom = 0.0,
            Cmd::Palette => {
                self.overlay = match self.overlay {
                    overlay::Overlay::Palette { .. } => overlay::Overlay::None,
                    _ => overlay::Overlay::Palette {
                        query: String::new(),
                        sel: 0,
                    },
                };
            }
            Cmd::Prefs => {
                self.overlay = match self.overlay {
                    overlay::Overlay::Prefs { .. } => overlay::Overlay::None,
                    _ => {
                        self.font_choices = config::fonts();
                        overlay::Overlay::Prefs { sel: 0 }
                    }
                };
            }
            Cmd::RenameTab => {
                if let Some(s) = self.sessions.get(self.active) {
                    self.overlay = overlay::Overlay::Rename {
                        text: s.name.clone(),
                    };
                }
            }
            Cmd::OpenConfig => {
                let _ = config::load();
                spawn_detached(
                    "/usr/bin/open",
                    &["-t".into(), config::path().display().to_string()],
                );
            }
            Cmd::Restart => self.restart(),
            Cmd::History => {
                self.overlay = match self.overlay {
                    overlay::Overlay::History { .. } => overlay::Overlay::None,
                    _ => {
                        self.load_history();
                        overlay::Overlay::History {
                            query: String::new(),
                            sel: 0,
                        }
                    }
                };
            }
            Cmd::Clear => {
                if let Some(s) = self.sessions.get_mut(self.active) {
                    s.term.lock().grid_mut().clear_history();
                    s.input(vec![0x0c]);
                }
            }
        }
    }

    /// Aplica y guarda un cambio hecho desde Preferencias.
    pub fn commit_cfg(&mut self, ctx: &egui::Context, old: &Config) {
        if old.font_path != self.cfg.font_path || old.font_bold_path != self.cfg.font_bold_path {
            install_fonts(ctx, &self.cfg);
        }
        if old.font_size != self.cfg.font_size {
            self.zoom = 0.0;
        }
        self.pal = Palette::new(&self.cfg.theme);
        config::save(&self.cfg);
        self.cfg_mtime = config::mtime();
    }

    fn run_script(&mut self, ctx: &egui::Context) {
        let Some(script) = self.script.as_mut() else {
            return;
        };
        ctx.request_repaint_after(Duration::from_millis(30));
        let step = script.next();
        if script.done() && step.is_none() {
            self.script = None;
            return;
        }
        match step {
            Some(debug::Step::Type(text)) => {
                if let Some(s) = self.sessions.get_mut(self.active) {
                    s.input(text.into_bytes());
                }
            }
            Some(debug::Step::Tab) => {
                let cwd = self.sessions.get(self.active).map(|s| s.cwd.clone());
                self.new_tab(ctx, cwd);
            }
            Some(debug::Step::Select(i)) => self.select(i),
            Some(debug::Step::Shot(path)) => {
                ctx.request_screenshot(move |image| debug::save_bmp(&path, &image))
            }
            Some(debug::Step::Procs) => self.show_procs = !self.show_procs,
            Some(debug::Step::Ports) => self.show_ports = !self.show_ports,
            Some(debug::Step::Hover) => self.force_hover = !self.force_hover,
            Some(debug::Step::KillPort(port)) => {
                let pid = self
                    .ports
                    .list
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|p| p.port == port)
                    .map(|p| p.pid);
                if let Some(pid) = pid {
                    self.stop_pid(pid);
                }
            }
            Some(debug::Step::Open(what)) => {
                self.run(
                    ctx,
                    match what.as_str() {
                        "prefs" => Cmd::Prefs,
                        "history" => Cmd::History,
                        _ => Cmd::Palette,
                    },
                );
            }
            Some(debug::Step::Pref(id, dir)) => self.pref_change(ctx, &id, dir),
            Some(debug::Step::Name(name)) => {
                if let Some(s) = self.sessions.get_mut(self.active) {
                    s.name = name;
                }
            }
            Some(debug::Step::Move(from, to)) => self.move_tab(from, to),
            Some(debug::Step::Key(name)) => {
                let name = match name.as_str() {
                    "up" => "ArrowUp",
                    "down" => "ArrowDown",
                    "enter" => "Enter",
                    "esc" => "Escape",
                    other => other,
                };
                self.script_keys.extend(egui::Key::from_name(name));
            }
            Some(debug::Step::Pointer(kind, x, y)) => {
                let pos = Pos2::new(x, y);
                self.script_pointer.push(egui::Event::PointerMoved(pos));
                if kind != "moveto" {
                    self.script_pointer.push(egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: kind == "press",
                        modifiers: egui::Modifiers::NONE,
                    });
                }
            }
            Some(debug::Step::Update) => self.force_update = !self.force_update,
            Some(debug::Step::Quit) => ctx.send_viewport_cmd(ViewportCommand::Close),
            Some(debug::Step::Wait(_)) | None => {}
        }
    }

    fn metrics(&self, ctx: &egui::Context) -> view::Metrics {
        let size = (self.cfg.font_size + self.zoom).clamp(6.0, 60.0);
        let font = FontId::new(size, FontFamily::Name("term".into()));
        let bold = FontId::new(size, FontFamily::Name("term-bold".into()));
        let (w, h) = ctx.fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
        let cell_h = (h * self.cfg.line_height.clamp(0.8, 2.5)).round().max(4.0);
        view::Metrics {
            font,
            bold,
            cell_w: w.max(2.0),
            cell_h,
            text_dy: ((cell_h - h) / 2.0).round(),
            pad: self.cfg.padding.clamp(0.0, 64.0),
            cursor: match self.cfg.cursor.as_str() {
                "bar" | "beam" => alacritty_terminal::vte::ansi::CursorShape::Beam,
                "underline" => alacritty_terminal::vte::ansi::CursorShape::Underline,
                _ => alacritty_terminal::vte::ansi::CursorShape::Block,
            },
        }
    }

    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let pal = self.pal.clone();
        let full = ui.max_rect();
        let time = ui.input(|i| i.time);
        let n = self.sessions.len().max(1);
        let show_update = self.update_ready || self.force_update;
        let right_reserved = if show_update { 252.0 } else { 158.0 };
        let avail = full.width() - TRAFFIC_LIGHTS_W - right_reserved - 34.0;
        let tab_w = (avail / n as f32).clamp(64.0, 230.0);
        let ui_font = FontId::proportional(12.5);

        let mut select = None;
        let mut close = None;
        let mut rename = None;
        let mut moved = None;
        let mut grabbed = None;
        let tabs_left = full.left() + TRAFFIC_LIGHTS_W;
        let pointer_x = ui.input(|i| i.pointer.latest_pos().map(|p| p.x));
        if !ui.input(|i| i.pointer.primary_down()) {
            self.drag_tab = None;
        }
        let mut x = tabs_left;
        for (i, s) in self.sessions.iter().enumerate() {
            let mut rect = Rect::from_min_size(
                Pos2::new(x, full.top() + 5.0),
                Vec2::new(tab_w - 4.0, TAB_BAR_H - 9.0),
            );
            x += tab_w;
            // La pestaña arrastrada sigue al puntero; al cruzar a otro hueco cambia de sitio.
            let dragging = match (self.drag_tab, pointer_x) {
                (Some((id, grab)), Some(px)) if id == s.id => {
                    let last = tabs_left + (self.sessions.len() - 1) as f32 * tab_w;
                    let left = (px - grab).clamp(tabs_left, last);
                    rect = rect.translate(Vec2::new(left - rect.left(), 0.0));
                    let slot = ((left + tab_w / 2.0 - tabs_left) / tab_w).floor() as usize;
                    if slot != i {
                        moved = Some((i, slot));
                    }
                    true
                }
                _ => false,
            };
            let resp = ui.interact(
                rect,
                ui.scope_id().with(("tab", s.id)),
                Sense::click_and_drag(),
            );
            if resp.drag_started() {
                if let Some(px) = pointer_x {
                    grabbed = Some((i, s.id, px - rect.left()));
                }
            }
            let active = i == self.active;
            let hovered = resp.hovered() && (self.drag_tab.is_none() || dragging);
            // Encima de las demás mientras se mueve.
            let p = &if dragging {
                ui.painter().clone().with_layer_id(egui::LayerId::new(
                    egui::Order::Foreground,
                    egui::Id::unique("tab-drag"),
                ))
            } else {
                ui.painter().clone()
            };
            if active {
                p.rect_filled(rect, CornerRadius::same(7), pal.tab_active);
                p.rect_stroke(
                    rect,
                    CornerRadius::same(7),
                    Stroke::new(1.0, pal.border),
                    StrokeKind::Inside,
                );
            } else if hovered {
                p.rect_filled(rect, CornerRadius::same(7), pal.border);
            }

            // Indicador a la izquierda: trabajando / terminó / agente en reposo.
            let ind = Pos2::new(rect.left() + 14.0, rect.center().y);
            if s.busy {
                let color = s.agent.map(|a| a.color()).unwrap_or(pal.accent);
                spinner(p, ind, 5.0, color, time);
            } else if s.unseen {
                p.circle_filled(ind, 4.0, pal.done);
            } else if let Some(a) = s.agent {
                agent_icon(p, ind, 5.5, a);
            } else {
                p.circle_filled(ind, 2.0, pal.muted.gamma_multiply(0.5));
            }

            let text_color = if active { pal.fg } else { pal.muted };
            let right_w = 26.0;
            let max_w = rect.width() - 28.0 - right_w;
            let galley = elided(&ctx, &s.tab_label(), &ui_font, text_color, max_w);
            p.galley(
                Pos2::new(rect.left() + 26.0, rect.center().y - galley.size().y / 2.0),
                galley,
                text_color,
            );

            // Derecha: cerrar al pasar el ratón; si no, el atajo.
            let close_rect = Rect::from_center_size(
                Pos2::new(rect.right() - 14.0, rect.center().y),
                Vec2::splat(18.0),
            );
            let over_close = hovered && !dragging && ui.rect_contains_pointer(close_rect);
            if hovered {
                if over_close {
                    p.rect_filled(
                        close_rect,
                        CornerRadius::same(4),
                        pal.muted.gamma_multiply(0.3),
                    );
                }
                p.text(
                    close_rect.center(),
                    Align2::CENTER_CENTER,
                    "×",
                    FontId::proportional(14.0),
                    pal.fg,
                );
            } else if i < 9 {
                p.text(
                    close_rect.center(),
                    Align2::CENTER_CENTER,
                    format!("⌘{}", i + 1),
                    FontId::proportional(10.5),
                    pal.muted.gamma_multiply(0.7),
                );
            }

            if resp.clicked() {
                if over_close {
                    close = Some(i)
                } else {
                    select = Some(i)
                }
            }
            if resp.middle_clicked() {
                close = Some(i);
            }
            if resp.double_clicked() && !over_close {
                rename = Some(i);
            }
            if self.drag_tab.is_none() {
                resp.on_hover_ui(|ui| tab_tooltip(ui, s, &pal));
            }
        }

        // Sobre las pestañas el arrastre es nuestro (reordenar), no de macOS (mover la ventana).
        let tabs_zone = Rect::from_min_max(
            Pos2::new(tabs_left, full.top()),
            Pos2::new(x + 30.0, full.bottom()),
        );
        let over_tabs = ui.input(|i| {
            i.pointer
                .latest_pos()
                .is_some_and(|p| tabs_zone.contains(p))
        });
        let movable = !(over_tabs || self.drag_tab.is_some() || grabbed.is_some());
        if self.window_movable != Some(movable) && blur::set_movable(movable) {
            self.window_movable = Some(movable);
        }

        // Botón de nueva pestaña.
        let plus = Rect::from_center_size(Pos2::new(x + 12.0, full.center().y), Vec2::splat(24.0));
        let plus_resp = ui.interact(plus, ui.scope_id().with("plus"), Sense::click());
        if plus_resp.hovered() {
            ui.painter()
                .rect_filled(plus, CornerRadius::same(6), pal.border);
        }
        ui.painter().text(
            plus.center(),
            Align2::CENTER_CENTER,
            "+",
            FontId::proportional(16.0),
            pal.muted,
        );
        let plus_clicked = plus_resp.clicked();
        plus_resp.on_hover_text("Nueva pestaña  ⌘T");

        // Un botón por apartado; el de agentes lleva el contador de los que terminaron sin verlos.
        let unseen = self.sessions.iter().filter(|s| s.unseen).count();
        let cy = full.center().y;
        let ports_btn =
            Rect::from_center_size(Pos2::new(full.right() - 26.0, cy), Vec2::new(30.0, 24.0));
        let agents_btn =
            Rect::from_center_size(Pos2::new(full.right() - 62.0, cy), Vec2::new(30.0, 24.0));
        let procs_btn =
            Rect::from_center_size(Pos2::new(full.right() - 98.0, cy), Vec2::new(30.0, 24.0));
        let toggle = procs_btn;
        for (rect, section, hint) in [
            (procs_btn, Section::Processes, "Procesos  ⌘⇧M"),
            (agents_btn, Section::Agents, "Agentes  ⌘⇧A"),
            (ports_btn, Section::Ports, "Puertos  ⌘⇧P"),
        ] {
            let resp = ui.interact(rect, ui.scope_id().with(("section", hint)), Sense::click());
            let on = match section {
                Section::Agents => self.show_agents,
                Section::Ports => self.show_ports,
                Section::Processes => self.show_procs,
            };
            let p = ui.painter();
            if on {
                p.rect_filled(rect, CornerRadius::same(6), pal.accent.gamma_multiply(0.18));
            } else if resp.hovered() {
                p.rect_filled(rect, CornerRadius::same(6), pal.border);
            }
            section_icon(
                p,
                rect.center(),
                section,
                if on { pal.accent } else { pal.muted },
            );
            if resp.clicked() {
                match section {
                    Section::Agents => self.show_agents = !on,
                    Section::Ports => self.show_ports = !on,
                    Section::Processes => self.show_procs = !on,
                }
            }
            resp.on_hover_text(hint);
        }
        if unseen > 0 {
            let c = Pos2::new(toggle.left() - 12.0, cy);
            let p = ui.painter();
            p.circle_filled(c, 8.0, pal.done);
            p.text(
                c,
                Align2::CENTER_CENTER,
                unseen.to_string(),
                FontId::proportional(10.5),
                Color32::BLACK,
            );
        }

        // Compilación nueva en disco: un clic reinicia y la toma.
        let mut right_edge = toggle.left() - 24.0;
        let mut restart = false;
        if show_update {
            let pill = Rect::from_min_max(
                Pos2::new(right_edge - 86.0, cy - 11.0),
                Pos2::new(right_edge, cy + 11.0),
            );
            let resp = ui.interact(pill, ui.scope_id().with("update"), Sense::click());
            let fill = if resp.hovered() {
                pal.accent
            } else {
                pal.accent.gamma_multiply(0.85)
            };
            let p = ui.painter();
            p.rect_filled(pill, CornerRadius::same(6), fill);
            p.text(
                pill.center(),
                Align2::CENTER_CENTER,
                "Actualizar",
                FontId::proportional(11.5),
                pal.bg,
            );
            restart = resp.clicked();
            resp.on_hover_text(
                "Hay una compilación nueva. Reinicia Colibrí y reabre tus pestañas;\nlo que esté corriendo en ellas se detiene.",
            );
            right_edge = pill.left() - 8.0;
        }

        // El resto de la barra arrastra la ventana.
        let drag_rect = Rect::from_min_max(
            Pos2::new(x + 30.0, full.top()),
            Pos2::new(right_edge, full.bottom()),
        );
        if drag_rect.width() > 4.0 {
            let drag = ui.interact(
                drag_rect,
                ui.scope_id().with("drag"),
                Sense::click_and_drag(),
            );
            if drag.drag_started() {
                ctx.send_viewport_cmd(ViewportCommand::StartDrag);
            }
            if drag.double_clicked() {
                let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                ctx.send_viewport_cmd(ViewportCommand::Maximized(!max));
            }
        }

        if let Some(i) = select {
            self.select(i);
        }
        if let Some((i, id, grab)) = grabbed {
            self.select(i);
            self.drag_tab = Some((id, grab));
        }
        if let Some((from, to)) = moved {
            self.move_tab(from, to);
        }
        if let Some(i) = close {
            self.close_tab(i);
        }
        if let Some(i) = rename {
            self.select(i);
            self.run(&ctx, Cmd::RenameTab);
        }
        if plus_clicked {
            let cwd = self.sessions.get(self.active).map(|s| s.cwd.clone());
            self.new_tab(&ctx, cwd);
        }
        if restart {
            self.restart();
        }
    }

    fn side_panel(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal.clone();
        let time = ui.input(|i| i.time);
        let title_font = FontId::proportional(13.0);
        let small = FontId::proportional(11.5);
        let mut select = None;

        if self.show_agents {
            if section_header(
                ui,
                &pal,
                Section::Agents,
                "Agentes",
                "⌘J salta al que terminó",
                0.0,
            ) {
                self.show_agents = false;
            }
            let agents: Vec<usize> = (0..self.sessions.len())
                .filter(|&i| self.sessions[i].agent.is_some())
                .collect();
            if agents.is_empty() {
                hint(
                    ui,
                    &pal,
                    "Sin agentes. Abre claude, codex, gemini… en una pestaña.",
                );
            }
            for i in agents {
                let s = &self.sessions[i];
                let agent = s.agent.unwrap();
                let (rect, resp) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), 62.0), Sense::click());
                let p = ui.painter();
                if i == self.active {
                    p.rect_filled(rect, 0.0, pal.border.gamma_multiply(0.7));
                } else if resp.hovered() {
                    p.rect_filled(rect, 0.0, pal.border.gamma_multiply(0.4));
                }
                let left = rect.left() + 14.0;
                agent_icon(p, Pos2::new(left + 6.0, rect.top() + 18.0), 6.5, agent);
                let tx = left + 22.0;
                let max_w = rect.right() - tx - 64.0;
                let name = elided(ui.ctx(), &s.project(), &title_font, pal.fg, max_w);
                p.galley(Pos2::new(tx, rect.top() + 9.0), name, pal.fg);
                let desc = if s.clean_title().is_empty() {
                    agent.label()
                } else {
                    s.clean_title()
                };
                let desc = elided(ui.ctx(), desc, &small, pal.muted, rect.right() - tx - 12.0);
                p.galley(Pos2::new(tx, rect.top() + 26.0), desc, pal.muted);

                let dot = Pos2::new(tx + 4.0, rect.top() + 49.0);
                let (label, color) = if s.busy {
                    spinner(p, dot, 4.0, agent.color(), time);
                    ("Trabajando…".to_string(), pal.fg)
                } else if s.unseen {
                    p.circle_filled(dot, 3.5, pal.done);
                    ("Terminó".to_string(), pal.done)
                } else {
                    p.circle_filled(dot, 3.0, pal.muted.gamma_multiply(0.6));
                    ("Esperando tu mensaje".to_string(), pal.muted)
                };
                p.text(
                    Pos2::new(tx + 14.0, dot.y),
                    Align2::LEFT_CENTER,
                    label,
                    small.clone(),
                    color,
                );
                if let Some(t) = s.finished_at.filter(|_| !s.busy) {
                    p.text(
                        Pos2::new(rect.right() - 12.0, rect.top() + 16.0),
                        Align2::RIGHT_CENTER,
                        ago(t.elapsed()),
                        small.clone(),
                        pal.muted,
                    );
                }
                p.line_segment(
                    [rect.left_bottom(), rect.right_bottom()],
                    Stroke::new(1.0, pal.border),
                );
                if resp.clicked() {
                    select = Some(i);
                }
                resp.on_hover_text(proc::pretty_path(&s.cwd));
            }
            ui.add_space(14.0);
        }

        if self.show_ports {
            let ports = self.ports.list.lock().unwrap().clone();
            let web: Vec<_> = ports.iter().filter(|p| p.web).collect();
            let other: Vec<_> = ports.iter().filter(|p| !p.web).collect();
            let count = match web.len() {
                1 => "1 servidor web".to_string(),
                n => format!("{n} servidores web"),
            };
            let header_top = ui.cursor().min.y;
            let header_right = ui.max_rect().right();
            if section_header(ui, &pal, Section::Ports, "Puertos", &count, 26.0) {
                self.show_ports = false;
            }
            let refresh = Rect::from_center_size(
                Pos2::new(header_right - 44.0, header_top + 20.0),
                Vec2::splat(22.0),
            );
            let id = ui.scope_id().with("ports-refresh");
            if icon_button(ui, &pal, refresh, id, Icon::Refresh, "Actualizar ahora").0 {
                self.ports.refresh.store(true, Ordering::Relaxed);
            }
            if web.is_empty() {
                hint(ui, &pal, "No hay servidores web corriendo");
            }
            for port in &web {
                self.port_row(ui, &pal, port, true);
            }
            if !other.is_empty() {
                let arrow = if self.show_other_ports { "⌄" } else { "›" };
                let label = format!("{arrow}  {} puertos más (no web)", other.len());
                let (rect, resp) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::click());
                ui.painter().text(
                    Pos2::new(rect.left() + 14.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    label,
                    small.clone(),
                    pal.muted,
                );
                if resp.clicked() {
                    self.show_other_ports = !self.show_other_ports;
                }
                if self.show_other_ports {
                    for port in &other {
                        self.port_row(ui, &pal, port, false);
                    }
                }
            }
            ui.add_space(14.0);
        }

        self.processes_section(ui, &pal);
        // Quien sigue vivo 5 s después de pedirle que termine pasa a «forzar» la próxima vez.
        let mut refresh = false;
        let stubborn = &mut self.stubborn;
        self.stopping.retain(|pid, since| {
            if !procs::alive(*pid) {
                stubborn.remove(pid);
                refresh = true;
                return false;
            }
            if since.elapsed() > Duration::from_secs(5) {
                stubborn.insert(*pid);
                return false;
            }
            true
        });
        if refresh {
            self.ports.refresh.store(true, Ordering::Relaxed);
        }
        if self
            .confirm_stop
            .is_some_and(|(_, t)| t.elapsed() > CONFIRM_WINDOW)
        {
            self.confirm_stop = None;
        }

        if let Some(i) = select {
            self.select(i);
        }
    }
}

impl App {
    fn processes_section(&mut self, ui: &mut egui::Ui, pal: &Palette) {
        if !self.show_procs {
            return;
        }
        let snap = self.monitor.data.lock().unwrap().clone();
        let gb = |b: u64| b as f64 / 1_073_741_824.0;
        let detail = format!(
            "CPU {:.0} %  ·  RAM {:.0}/{:.0} GB",
            snap.cpu_total,
            gb(snap.mem_used),
            gb(snap.mem_total)
        );
        if section_header(ui, pal, Section::Processes, "Procesos", &detail, 0.0) {
            self.show_procs = false;
            self.search_focus = false;
            return;
        }

        // Buscador y columnas ordenables.
        let small = FontId::proportional(11.5);
        let (bar, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::hover());
        let cpu_x = bar.right() - 100.0;
        let mem_x = bar.right() - 36.0;
        let field = Rect::from_min_max(
            Pos2::new(bar.left() + 14.0, bar.top() + 6.0),
            Pos2::new(cpu_x - 44.0, bar.bottom() - 6.0),
        );
        let edit = ui.put(
            field,
            egui::TextEdit::singleline(&mut self.proc_search)
                .hint_text("Buscar…")
                .font(small.clone())
                .desired_width(field.width()),
        );
        self.search_focus = edit.has_focus();
        if self.search_focus
            && ui.input(|i| i.key_pressed(egui::Key::Escape) || i.key_pressed(egui::Key::Enter))
        {
            edit.surrender_focus();
            self.search_focus = false;
        }
        for (label, x, by_mem) in [("CPU", cpu_x, false), ("Memoria", mem_x, true)] {
            let active = self.proc_sort_mem == by_mem;
            let color = if active { pal.accent } else { pal.muted };
            let r = ui.painter().text(
                Pos2::new(x, bar.center().y),
                Align2::RIGHT_CENTER,
                label,
                small.clone(),
                color,
            );
            let resp = ui.interact(
                r.expand(5.0),
                ui.scope_id().with(("sort", label)),
                Sense::click(),
            );
            if resp.clicked() {
                self.proc_sort_mem = by_mem;
            }
            resp.on_hover_text("Ordenar por esta columna");
        }
        ui.painter().line_segment(
            [bar.left_bottom(), bar.right_bottom()],
            Stroke::new(1.0, pal.border),
        );

        let ports = self.ports.list.lock().unwrap().clone();
        let query = self.proc_search.trim().to_lowercase();
        let mut procs: Vec<&procs::Proc> = snap
            .procs
            .iter()
            .filter(|p| {
                query.is_empty()
                    || p.name.to_lowercase().contains(&query)
                    || p.app.to_lowercase().contains(&query)
                    || p.pid.to_string() == query
                    || ports
                        .iter()
                        .any(|port| port.pid == p.pid && port.port.to_string().contains(&query))
            })
            .collect();
        if self.proc_sort_mem {
            procs.sort_by(|a, b| b.mem.cmp(&a.mem));
        } else {
            procs.sort_by(|a, b| b.cpu.total_cmp(&a.cpu));
        }
        if procs.is_empty() {
            hint(
                ui,
                pal,
                if snap.procs.is_empty() {
                    "Leyendo procesos…"
                } else {
                    "Nada coincide con la búsqueda"
                },
            );
        }

        let name_font = FontId::proportional(12.5);
        let mono = FontId::monospace(11.5);
        for proc in procs.into_iter().take(60) {
            let (rect, resp) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 42.0), Sense::hover());
            if resp.hovered() {
                ui.painter()
                    .rect_filled(rect, 0.0, pal.border.gamma_multiply(0.4));
            }
            let has_app = !proc.app.is_empty();
            let name_y = if has_app {
                rect.top() + 7.0
            } else {
                rect.center().y - 8.0
            };
            let port = ports.iter().find(|p| p.pid == proc.pid);
            let port_w = if port.is_some() { 48.0 } else { 0.0 };
            let max_w = cpu_x - 46.0 - rect.left() - 14.0 - port_w;
            let name = elided(ui.ctx(), &proc.name, &name_font, pal.fg, max_w);
            let name_w = name.size().x;
            let p = ui.painter();
            p.galley(Pos2::new(rect.left() + 14.0, name_y), name, pal.fg);
            if let Some(port) = port {
                p.text(
                    Pos2::new(rect.left() + 20.0 + name_w, name_y + 1.0),
                    Align2::LEFT_TOP,
                    format!(":{}", port.port),
                    mono.clone(),
                    pal.accent,
                );
            }
            if has_app {
                p.text(
                    Pos2::new(rect.left() + 14.0, rect.top() + 24.0),
                    Align2::LEFT_TOP,
                    &proc.app,
                    small.clone(),
                    pal.muted,
                );
            }
            let cpu_color = if proc.cpu >= 50.0 {
                Color32::from_rgb(0xe0, 0x9b, 0x4f)
            } else if proc.cpu >= 1.0 {
                pal.fg
            } else {
                pal.muted
            };
            let cpu = if proc.cpu >= 10.0 {
                format!("{:.0} %", proc.cpu)
            } else {
                format!("{:.1} %", proc.cpu)
            };
            p.text(
                Pos2::new(cpu_x, rect.center().y),
                Align2::RIGHT_CENTER,
                cpu,
                mono.clone(),
                cpu_color,
            );
            p.text(
                Pos2::new(mem_x, rect.center().y),
                Align2::RIGHT_CENTER,
                procs::human(proc.mem),
                mono.clone(),
                pal.muted,
            );
            p.line_segment(
                [rect.left_bottom(), rect.right_bottom()],
                Stroke::new(1.0, pal.border.gamma_multiply(0.6)),
            );
            let (_, stop) = stop_control(
                ui,
                pal,
                rect,
                resp.hovered(),
                proc.pid,
                "proc",
                &mut self.confirm_stop,
            );
            if stop {
                self.stop_pid(proc.pid);
            }
        }
    }

    /// Pide terminar un proceso; si ya ignoró una petición anterior, lo fuerza.
    fn stop_pid(&mut self, pid: i32) {
        procs::stop(pid, self.stubborn.contains(&pid));
        self.stopping.insert(pid, Instant::now());
        self.ports.refresh.store(true, Ordering::Relaxed);
    }

    /// Pestaña desde la que se lanzó un proceso (su shell es antepasado del proceso).
    fn owner_tab(&self, pid: i32) -> Option<usize> {
        let mut current = pid;
        for _ in 0..16 {
            if let Some(i) = self.sessions.iter().position(|s| s.shell_pid() == current) {
                return Some(i);
            }
            current = proc::parent(current)?;
        }
        None
    }

    fn port_row(&mut self, ui: &mut egui::Ui, pal: &Palette, port: &ports::Port, web: bool) {
        let ctx = ui.ctx().clone();
        let (rect, resp) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 46.0), Sense::click());
        let hovered = resp.hovered() || ui.rect_contains_pointer(rect) || self.force_hover;
        if hovered {
            ui.painter()
                .rect_filled(rect, 0.0, pal.border.gamma_multiply(0.4));
        }
        let red = Color32::from_rgb(0xe5, 0x53, 0x4b);
        let small = FontId::proportional(11.5);
        let url = format!("http://localhost:{}", port.port);
        let owner = self.owner_tab(port.pid);
        let stopping = self.stopping.contains_key(&port.pid);
        let armed = self
            .confirm_stop
            .is_some_and(|(p, t)| p == port.pid && t.elapsed() <= CONFIRM_WINDOW);
        let cy = rect.center().y;
        let mut over_button = false;
        let mut reserved = 12.0;

        if stopping {
            let r = ui.painter().text(
                Pos2::new(rect.right() - 12.0, cy),
                Align2::RIGHT_CENTER,
                "Deteniendo…",
                small.clone(),
                red,
            );
            reserved = r.width() + 24.0;
            ctx.request_repaint_after(Duration::from_millis(500));
        } else if armed {
            let label = if self.stubborn.contains(&port.pid) {
                "Forzar"
            } else {
                "Detener"
            };
            let pill = Rect::from_min_size(
                Pos2::new(rect.right() - 70.0, cy - 11.0),
                Vec2::new(60.0, 22.0),
            );
            let r = ui.interact(
                pill,
                ui.scope_id().with(("port-confirm", port.pid, port.port)),
                Sense::click(),
            );
            ui.painter().rect_filled(pill, CornerRadius::same(5), red);
            ui.painter().text(
                pill.center(),
                Align2::CENTER_CENTER,
                label,
                small.clone(),
                Color32::WHITE,
            );
            over_button = r.hovered();
            if r.clicked() {
                self.confirm_stop = None;
                self.stop_pid(port.pid);
            }
            reserved = 82.0;
            ctx.request_repaint_after(CONFIRM_WINDOW);
        } else if hovered {
            // De derecha a izquierda: detener, copiar, abrir, ir a la pestaña.
            let mut x = rect.right() - 20.0;
            let mut button = |ui: &mut egui::Ui, icon: Icon, tip: &str| -> bool {
                let r = Rect::from_center_size(Pos2::new(x, cy), Vec2::splat(24.0));
                x -= 27.0;
                let id = ui.scope_id().with(("port-btn", tip, port.pid, port.port));
                let (clicked, over) = icon_button(ui, pal, r, id, icon, tip);
                over_button |= over;
                clicked
            };
            if button(ui, Icon::Trash, "Detener el proceso") {
                self.confirm_stop = Some((port.pid, Instant::now()));
            }
            if button(ui, Icon::Copy, "Copiar la URL") {
                ctx.copy_text(url.clone());
            }
            if web && button(ui, Icon::External, "Abrir en el navegador") {
                spawn_detached("/usr/bin/open", &[url.clone()]);
            }
            if let Some(tab) = owner {
                if button(ui, Icon::Arrow, "Ir a la pestaña que lo lanzó") {
                    self.select(tab);
                }
            }
            reserved = rect.right() - x - 6.0;
        }

        let p = ui.painter();
        let color = if web { pal.accent } else { pal.muted };
        p.text(
            Pos2::new(rect.left() + 14.0, rect.top() + 15.0),
            Align2::LEFT_CENTER,
            format!(":{}", port.port),
            FontId::monospace(12.5),
            color,
        );
        let tx = rect.left() + 78.0;
        let max_w = (rect.right() - reserved - tx).max(20.0);
        let name = elided(
            &ctx,
            &port.command,
            &FontId::proportional(12.5),
            pal.fg,
            max_w,
        );
        p.galley(
            Pos2::new(tx, rect.top() + 15.0 - name.size().y / 2.0),
            name,
            pal.fg,
        );
        let place = if port.cwd.is_empty() || port.cwd == "/" {
            format!("pid {}", port.pid)
        } else {
            port.cwd.clone()
        };
        let place = elided(&ctx, &place, &small, pal.muted, max_w);
        p.galley(
            Pos2::new(tx, rect.top() + 31.0 - place.size().y / 2.0),
            place,
            pal.muted,
        );

        if web && resp.clicked() && !over_button && !stopping {
            spawn_detached("/usr/bin/open", &[url]);
        }
    }
}

impl eframe::App for App {
    // Fondo de ventana transparente: lo que se ve lo pinta cada panel con su opacidad.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        raw.events.append(&mut self.script_pointer);
        for key in self.script_keys.drain(..) {
            for pressed in [true, false] {
                raw.events.push(egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));

        if self.cfg_checked.elapsed() > Duration::from_secs(1) {
            self.cfg_checked = Instant::now();
            let m = config::mtime();
            if m != self.cfg_mtime {
                self.cfg_mtime = m;
                self.reload_config(ctx);
            }
        }

        self.run_script(ctx);
        self.check_update(ctx);

        let mut notes = Vec::new();
        for (i, s) in self.sessions.iter_mut().enumerate() {
            s.drain(ctx, &self.pal);
            let watching = self.focused && i == self.active;
            if s.poll(watching) {
                let what = s
                    .agent
                    .map(|a| a.label().to_string())
                    .unwrap_or_else(|| "Comando".into());
                notes.push((
                    format!("{what} terminó"),
                    format!("{} — clic o ⌘J para ir", s.project()),
                    s.id,
                ));
            }
        }
        if !notes.is_empty() {
            if !self.focused {
                ctx.send_viewport_cmd(ViewportCommand::RequestUserAttention(
                    egui::UserAttentionType::Informational,
                ));
            }
            if self.cfg.notify {
                for (title, body, id) in notes {
                    notify::send(&title, &body, id);
                }
            }
        }

        // Clic en una notificación: traer la ventana al frente e ir a esa pestaña.
        for id in notify::take_clicked() {
            if let Some(i) = self.sessions.iter().position(|s| s.id == id) {
                self.select(i);
            }
            ctx.send_viewport_cmd(ViewportCommand::Focus);
        }

        let before = self.sessions.len();
        let active_id = self.sessions.get(self.active).map(|s| s.id);
        self.sessions.retain(|s| !s.exited);
        if self.sessions.len() != before {
            self.active = active_id
                .and_then(|id| self.sessions.iter().position(|s| s.id == id))
                .unwrap_or_else(|| self.active.min(self.sessions.len().saturating_sub(1)));
        }
        self.save_tabs();
        if self.sessions.is_empty() {
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }
        if self.focused {
            if let Some(s) = self.sessions.get_mut(self.active) {
                s.unseen = false;
            }
        }

        self.ports
            .enabled
            .store(self.show_ports || self.show_procs, Ordering::Relaxed);
        self.monitor
            .enabled
            .store(self.show_procs, Ordering::Relaxed);
        // Solo hay trabajo periódico mientras algo corre; en reposo no se repinta nada.
        if self.sessions.iter().any(|s| s.wants_ticks()) {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.shortcuts(&ctx);
        self.overlay_ui(&ctx);
        if self.sessions.is_empty() {
            return;
        }
        let pal = self.pal.clone();

        let opacity = self.cfg.opacity.clamp(0.3, 1.0);
        // El desenfoque es una propiedad de la ventana: se aplica al cambiar la preferencia.
        let radius = if self.cfg.blur && opacity < 0.999 {
            30
        } else {
            0
        };
        if self.blur_applied != Some(radius) && blur::set(radius) {
            self.blur_applied = Some(radius);
        }
        egui::Panel::top("tabs")
            .exact_size(TAB_BAR_H)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(fade(pal.tab_bar, opacity)))
            .show(ui, |ui| self.tab_bar(ui));

        if !self.show_procs {
            self.search_focus = false;
        }
        if self.show_agents || self.show_ports || self.show_procs {
            egui::Panel::right("side")
                .default_size(300.0)
                .size_range(220.0..=520.0)
                .resizable(true)
                .frame(egui::Frame::new().fill(pal.panel))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            self.side_panel(ui);
                        });
                });
        }

        let metrics = self.metrics(&ctx);
        let focused = self.focused;
        if !self.search_focus && self.overlay == overlay::Overlay::None {
            self.history_key(&ctx);
        }
        let keyboard = !self.search_focus && self.overlay == overlay::Overlay::None;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(fade(pal.bg, opacity)))
            .show(ui, |ui| {
                if let Some(s) = self.sessions.get_mut(self.active) {
                    view::show(ui, s, &pal, &metrics, focused, keyboard);
                }
            });

        // Los spinners se animan a ~30 cuadros por segundo con la ventana al frente;
        // en segundo plano basta con refrescar de vez en cuando.
        if self.sessions.iter().any(|s| s.busy) {
            let ms = if self.focused { 33 } else { 250 };
            ctx.request_repaint_after(Duration::from_millis(ms));
        }
    }
}

/// Color con la opacidad indicada (1.0 = sólido).
fn fade(c: Color32, opacity: f32) -> Color32 {
    if opacity >= 0.999 {
        return c;
    }
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (opacity * 255.0).round() as u8)
}

fn take_shortcut(ctx: &egui::Context, sc: &Shortcut) -> bool {
    ctx.input_mut(|i| {
        let mut hit = false;
        i.events.retain(|e| match e {
            egui::Event::Key {
                key,
                physical_key,
                pressed: true,
                modifiers,
                ..
            } if sc.matches(*key, *physical_key, modifiers) => {
                hit = true;
                false
            }
            _ => true,
        });
        hit
    })
}

fn install_fonts(ctx: &egui::Context, cfg: &Config) {
    let mut defs = FontDefinitions::default();
    let fallback = defs
        .families
        .get(&FontFamily::Monospace)
        .cloned()
        .unwrap_or_default();
    // `weight` solo actúa en fuentes variables (un archivo con todos los pesos).
    let mut load = |name: &str, candidates: &[(&str, u32, Option<f32>)]| -> bool {
        for (path, index, weight) in candidates {
            if path.is_empty() {
                continue;
            }
            if let Ok(bytes) = std::fs::read(path) {
                let mut data = FontData::from_owned(bytes);
                data.index = *index;
                if let Some(w) = weight {
                    if data
                        .variation_axes()
                        .iter()
                        .any(|axis| axis.tag.to_string() == "wght")
                    {
                        data.tweak.coords =
                            egui::epaint::text::VariationCoords::new([(b"wght", *w)]);
                    }
                }
                defs.font_data.insert(name.to_string(), Arc::new(data));
                return true;
            }
        }
        false
    };
    let menlo = "/System/Library/Fonts/Menlo.ttc";
    let has_regular = load(
        "term-regular",
        &[(cfg.font_path.as_str(), 0, Some(400.0)), (menlo, 0, None)],
    );
    let bold_from_user = if cfg.font_path.is_empty() {
        ""
    } else {
        cfg.font_path.as_str()
    };
    let has_bold = load(
        "term-bold",
        &[
            (cfg.font_bold_path.as_str(), 0, Some(700.0)),
            (bold_from_user, 0, Some(700.0)),
            (menlo, 1, None),
        ],
    );
    // Menlo va siempre detrás de la fuente elegida: trae los símbolos (✳ ✢ ✶ ✻ ✽ …) que
    // usan los agentes y que casi ninguna fuente de programar incluye. Sin él, esos
    // caracteres caían en el emoji de color del sistema o en un recuadro vacío.
    let has_menlo = !cfg.font_path.is_empty() && load("term-menlo", &[(menlo, 0, None)]);
    let has_symbols = load(
        "term-symbols",
        &[("/System/Library/Fonts/Apple Symbols.ttf", 0, None)],
    );

    let chain = |first: Option<&str>| {
        let mut v: Vec<String> = Vec::new();
        v.extend(first.map(str::to_string));
        if has_regular && first != Some("term-regular") {
            v.push("term-regular".into());
        }
        if has_menlo {
            v.push("term-menlo".into());
        }
        v.extend(fallback.iter().cloned());
        if has_symbols {
            v.push("term-symbols".into());
        }
        v
    };
    defs.families.insert(
        FontFamily::Name("term".into()),
        chain(has_regular.then_some("term-regular")),
    );
    defs.families.insert(
        FontFamily::Name("term-bold".into()),
        chain(has_bold.then_some("term-bold")),
    );
    if has_symbols {
        if let Some(prop) = defs.families.get_mut(&FontFamily::Proportional) {
            prop.push("term-symbols".into());
        }
    }
    ctx.set_fonts(defs);
}

fn spinner(p: &egui::Painter, center: Pos2, r: f32, color: Color32, time: f64) {
    let start = (time * 5.5) as f32;
    let points: Vec<Pos2> = (0..=14)
        .map(|i| {
            let a = start + i as f32 / 14.0 * std::f32::consts::PI * 1.5;
            center + Vec2::new(a.cos(), a.sin()) * r
        })
        .collect();
    p.add(egui::Shape::line(points, Stroke::new(1.6, color)));
}

/// Iconos dibujados a mano: no dependen de qué glifos tenga la fuente.
fn agent_icon(p: &egui::Painter, c: Pos2, r: f32, agent: session::Agent) {
    let color = agent.color();
    match agent {
        session::Agent::Claude => {
            for i in 0..4 {
                let a = i as f32 * std::f32::consts::FRAC_PI_4;
                let d = Vec2::new(a.cos(), a.sin()) * r;
                p.line_segment([c - d, c + d], Stroke::new(1.6, color));
            }
        }
        session::Agent::Codex => {
            p.circle_stroke(c, r - 0.5, Stroke::new(1.6, color));
            p.circle_filled(c, 1.6, color);
        }
        _ => {
            let pts = vec![
                c + Vec2::new(0.0, -r),
                c + Vec2::new(r, 0.0),
                c + Vec2::new(0.0, r),
                c + Vec2::new(-r, 0.0),
            ];
            p.add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
        }
    }
}

fn elided(
    ctx: &egui::Context,
    text: &str,
    font: &FontId,
    color: Color32,
    max_w: f32,
) -> Arc<egui::Galley> {
    let layout = |t: String| ctx.fonts_mut(|f| f.layout_no_wrap(t, font.clone(), color));
    let galley = layout(text.to_string());
    if galley.size().x <= max_w || max_w <= 0.0 {
        return galley;
    }
    // Recorta por el inicio: en una ruta lo que importa es el final.
    let chars: Vec<char> = text.chars().collect();
    let mut keep = ((chars.len() as f32) * max_w / galley.size().x) as usize;
    loop {
        keep = keep.saturating_sub(1);
        let tail: String = chars[chars.len() - keep.min(chars.len())..]
            .iter()
            .collect();
        let g = layout(format!("…{tail}"));
        if g.size().x <= max_w || keep == 0 {
            return g;
        }
    }
}

fn tab_tooltip(ui: &mut egui::Ui, s: &Session, pal: &Palette) {
    ui.label(
        egui::RichText::new(proc::pretty_path(&s.cwd))
            .color(pal.fg)
            .strong(),
    );
    let what = match s.agent {
        Some(a) => a.label().to_string(),
        None if s.fg_name.is_empty() => "shell".to_string(),
        None => s.fg_name.clone(),
    };
    ui.label(egui::RichText::new(what).color(pal.muted));
    if !s.clean_title().is_empty() {
        ui.label(egui::RichText::new(s.clean_title()).color(pal.muted));
    }
    if s.busy {
        ui.label(egui::RichText::new("Corriendo…").color(pal.accent));
    } else if let Some(t) = s.finished_at {
        ui.label(egui::RichText::new(format!("Terminó hace {}", ago(t.elapsed()))).color(pal.done));
    }
}

#[derive(Clone, Copy)]
enum Section {
    Agents,
    Ports,
    Processes,
}

fn section_icon(p: &egui::Painter, c: Pos2, section: Section, color: Color32) {
    let stroke = Stroke::new(1.3, color);
    match section {
        Section::Agents => {
            // Robot: cabeza, ojos y antena.
            let head = Rect::from_center_size(c + Vec2::new(0.0, 1.5), Vec2::new(14.0, 10.0));
            p.rect_stroke(head, CornerRadius::same(3), stroke, StrokeKind::Inside);
            p.circle_filled(head.center() + Vec2::new(-3.0, 0.0), 1.3, color);
            p.circle_filled(head.center() + Vec2::new(3.0, 0.0), 1.3, color);
            p.line_segment(
                [Pos2::new(c.x, head.top()), Pos2::new(c.x, head.top() - 3.0)],
                stroke,
            );
            p.circle_filled(Pos2::new(c.x, head.top() - 3.5), 1.2, color);
        }
        Section::Processes => {
            // Pulso.
            let x = [-7.0, -3.5, -1.5, 1.5, 3.5, 7.0];
            let y = [0.5, 0.5, -5.0, 5.0, 0.5, 0.5];
            let pts: Vec<Pos2> = x.iter().zip(y).map(|(x, y)| c + Vec2::new(*x, y)).collect();
            p.add(egui::Shape::line(pts, stroke));
        }
        Section::Ports => {
            // Globo: círculo, meridiano y ecuador.
            let r = 6.5;
            p.circle_stroke(c, r, stroke);
            p.line_segment([c - Vec2::new(r, 0.0), c + Vec2::new(r, 0.0)], stroke);
            let meridian: Vec<Pos2> = (0..24)
                .map(|i| {
                    let a = i as f32 / 24.0 * std::f32::consts::TAU;
                    c + Vec2::new(a.cos() * r * 0.45, a.sin() * r)
                })
                .collect();
            p.add(egui::Shape::closed_line(meridian, stroke));
        }
    }
}

/// Encabezado de apartado. Devuelve true si se pulsó la × para ocultarlo.
fn section_header(
    ui: &mut egui::Ui,
    pal: &Palette,
    section: Section,
    title: &str,
    detail: &str,
    reserve: f32,
) -> bool {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), Sense::hover());
    let close = Rect::from_center_size(
        Pos2::new(rect.right() - 18.0, rect.center().y),
        Vec2::splat(20.0),
    );
    let resp = ui.interact(
        close,
        ui.scope_id().with(("close-section", title)),
        Sense::click(),
    );
    let p = ui.painter();
    section_icon(
        p,
        Pos2::new(rect.left() + 22.0, rect.center().y),
        section,
        pal.accent,
    );
    p.text(
        Pos2::new(rect.left() + 40.0, rect.center().y),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(13.5),
        pal.fg,
    );
    p.text(
        Pos2::new(close.left() - 8.0 - reserve, rect.center().y),
        Align2::RIGHT_CENTER,
        detail,
        FontId::proportional(11.0),
        pal.muted,
    );
    if resp.hovered() {
        p.rect_filled(close, CornerRadius::same(4), pal.border);
    }
    p.text(
        close.center(),
        Align2::CENTER_CENTER,
        "×",
        FontId::proportional(15.0),
        if resp.hovered() { pal.fg } else { pal.muted },
    );
    p.line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, pal.border),
    );
    let clicked = resp.clicked();
    resp.on_hover_text("Ocultar");
    clicked
}

fn hint(ui: &mut egui::Ui, pal: &Palette, text: &str) {
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.add(egui::Label::new(egui::RichText::new(text).color(pal.muted).size(12.0)).wrap());
        });
}

#[derive(Clone, Copy)]
enum Icon {
    Arrow,
    External,
    Copy,
    Trash,
    Refresh,
}

/// Botón pequeño con un icono dibujado. Devuelve (pulsado, puntero encima).
fn icon_button(
    ui: &mut egui::Ui,
    pal: &Palette,
    rect: Rect,
    id: egui::Id,
    icon: Icon,
    tip: &str,
) -> (bool, bool) {
    let resp = ui.interact(rect, id, Sense::click());
    let over = resp.hovered();
    let p = ui.painter();
    if over {
        p.rect_filled(rect, CornerRadius::same(5), pal.border);
    }
    let stroke = Stroke::new(1.3, if over { pal.fg } else { pal.muted });
    let c = rect.center();
    let at = |x: f32, y: f32| c + Vec2::new(x, y);
    let path = |pts: &[(f32, f32)]| {
        egui::Shape::line(pts.iter().map(|(x, y)| at(*x, *y)).collect(), stroke)
    };
    match icon {
        Icon::Arrow => {
            p.add(path(&[(-5.0, 0.0), (5.0, 0.0)]));
            p.add(path(&[(1.0, -4.0), (5.0, 0.0), (1.0, 4.0)]));
        }
        Icon::External => {
            p.add(path(&[
                (-1.0, -5.0),
                (-5.0, -5.0),
                (-5.0, 5.0),
                (5.0, 5.0),
                (5.0, 1.0),
            ]));
            p.add(path(&[(0.0, 0.0), (5.5, -5.5)]));
            p.add(path(&[(1.5, -5.5), (5.5, -5.5), (5.5, -1.5)]));
        }
        Icon::Copy => {
            p.rect_stroke(
                Rect::from_min_max(at(-2.0, -2.0), at(5.5, 5.5)),
                CornerRadius::same(1),
                stroke,
                StrokeKind::Middle,
            );
            p.add(path(&[(-5.0, 2.0), (-5.0, -5.0), (2.0, -5.0)]));
        }
        Icon::Trash => {
            p.add(path(&[(-5.0, -4.0), (5.0, -4.0)]));
            p.add(path(&[(-1.5, -6.0), (1.5, -6.0)]));
            p.add(path(&[(-4.0, -4.0), (-3.5, 5.5), (3.5, 5.5), (4.0, -4.0)]));
        }
        Icon::Refresh => {
            let r = 5.0;
            let pts: Vec<Pos2> = (0..=20)
                .map(|i| {
                    let a = -0.6 + i as f32 / 20.0 * std::f32::consts::TAU * 0.82;
                    c + Vec2::new(a.cos(), a.sin()) * r
                })
                .collect();
            p.add(egui::Shape::line(pts, stroke));
            let tip_pt = at(r * (-0.6f32).cos(), r * (-0.6f32).sin());
            p.add(egui::Shape::line(
                vec![
                    tip_pt + Vec2::new(-4.0, -1.0),
                    tip_pt,
                    tip_pt + Vec2::new(1.0, -4.0),
                ],
                stroke,
            ));
        }
    }
    let clicked = resp.clicked();
    resp.on_hover_text(tip);
    (clicked, over)
}

const CONFIRM_WINDOW: Duration = Duration::from_secs(4);

/// Control para detener un proceso desde una fila: × al pasar el ratón y, tras el primer
/// clic, un botón rojo «Detener» que hay que pulsar para confirmar. Devuelve true si el
/// puntero está sobre el control (para que la fila no tome ese clic como suyo).
fn stop_control(
    ui: &mut egui::Ui,
    pal: &Palette,
    row: Rect,
    row_hovered: bool,
    pid: i32,
    salt: &str,
    confirm: &mut Option<(i32, Instant)>,
) -> (bool, bool) {
    let mut stopped = false;
    let armed = confirm.is_some_and(|(p, t)| p == pid && t.elapsed() <= CONFIRM_WINDOW);
    let over_row = row_hovered || ui.rect_contains_pointer(row);
    if !armed && !over_row {
        return (false, false);
    }
    let red = Color32::from_rgb(0xe5, 0x53, 0x4b);
    let size = if armed {
        Vec2::new(58.0, 22.0)
    } else {
        Vec2::splat(20.0)
    };
    let rect = Rect::from_min_size(
        Pos2::new(row.right() - 8.0 - size.x, row.center().y - size.y / 2.0),
        size,
    );
    let resp = ui.interact(
        rect,
        ui.scope_id().with(("stop", salt, pid)),
        Sense::click(),
    );
    let p = ui.painter();
    if armed {
        p.rect_filled(rect, CornerRadius::same(5), red);
        p.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "Detener",
            FontId::proportional(11.5),
            Color32::WHITE,
        );
        if resp.clicked() {
            *confirm = None;
            stopped = true;
        }
        ui.ctx().request_repaint_after(CONFIRM_WINDOW);
    } else {
        p.rect_filled(
            rect,
            CornerRadius::same(4),
            if resp.hovered() {
                red.gamma_multiply(0.35)
            } else {
                pal.panel
            },
        );
        p.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "×",
            FontId::proportional(14.0),
            if resp.hovered() { pal.fg } else { pal.muted },
        );
        if resp.clicked() {
            *confirm = Some((pid, Instant::now()));
        }
    }
    let over = resp.hovered();
    resp.on_hover_text(if armed {
        format!("Confirmar: terminar pid {pid}")
    } else {
        format!("Detener proceso (pid {pid})")
    });
    (over, stopped)
}

fn ago(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..=59 => "un momento".into(),
        60..=3599 => format!("{} min", s / 60),
        _ => format!("{} h", s / 3600),
    }
}

fn exe_mtime() -> Option<SystemTime> {
    std::fs::metadata(std::env::current_exe().ok()?)
        .and_then(|m| m.modified())
        .ok()
}

fn spawn_detached(program: &str, args: &[String]) {
    if let Ok(mut child) = std::process::Command::new(program).args(args).spawn() {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}
