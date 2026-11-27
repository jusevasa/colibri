//! Colibrí — una terminal rápida para correr varios agentes.
//! Sin telemetría: la app no abre ninguna conexión de red propia.

mod agents;
mod anim;
mod blur;
mod claude;
mod clip;
mod complete;
mod config;
mod debug;
mod disk;
mod drag;
mod git;
mod grid;
mod history;
mod hotkey;
mod i18n;
mod macos;
mod menu;
mod metal;
mod notify;
mod overlay;
mod ports;
mod proc;
mod procs;
mod session;
mod shell;
mod state;
mod switch;
mod updates;
mod view;

use config::{Config, Palette, Shortcut};
use egui::{
    self, Align2, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Pos2, Rect,
    Sense, Stroke, StrokeKind, Vec2, ViewportCommand,
};
use session::Session;
use shell::WindowId;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

const TAB_BAR_H: f32 = 38.0;
/// Colores que se le pueden poner a una pestaña desde su menú (clic derecho).
/// El nombre es el que se guarda en `session.toml`; el que se ve, `i18n::chrome::T::color_names`.
const TAB_COLORS: [(&str, Color32); 7] = [
    ("rojo", Color32::from_rgb(0xf4, 0x70, 0x67)),
    ("naranja", Color32::from_rgb(0xf0, 0x88, 0x3e)),
    ("amarillo", Color32::from_rgb(0xe0, 0xb2, 0x5b)),
    ("verde", Color32::from_rgb(0x57, 0xc9, 0x6b)),
    ("azul", Color32::from_rgb(0x6c, 0xb6, 0xff)),
    ("morado", Color32::from_rgb(0xb0, 0x83, 0xf0)),
    ("rosa", Color32::from_rgb(0xf7, 0x78, 0xba)),
];
const TRAFFIC_LIGHTS_W: f32 = 78.0;

/// Un archivo soltado por el guion de depuración.
#[derive(Debug)]
struct Dropped(PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &std::path::Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}

fn main() -> Result<(), String> {
    i18n::init();
    let defaults = objc2_foundation::NSUserDefaults::standardUserDefaults();
    // Con la entrada de texto del sistema activa (hace falta para ver las teclas muertas),
    // macOS cambiaría la repetición de una tecla mantenida por su menú de acentos. En una
    // terminal mantener una tecla debe repetirla.
    defaults.setBool_forKey(
        false,
        objc2_foundation::ns_string!("ApplePressAndHoldEnabled"),
    );
    // Con esa misma entrada de texto, AppKit lanza «AutoFill (Colibrí)»
    // (`SafariPlatformSupport.Helper`: otro proceso de 18 MB y 5 hilos, medidos con `footprint`
    // y el Monitor de Actividad) para ofrecer autorrelleno de contactos o contraseñas, que en
    // una terminal no tiene dónde ir. Es un valor privado de AppKit (si desaparece, no tiene
    // efecto) y va en el dominio de registro: no se guarda en las preferencias y
    // `defaults write` puede volver a activarlo.
    {
        use objc2::runtime::AnyObject;
        use objc2_foundation::{NSDictionary, NSNumber, NSString, ns_string};
        let no = NSNumber::numberWithBool(false);
        let autofill = NSDictionary::<NSString, AnyObject>::from_slices(
            &[ns_string!("_NSEnableTextInputContextBasedAutoFill")],
            &[&no],
        );
        // SAFETY: claves `NSString` y valores `NSNumber`, tipos de lista de propiedades.
        unsafe { defaults.registerDefaults(&autofill) };
    }
    // Sin icono propio: en el Dock va el del bundle (assets/Colibri.icns).
    let viewport = egui::ViewportBuilder::default()
        .with_title("Colibrí")
        .with_inner_size([1180.0, 760.0])
        .with_min_inner_size([480.0, 300.0])
        .with_fullsize_content_view(true)
        .with_titlebar_shown(false)
        .with_title_shown(false)
        .with_transparent(true);
    macos::install();
    // Cada ventana es una ventana: sin esto, con «Preferir pestañas» en Ajustes, macOS
    // juntaría las ventanas nuevas como pestañas suyas, encima de las nuestras.
    unsafe {
        let _: () = objc2::msg_send![
            objc2::class!(NSWindow),
            setAllowsAutomaticWindowTabbing: false
        ];
    }
    // ⌘C/⌘X/⌘V los traduce `clip::translate` (el lanzador apaga los atajos de egui).
    shell::run(viewport, || Box::new(App::new()))
}

/// Todo lo que se puede hacer desde un atajo o desde la paleta de comandos.
#[derive(Clone, Copy, PartialEq)]
pub enum Cmd {
    NewTab,
    CloseTab,
    NewWindow,
    CloseWindow,
    ReopenClosed,
    NextTab,
    PrevTab,
    MoveTabLeft,
    MoveTabRight,
    NextAgent,
    PrevAgent,
    JumpFinished,
    TogglePanel,
    ToggleAgents,
    SearchAgents,
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
            (Cmd::NewWindow, &k.new_window),
            (Cmd::CloseWindow, &k.close_window),
            (Cmd::ReopenClosed, &k.reopen_closed),
            (Cmd::NextTab, &k.next_tab),
            (Cmd::PrevTab, &k.prev_tab),
            (Cmd::MoveTabLeft, &k.move_tab_left),
            (Cmd::MoveTabRight, &k.move_tab_right),
            (Cmd::NextAgent, &k.next_agent),
            (Cmd::PrevAgent, &k.prev_agent),
            (Cmd::JumpFinished, &k.jump_finished),
            (Cmd::TogglePanel, &k.toggle_panel),
            (Cmd::ToggleProcs, &k.toggle_processes),
            (Cmd::ToggleAgents, &k.toggle_agents),
            (Cmd::SearchAgents, &k.search_agents),
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
    /// «Reducir movimiento» de macOS (Accesibilidad → Pantalla): las pestañas saltan a su
    /// sitio en vez de deslizarse. Se mira con la configuración, cada segundo.
    reduce_motion: bool,
    /// La ventana en curso: aquella cuya pasada se está corriendo. Casi todo el código
    /// trabaja con `self.w` sin saber que hay más ventanas (ver `enter`).
    w: Win,
    /// De qué ventana es `w` (`None` antes de abrir la primera).
    cur: Option<WindowId>,
    /// Las demás ventanas, apartadas mientras no les toca.
    parked: HashMap<WindowId, Win>,
    /// La última ventana que tuvo el foco: la que reciben lo que llega de fuera (Finder,
    /// atajo global, guion).
    front: Option<WindowId>,
    /// Acciones elegidas en la barra de menús, para la siguiente `logic()`.
    menu_cmds: Vec<Cmd>,
    /// Ventanas nuevas pedidas que el lanzador aún no ha abierto, con su posición y tamaño.
    want_windows: Vec<Option<[f32; 4]>>,
    /// Qué va en cada una de esas ventanas, en el mismo orden.
    opening: std::collections::VecDeque<Fill>,
    /// Lo que se cerró hace poco, lo último al final (⌘⇧T lo reabre).
    closed: Vec<Closed>,
    /// Los atajos con que se construyó la barra de menús (`None` = aún no se ha puesto).
    menu_keys: Option<config::Keys>,
    /// Las ventanas abiertas, por orden de apertura.
    order: Vec<WindowId>,
    next_id: u64,
    show_agents: bool,
    /// Filtro y texto del buscador del panel de agentes (compartidos por las ventanas).
    agents_filter: agents::Filter,
    agents_query: String,
    /// Agentes que se pueden lanzar desde el panel (`None` hasta que el hilo responda).
    installed: Arc<std::sync::Mutex<Option<Vec<session::Agent>>>>,
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
    zoom: f32,
    ports: ports::Ports,
    show_other_ports: bool,
    script: Option<debug::Script>,
    state_path: Option<std::path::PathBuf>,
    saved: state::Saved,
    state_checked: Instant,
    font_choices: Vec<(String, String, String)>,
    /// Fecha del ejecutable al arrancar: si cambia, hay una compilación nueva.
    exe_started: Option<SystemTime>,
    exe_checked: Instant,
    update_ready: bool,
    /// Solo para pruebas: muestra el botón «Actualizar» aunque no haya nada nuevo.
    force_update: bool,
    /// «Buscar actualizaciones»: el hilo que mira el cask del tap (`updates.rs`).
    updates: updates::Updates,
    /// Versión publicada más nueva que la que corre, si la preferencia está encendida. El
    /// botón de la barra la ofrece mientras no haya ya una compilación nueva en disco.
    update_available: Option<String>,
    /// Solo para pruebas: si el aviso de versión nueva actúa como con Homebrew (`Some(true)`)
    /// o abre la página (`Some(false)`), en vez de mirarlo en el disco.
    assume_brew: Option<bool>,
    /// Solo para pruebas: teclas del guion que entran como si se pulsaran de verdad.
    script_keys: Vec<(egui::Key, egui::Modifiers)>,
    /// Solo para pruebas: eventos de puntero del guion (`press`, `moveto`, `release`).
    script_pointer: Vec<egui::Event>,
    /// Archivos que el guion suelta sobre la ventana (`drop`).
    script_drops: Vec<PathBuf>,
    /// Número que lleva ahora el icono del Dock: pestañas que terminaron sin verse.
    badge: usize,
    /// La entrada segura de teclado está puesta.
    secure: bool,
}

/// Lo que es de cada ventana: sus pestañas y el estado de su interfaz.
pub struct Win {
    /// El contexto de egui de la ventana (para pedirle cuadros o comandos desde otra).
    ctx: egui::Context,
    sessions: Vec<Session>,
    active: usize,
    focused: bool,
    /// Radio de desenfoque aplicado a la ventana, si ya se aplicó alguno.
    blur_applied: Option<i64>,
    search_focus: bool,
    /// El buscador del panel de agentes tiene el teclado (la terminal no lo recibe).
    agents_search_focus: bool,
    /// Fila elegida con ↑/↓ mientras ese buscador tiene el teclado (id de la pestaña);
    /// `None` = la primera que se ve. Enter va a ella. Es de cada ventana: la búsqueda se
    /// comparte, pero el teclado solo lo tiene una, y las demás la borrarían al repintar.
    agents_pick: Option<u64>,
    overlay: overlay::Overlay,
    /// Pestaña que se está arrastrando: su id y a qué distancia de su borde se agarró.
    drag_tab: Option<(u64, f32)>,
    /// Esa pestaña está despegada: el puntero se alejó de la barra (`drag::detaching`). Se
    /// queda atenuada en su hueco y, al soltarla, pasa a otra ventana o a una nueva.
    detached: bool,
    /// Ancho y origen de los huecos de las pestañas en el último cuadro (para soltar aquí una
    /// pestaña de otra ventana, `drag::drop_slot`).
    tab_layout: (f32, f32),
    /// Ancho disponible y origen de la barra en el cuadro anterior: si cambian (se
    /// redimensiona la ventana, se abre un panel) las pestañas se recolocan sin animación.
    /// `None` hasta el primer cuadro: las pestañas con que nace la ventana no se animan.
    tab_space: Option<(f32, f32)>,
    /// Dónde se ve cada pestaña camino de su hueco, por id: su borde izquierdo y su ancho.
    tab_springs: HashMap<u64, (anim::Spring, anim::Spring)>,
    /// El botón «+», que va tras la última pestaña.
    plus_spring: anim::Spring,
    /// Comandos que muestra el historial, del más reciente al más antiguo.
    history: Vec<history::Entry>,
    /// Lo que ofrece el desplegable de una caja del prompt (ramas, o carpetas y archivos)
    /// y dónde se ancla en pantalla.
    picks: Vec<overlay::Pick>,
    pick_anchor: Pos2,
    /// El selector de carpetas se abrió desde el panel de agentes: elegir una abre una
    /// pestaña nueva en ella en vez de hacer `cd`.
    pick_new_tab: bool,
    /// Carpeta que lista ese selector (se puede entrar en otras con →); `None` = la de la
    /// pestaña activa.
    pick_base: Option<PathBuf>,
    /// Rama y cambios de la carpeta de la pestaña activa (barra de estado).
    git: git::Watch,
    /// Vista previa (Quick Look) abierta desde el desplegable de carpetas.
    preview: Option<std::process::Child>,
    /// Si macOS puede mover la ventana ahora mismo (se apaga con el puntero sobre las pestañas).
    window_movable: Option<bool>,
    /// Zona de la terminal en el último cuadro (para que las listas usen todo su ancho).
    pub term_rect: Rect,
    /// Posición y tamaño de la ventana (`[x, y, ancho, alto]` en puntos), para guardarla.
    frame: Option<[f32; 4]>,
    /// Título puesto a la ventana (el de su pestaña activa: lo enseña el menú Ventana).
    title: String,
}

/// Lo que va en una ventana nueva.
enum Fill {
    /// Una pestaña en esa carpeta.
    Fresh(Option<PathBuf>),
    /// Las pestañas guardadas de la última vez.
    Saved(state::Window),
    /// Una ventana cerrada hace poco que se reabre.
    Reopen(Vec<state::Tab>, usize),
    /// Una pestaña que llega viva de otra ventana (despegada arrastrándola).
    Move(Box<Session>),
}

/// Algo que se cerró hace poco, para reabrirlo con ⌘⇧T: una pestaña (y de qué ventana y en
/// qué puesto) o una ventana. Solo queda lo guardado: al cerrar, el proceso termina en el
/// acto (como en Terminal, Ghostty o Warp) y al reabrir se abre una pestaña nueva en su
/// carpeta, con la orden para retomar la conversación si corría Claude Code.
enum Closed {
    Tab {
        tab: state::Tab,
        win: WindowId,
        index: usize,
    },
    Window {
        tabs: Vec<state::Tab>,
        active: usize,
        frame: Option<[f32; 4]>,
    },
}

/// Cuántas cosas cerradas se recuerdan.
const CLOSED_MAX: usize = 10;

impl Win {
    fn new(ctx: &egui::Context) -> Self {
        Self {
            ctx: ctx.clone(),
            sessions: Vec::new(),
            active: 0,
            focused: true,
            blur_applied: None,
            search_focus: false,
            agents_search_focus: false,
            agents_pick: None,
            overlay: overlay::Overlay::None,
            drag_tab: None,
            detached: false,
            tab_layout: (0.0, 0.0),
            tab_space: None,
            tab_springs: HashMap::new(),
            plus_spring: anim::Spring::new(0.0, 0.0),
            history: Vec::new(),
            picks: Vec::new(),
            pick_anchor: Pos2::ZERO,
            pick_new_tab: false,
            pick_base: None,
            git: git::Watch::new(),
            preview: None,
            window_movable: None,
            term_rect: Rect::NOTHING,
            frame: None,
            title: String::new(),
        }
    }
}

impl App {
    fn new() -> Self {
        let cfg = config::load();
        notify::init();
        hotkey::set(hotkey::Global::Jump, &cfg.keys.global_jump);
        hotkey::set(hotkey::Global::Focus, &cfg.keys.global_focus);
        let script = debug::Script::from_env();
        // Con el guion no hay hilo de actualizaciones: nunca se conecta (el aviso se simula).
        let updates = updates::Updates::start(env!("CARGO_PKG_VERSION"), script.is_none());
        let mut app = Self {
            pal: Palette::new(&cfg.theme),
            keys: Keys::new(&cfg.keys),
            cfg_mtime: config::mtime(),
            cfg_checked: Instant::now(),
            reduce_motion: macos::reduce_motion(),
            // Hasta que se abra la primera ventana, una de relleno sin pestañas.
            w: Win::new(&egui::Context::default()),
            cur: None,
            parked: HashMap::new(),
            front: None,
            want_windows: Vec::new(),
            opening: Default::default(),
            closed: Vec::new(),
            menu_keys: None,
            order: Vec::new(),
            next_id: 1,
            show_agents: cfg.show_agents,
            agents_filter: Default::default(),
            agents_query: String::new(),
            installed: agents::find_installed(&cfg.shell),
            show_ports: cfg.show_ports,
            show_procs: cfg.show_processes,
            monitor: procs::Monitor::start(),
            proc_search: String::new(),
            proc_sort_mem: false,
            confirm_stop: None,
            stopping: Default::default(),
            stubborn: Default::default(),
            force_hover: false,
            zoom: 0.0,
            ports: ports::Ports::start(),
            show_other_ports: false,
            state_path: state::path(script.is_some()),
            script,
            saved: state::Saved::default(),
            state_checked: Instant::now(),
            font_choices: Vec::new(),
            exe_started: exe_mtime(),
            exe_checked: Instant::now(),
            update_ready: false,
            force_update: false,
            updates,
            update_available: None,
            assume_brew: None,
            script_keys: Vec::new(),
            script_pointer: Vec::new(),
            script_drops: Vec::new(),
            badge: 0,
            secure: false,
            menu_cmds: Vec::new(),
            cfg,
        };
        // Las ventanas de la última vez, cada una donde estaba. Si no hay ninguna, el
        // lanzador abre una vacía.
        if let Some(path) = app.state_path.clone().filter(|_| app.cfg.restore_tabs) {
            let saved = state::load(&path);
            for w in saved.windows() {
                app.want_windows.push(w.frame);
                app.opening.push_back(Fill::Saved(w));
            }
            app.saved = saved;
        }
        app
    }

    fn win(&self, id: WindowId) -> Option<&Win> {
        if self.cur == Some(id) {
            Some(&self.w)
        } else {
            self.parked.get(&id)
        }
    }

    /// Pide al lanzador una ventana nueva con una pestaña en esa carpeta.
    fn new_window(&mut self, cwd: Option<PathBuf>) {
        self.want_windows.push(None);
        self.opening.push_back(Fill::Fresh(cwd));
    }

    fn remember(&mut self, what: Closed) {
        self.closed.push(what);
        if self.closed.len() > CLOSED_MAX {
            self.closed.remove(0);
        }
    }

    /// Pone en la ventana en curso una pestaña nueva con lo guardado de una cerrada.
    fn revive(&mut self, ctx: &egui::Context, tab: &state::Tab, at: Option<usize>) {
        if !self.open_saved_tab(ctx, tab) {
            return;
        }
        // Se abrió junto a la activa; sin puesto guardado se queda ahí.
        let s = self.w.sessions.remove(self.w.active);
        let i = at.unwrap_or(self.w.active).min(self.w.sessions.len());
        self.w.sessions.insert(i, s);
        self.w.active = i;
        ctx.request_repaint();
    }

    /// ⌘⇧T: reabre lo último que se cerró. Una pestaña vuelve a su ventana y a su puesto
    /// si la ventana sigue abierta; si no, a la del frente. Una ventana vuelve donde estaba.
    fn reopen_closed(&mut self) {
        let Some(closed) = self.closed.pop() else {
            return;
        };
        match closed {
            Closed::Tab { tab, win, index } => {
                let back = self.cur;
                let home = self.order.contains(&win);
                if home {
                    self.enter(win);
                }
                let ctx = self.w.ctx.clone();
                self.revive(&ctx, &tab, home.then_some(index));
                ctx.send_viewport_cmd(ViewportCommand::Focus);
                if let Some(back) = back {
                    self.enter(back);
                }
            }
            Closed::Window {
                tabs,
                active,
                frame,
            } => {
                self.want_windows.push(frame);
                self.opening.push_back(Fill::Reopen(tabs, active));
            }
        }
    }

    /// La barra de menús. Las entradas de Colibrí llevan el atajo de la configuración.
    fn menu_spec(&self) -> Vec<menu::Menu> {
        use menu::{Action, Entry, Menu};
        let sc = |cmd: Cmd| {
            self.keys
                .bindings
                .iter()
                .find(|(c, _, _)| *c == cmd)
                .map(|(_, sc, _)| sc.clone())
        };
        let cmd = |title: &str, c: Cmd| Entry::item(title, Action::Cmd(c), sc(c));
        let fixed = |text: &str| Shortcut::parse(text);
        let system = |title: &str, sel: objc2::runtime::Sel, key: &str| {
            Entry::item(title, Action::System(sel), fixed(key))
        };
        // Edición: la tecla vuelve a entrar como si se pulsara (portapapeles, zle…).
        let key = |title: &str, k: egui::Key| {
            Entry::item(
                title,
                Action::Key(
                    k,
                    egui::Modifiers {
                        mac_cmd: true,
                        command: true,
                        ..Default::default()
                    },
                ),
                Some(Shortcut {
                    cmd: true,
                    ctrl: false,
                    shift: false,
                    alt: false,
                    key: k,
                }),
            )
        };
        use objc2::sel;
        let m = &i18n::t().menu;
        vec![
            Menu {
                title: m.app.into(),
                windows: false,
                entries: vec![
                    system(m.about, sel!(orderFrontStandardAboutPanel:), ""),
                    Entry::Separator,
                    cmd(m.prefs, Cmd::Prefs),
                    cmd(m.open_config, Cmd::OpenConfig),
                    Entry::Separator,
                    Entry::Services,
                    Entry::Separator,
                    system(m.hide, sel!(hide:), "cmd+h"),
                    system(m.hide_others, sel!(hideOtherApplications:), "cmd+alt+h"),
                    system(m.show_all, sel!(unhideAllApplications:), ""),
                    Entry::Separator,
                    cmd(m.restart, Cmd::Restart),
                    system(m.quit, sel!(terminate:), "cmd+q"),
                ],
            },
            Menu {
                title: m.file.into(),
                windows: false,
                entries: vec![
                    cmd(m.new_window, Cmd::NewWindow),
                    cmd(m.new_tab, Cmd::NewTab),
                    Entry::Separator,
                    cmd(m.close_tab, Cmd::CloseTab),
                    cmd(m.close_window, Cmd::CloseWindow),
                    cmd(m.reopen_closed, Cmd::ReopenClosed),
                    Entry::Separator,
                    cmd(m.rename_tab, Cmd::RenameTab),
                ],
            },
            Menu {
                title: m.edit.into(),
                windows: false,
                entries: vec![
                    key(m.cut, egui::Key::X),
                    key(m.copy, egui::Key::C),
                    key(m.paste, egui::Key::V),
                    key(m.select_all, egui::Key::A),
                    Entry::Separator,
                    cmd(m.history, Cmd::History),
                    cmd(m.clear, Cmd::Clear),
                ],
            },
            Menu {
                title: m.view.into(),
                windows: false,
                entries: vec![
                    cmd(m.palette, Cmd::Palette),
                    Entry::Separator,
                    cmd(m.toggle_panel, Cmd::TogglePanel),
                    cmd(m.agents_panel, Cmd::ToggleAgents),
                    cmd(m.search_agents, Cmd::SearchAgents),
                    cmd(m.ports_panel, Cmd::TogglePorts),
                    cmd(m.procs_panel, Cmd::ToggleProcs),
                    Entry::Separator,
                    cmd(m.font_bigger, Cmd::FontBigger),
                    cmd(m.font_smaller, Cmd::FontSmaller),
                    cmd(m.font_reset, Cmd::FontReset),
                    Entry::Separator,
                    system(m.full_screen, sel!(toggleFullScreen:), "cmd+ctrl+f"),
                ],
            },
            Menu {
                title: m.window.into(),
                windows: true,
                entries: vec![
                    system(m.minimize, sel!(performMiniaturize:), "cmd+m"),
                    system(m.zoom, sel!(performZoom:), ""),
                    Entry::Separator,
                    cmd(m.next_tab, Cmd::NextTab),
                    cmd(m.prev_tab, Cmd::PrevTab),
                    cmd(m.move_tab_left, Cmd::MoveTabLeft),
                    cmd(m.move_tab_right, Cmd::MoveTabRight),
                    Entry::Separator,
                    cmd(m.next_agent, Cmd::NextAgent),
                    cmd(m.prev_agent, Cmd::PrevAgent),
                    cmd(m.jump_finished, Cmd::JumpFinished),
                    Entry::Separator,
                    system(m.bring_all, sel!(arrangeInFront:), ""),
                ],
            },
        ]
    }

    /// Hace de `win` la ventana en curso: aparta la que lo era y trae esa a `self.w`.
    fn enter(&mut self, win: WindowId) {
        if self.cur == Some(win) {
            return;
        }
        let Some(next) = self.parked.remove(&win) else {
            return;
        };
        let prev = std::mem::replace(&mut self.w, next);
        if let Some(prev_id) = self.cur {
            self.parked.insert(prev_id, prev);
        }
        self.cur = Some(win);
    }

    /// Todas las ventanas, empezando por la que está en curso.
    fn windows(&self) -> impl Iterator<Item = &Win> {
        std::iter::once(&self.w)
            .filter(|_| self.cur.is_some())
            .chain(self.parked.values())
    }

    /// Instala las fuentes en el contexto de cada ventana (cada una tiene su atlas).
    fn install_fonts_everywhere(&self, cfg: &Config) {
        for w in self.windows() {
            install_fonts(&w.ctx, cfg);
        }
    }

    /// Abre en la ventana en curso una pestaña guardada, en su carpeta y con su nombre y su
    /// color. Donde corría Claude Code deja la orden para retomar *su* conversación como
    /// sugerencia en gris (`claude::resume_command`: `claude attach` si sigue viva en
    /// segundo plano, `claude --resume <sesión>` si no, `--continue` si no se supo cuál). No
    /// la escribe en el shell: Enter la ejecuta, Tab o → la deja escrita para editarla.
    /// Devuelve false si la carpeta ya no existe.
    fn open_saved_tab(&mut self, ctx: &egui::Context, tab: &state::Tab) -> bool {
        let dir = std::path::PathBuf::from(&tab.cwd);
        if !dir.is_dir() {
            return false;
        }
        if !self.new_tab(ctx, Some(dir)) {
            return false;
        }
        if let Some(s) = self.w.sessions.get_mut(self.w.active) {
            s.name = tab.name.clone();
            s.color = TAB_COLORS
                .iter()
                .position(|(name, _)| *name == tab.color)
                .map(|k| k as u8);
            if tab.agent == "claude" {
                let live = claude::running();
                s.suggestion = Some(claude::resume_command(
                    &tab.claude_session,
                    &tab.claude_job,
                    &live,
                ));
            }
        }
        true
    }

    /// Llena la ventana en curso con las pestañas guardadas de una ventana.
    fn open_saved_window(&mut self, ctx: &egui::Context, saved: &state::Window) {
        for tab in &saved.tabs {
            self.open_saved_tab(ctx, tab);
        }
        if !self.w.sessions.is_empty() {
            self.w.active = saved.active.min(self.w.sessions.len() - 1);
        }
    }

    /// Guarda las ventanas y sus pestañas cuando cambian (como mucho cada 2 s, y siempre al
    /// quedarse una ventana sin pestañas).
    fn save_tabs(&mut self) {
        let Some(path) = self.state_path.clone().filter(|_| self.cfg.restore_tabs) else {
            return;
        };
        if !self.w.sessions.is_empty() && self.state_checked.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.state_checked = Instant::now();
        // En el orden en que se abrieron, que es como se vuelven a abrir.
        let windows = self
            .order
            .iter()
            .filter_map(|win| self.win(*win))
            .filter(|w| !w.sessions.is_empty())
            .map(|w| state::Window {
                active: w.active,
                tabs: w.sessions.iter().map(tab_record).collect(),
                frame: w.frame,
            })
            .collect();
        let now = state::Saved {
            windows,
            ..Default::default()
        };
        if now != self.saved {
            state::save(&path, &now);
            self.saved = now;
        }
    }

    /// Selector de carpetas en modo «pestaña nueva» (botón de carpeta del panel de
    /// agentes): solo `..` y las carpetas, empezando por la de la pestaña activa.
    fn open_dir_picker(&mut self, ctx: &egui::Context, at: Pos2) {
        self.open_picker(ctx, session::CHIP_DIR, at);
        if self.w.overlay != overlay::Overlay::None {
            self.w.picks.retain(|p| p.kind != overlay::PickKind::File);
            self.w.pick_new_tab = true;
        }
    }

    /// La carpeta que lista el selector de carpetas.
    fn pick_dir(&self) -> Option<PathBuf> {
        self.w
            .pick_base
            .clone()
            .or_else(|| self.w.sessions.get(self.w.active).map(|s| s.cwd.clone()))
    }

    /// Abre el desplegable de una caja del prompt, anclado en `at`: las ramas del repo
    /// (caja de la rama) o lo que hay en la carpeta (caja de la carpeta).
    fn open_picker(&mut self, ctx: &egui::Context, chip: char, at: Pos2) {
        use overlay::{Pick, PickKind};
        let Some(cwd) = self.w.sessions.get(self.w.active).map(|s| s.cwd.clone()) else {
            return;
        };
        self.w.pick_new_tab = false;
        self.w.pick_base = None;
        let mut picks: Vec<Pick> = Vec::new();
        if chip == session::CHIP_BRANCH {
            let current = git::branch(&cwd);
            for name in git::branches(&cwd) {
                let kind = if Some(&name) == current.as_ref() {
                    PickKind::CurrentBranch
                } else {
                    PickKind::Branch
                };
                picks.push(Pick { kind, name });
            }
            // La rama en la que estás va primero.
            picks.sort_by_key(|p| p.kind != PickKind::CurrentBranch);
        } else {
            picks = overlay::dir_picks(&cwd, true);
        }
        if picks.is_empty() {
            return;
        }
        self.w.picks = picks;
        self.w.pick_anchor = at;
        self.w.overlay = overlay::Overlay::Picker {
            query: String::new(),
            sel: 0,
        };
        ctx.request_repaint();
    }

    /// Lee el historial del shell y le suma lo tecleado aquí (primero la pestaña activa).
    fn load_history(&mut self) {
        let shell = if self.cfg.shell.is_empty() {
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into())
        } else {
            self.cfg.shell.clone()
        };
        let mut typed: Vec<String> = Vec::new();
        for (i, s) in self.w.sessions.iter().enumerate() {
            if i != self.w.active {
                typed.extend(s.typed.iter().cloned());
            }
        }
        if let Some(s) = self.w.sessions.get(self.w.active) {
            typed.extend(s.typed.iter().cloned());
        }
        self.w.history = history::load(&shell, &typed);
    }

    /// Abre el historial sobre el prompt, filtrado por lo que ya hay escrito (como prefijo),
    /// y deja a la vista en el prompt el comando más reciente. False si no hay nada que
    /// ofrecer o no se está en el prompt.
    fn open_history(&mut self, ctx: &egui::Context) -> bool {
        let Some(original) = self
            .w
            .sessions
            .get(self.w.active)
            .filter(|s| s.at_prompt())
            .map(|s| s.prompt_line().unwrap_or("").to_string())
        else {
            return false;
        };
        self.load_history();
        let prefix = original.to_lowercase();
        self.w
            .history
            .retain(|e| e.cmd != original && e.cmd.to_lowercase().starts_with(&prefix));
        let (Some(first), Some(s)) = (
            self.w.history.first(),
            self.w.sessions.get_mut(self.w.active),
        ) else {
            return false;
        };
        s.insert_command(&first.cmd);
        self.w.overlay = overlay::Overlay::History { sel: 0, original };
        ctx.request_repaint();
        true
    }

    /// ↑ en el prompt abre el historial; en cualquier otro caso la tecla sigue su camino.
    fn history_key(&mut self, ctx: &egui::Context) {
        if !self.cfg.history_popup
            || !self
                .w
                .sessions
                .get(self.w.active)
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
        if !self.open_history(ctx) {
            // Sin historial que mostrar, el shell recibe su flecha de siempre.
            if let Some(s) = self.w.sessions.get_mut(self.w.active) {
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
        }
    }

    /// Tab en el prompt: sugiere carpetas y archivos para la palabra que se teclea. Si no se
    /// sabe qué hay escrito, o nada coincide, el Tab le llega al shell como siempre.
    /// Con una sugerencia en gris en el prompt: Enter la ejecuta, Tab o → la dejan escrita.
    fn suggest_key(&mut self, ctx: &egui::Context) {
        let Some(s) = self.w.sessions.get_mut(self.w.active) else {
            return;
        };
        if s.suggested().is_none() {
            return;
        }
        let key = |key| Shortcut {
            cmd: false,
            ctrl: false,
            shift: false,
            alt: false,
            key,
        };
        if take_shortcut(ctx, &key(egui::Key::Enter)) {
            s.accept_suggestion(true);
        } else if take_shortcut(ctx, &key(egui::Key::Tab))
            || take_shortcut(ctx, &key(egui::Key::ArrowRight))
        {
            s.accept_suggestion(false);
        }
    }

    fn complete_key(&mut self, ctx: &egui::Context) {
        if !self.cfg.complete_popup {
            return;
        }
        let Some(s) = self
            .w
            .sessions
            .get_mut(self.w.active)
            .filter(|s| s.at_prompt())
        else {
            return;
        };
        let tab_down = ctx.input(|i| {
            i.events.iter().any(|e| {
                matches!(e, egui::Event::Key { key: egui::Key::Tab, pressed: true, modifiers, .. }
                    if modifiers.is_none())
            })
        });
        if tab_down {
            s.resync_line();
        }
        let Some(line) = s
            .prompt_line()
            .filter(|l| complete::target(l).is_some())
            .map(str::to_string)
        else {
            return;
        };
        let tab = Shortcut {
            cmd: false,
            ctrl: false,
            shift: false,
            alt: false,
            key: egui::Key::Tab,
        };
        if !take_shortcut(ctx, &tab) {
            return;
        }
        // Las ramas se leen de git aquí, una vez; con la lista abierta se reutilizan.
        let mut git = complete::GitData::default();
        let aliases = complete::shell_aliases();
        let offer = complete::offer(&line, &s.cwd, &|n| aliases.get(n).cloned(), &mut git);
        let Some(s) = self.w.sessions.get_mut(self.w.active) else {
            return;
        };
        let Some((target, items)) = offer else {
            // Ahí no van ni ramas ni rutas (un nombre de rama nueva, un remoto): para el shell.
            s.input(b"\t".to_vec());
            return;
        };
        match items.as_slice() {
            [] => s.input(b"\t".to_vec()),
            [only] => s.replace_word(
                target.token.chars().count(),
                &complete::replacement(&target, only),
            ),
            _ => {
                // Anclado donde empieza la palabra, para que no se mueva al elegir.
                let x = s.cursor_px.map_or(0.0, |c| {
                    c.left() - target.token.chars().count() as f32 * c.width()
                });
                self.w.overlay = overlay::Overlay::Complete {
                    line,
                    target,
                    items,
                    sel: None,
                    x,
                    git,
                    detail: None,
                };
                ctx.request_repaint();
            }
        }
    }

    /// ¿El aviso de versión nueva instala con Homebrew? Si esta copia la instaló él.
    fn upgrades_with_brew(&self) -> bool {
        self.assume_brew
            .unwrap_or_else(updates::installed_with_brew)
    }

    /// Clic en el aviso de versión nueva: con Homebrew, una pestaña nueva que ejecuta
    /// `brew upgrade --cask colibri` en cuanto la shell está lista (se ve todo lo que hace);
    /// al reemplazar el ejecutable aparece «Actualizar». Sin Homebrew, la página de la versión.
    fn start_upgrade(&mut self, ctx: &egui::Context) {
        let Some(version) = self.update_available.clone() else {
            return;
        };
        if self.upgrades_with_brew() {
            // Si la pestaña no se pudo abrir, la activa es otra que ya estaba: nada de escribir
            // la orden ahí.
            if self.new_tab(ctx, dirs::home_dir())
                && let Some(s) = self.w.sessions.get_mut(self.w.active)
            {
                s.run_when_ready(updates::BREW_UPGRADE.to_string());
            }
        } else {
            let url = updates::release_url(&version);
            if self.script.is_some() {
                eprintln!("enlace: {url}");
            } else {
                spawn_detached("/usr/bin/open", &[url]);
            }
        }
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

    /// Abre una pestaña a la derecha de la activa (al principio si no hay ninguna) y la deja
    /// activa. Devuelve false si no se pudo abrir la terminal.
    fn new_tab(&mut self, ctx: &egui::Context, cwd: Option<std::path::PathBuf>) -> bool {
        match Session::new(self.next_id, &self.cfg, cwd, ctx) {
            Ok(s) => {
                self.next_id += 1;
                let i = if self.w.sessions.is_empty() {
                    0
                } else {
                    (self.w.active + 1).min(self.w.sessions.len())
                };
                self.w.sessions.insert(i, s);
                self.w.active = i;
                true
            }
            Err(e) => {
                eprintln!("colibri: no se pudo abrir la terminal: {e}");
                false
            }
        }
    }

    fn close_tab(&mut self, i: usize) {
        if i >= self.w.sessions.len() {
            return;
        }
        // Se apunta para ⌘⇧T y se suelta: el proceso termina ya (`Pty::drop`).
        let tab = tab_record(&self.w.sessions.remove(i));
        if let Some(win) = self.cur {
            self.remember(Closed::Tab { tab, win, index: i });
        }
        if self.w.active > i || self.w.active >= self.w.sessions.len() {
            self.w.active = self.w.active.saturating_sub(1);
        }
    }

    /// Cambia una pestaña de posición; la activa sigue siendo la misma.
    fn move_tab(&mut self, from: usize, to: usize) {
        let n = self.w.sessions.len();
        if from >= n || to >= n || from == to {
            return;
        }
        let active_id = self.w.sessions.get(self.w.active).map(|s| s.id);
        let tab = self.w.sessions.remove(from);
        self.w.sessions.insert(to, tab);
        if let Some(i) = active_id.and_then(|id| self.w.sessions.iter().position(|s| s.id == id)) {
            self.w.active = i;
        }
    }

    /// Saca la pestaña i de la ventana en curso sin soltarla (su proceso sigue vivo) para
    /// llevarla a otra: cierra el overlay, que puede apuntar a ella, y vacía su caché de
    /// rejilla, que es de la caché de glifos de esta ventana. No se apunta para ⌘⇧T.
    fn take_tab(&mut self, i: usize) -> Option<Session> {
        if i >= self.w.sessions.len() {
            return None;
        }
        if !matches!(self.w.overlay, overlay::Overlay::None) {
            self.cancel_overlay();
        }
        let mut s = self.w.sessions.remove(i);
        if self.w.active > i || self.w.active >= self.w.sessions.len() {
            self.w.active = self.w.active.saturating_sub(1);
        }
        s.grid = view::GridCache::default();
        Some(s)
    }

    /// Pasa la pestaña i, viva, a otra ventana: al hueco `slot` (o al final), activa allí y
    /// con esa ventana al frente. Si la de origen se queda vacía la cierra `logic()`, y
    /// `window_closed` no la apunta para ⌘⇧T porque no le quedan pestañas.
    fn move_to_window(&mut self, i: usize, win: WindowId, slot: Option<usize>) {
        if self.cur == Some(win) || !self.parked.contains_key(&win) {
            return;
        }
        let Some(mut s) = self.take_tab(i) else {
            return;
        };
        // Un overlay abierto allí (historial, renombrar, desplegable) actúa sobre la pestaña
        // activa: se cierra antes, mientras esa sigue siendo la que era, para que no acabe
        // escribiendo en la que llega.
        if self
            .parked
            .get(&win)
            .is_some_and(|w| !matches!(w.overlay, overlay::Overlay::None))
        {
            let back = self.cur;
            self.enter(win);
            self.cancel_overlay();
            if let Some(back) = back {
                self.enter(back);
            }
        }
        let dest = self.parked.get_mut(&win).expect("comprobada arriba");
        s.set_context(&dest.ctx);
        s.unseen = false;
        let at = slot.unwrap_or(usize::MAX).min(dest.sessions.len());
        dest.sessions.insert(at, s);
        dest.active = at;
        // `send_viewport_cmd` pide además un cuadro a esa ventana.
        dest.ctx.send_viewport_cmd(ViewportCommand::Focus);
        self.front = Some(win);
        self.w.ctx.request_repaint();
    }

    /// Lleva la pestaña i, viva, a una ventana nueva con la esquina en `at` (puntos de
    /// pantalla) y el tamaño de esta. Con una sola pestaña no hace nada: saldría una ventana
    /// igual a la que ya hay.
    fn detach_to_new_window(&mut self, i: usize, at: Pos2) {
        if self.w.sessions.len() <= 1 {
            return;
        }
        let Some(s) = self.take_tab(i) else {
            return;
        };
        let frame = self.w.frame.map(|[_, _, w, h]| [at.x, at.y, w, h]);
        self.want_windows.push(frame);
        self.opening.push_back(Fill::Move(Box::new(s)));
        self.w.ctx.request_repaint();
    }

    /// Se soltó despegada la pestaña `id` (agarrada a `grab` de su borde izquierdo): a la
    /// ventana de Colibrí que macOS ve bajo el puntero (al hueco de su barra si cae en ella,
    /// si no al final) o, si no hay ninguna, a una ventana nueva con la pestaña bajo el
    /// puntero. Sobre la propia ventana no pasa nada. Con el guion, el punto y la ventana
    /// salen de su puntero simulado (`pointer`, en puntos de esta ventana) y de los marcos de
    /// las ventanas, no del ratón de verdad.
    fn drop_detached(&mut self, ctx: &egui::Context, id: u64, grab: f32, pointer: Option<Pos2>) {
        let Some(i) = self.w.sessions.iter().position(|s| s.id == id) else {
            return;
        };
        let (p, target) = if self.script.is_some() {
            let (Some([fx, fy, ..]), Some(local)) = (self.w.frame, pointer) else {
                return;
            };
            let p = Pos2::new(fx + local.x, fy + local.y);
            let frames: Vec<_> = self
                .order
                .iter()
                .rev()
                .filter_map(|&w| Some((w, self.win(w)?.frame?)))
                .collect();
            (p, drag::frame_at(&frames, p))
        } else {
            let Some(p) = macos::pointer_on_screen() else {
                return;
            };
            (
                p,
                macos::window_number_at(p).and_then(shell::window_with_number),
            )
        };
        match target {
            Some(win) if Some(win) == self.cur => {}
            Some(win) => {
                let slot = self.parked.get(&win).and_then(|w| {
                    let [fx, fy, ..] = w.frame?;
                    let (tab_w, tabs_left) = w.tab_layout;
                    let y = p.y - fy;
                    (0.0..=TAB_BAR_H)
                        .contains(&y)
                        .then(|| drag::drop_slot(p.x - fx, tabs_left, tab_w, w.sessions.len()))
                });
                self.move_to_window(i, win, slot);
                // Si era su última pestaña, esta pasada ya pintó media ventana vacía: se
                // descarta (egui la repite sin pestañas y no pinta nada) y se cierra ya.
                if self.w.sessions.is_empty() {
                    ctx.request_discard("se movió la última pestaña de la ventana");
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            }
            None => {
                let size = self
                    .w
                    .frame
                    .map_or(Vec2::ZERO, |[_, _, w, h]| Vec2::new(w, h));
                // En la ventana nueva será la única pestaña, al principio de la barra.
                let grab = Vec2::new(self.w.tab_layout.1 + grab, TAB_BAR_H / 2.0);
                let corner = drag::new_window_corner(p, grab, size, macos::visible_frame_at(p));
                self.detach_to_new_window(i, corner);
            }
        }
    }

    fn select(&mut self, i: usize) {
        if i < self.w.sessions.len() {
            self.w.active = i;
            self.w.sessions[i].unseen = false;
        }
    }

    /// Recorre solo las pestañas con agente, en el orden del panel «Agentes», dando la vuelta.
    fn step_agent(&mut self, forward: bool) {
        let n = self.w.sessions.len();
        let target = (1..=n)
            .map(|d| {
                if forward {
                    (self.w.active + d) % n
                } else {
                    (self.w.active + n - d) % n
                }
            })
            .find(|&i| self.w.sessions[i].agent.is_some());
        if let Some(i) = target {
            self.select(i);
        }
    }

    /// Va al agente (o comando largo) que terminó más recientemente; primero los no vistos.
    fn jump_finished(&mut self) {
        let here = self.w.sessions.get(self.w.active).map(|s| s.id);
        let best = |unseen_only: bool| {
            self.windows()
                .flat_map(|w| w.sessions.iter())
                .filter(|s| {
                    Some(s.id) != here
                        && s.finished_at.is_some()
                        && !s.busy
                        && (!unseen_only || s.unseen)
                })
                .max_by_key(|s| s.finished_at)
                .map(|s| s.id)
        };
        // En cualquier ventana: si está en otra, esa pasa al frente.
        if let Some(id) = best(true).or_else(|| best(false)) {
            self.focus_session(id);
        }
    }

    /// Trae al frente la ventana de esa pestaña y la elige (clic en una notificación).
    fn focus_session(&mut self, id: u64) {
        let win = self
            .parked
            .iter()
            .find(|(_, w)| w.sessions.iter().any(|s| s.id == id))
            .map(|(win, _)| *win)
            .or(self.cur);
        // Se vuelve a la ventana en curso al acabar: esto corre dentro de su pasada.
        let back = self.cur;
        if let Some(win) = win {
            self.enter(win);
        }
        if let Some(i) = self.w.sessions.iter().position(|s| s.id == id) {
            self.select(i);
        }
        // `send_viewport_cmd` pide además un cuadro a esa ventana.
        self.w.ctx.send_viewport_cmd(ViewportCommand::Focus);
        if let Some(back) = back {
            self.enter(back);
        }
    }

    fn reload_config(&mut self) {
        let new = config::load();
        if new.font_path != self.cfg.font_path || new.font_bold_path != self.cfg.font_bold_path {
            self.install_fonts_everywhere(&new);
        }
        self.pal = Palette::new(&new.theme);
        self.keys = Keys::new(&new.keys);
        if new.keys.global_jump != self.cfg.keys.global_jump {
            hotkey::set(hotkey::Global::Jump, &new.keys.global_jump);
        }
        if new.keys.global_focus != self.cfg.keys.global_focus {
            hotkey::set(hotkey::Global::Focus, &new.keys.global_focus);
        }
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
                let n = self.w.sessions.len();
                self.run(
                    ctx,
                    Cmd::GoTab(if i == 8 { n.saturating_sub(1) } else { i }),
                );
            }
        }
    }

    pub fn run(&mut self, ctx: &egui::Context, cmd: Cmd) {
        let n = self.w.sessions.len();
        match cmd {
            Cmd::NewTab => {
                let cwd = self.w.sessions.get(self.w.active).map(|s| s.cwd.clone());
                self.new_tab(ctx, cwd);
            }
            Cmd::CloseTab => self.close_tab(self.w.active),
            Cmd::NewWindow => {
                let cwd = self.w.sessions.get(self.w.active).map(|s| s.cwd.clone());
                self.new_window(cwd);
                // Para que el lanzador la abra ya (lo mira tras cada evento).
                ctx.request_repaint();
            }
            Cmd::CloseWindow => ctx.send_viewport_cmd(ViewportCommand::Close),
            Cmd::ReopenClosed => {
                self.reopen_closed();
                ctx.request_repaint();
            }
            Cmd::NextTab if n > 0 => self.select((self.w.active + 1) % n),
            Cmd::PrevTab if n > 0 => self.select((self.w.active + n - 1) % n),
            Cmd::NextTab | Cmd::PrevTab => {}
            Cmd::MoveTabLeft if self.w.active > 0 => {
                self.move_tab(self.w.active, self.w.active - 1)
            }
            Cmd::MoveTabRight => self.move_tab(self.w.active, self.w.active + 1),
            Cmd::MoveTabLeft => {}
            Cmd::GoTab(i) => self.select(i),
            Cmd::NextAgent => self.step_agent(true),
            Cmd::PrevAgent => self.step_agent(false),
            Cmd::JumpFinished => self.jump_finished(),
            Cmd::TogglePanel => {
                let open = !(self.show_agents || self.show_ports || self.show_procs);
                self.show_agents = open;
                self.show_ports = open;
                self.show_procs = false;
            }
            Cmd::ToggleProcs => self.show_procs = !self.show_procs,
            Cmd::ToggleAgents => self.show_agents = !self.show_agents,
            Cmd::SearchAgents => {
                self.show_agents = true;
                self.w.agents_pick = None;
                let id = agents::search_id();
                ctx.memory_mut(|m| m.request_focus(id));
                // Lo que ya hubiera escrito queda seleccionado: lo que se teclee lo sustituye.
                let mut edit = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
                let all = egui::text::CCursorRange::select_all_str(&self.agents_query);
                edit.cursor.set_char_range(Some(all));
                edit.store(ctx, id);
            }
            Cmd::TogglePorts => self.show_ports = !self.show_ports,
            Cmd::FontBigger => self.zoom = (self.zoom + 1.0).min(24.0),
            Cmd::FontSmaller => self.zoom = (self.zoom - 1.0).max(6.0 - self.cfg.font_size),
            Cmd::FontReset => self.zoom = 0.0,
            Cmd::Palette => {
                self.w.overlay = match self.w.overlay {
                    overlay::Overlay::Palette { .. } => overlay::Overlay::None,
                    _ => overlay::Overlay::Palette {
                        query: String::new(),
                        sel: 0,
                    },
                };
            }
            Cmd::Prefs => {
                self.w.overlay = match self.w.overlay {
                    overlay::Overlay::Prefs { .. } => overlay::Overlay::None,
                    _ => {
                        self.font_choices = config::fonts();
                        overlay::Overlay::Prefs { sel: 0 }
                    }
                };
            }
            Cmd::RenameTab => {
                if let Some(s) = self.w.sessions.get(self.w.active) {
                    self.w.overlay = overlay::Overlay::Rename {
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
                if matches!(self.w.overlay, overlay::Overlay::History { .. }) {
                    self.cancel_overlay();
                } else {
                    self.open_history(ctx);
                }
            }
            Cmd::Clear => {
                if let Some(s) = self.w.sessions.get_mut(self.w.active) {
                    s.term.lock().grid_mut().clear_history();
                    s.input(vec![0x0c]);
                }
            }
        }
    }

    /// Aplica y guarda un cambio hecho desde Preferencias.
    pub fn commit_cfg(&mut self, old: &Config) {
        if old.font_path != self.cfg.font_path || old.font_bold_path != self.cfg.font_bold_path {
            self.install_fonts_everywhere(&self.cfg);
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
        // Por defecto el guion repinta sin parar, lo que tapa los fallos de «falta pedir un
        // cuadro». Con `COLIBRI_LAZY` solo despierta cuando toca el siguiente paso.
        if std::env::var_os("COLIBRI_LAZY").is_some() {
            ctx.request_repaint_after(script.until_next());
        } else {
            ctx.request_repaint_after(Duration::from_millis(30));
        }
        let step = script.next();
        if script.done() && step.is_none() {
            self.script = None;
            return;
        }
        match step {
            Some(debug::Step::Type(text)) => {
                if let Some(s) = self.w.sessions.get_mut(self.w.active) {
                    s.input(text.into_bytes());
                }
            }
            Some(debug::Step::Tab) => {
                let cwd = self.w.sessions.get(self.w.active).map(|s| s.cwd.clone());
                self.new_tab(ctx, cwd);
            }
            Some(debug::Step::Select(i)) => self.select(i),
            Some(debug::Step::Shot(path)) => {
                ctx.request_screenshot(move |image| debug::save_bmp(&path, &image))
            }
            Some(debug::Step::Procs) => self.show_procs = !self.show_procs,
            Some(debug::Step::Agents(what)) => {
                let (kind, arg) = what.split_once(' ').unwrap_or((what.as_str(), ""));
                match kind {
                    "filter" => {
                        self.agents_filter = match arg {
                            "waiting" => agents::Filter::Waiting,
                            "working" => agents::Filter::Working,
                            _ => agents::Filter::All,
                        }
                    }
                    "search" => self.agents_query = arg.to_string(),
                    // Da el teclado al buscador, como un clic en él (el clic simulado falla
                    // si el puntero de verdad se mueve sobre la ventana).
                    "focus" => ctx.memory_mut(|m| m.request_focus(agents::search_id())),
                    // Lo mismo que el botón de carpeta del panel.
                    "folder" => self.open_dir_picker(ctx, Pos2::new(220.0, 76.0)),
                    _ => eprintln!("guion: agents {what}?"),
                }
            }
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
                        "branches" | "dirs" => {
                            let at = self
                                .w
                                .sessions
                                .get(self.w.active)
                                .and_then(|s| s.chips_px.last().map(|(r, _)| r.left_bottom()))
                                .unwrap_or(Pos2::new(40.0, 80.0));
                            let kind = if what == "dirs" {
                                session::CHIP_DIR
                            } else {
                                session::CHIP_BRANCH
                            };
                            self.open_picker(ctx, kind, at);
                            return;
                        }
                        _ => Cmd::Palette,
                    },
                );
            }
            Some(debug::Step::Pref(id, dir)) => self.pref_change(ctx, &id, dir),
            Some(debug::Step::Preedit(text)) => {
                self.script_pointer
                    .push(egui::Event::Ime(egui::ImeEvent::Preedit {
                        text,
                        active_range_chars: None,
                    }));
            }
            Some(debug::Step::Name(name)) => {
                if let Some(s) = self.w.sessions.get_mut(self.w.active) {
                    s.name = name;
                }
            }
            Some(debug::Step::Move(from, to)) => self.move_tab(from, to),
            Some(debug::Step::Agent(forward)) => self.step_agent(forward),
            Some(debug::Step::Key(name)) => {
                // «shift+left», «cmd+a»…: modificadores con «+» delante de la tecla.
                let mut parts: Vec<&str> = name.split('+').collect();
                let key = parts.pop().unwrap_or_default();
                let mods = egui::Modifiers {
                    shift: parts.contains(&"shift"),
                    alt: parts.contains(&"alt"),
                    ctrl: parts.contains(&"ctrl"),
                    mac_cmd: parts.contains(&"cmd"),
                    command: parts.contains(&"cmd"),
                };
                let upper = key.to_uppercase();
                let key = match key {
                    "up" => "ArrowUp",
                    "down" => "ArrowDown",
                    "left" => "ArrowLeft",
                    "right" => "ArrowRight",
                    "enter" => "Enter",
                    "esc" => "Escape",
                    "tab" => "Tab",
                    "space" => "Space",
                    "paste" => "Paste",
                    "backspace" => "Backspace",
                    k if k.len() == 1 => upper.as_str(),
                    other => other,
                };
                self.script_keys
                    .extend(egui::Key::from_name(key).map(|k| (k, mods)));
            }
            Some(debug::Step::Pointer(kind, x, y)) => {
                let pos = Pos2::new(x, y);
                self.script_pointer.push(egui::Event::PointerMoved(pos));
                if kind == "rclick" {
                    for pressed in [true, false] {
                        self.script_pointer.push(egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Secondary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        });
                    }
                } else if kind != "moveto" {
                    self.script_pointer.push(egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: kind == "press",
                        modifiers: egui::Modifiers::NONE,
                    });
                }
            }
            Some(debug::Step::Cmd(down)) => {
                self.script_pointer
                    .push(egui::Event::ModifiersChanged(egui::Modifiers {
                        mac_cmd: down,
                        command: down,
                        ..Default::default()
                    }));
            }
            Some(debug::Step::Drop(path)) => self.script_drops.push(PathBuf::from(path)),
            Some(debug::Step::Minimize) => ctx.send_viewport_cmd(ViewportCommand::Minimized(true)),
            Some(debug::Step::Scroll(lines)) => {
                if let Some(s) = self.w.sessions.get(self.w.active) {
                    s.term
                        .lock()
                        .scroll_display(alacritty_terminal::grid::Scroll::Delta(lines));
                }
            }
            Some(debug::Step::Update) => self.force_update = !self.force_update,
            Some(debug::Step::UpdateAvailable(version, brew)) => {
                self.updates.set_available(version);
                self.assume_brew = Some(brew);
            }
            Some(debug::Step::Menu(title)) => {
                if !menu::perform(&title) {
                    eprintln!("menú: no hay ninguna entrada «{title}»");
                }
            }
            Some(debug::Step::Detach(tab, to)) => {
                if to == "new" {
                    let at = self
                        .w
                        .frame
                        .map(|[x, y, ..]| Pos2::new(x + 40.0, y + 40.0))
                        .unwrap_or(Pos2::new(80.0, 80.0));
                    self.detach_to_new_window(tab, at);
                } else if let Some(&win) = to.parse::<usize>().ok().and_then(|m| self.order.get(m))
                {
                    self.move_to_window(tab, win, None);
                }
            }
            Some(debug::Step::Window(what)) => match what.as_str() {
                "new" => {
                    let cwd = self.w.sessions.get(self.w.active).map(|s| s.cwd.clone());
                    self.new_window(cwd);
                }
                "close" => ctx.send_viewport_cmd(ViewportCommand::Close),
                n => {
                    if let Some(&win) = n.parse::<usize>().ok().and_then(|n| self.order.get(n)) {
                        // El guion pasa a esa ventana aunque macOS no le dé el foco.
                        self.front = Some(win);
                        let w = if self.cur == Some(win) {
                            &self.w
                        } else {
                            &self.parked[&win]
                        };
                        w.ctx.send_viewport_cmd(ViewportCommand::Focus);
                    }
                }
            },
            Some(debug::Step::Quit) => {
                for w in self.windows() {
                    w.ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            }
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
            blink: self.cfg.cursor_blink,
            editor: self.cfg.editor.clone(),
            link_click: self.cfg.link_click,
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
        let n = self.w.sessions.len().max(1);
        let show_update = self.update_ready || self.force_update || self.update_available.is_some();
        // Con «Actualizar», un botón más de 30 pt a 6 pt de Procesos.
        let right_reserved = if show_update { 158.0 } else { 122.0 };
        // Puertos/Procesos a la izquierda (agentes a la derecha) van bajo la barra: las
        // pestañas empiezan donde acaba ese panel (el del cuadro anterior), o la primera,
        // siendo la activa, se «uniría» a él en vez de a la terminal.
        let term = self.w.term_rect;
        let side_left = if self.cfg.agents_on_right()
            && (self.show_ports || self.show_procs)
            && term.is_positive()
        {
            term.left() - full.left()
        } else {
            0.0
        };
        let left_reserved =
            tabs_left_reserved(self.show_agents, self.cfg.agents_on_right(), side_left);
        let avail = tabs_avail(full, right_reserved, self.w.term_rect, left_reserved);
        let tab_w = (avail / n as f32).clamp(64.0, 230.0);
        let ui_font = FontId::proportional(12.5);
        let opacity = self.cfg.opacity.clamp(0.3, 1.0);
        // El fondo de la barra se pinta al final (depende de dónde se ve la activa), pero en
        // este hueco, debajo de todo lo demás.
        let bar_bg = ui.painter().add(egui::Shape::Noop);
        let mut active_body = None;

        let mut select = None;
        let mut close = None;
        let mut rename = None;
        let mut moved = None;
        let mut grabbed = None;
        let mut recolor = None;
        let tabs_left = full.left() + left_reserved;
        let pointer = ui.input(|i| i.pointer.latest_pos());
        let mut dropped = None;
        // Red por si no llega el soltar (soltar fuera de la ventana sin mover más el ratón):
        // se pregunta al sistema y, mientras se arrastra, se mira cada 50 ms aunque no haya
        // eventos. El guion no: su puntero simulado no pulsa el ratón de verdad.
        let mut released = !ui.input(|i| i.pointer.primary_down());
        if self.w.drag_tab.is_some() && self.script.is_none() {
            released |= !macos::primary_button_down();
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        if released {
            if self.w.detached {
                dropped = self.w.drag_tab;
            }
            self.w.drag_tab = None;
        }
        self.w.detached = self.w.drag_tab.is_some()
            && drag::detaching(pointer, full, ctx.content_rect(), self.w.detached);
        // Despegada, la pestaña no sigue al puntero: vuelve a su hueco y no reordena.
        let pointer_x = pointer.map(|p| p.x).filter(|_| !self.w.detached);
        self.w.tab_layout = (tab_w, tabs_left);
        let last = tabs_left + (self.w.sessions.len().max(1) - 1) as f32 * tab_w;
        let now = ctx.input(|i| i.time);
        // Ventana redimensionada o un panel abierto o cerrado: las pestañas saltan a su
        // sitio (animadas irían por detrás del borde de la ventana). Abrir, cerrar y mover
        // pestañas sí se anima. Las del primer cuadro de la ventana tampoco: ya estaban.
        let space = (avail, tabs_left);
        let snap = self.reduce_motion || self.w.tab_space != Some(space);
        self.w.tab_space = Some(space);
        // La que sigue al puntero y por dónde se agarró.
        let held = match (self.w.drag_tab, pointer_x) {
            (Some((id, grab)), Some(px)) => Some((id, (px - grab).clamp(tabs_left, last))),
            _ => None,
        };
        // Dónde se ve cada pestaña: su hueco, o camino de él (`anim::Spring` del borde
        // izquierdo y del ancho), o bajo el puntero. Se calcula una vez por cuadro: antes la
        // activa se calculaba dos veces con sumas distintas y, si diferían en el último
        // decimal, su animación volvía a empezar en cada cuadro y la ventana repintaba sin
        // parar (medido con 5 pestañas en reposo: ~60 pasadas/s en vez de ninguna).
        let w = &mut self.w;
        w.tab_springs
            .retain(|id, _| w.sessions.iter().any(|s| s.id == *id));
        let mut moving = false;
        let mut rects = Vec::with_capacity(w.sessions.len());
        for (i, s) in w.sessions.iter().enumerate() {
            let slot = tabs_left + i as f32 * tab_w;
            // Una pestaña nueva nace sin ancho en su hueco y crece; las demás le hacen sitio.
            let (x, width) = w.tab_springs.entry(s.id).or_insert_with(|| {
                let born = if snap { tab_w } else { 0.0 };
                (anim::Spring::new(slot, now), anim::Spring::new(born, now))
            });
            let left = match held {
                // Al soltarla se desliza desde donde quedó hasta su hueco.
                Some((id, left)) if id == s.id => {
                    x.snap(left, now);
                    left
                }
                _ if snap => {
                    x.snap(slot, now);
                    slot
                }
                _ => x.update(slot, now),
            };
            let shown_w = if snap {
                width.snap(tab_w, now);
                tab_w
            } else {
                width.update(tab_w, now)
            };
            moving |= x.moving() || width.moving();
            // Rectas, de toda la altura de la barra y pegadas unas a otras.
            rects.push(Rect::from_min_size(
                Pos2::new(left, full.top()),
                Vec2::new(shown_w, full.height()),
            ));
        }
        // El «+» va tras la última pestaña y la acompaña al abrir o cerrar.
        let end = tabs_left + w.sessions.len() as f32 * tab_w;
        let x = if snap {
            w.plus_spring.snap(end, now);
            end
        } else {
            w.plus_spring.update(end, now)
        };
        moving |= w.plus_spring.moving();
        if moving {
            ctx.request_repaint();
        }
        if moving || w.drag_tab.is_some() {
            shell::animating(&ctx);
        }
        // Dónde se ve la activa (deslizándose o arrastrada): las demás se recortan por ahí. Con
        // la ventana translúcida su fondo no es opaco y se verían a través de ella.
        let active_cover = rects.get(self.w.active).copied();
        for (i, s) in self.w.sessions.iter().enumerate() {
            let rect = rects[i];
            let in_slot = rect.left() == tabs_left + i as f32 * tab_w && rect.width() == tab_w;
            // La pestaña arrastrada sigue al puntero; al cruzar a otro hueco cambia de sitio.
            let dragging = held.is_some_and(|(id, _)| id == s.id);
            if dragging {
                let slot = ((rect.left() + tab_w / 2.0 - tabs_left) / tab_w).floor() as usize;
                if slot != i {
                    moved = Some((i, slot));
                }
            }
            let active = i == self.w.active;
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
            let hovered = resp.hovered() && (self.w.drag_tab.is_none() || dragging);
            // Encima de las demás mientras se mueve (arrastrada, o la activa deslizándose).
            // `Middle` y no `Foreground`: los menús emergentes (clic derecho) van en
            // `Foreground` y esta capa, creada después, los tapaba.
            let p = &if dragging || (active && !in_slot) {
                ui.painter().clone().with_layer_id(egui::LayerId::new(
                    egui::Order::Middle,
                    egui::Id::unique("tab-drag"),
                ))
            } else if active {
                ui.painter().clone()
            } else {
                let bar = Rect::from_x_y_ranges(rect.x_range(), full.y_range());
                ui.painter().with_clip_rect(uncovered(bar, active_cover))
            };
            let tint = s
                .color
                .and_then(|k| TAB_COLORS.get(k as usize))
                .map(|c| c.1);
            if active {
                // Unida a la terminal: su mismo fondo y sin línea por abajo; rayas finas a
                // los lados. El color de la pestaña va en una raya arriba (en el relleno
                // rompería la unión).
                // Redondeada a píxeles como la redondea `rect_filled`: el hueco del fondo de
                // la barra tiene que coincidir con ella. Si no, con el borde en fracción de
                // píxel quedaba una columna sin pintar y se veía lo de detrás de la ventana.
                let rect = egui::emath::GuiRounding::round_to_pixels(rect, ctx.pixels_per_point());
                active_body = Some(rect);
                p.rect_filled(rect, 0.0, fade(pal.bg, opacity));
                for x in [rect.left() + 0.5, rect.right() - 0.5] {
                    p.line_segment(
                        [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                        Stroke::new(1.0, pal.border),
                    );
                }
                if let Some(c) = tint {
                    let y = rect.top() + 1.0;
                    p.line_segment(
                        [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
                        Stroke::new(2.0, c),
                    );
                }
            } else {
                if let Some(c) = tint {
                    let strength = if hovered { 0.22 } else { 0.14 };
                    p.rect_filled(rect, 0.0, c.gamma_multiply(strength));
                    // La raya de abajo es lo que deja ver el color de un vistazo.
                    let y = rect.bottom() - 2.0;
                    p.line_segment(
                        [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
                        Stroke::new(2.0, c),
                    );
                } else if hovered {
                    p.rect_filled(rect, 0.0, pal.border.gamma_multiply(0.6));
                }
                // Separador corto a la derecha, salvo pegado a la activa.
                if divider_after(i, self.w.active) && !dragging {
                    let x = rect.right() - 0.5;
                    let c = rect.center().y;
                    p.line_segment(
                        [Pos2::new(x, c - 8.0), Pos2::new(x, c + 8.0)],
                        Stroke::new(1.0, pal.border),
                    );
                }
            }

            // Lo de dentro, recortado a la pestaña: mientras nace no se sale de su ancho, y
            // aparece poco a poco entre 16 y 64 pt (el ancho mínimo de una pestaña).
            let pc = &{
                let mut pc = p.with_clip_rect(p.clip_rect().intersect(rect));
                pc.multiply_opacity(((rect.width() - 16.0) / 48.0).clamp(0.0, 1.0));
                pc
            };
            // Indicador a la izquierda: trabajando / terminó / agente en reposo.
            let ind = Pos2::new(rect.left() + 14.0, rect.center().y);
            // Con agente el icono se queda y el estado va en una insignia en su esquina.
            let surface = fade(if active { pal.bg } else { pal.tab_bar }, opacity);
            match (s.agent, s.busy, s.unseen) {
                (Some(a), true, _) => badged_icon(pc, ind, 5.5, a, Badge::Working(pal.fg), surface),
                (Some(a), false, true) => {
                    badged_icon(pc, ind, 5.5, a, Badge::Done(pal.done), surface)
                }
                (Some(a), false, false) => agent_icon(pc, ind, 5.5, a),
                (None, true, _) => spinner(pc, ind, 5.0, pal.accent),
                (None, false, true) => {
                    pc.circle_filled(ind, 4.0, pal.done);
                }
                (None, false, false) => {
                    pc.circle_filled(ind, 2.0, pal.muted.gamma_multiply(0.5));
                }
            }

            let text_color = if active { pal.fg } else { pal.muted };
            let right_w = 26.0;
            let max_w = rect.width() - 28.0 - right_w;
            let galley = elided(&ctx, &s.tab_label(), &ui_font, text_color, max_w);
            pc.galley(
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
                    pc.rect_filled(
                        close_rect,
                        CornerRadius::same(4),
                        pal.muted.gamma_multiply(0.3),
                    );
                }
                pc.text(
                    close_rect.center(),
                    Align2::CENTER_CENTER,
                    "×",
                    FontId::proportional(14.0),
                    pal.fg,
                );
            } else if i < 9 {
                pc.text(
                    close_rect.center(),
                    Align2::CENTER_CENTER,
                    format!("⌘{}", i + 1),
                    FontId::proportional(10.5),
                    pal.muted.gamma_multiply(0.7),
                );
            }
            // Despegada: atenuada con un velo de su propio fondo (con opacidad, el fondo de la
            // activa dejaría ver lo de detrás de la ventana).
            if self.w.detached && self.w.drag_tab.is_some_and(|(id, _)| id == s.id) {
                p.rect_filled(rect, 0.0, surface.gamma_multiply(0.6));
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
            // Clic derecho: color de la pestaña, renombrar y cerrar.
            resp.context_menu(|ui| {
                let tx = &i18n::t().chrome;
                ui.set_min_width(196.0);
                ui.label(egui::RichText::new(tx.color).color(pal.muted).size(11.0));
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let choices =
                        std::iter::once(None).chain((0..TAB_COLORS.len() as u8).map(Some));
                    for choice in choices {
                        let (dot, sw) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::click());
                        let p = ui.painter();
                        let c = dot.center();
                        let swatch = choice.and_then(|k| {
                            let k = k as usize;
                            Some((TAB_COLORS.get(k)?.1, *tx.color_names.get(k)?))
                        });
                        let label = match swatch {
                            Some((color, name)) => {
                                p.circle_filled(c, 7.0, color);
                                name
                            }
                            None => {
                                p.circle_stroke(c, 6.5, Stroke::new(1.2, pal.muted));
                                p.line_segment(
                                    [c + Vec2::new(-4.5, 4.5), c + Vec2::new(4.5, -4.5)],
                                    Stroke::new(1.2, pal.muted),
                                );
                                tx.no_color
                            }
                        };
                        if s.color == choice || sw.hovered() {
                            p.circle_stroke(c, 9.5, Stroke::new(1.5, pal.fg));
                        }
                        if sw.clicked() {
                            recolor = Some((i, choice));
                            ui.close();
                        }
                        sw.on_hover_text(label);
                    }
                });
                ui.separator();
                if ui.button(tx.rename).clicked() {
                    rename = Some(i);
                    ui.close();
                }
                if ui.button(tx.close_tab).clicked() {
                    close = Some(i);
                    ui.close();
                }
            });
            if self.w.drag_tab.is_none() && !resp.context_menu_opened() {
                resp.on_hover_ui(|ui| tab_tooltip(ui, s, &pal));
            }
        }

        ui.painter().set(
            bar_bg,
            bar_background(full, active_body, 0.0, fade(pal.tab_bar, opacity)),
        );
        let gap = active_body.map(|b| (b.left(), b.right()));
        for (x0, x1) in bottom_line(full.left(), full.right(), gap) {
            let y = full.bottom() - 0.5;
            ui.painter().line_segment(
                [Pos2::new(x0, y), Pos2::new(x1, y)],
                Stroke::new(1.0, pal.border),
            );
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
        let movable = !(over_tabs || self.w.drag_tab.is_some() || grabbed.is_some());
        if self.w.window_movable != Some(movable) && blur::set_movable(movable) {
            self.w.window_movable = Some(movable);
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
        plus_resp.on_hover_text(i18n::t().chrome.new_tab_hint);

        // Panel de agentes oculto: el botón que lo muestra va tras los semáforos.
        if !self.show_agents {
            let c = Pos2::new(
                full.left() + TRAFFIC_LIGHTS_W + agents::TOGGLE_W / 2.0,
                full.center().y,
            );
            self.toggle_button(ui, c);
        }

        // Un botón por apartado.
        let cy = full.center().y;
        let ports_btn =
            Rect::from_center_size(Pos2::new(full.right() - 26.0, cy), Vec2::new(30.0, 24.0));
        let procs_btn =
            Rect::from_center_size(Pos2::new(full.right() - 62.0, cy), Vec2::new(30.0, 24.0));
        let toggle = procs_btn;
        for (rect, section, hint) in [
            (procs_btn, Section::Processes, i18n::t().chrome.procs_hint),
            (ports_btn, Section::Ports, i18n::t().chrome.ports_hint),
        ] {
            let resp = ui.interact(rect, ui.scope_id().with(("section", hint)), Sense::click());
            let on = match section {
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
                    Section::Ports => self.show_ports = !on,
                    Section::Processes => self.show_procs = !on,
                }
            }
            resp.on_hover_text(hint);
        }

        // Compilación nueva en disco: un clic reinicia y la toma. Si no, pero hay una versión
        // publicada más nueva («Buscar actualizaciones»), el clic la instala con Homebrew en
        // una pestaña (o abre su página); al reemplazar brew el ejecutable, el botón pasa a
        // reiniciar.
        let mut right_edge = toggle.left() - 24.0;
        let mut restart = false;
        let mut upgrade = false;
        if show_update {
            let available = (!self.update_ready && !self.force_update)
                .then(|| self.update_available.clone())
                .flatten();
            // Un botón más junto a Procesos y Puertos, del mismo tamaño y a la misma distancia:
            // flecha circular (reiniciar) o hacia abajo (descargar) y un punto de acento que
            // avisa de que hay algo nuevo. Qué hace lo dice el aviso al pasar el puntero.
            let btn = Rect::from_center_size(
                Pos2::new(procs_btn.center().x - 36.0, cy),
                Vec2::new(30.0, 24.0),
            );
            let resp = ui.interact(btn, ui.scope_id().with("update"), Sense::click());
            let over = resp.hovered();
            let p = ui.painter();
            let surface = if over {
                p.rect_filled(btn, CornerRadius::same(6), pal.border);
                pal.border
            } else {
                fade(pal.tab_bar, opacity)
            };
            let icon_color = if over { pal.fg } else { pal.muted };
            if available.is_some() {
                download_icon(p, btn.center(), icon_color);
            } else {
                update_icon(p, btn.center(), icon_color);
            }
            // La insignia lleva un aro del color de debajo, como la de las pestañas.
            // Arriba a la derecha, sin tapar la punta de la flecha.
            let dot = btn.center() + Vec2::new(8.0, -7.0);
            p.circle_filled(dot, 4.0, surface);
            p.circle_filled(dot, 2.75, pal.accent);
            let t = &i18n::t().chrome;
            match available {
                Some(version) => {
                    upgrade = resp.clicked();
                    let hint = if self.upgrades_with_brew() {
                        t.update_available_brew
                    } else {
                        t.update_available_web
                    };
                    resp.on_hover_ui(|ui| {
                        ui.strong((t.update_available)(&version));
                        ui.label(hint);
                    });
                }
                None => {
                    restart = resp.clicked();
                    resp.on_hover_ui(|ui| {
                        ui.strong(t.update);
                        ui.label(t.update_hint);
                    });
                }
            }
            right_edge = btn.left() - 24.0;
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
            self.w.drag_tab = Some((id, grab));
        }
        if let Some((from, to)) = moved {
            self.move_tab(from, to);
        }
        if let Some((id, grab)) = dropped {
            self.drop_detached(&ctx, id, grab, pointer);
        }
        if let Some((i, color)) = recolor
            && let Some(s) = self.w.sessions.get_mut(i)
        {
            s.color = color;
        }
        if let Some(i) = close {
            self.close_tab(i);
        }
        if let Some(i) = rename {
            self.select(i);
            self.run(&ctx, Cmd::RenameTab);
        }
        if plus_clicked {
            let cwd = self.w.sessions.get(self.w.active).map(|s| s.cwd.clone());
            self.new_tab(&ctx, cwd);
        }
        if restart {
            self.restart();
        }
        if upgrade {
            self.start_upgrade(&ctx);
        }
    }

    fn side_panel(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal.clone();
        let small = FontId::proportional(11.5);
        let tx = &i18n::t().chrome;

        if self.show_ports {
            let ports = self.ports.list.lock().unwrap().clone();
            let web: Vec<_> = ports.iter().filter(|p| p.web).collect();
            let other: Vec<_> = ports.iter().filter(|p| !p.web).collect();
            let count = (tx.web_servers)(web.len());
            let header_top = ui.cursor().min.y;
            let header_right = ui.max_rect().right();
            if section_header(ui, &pal, Section::Ports, tx.ports, &count, 26.0) {
                self.show_ports = false;
            }
            let refresh = Rect::from_center_size(
                Pos2::new(header_right - 44.0, header_top + 20.0),
                Vec2::splat(22.0),
            );
            let id = ui.scope_id().with("ports-refresh");
            if icon_button(ui, &pal, refresh, id, Icon::Refresh, tx.refresh_now).0 {
                self.ports.refresh();
            }
            if web.is_empty() {
                hint(ui, &pal, tx.no_web_servers);
            }
            for port in &web {
                self.port_row(ui, &pal, port, true);
            }
            if !other.is_empty() {
                let arrow = if self.show_other_ports { "⌄" } else { "›" };
                let label = format!("{arrow}  {}", (tx.more_ports)(other.len()));
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
            self.ports.refresh();
        }
        if self
            .confirm_stop
            .is_some_and(|(_, t)| t.elapsed() > CONFIRM_WINDOW)
        {
            self.confirm_stop = None;
        }
    }
}

impl App {
    fn processes_section(&mut self, ui: &mut egui::Ui, pal: &Palette) {
        if !self.show_procs {
            return;
        }
        let tx = &i18n::t().chrome;
        let snap = self.monitor.data.lock().unwrap().clone();
        let gb = |b: u64| b as f64 / 1_073_741_824.0;
        let detail = (tx.procs_detail)(snap.cpu_total, gb(snap.mem_used), gb(snap.mem_total));
        if section_header(ui, pal, Section::Processes, tx.processes, &detail, 0.0) {
            self.show_procs = false;
            self.w.search_focus = false;
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
                .hint_text(tx.search)
                .font(small.clone())
                .desired_width(field.width()),
        );
        self.w.search_focus = edit.has_focus();
        if self.w.search_focus
            && ui.input(|i| i.key_pressed(egui::Key::Escape) || i.key_pressed(egui::Key::Enter))
        {
            edit.surrender_focus();
            self.w.search_focus = false;
        }
        for (label, x, by_mem) in [(tx.col_cpu, cpu_x, false), (tx.col_memory, mem_x, true)] {
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
            resp.on_hover_text(tx.sort_by);
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
                    tx.reading_procs
                } else {
                    tx.no_match
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
            let cpu = (tx.percent)(proc.cpu, if proc.cpu >= 10.0 { 0 } else { 1 });
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
        self.ports.refresh();
    }

    /// Pestaña desde la que se lanzó un proceso (su shell es antepasado del proceso).
    fn owner_tab(&self, pid: i32) -> Option<usize> {
        let mut current = pid;
        for _ in 0..16 {
            if let Some(i) = self
                .w
                .sessions
                .iter()
                .position(|s| s.shell_pid() == current)
            {
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
        let tx = &i18n::t().chrome;
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
                tx.stopping,
                small.clone(),
                red,
            );
            reserved = r.width() + 24.0;
            ctx.request_repaint_after(Duration::from_millis(500));
        } else if armed {
            let label = if self.stubborn.contains(&port.pid) {
                tx.force
            } else {
                tx.stop
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
            if button(ui, Icon::Trash, tx.stop_process) {
                self.confirm_stop = Some((port.pid, Instant::now()));
            }
            if button(ui, Icon::Copy, tx.copy_url) {
                ctx.copy_text(url.clone());
            }
            if web && button(ui, Icon::External, tx.open_browser) {
                spawn_detached("/usr/bin/open", &[url.clone()]);
            }
            if let Some(tab) = owner {
                if button(ui, Icon::Arrow, tx.go_to_tab) {
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

impl shell::App for App {
    // Fondo de ventana transparente: lo que se ve lo pinta cada panel con su opacidad.
    fn clear_color(&self) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn window_opened(&mut self, win: WindowId, ctx: &egui::Context) {
        ctx.set_theme(egui::Theme::Dark);
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        install_fonts(ctx, &self.cfg);
        // Sin nada pedido, una pestaña en la carpeta de la ventana del frente (o donde se
        // arrancó Colibrí, la primera vez).
        let here = self.w.sessions.get(self.w.active).map(|s| s.cwd.clone());
        let start_dir = here.clone().or_else(|| {
            std::env::current_dir()
                .ok()
                .filter(|p| p.as_os_str() != "/")
        });
        let fill = self
            .opening
            .pop_front()
            .unwrap_or_else(|| Fill::Fresh(start_dir.clone()));
        self.parked.insert(win, Win::new(ctx));
        self.order.push(win);
        self.enter(win);
        self.front = Some(win);
        match fill {
            Fill::Fresh(dir) => {
                self.new_tab(ctx, dir);
            }
            Fill::Saved(saved) => self.open_saved_window(ctx, &saved),
            Fill::Move(s) => {
                s.set_context(ctx);
                self.w.sessions.push(*s);
                self.w.active = 0;
            }
            Fill::Reopen(tabs, active) => {
                for tab in &tabs {
                    self.revive(ctx, tab, None);
                }
                self.w.active = active.min(self.w.sessions.len().saturating_sub(1));
            }
        }
        if self.w.sessions.is_empty() {
            self.new_tab(ctx, start_dir);
        }
    }

    fn window_closed(&mut self, win: WindowId) {
        self.order.retain(|w| *w != win);
        // Saca la ventana y deja otra (si queda) como la ventana en curso.
        let gone = if self.cur == Some(win) {
            let next = self.parked.keys().next().copied();
            let blank = Win::new(&egui::Context::default());
            let gone = std::mem::replace(&mut self.w, blank);
            self.cur = None;
            if let Some(next) = next
                && let Some(w) = self.parked.remove(&next)
            {
                self.w = w;
                self.cur = Some(next);
            }
            Some(gone)
        } else {
            self.parked.remove(&win)
        };
        if self.front == Some(win) {
            self.front = self.cur;
        }
        // Sus pestañas quedan apuntadas para ⌘⇧T y sus procesos terminan al soltar `gone`.
        // Si era la última, la aplicación sale.
        if let Some(gone) = gone
            && !gone.sessions.is_empty()
            && self.cur.is_some()
        {
            self.remember(Closed::Window {
                tabs: gone.sessions.iter().map(tab_record).collect(),
                active: gone.active,
                frame: gone.frame,
            });
        }
    }

    fn new_windows(&mut self) -> Vec<Option<[f32; 4]>> {
        std::mem::take(&mut self.want_windows)
    }

    fn window_failed(&mut self) {
        // Lo de esa ventana sale de la cola, para que no acabe en la siguiente que se abra.
        // Una pestaña que llegaba viva vuelve a la ventana en curso (la de la que salió);
        // sin ninguna abierta se suelta y su proceso termina.
        if let Some(Fill::Move(s)) = self.opening.pop_front()
            && self.cur.is_some()
        {
            s.set_context(&self.w.ctx);
            self.w.sessions.push(*s);
            self.w.active = self.w.sessions.len() - 1;
            self.w.ctx.request_repaint();
        }
    }

    fn raw_input_hook(&mut self, win: WindowId, ctx: &egui::Context, raw: &mut egui::RawInput) {
        // Lo que simula el guion y lo elegido en la barra de menús entra por la ventana del
        // frente.
        if self.front == Some(win) {
            for action in menu::take() {
                match action {
                    menu::Action::Key(key, modifiers) => self.script_keys.push((key, modifiers)),
                    menu::Action::Cmd(cmd) => self.menu_cmds.push(cmd),
                    menu::Action::System(_) => {}
                }
            }
            raw.events.append(&mut self.script_pointer);
            for path in self.script_drops.drain(..) {
                raw.dropped_files.push(std::sync::Arc::new(Dropped(path)));
            }
            for (key, modifiers) in self.script_keys.drain(..) {
                for pressed in [true, false] {
                    raw.events.push(egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers,
                    });
                    // Una letra o cifra sin ⌘ ni ⌃ trae también su texto, como el teclado
                    // de verdad (para probar a quién llega lo que se escribe).
                    let name = key.name();
                    if pressed
                        && !modifiers.command
                        && !modifiers.ctrl
                        && name.chars().count() == 1
                        && name.chars().all(|c| c.is_ascii_alphanumeric())
                    {
                        let text = if modifiers.shift {
                            name.to_string()
                        } else {
                            name.to_lowercase()
                        };
                        raw.events.push(egui::Event::Text(text));
                    }
                }
            }
        }
        // Después de lo simulado, para que `key paste` del guion pase por aquí como ⌘V.
        clip::translate(ctx, &mut raw.events);
    }

    fn logic(&mut self, win: WindowId, ctx: &egui::Context) {
        self.enter(win);
        self.w.focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        if self.w.focused {
            self.front = Some(win);
        }
        // Lo que no es de una ventana (Finder, atajo global, notificaciones, menú, guion)
        // lo atiende la del frente.
        let front = self.front == Some(win);

        if self.cfg_checked.elapsed() > Duration::from_secs(1) {
            self.cfg_checked = Instant::now();
            self.reduce_motion = macos::reduce_motion();
            let m = config::mtime();
            if m != self.cfg_mtime {
                self.cfg_mtime = m;
                self.reload_config();
            }
        }

        if front {
            self.run_script(ctx);
        }
        self.check_update(ctx);

        let panel_marks = |sessions: &[Session]| {
            agents::marks(
                sessions
                    .iter()
                    .map(|s| (s.id, s.agent, s.busy, s.unseen, s.clean_title())),
            )
        };
        let listed = self.show_agents.then(|| panel_marks(&self.w.sessions));
        let mut notes = Vec::new();
        for (i, s) in self.w.sessions.iter_mut().enumerate() {
            s.drain(ctx, &self.pal);
            let watching = self.w.focused && i == self.w.active;
            if s.poll(ctx, watching) {
                let tx = &i18n::t().chrome;
                let what = s.agent.map_or(tx.command, |a| a.label());
                notes.push((
                    (tx.finished_title)(what),
                    (tx.finished_body)(&s.project()),
                    s.id,
                ));
            }
        }
        // El panel de agentes de las otras ventanas enseña estas pestañas: si algo de lo
        // que muestra cambió, que repinten una vez (si no, se quedaba viejo hasta mover el
        // ratón sobre ellas).
        if let Some(before) = listed
            && before != panel_marks(&self.w.sessions)
        {
            for w in self.parked.values() {
                w.ctx.request_repaint();
            }
        }
        if !notes.is_empty() {
            if !self.w.focused {
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

        // La barra de menús, con los atajos de la configuración. Se reintenta hasta que
        // macOS termina de arrancar, y se rehace si cambian los atajos.
        if self.menu_keys.as_ref() != Some(&self.cfg.keys) && menu::build(&self.menu_spec()) {
            self.menu_keys = Some(self.cfg.keys.clone());
        }
        for cmd in std::mem::take(&mut self.menu_cmds) {
            self.run(ctx, cmd);
            ctx.request_repaint();
        }

        // Carpetas que llegan de Finder («Abrir con», el icono del Dock, el servicio): una
        // pestaña en cada una (en la carpeta del archivo, si es un archivo).
        for path in front.then(macos::take_opened).unwrap_or_default() {
            let dir = if path.is_dir() {
                Some(path)
            } else {
                path.parent().map(PathBuf::from)
            };
            self.new_tab(ctx, dir);
            ctx.send_viewport_cmd(ViewportCommand::Focus);
        }

        // Atajo global: la app ya viene al frente; ir al agente que terminó.
        if front && hotkey::take_pressed() {
            self.jump_finished();
            ctx.send_viewport_cmd(ViewportCommand::Focus);
        }

        // Clic en una notificación: traer la ventana al frente e ir a esa pestaña.
        for id in front.then(notify::take_clicked).unwrap_or_default() {
            self.focus_session(id);
        }

        let before = self.w.sessions.len();
        let active_id = self.w.sessions.get(self.w.active).map(|s| s.id);
        self.w.sessions.retain(|s| !s.exited);
        if self.w.sessions.len() != before {
            self.w.active = active_id
                .and_then(|id| self.w.sessions.iter().position(|s| s.id == id))
                .unwrap_or_else(|| self.w.active.min(self.w.sessions.len().saturating_sub(1)));
        }
        self.save_tabs();
        if self.w.sessions.is_empty() {
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }
        if self.w.focused {
            if let Some(s) = self.w.sessions.get_mut(self.w.active) {
                s.unseen = false;
            }
        }

        // El icono del Dock lleva cuántas pestañas terminaron sin que las hayas visto.
        let unseen = self
            .windows()
            .flat_map(|w| w.sessions.iter())
            .filter(|s| s.unseen)
            .count();
        if unseen != self.badge {
            self.badge = unseen;
            macos::set_badge(unseen);
        }
        // La entrada segura es de todo el sistema: solo con Colibrí al frente y, en el modo
        // de contraseñas, mientras la pestaña activa de esa ventana pide una. Se decide en
        // cada pasada: el aviso de la contraseña («Password:») ya pide un cuadro, y con
        // `sudo` o `ssh` en primer plano la pestaña refresca cada 250 ms (`wants_ticks`).
        // Encendida siempre, macOS no deja que lo que se abre desde una pestaña (`open`)
        // pase al frente; por eso no conviene tenerla así todo el rato.
        let secure = self.windows().filter(|w| w.focused).any(|w| {
            self.cfg.secure_input_on(|| {
                w.sessions
                    .get(w.active)
                    .is_some_and(|s| s.password_prompt())
            })
        });
        if secure != self.secure {
            self.secure = secure;
            macos::secure_input(secure);
            if self.script.is_some() {
                eprintln!("entrada segura: {}", if secure { "sí" } else { "no" });
            }
        }
        // Dónde está la ventana (para guardarla) y su título: el de la pestaña activa, que
        // es lo que enseñan el menú Ventana y Mission Control.
        let (outer, inner) = ctx.input(|i| (i.viewport().outer_rect, i.viewport().inner_rect));
        if let (Some(outer), Some(inner)) = (outer, inner) {
            self.w.frame = Some([outer.min.x, outer.min.y, inner.width(), inner.height()]);
        }
        let title = self
            .w
            .sessions
            .get(self.w.active)
            .map(|s| s.tab_label())
            .unwrap_or_default();
        if title != self.w.title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.w.title = title;
        }

        // Solo la pestaña activa de una ventana a la vista pide cuadros a 30 Hz; las demás
        // (y todas si la ventana está minimizada o tapada) se ponen al día cada 250 ms.
        let hidden = macos::hidden() || ctx.input(|i| i.viewport().visible() == Some(false));
        for (i, s) in self.w.sessions.iter().enumerate() {
            s.set_shown(!hidden && i == self.w.active);
        }

        self.ports.switch.set(self.show_ports || self.show_procs);
        self.monitor.switch.set(self.show_procs);
        // «Buscar actualizaciones»: apagada, el hilo no mira y el aviso desaparece. El hilo
        // despierta todas las ventanas cuando encuentra algo; aquí solo se recoge.
        self.updates.switch.set(self.cfg.check_updates);
        let available = self
            .cfg
            .check_updates
            .then(|| self.updates.available())
            .flatten();
        if available != self.update_available {
            self.update_available = available;
            ctx.request_repaint();
        }
        // Solo hay trabajo periódico mientras algo corre; en reposo no se repinta nada.
        if self.w.sessions.iter().any(|s| s.wants_ticks()) {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn ui(&mut self, win: WindowId, ui: &mut egui::Ui) {
        self.enter(win);
        debug::count_frame();
        if let Some(s) = self.w.sessions.get(self.w.active) {
            s.frame_painted();
        }
        let ctx = ui.ctx().clone();
        self.shortcuts(&ctx);
        self.overlay_ui(&ctx);
        if self.w.sessions.is_empty() {
            return;
        }
        // Archivos soltados desde Finder (o una captura arrastrada): se pegan sus rutas,
        // igual que con ⌘V.
        let dropped: Vec<String> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| clip::escape(&f.path().to_string_lossy()))
                .collect()
        });
        if !dropped.is_empty() {
            if let Some(s) = self.w.sessions.get_mut(self.w.active) {
                s.paste(&format!("{} ", dropped.join(" ")));
            }
            ctx.send_viewport_cmd(ViewportCommand::Focus);
        }
        let pal = self.pal.clone();

        let opacity = self.cfg.opacity.clamp(0.3, 1.0);
        // El desenfoque es una propiedad de la ventana: se aplica al cambiar la preferencia.
        let radius = if self.cfg.blur && opacity < 0.999 {
            30
        } else {
            0
        };
        if self.w.blur_applied != Some(radius) && blur::set(radius) {
            self.w.blur_applied = Some(radius);
        } else if radius > 0 {
            // AppKit puede volver a poner el tinte del desenfoque (ver `blur.rs`).
            blur::refresh(radius);
        }
        // El panel de agentes se declara antes que la barra de pestañas: así ocupa toda la
        // altura de la ventana y la barra empieza donde él acaba.
        let agents_right = self.cfg.agents_on_right();
        if self.show_agents {
            let panel = if agents_right {
                egui::Panel::right("agents")
            } else {
                egui::Panel::left("agents")
            };
            panel
                .default_size(260.0)
                .size_range(220.0..=420.0)
                .resizable(true)
                .frame(egui::Frame::new().fill(pal.panel))
                .show(ui, |ui| self.agents_panel(ui));
        }
        egui::Panel::top("tabs")
            .exact_size(TAB_BAR_H)
            .show_separator_line(false)
            // Sin relleno: el fondo lo pinta `tab_bar`, con el hueco de la pestaña activa.
            .frame(egui::Frame::new())
            .show(ui, |ui| self.tab_bar(ui));

        if !self.show_procs {
            self.w.search_focus = false;
        }
        if self.show_ports || self.show_procs {
            // Puertos y Procesos, bajo la barra, en el lado contrario al de los agentes.
            let panel = if agents_right {
                egui::Panel::left("side")
            } else {
                egui::Panel::right("side")
            };
            panel
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
        let focused = self.w.focused;
        if !self.show_agents {
            self.w.agents_search_focus = false;
        }
        let typing = self.w.search_focus || self.w.agents_search_focus;
        if !typing && self.w.overlay == overlay::Overlay::None {
            self.suggest_key(&ctx);
            self.history_key(&ctx);
            self.complete_key(&ctx);
        }
        // Con las sugerencias abiertas se sigue escribiendo en el shell: la lista se filtra.
        let keyboard = !typing
            && matches!(
                self.w.overlay,
                overlay::Overlay::None | overlay::Overlay::Complete { .. }
            );
        // Carpeta y rama en cajas, en la propia línea del prompt.
        let chips = self
            .w
            .sessions
            .get_mut(self.w.active)
            .filter(|_| self.cfg.status_bar)
            .map(|s| (s.cwd.clone(), s.git_activity()))
            .map(|(cwd, activity)| {
                let info = self.w.git.info(&ctx, &cwd, activity);
                (cwd, info)
            });
        let mut chip_clicked = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(fade(pal.bg, opacity)))
            .show(ui, |ui| {
                self.w.term_rect = ui.max_rect();
                if let Some(s) = self.w.sessions.get_mut(self.w.active) {
                    if let Some((_, info)) = &chips {
                        s.stamp_prompt(
                            info.as_ref().map(|i| i.branch.as_str()),
                            info.as_ref().and_then(|i| i.changes),
                        );
                    }
                    view::show(ui, s, &pal, &metrics, focused, keyboard, chips.is_some());
                    // Sugerencia en gris a partir del cursor, con qué tecla hace qué.
                    if let (Some(text), Some(cell)) = (s.suggested(), s.cursor_px) {
                        let p = ui.painter();
                        let at = cell.min + Vec2::new(3.0, metrics.text_dy);
                        let r = p.text(
                            at,
                            Align2::LEFT_TOP,
                            text,
                            metrics.font.clone(),
                            pal.muted.gamma_multiply(0.8),
                        );
                        p.text(
                            Pos2::new(r.right() + 14.0, cell.center().y),
                            Align2::LEFT_CENTER,
                            i18n::t().chrome.suggestion_keys,
                            FontId::proportional(10.5),
                            pal.muted.gamma_multiply(0.55),
                        );
                    }
                    // Solo las cajas del prompt en uso son botones; las de arriba son historia.
                    let live = s.at_prompt().then(|| s.chips_px.len().saturating_sub(1));
                    for (i, (area, parts)) in s.chips_px.iter().enumerate() {
                        let size = metrics.font.size;
                        if let Some(hit) =
                            prompt_chips(ui, &pal, *area, parts, live == Some(i), size)
                        {
                            chip_clicked = Some(hit);
                        }
                    }
                }
            });

        if let Some((chip, rect)) = chip_clicked {
            self.open_picker(&ctx, chip, rect.left_bottom());
        }

        // Con un agente trabajando, sus puntos giran sin saltos (20 cuadros/s) con la ventana
        // al frente y a saltos sin el foco (4 cuadros/s); si solo trabaja otra cosa, la luna
        // avanza a saltos con un refresco pausado. `ui()`
        // no corre con la ventana minimizada, así que una ventana que no se ve no pide nada.
        // El panel de agentes enseña también los de las otras ventanas: sus puntos giran aquí.
        let sessions = &self.w.sessions;
        let elsewhere = self.show_agents
            && self
                .parked
                .values()
                .flat_map(|w| w.sessions.iter())
                .any(|s| s.busy && s.agent.is_some());
        let agent_busy = elsewhere || sessions.iter().any(|s| s.busy && s.agent.is_some());
        let now = ctx.input(|i| i.time);
        let focused = self.w.focused;
        if let Some(t) = tick_for(agent_busy, sessions.iter().any(|s| s.busy), now, focused) {
            ctx.request_repaint_after(t);
        }
    }
}

/// Lo que se guarda de una pestaña para volver a abrirla.
fn tab_record(s: &Session) -> state::Tab {
    state::Tab {
        cwd: s.cwd.display().to_string(),
        name: s.name.clone(),
        color: s
            .color
            .and_then(|k| TAB_COLORS.get(k as usize))
            .map(|(name, _)| name.to_string())
            .unwrap_or_default(),
        agent: match s.agent {
            Some(session::Agent::Claude) => "claude".into(),
            _ => String::new(),
        },
        ..claude_ids(s)
    }
}

/// La sesión de Claude Code de la pestaña para guardarla: se relee al guardar (la tarea
/// aparcada puede aparecer con la pestaña en reposo, sin sondeos) y, si ya no está, vale la
/// última que vio `Session::poll`.
fn claude_ids(s: &Session) -> state::Tab {
    if !matches!(s.agent, Some(session::Agent::Claude)) {
        return state::Tab::default();
    }
    let seen = claude::of_pid(s.fg_pid).or_else(|| s.claude.clone());
    state::Tab {
        claude_session: seen.as_ref().map(|r| r.session.clone()).unwrap_or_default(),
        claude_job: seen.and_then(|r| r.job).unwrap_or_default(),
        ..Default::default()
    }
}

/// Una caja por dato (carpeta, rama de git, archivos con cambios) de una línea de cajas,
/// pintadas en `full`: el hueco de esa línea en pantalla. Con `live`, las cajas de la
/// carpeta y de la rama se pueden pulsar: devuelve cuál (su marca) y dónde está.
fn prompt_chips(
    ui: &mut egui::Ui,
    pal: &Palette,
    full: Rect,
    parts: &[(char, String)],
    live: bool,
    font_size: f32,
) -> Option<(char, Rect)> {
    let mut clicked = None;
    #[derive(Clone, Copy)]
    enum Icon {
        Folder,
        Branch,
        Person,
        None,
    }
    // Las cajas se suben un poco dentro de su fila (encima suele haber una línea en blanco,
    // o el margen de la terminal) para dejar aire sobre la línea donde se escribe.
    // En un prompt remoto comparten fila con el comando: van centradas en ella.
    let remote = parts.iter().any(|(kind, _)| *kind == session::CHIP_HOST);
    let (up, down) = if remote { (-2.0, 2.0) } else { (3.0, 5.0) };
    let full = Rect::from_min_max(
        Pos2::new(full.left(), full.top() - up),
        Pos2::new(full.right(), full.bottom() - down),
    );
    // En una máquina remota las cajas solo informan: los desplegables listan lo de aquí.
    let live = live && !remote;
    let h = full.height();
    // El mismo tamaño que el texto de la terminal (la caja mide una fila, así que cabe).
    let font = FontId::new(font_size.min(h * 0.8), FontFamily::Name("term-bold".into()));
    let chips: Vec<(Icon, String, Color32)> = parts
        .iter()
        .map(|(kind, text)| match *kind {
            session::CHIP_BRANCH => (Icon::Branch, text.clone(), pal.done),
            session::CHIP_HOST => (Icon::Person, text.clone(), pal.accent),
            session::CHIP_CHANGES => {
                let color = if text == "0" { pal.muted } else { pal.ansi[3] };
                (Icon::None, format!("± {text}"), color)
            }
            _ => (Icon::Folder, text.clone(), pal.fg),
        })
        .collect();
    let mut x = full.left();
    // La carpeta se recorta por el inicio si no cabe; las demás cajas tienen prioridad.
    let others: f32 = chips.len().saturating_sub(1) as f32 * 120.0;
    for (icon, text, color) in chips {
        let icon_w = if matches!(icon, Icon::None) {
            0.0
        } else {
            18.0
        };
        let max_w = match icon {
            Icon::Folder => (full.right() - x - others - icon_w - 30.0).max(60.0),
            _ => 220.0,
        };
        let galley = elided(ui.ctx(), &text, &font, color, max_w);
        let w = 9.0 + icon_w + galley.size().x + 9.0;
        let rect = Rect::from_min_size(Pos2::new(x, full.top()), Vec2::new(w, h));
        if rect.right() > full.right() {
            break;
        }
        let button = match icon {
            Icon::Folder => Some(session::CHIP_DIR),
            Icon::Branch => Some(session::CHIP_BRANCH),
            Icon::Person | Icon::None => None,
        }
        .filter(|_| live);
        let hovered = button.is_some_and(|chip| {
            let resp = ui.interact(rect, ui.scope_id().with(("chip", chip)), Sense::click());
            if resp.clicked() {
                clicked = Some((chip, rect));
            }
            resp.on_hover_cursor(egui::CursorIcon::PointingHand)
                .hovered()
        });
        // Fondo y borde son el texto del tema sobre su fondo, al 5 % y al 15 % (como en
        // Warp); al pasar el ratón el fondo se aclara al 10 %, sin borde de color ni rótulo.
        let fill = pal
            .bg
            .lerp_to_gamma(pal.fg, if hovered { 0.10 } else { 0.05 });
        let border = pal.bg.lerp_to_gamma(pal.fg, 0.15);
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius::same(4), fill);
        p.rect_stroke(
            rect,
            CornerRadius::same(4),
            Stroke::new(1.0, border),
            StrokeKind::Inside,
        );
        let c = Pos2::new(rect.left() + 9.0 + 6.0, rect.center().y);
        match icon {
            Icon::Folder => file_icon(p, c, true, color),
            Icon::Branch => branch_icon(p, c, color),
            Icon::Person => {
                // Cabeza y hombros.
                let stroke = Stroke::new(1.2, color);
                p.circle_stroke(c + Vec2::new(0.0, -2.5), 2.4, stroke);
                let shoulders: Vec<Pos2> = (0..=8)
                    .map(|i| {
                        let a = std::f32::consts::PI * (1.0 + i as f32 / 8.0);
                        c + Vec2::new(a.cos() * 4.6, 5.5 + a.sin() * 4.0)
                    })
                    .collect();
                p.add(egui::Shape::line(shoulders, stroke));
            }
            Icon::None => {}
        }
        p.galley(
            Pos2::new(
                rect.left() + 9.0 + icon_w,
                rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            color,
        );
        x = rect.right() + 6.0;
    }
    clicked
}

/// Carpeta (con su pestaña) u hoja de archivo, dibujadas a mano.
/// Icono de rama centrado en `c`: dos commits en una línea y un tercero que se ramifica.
pub fn branch_icon(p: &egui::Painter, c: Pos2, color: Color32) {
    let stroke = Stroke::new(1.2, color);
    let (a, b, d) = (
        c + Vec2::new(-3.0, -4.5),
        c + Vec2::new(-3.0, 4.5),
        c + Vec2::new(3.5, -2.0),
    );
    p.line_segment([a, b], stroke);
    p.line_segment([d, c + Vec2::new(-3.0, 2.0)], stroke);
    for dot in [a, b, d] {
        p.circle_filled(dot, 1.9, color);
    }
}

pub fn file_icon(p: &egui::Painter, c: Pos2, is_dir: bool, color: Color32) {
    let stroke = Stroke::new(1.2, color);
    if is_dir {
        let body = Rect::from_center_size(c + Vec2::new(0.0, 1.0), Vec2::new(12.0, 8.0));
        p.rect_stroke(body, CornerRadius::same(2), stroke, StrokeKind::Inside);
        p.line_segment(
            [
                Pos2::new(body.left() + 1.0, body.top() - 1.5),
                Pos2::new(body.left() + 5.0, body.top() - 1.5),
            ],
            stroke,
        );
    } else {
        let page = Rect::from_center_size(c, Vec2::new(9.0, 11.0));
        p.rect_stroke(page, CornerRadius::same(2), stroke, StrokeKind::Inside);
        for dy in [-2.0, 1.0] {
            p.line_segment(
                [
                    Pos2::new(page.left() + 2.5, c.y + dy),
                    Pos2::new(page.right() - 2.5, c.y + dy),
                ],
                Stroke::new(1.0, color),
            );
        }
    }
}

/// Ancho para las pestañas: desde lo reservado a la izquierda (semáforos y botón del panel
/// de agentes, o nada si ese panel está a la izquierda) hasta lo reservado a la derecha y, con el
/// panel lateral abierto, hasta donde acaba la terminal (la del cuadro anterior). Si no, la
/// última pestaña quedaba encima del panel y, siendo la activa, se «unía» a él.
fn tabs_avail(full: Rect, right_reserved: f32, term: Rect, left_reserved: f32) -> f32 {
    let mut right = full.right() - right_reserved;
    if term.is_positive() {
        right = right.min(term.right());
    }
    right - full.left() - left_reserved - 34.0
}

/// Lo reservado a la izquierda de las pestañas: nada con el panel de agentes a la izquierda
/// (los semáforos van en él); los semáforos con él a la derecha; semáforos y el botón que lo
/// muestra si está oculto. Nunca menos que lo que ocupe Puertos/Procesos a la izquierda
/// (`side_left`).
fn tabs_left_reserved(show_agents: bool, agents_right: bool, side_left: f32) -> f32 {
    let base = match (show_agents, agents_right) {
        (true, false) => 0.0,
        (true, true) => TRAFFIC_LIGHTS_W,
        (false, _) => TRAFFIC_LIGHTS_W + agents::TOGGLE_W,
    };
    base.max(side_left)
}

/// Lo que queda a la vista de `rect` con `cover` encima (solo cuenta en horizontal: todas
/// las pestañas van a la misma altura). Tapada del todo, un rectángulo vacío.
fn uncovered(rect: Rect, cover: Option<Rect>) -> Rect {
    let Some(c) = cover.filter(|c| c.left() < rect.right() && c.right() > rect.left()) else {
        return rect;
    };
    let (l, r) = if c.left() > rect.left() {
        (rect.left(), c.left())
    } else {
        (c.right(), rect.right())
    };
    if r > l {
        Rect::from_x_y_ranges(l..=r, rect.y_range())
    } else {
        Rect::NOTHING
    }
}

/// Si va un separador a la derecha de la pestaña `i`: entre inactivas, no pegado a la
/// activa (`active`), que ya se distingue por su fondo.
fn divider_after(i: usize, active: usize) -> bool {
    i != active && i + 1 != active
}

/// Tramos de la línea inferior de la barra de pestañas, sin el hueco de la activa (que se
/// une a la terminal).
fn bottom_line(left: f32, right: f32, gap: Option<(f32, f32)>) -> Vec<(f32, f32)> {
    let Some((a, b)) = gap else {
        return vec![(left, right)];
    };
    [(left, a.min(right)), (b.max(left), right)]
        .into_iter()
        .filter(|(x0, x1)| x1 > x0)
        .collect()
}

/// Fondo de la barra de pestañas salvo la forma de la pestaña activa (esquinas superiores
/// redondeadas de radio `r`). La activa lleva el fondo de la terminal y no puede ir encima
/// del de la barra: con la ventana translúcida las dos capas sumadas se verían más opacas
/// que la terminal y se notaría la costura.
fn bar_background(full: Rect, tab: Option<Rect>, r: f32, color: Color32) -> egui::Mesh {
    let mut m = egui::Mesh::default();
    let mut rect = |min: Pos2, max: Pos2| {
        if max.x > min.x && max.y > min.y {
            m.add_colored_rect(Rect::from_min_max(min, max), color);
        }
    };
    let Some(t) = tab else {
        rect(full.min, full.max);
        return m;
    };
    let (l, rr) = (t.left().max(full.left()), t.right().min(full.right()));
    rect(full.min, Pos2::new(l, full.bottom()));
    rect(Pos2::new(rr, full.top()), full.max);
    rect(Pos2::new(l, full.top()), Pos2::new(rr, t.top()));
    // Esquinas redondeadas (r > 0): abanico desde el vértice del rectángulo hasta el arco.
    let r = r.min(t.width() / 2.0).min(t.height());
    if r <= 0.0 {
        return m;
    }
    let corners = [
        (
            Pos2::new(t.left(), t.top()),
            Pos2::new(t.left() + r, t.top() + r),
            180.0_f32,
        ),
        (
            Pos2::new(t.right(), t.top()),
            Pos2::new(t.right() - r, t.top() + r),
            270.0,
        ),
    ];
    const STEPS: u32 = 8;
    for (vertex, center, from) in corners {
        let tip = m.vertices.len() as u32;
        m.colored_vertex(vertex, color);
        for k in 0..=STEPS {
            let a = (from + 90.0 * k as f32 / STEPS as f32).to_radians();
            m.colored_vertex(center + Vec2::new(a.cos(), a.sin()) * r, color);
        }
        for k in 0..STEPS {
            m.add_triangle(tip, tip + 1 + k, tip + 2 + k);
        }
    }
    m
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
    // Cada archivo se lee una sola vez y sus bytes se comparten: Menlo.ttc trae la normal
    // y la negrita y, sin esto, se cargaba dos (o tres) veces en memoria (~2 MB cada una).
    let mut blobs: std::collections::HashMap<String, egui::epaint::text::Blob> =
        std::collections::HashMap::new();
    // `weight` solo actúa en fuentes variables (un archivo con todos los pesos).
    let mut load = |name: &str, candidates: &[(&str, u32, Option<f32>)]| -> bool {
        for (path, index, weight) in candidates {
            if path.is_empty() {
                continue;
            }
            let blob = match blobs.get(*path) {
                Some(b) => Some(b.clone()),
                None => config::font_bytes(path).inspect(|b| {
                    blobs.insert(path.to_string(), b.clone());
                }),
            };
            if let Some(blob) = blob {
                let mut data = FontData::from_blob(blob, *index);
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

/// Indicador de «trabajando»: una luna que se ilumina de izquierda a derecha en cuatro
/// pasos (cuarto, media, tres cuartos, llena) y vuelve a empezar. No pide repintados
/// propios: avanza con el refresco de 250 ms que ya hay mientras algo trabaja (un spinner
/// continuo obligaba a repintar la ventana 30 veces por segundo). El paso dura algo más
/// que ese refresco para que ningún cuadro se salte una fase.
fn spinner(p: &egui::Painter, center: Pos2, r: f32, color: Color32) {
    let step = (p.ctx().input(|i| i.time) / 0.3) as u64 % 4;
    let lit = (step + 1) as f32 / 4.0;
    // Parte del círculo a la izquierda de la línea vertical que avanza: el arco que
    // pasa por el lado izquierdo, entre los dos puntos donde la línea corta el círculo.
    let cut = (lit * 2.0 - 1.0).clamp(-1.0, 1.0).acos();
    let arc = 2.0 * (std::f32::consts::PI - cut);
    let pts: Vec<Pos2> = (0..=16)
        .map(|i| {
            let a = cut + arc * i as f32 / 16.0;
            center + Vec2::new(a.cos(), -a.sin()) * r
        })
        .collect();
    p.add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
    p.circle_stroke(center, r - 0.6, Stroke::new(1.2, color));
}

/// Segundos por vuelta de los puntos de «trabajando» en la insignia del icono de un agente:
/// uno con la ventana al frente; sin el foco, dos (la mitad de cuadros, y se sigue viendo que
/// trabaja). Mismo foco que `Win::focused` (`viewport().focused`).
fn ring_period(focused: bool) -> f64 {
    if focused { 1.0 } else { 2.0 }
}
/// Refresco pausado mientras algo trabaja sin agente: la luna avanza a saltos de 300 ms.
const IDLE_TICK: Duration = Duration::from_millis(250);

/// Refresco de los puntos con la ventana al frente: giran sin saltos a 20 cuadros/s. Medido
/// (release, pantalla de 60 Hz, una ventana con un agente trabajando): CPU de Colibrí de 2,7 %
/// con 8 saltos/s a 6,2 %, sin memoria de más (a 20 cuadros/s bastan dos drawables). A 30/s
/// hacía falta el tercero y subía a 9,1 %; a 60/s, a 15,2 %. Sin el foco no se mira: saltos.
const SMOOTH_TICK: Duration = Duration::from_millis(50);

/// Duración de cada paso de la cabeza del indicador: `SMOOTH_TICK` con la ventana al frente
/// (20 por vuelta, a la vista sin saltos) y, sin el foco, de punto en punto (`BADGE_DOTS` por
/// vuelta, dos segundos por vuelta).
fn ring_step(focused: bool) -> f64 {
    if focused {
        SMOOTH_TICK.as_secs_f64()
    } else {
        ring_period(false) / BADGE_DOTS as f64
    }
}

/// Ángulo de la cabeza del indicador (radianes, en `[0, 2π)`) para el reloj de egui `t`.
/// Avanza a pasos fijos del reloj (`ring_step`), no según cuándo toque la pasada: cualquier
/// otra pasada entre medias (salida de la terminal, otra pestaña con un agente trabajando)
/// pinta el mismo ángulo que la anterior, así que el giro no va a tirones. Antes giraba con
/// el reloj de cada pasada y el refresco de 50 ms se contaba desde la última: con salida en
/// las pestañas los cuadros del indicador salían a 33 y 50 ms mezclados.
fn ring_angle(t: f64, focused: bool) -> f32 {
    let period = ring_period(focused);
    let steps = (period / ring_step(focused)).round();
    let turn = t.rem_euclid(period) / period;
    let turn = (turn * steps).floor() / steps;
    (std::f64::consts::TAU * turn) as f32
}

/// Cada cuánto repintar: si un agente trabaja, justo después del próximo paso de sus puntos
/// (`ring_step`: cada `SMOOTH_TICK` con la ventana al frente, de punto en punto sin el foco;
/// más 1 ms, para que el reloj de esa pasada ya lo haya cruzado). Se cuenta desde el reloj,
/// no desde la pasada, para que las pasadas de más no corran los pasos. Pausado si solo
/// trabaja otra cosa (un servidor en primer plano cuenta como trabajando sin fin y no debe
/// tener la app a 20 cuadros/s); nada en reposo.
fn tick_for(agent_busy: bool, any_busy: bool, t: f64, focused: bool) -> Option<Duration> {
    if agent_busy {
        let step = ring_step(focused);
        Some(Duration::from_secs_f64(step - t.rem_euclid(step) + 0.001))
    } else if any_busy {
        Some(IDLE_TICK)
    } else {
        None
    }
}

/// Insignia en la esquina superior derecha del icono de un agente.
#[derive(Clone, Copy)]
enum Badge {
    /// Trabajando: puntos que giran, del color dado.
    Working(Color32),
    /// Terminó y no se ha mirado: punto de ese color (verde).
    Done(Color32),
}

/// Puntos del indicador de «trabajando».
const BADGE_DOTS: usize = 8;

/// Brillo (0,3–1) del punto `k` de `n` con la cabeza en el ángulo `head`: la cabeza brilla
/// del todo y los que la siguen se apagan poco a poco, como el indicador de actividad de
/// macOS. Con la cabeza entre dos puntos (girando sin saltos), el de delante se enciende a
/// medida que ella llega, así que el brillo pasa de uno a otro sin saltar; con la cabeza
/// sobre un punto da lo mismo que a saltos. Los ángulos crecen en el sentido de las agujas
/// del reloj (la y va hacia abajo).
fn dot_alpha(k: usize, n: usize, head: f32) -> f32 {
    use std::f32::consts::TAU;
    let n = n as f32;
    let at = TAU * k as f32 / n;
    // Qué parte de la vuelta lleva la cabeza pasado este punto (0 = encima, 1 = una vuelta).
    let behind = (head - at).rem_euclid(TAU) / TAU;
    let trail = 1.0 - behind;
    let coming = ((behind - (1.0 - 1.0 / n)) * n).max(0.0);
    0.2 + 0.8 * trail.max(coming)
}

/// Icono del agente con una insignia arriba a la derecha. `surface` es el color de lo que hay
/// debajo: el disco de la insignia lo usa para separarse del icono.
fn badged_icon(
    p: &egui::Painter,
    c: Pos2,
    r: f32,
    agent: session::Agent,
    badge: Badge,
    surface: Color32,
) {
    agent_icon(p, c, r, agent);
    let br = r * 0.8;
    let b = c + Vec2::new(r * 0.85, -r * 0.85);
    p.circle_filled(b, br + 1.0, surface);
    match badge {
        Badge::Working(ink) => {
            let (t, focused) = p
                .ctx()
                .input(|i| (i.time, i.viewport().focused.unwrap_or(true)));
            let head = ring_angle(t, focused);
            for k in 0..BADGE_DOTS {
                let a = std::f32::consts::TAU * k as f32 / BADGE_DOTS as f32;
                let at = b + Vec2::new(a.cos(), a.sin()) * (br * 0.62);
                let alpha = dot_alpha(k, BADGE_DOTS, head);
                p.circle_filled(at, br * 0.18, ink.gamma_multiply(alpha));
            }
        }
        Badge::Done(color) => {
            p.circle_filled(b, br * 0.72, color);
        }
    }
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
        ui.label(egui::RichText::new(i18n::t().chrome.running).color(pal.accent));
    } else if let Some(t) = s.finished_at {
        let text = (i18n::t().chrome.finished_ago)(t.elapsed().as_secs());
        ui.label(egui::RichText::new(text).color(pal.done));
    }
}

#[derive(Clone, Copy)]
enum Section {
    Ports,
    Processes,
}

fn section_icon(p: &egui::Painter, c: Pos2, section: Section, color: Color32) {
    let stroke = Stroke::new(1.3, color);
    match section {
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

/// Flecha circular del botón «Actualizar» (reinicia con la compilación nueva).
fn update_icon(p: &egui::Painter, c: Pos2, color: Color32) {
    let stroke = Stroke::new(1.3, color);
    // Desde la derecha, en el sentido de las agujas del reloj, hasta arriba a la derecha.
    let (r, end, steps) = (5.2, 314f32.to_radians(), 28);
    let arc: Vec<Pos2> = (0..=steps)
        .map(|i| {
            let a = end * i as f32 / steps as f32;
            c + Vec2::new(a.cos(), a.sin()) * r
        })
        .collect();
    p.add(egui::Shape::line(arc, stroke));
    // La punta: un ángulo recto que señala hacia donde sigue el giro.
    let tip = [(4.2, -6.1), (4.2, -3.5), (1.6, -3.5)];
    let tip: Vec<Pos2> = tip.iter().map(|&(x, y)| c + Vec2::new(x, y)).collect();
    p.add(egui::Shape::line(tip, stroke));
}

/// Flecha hacia abajo sobre una bandeja: hay una versión publicada para descargar.
fn download_icon(p: &egui::Painter, c: Pos2, color: Color32) {
    let stroke = Stroke::new(1.3, color);
    p.line_segment([c + Vec2::new(0.0, -5.5), c + Vec2::new(0.0, 2.2)], stroke);
    let head = [(-3.2, -1.0), (0.0, 2.2), (3.2, -1.0)];
    let head: Vec<Pos2> = head.iter().map(|&(x, y)| c + Vec2::new(x, y)).collect();
    p.add(egui::Shape::line(head, stroke));
    let tray = [(-5.0, 2.6), (-5.0, 5.2), (5.0, 5.2), (5.0, 2.6)];
    let tray: Vec<Pos2> = tray.iter().map(|&(x, y)| c + Vec2::new(x, y)).collect();
    p.add(egui::Shape::line(tray, stroke));
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
    resp.on_hover_text(i18n::t().chrome.hide);
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
            i18n::t().chrome.stop,
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
    let tx = &i18n::t().chrome;
    resp.on_hover_text(if armed {
        (tx.confirm_stop_pid)(pid)
    } else {
        (tx.stop_pid)(pid)
    });
    (over, stopped)
}

fn ago(d: Duration) -> String {
    (i18n::t().chrome.ago)(d.as_secs())
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Algún triángulo de la malla contiene `p`.
    fn cubre(m: &egui::Mesh, p: Pos2) -> bool {
        m.indices.chunks(3).any(|t| {
            let [a, b, c] = [0, 1, 2].map(|k| m.vertices[t[k] as usize].pos);
            let cross =
                |o: Pos2, u: Pos2, v: Pos2| (u.x - o.x) * (v.y - o.y) - (u.y - o.y) * (v.x - o.x);
            let (d1, d2, d3) = (cross(a, b, p), cross(b, c, p), cross(c, a, p));
            let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
            let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
            !(neg && pos)
        })
    }

    #[test]
    fn pestanas_no_pasan_sobre_el_panel() {
        let full = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1180.0, 38.0));
        let lights = TRAFFIC_LIGHTS_W;
        // Sin panel (la terminal llega al borde) manda lo reservado a la derecha.
        let term = Rect::from_min_max(Pos2::new(0.0, 38.0), Pos2::new(1180.0, 760.0));
        assert_eq!(
            tabs_avail(full, 122.0, term, lights),
            1180.0 - 78.0 - 122.0 - 34.0
        );
        // Con panel, las pestañas (y el «+») acaban donde acaba la terminal.
        let term = Rect::from_min_max(Pos2::new(0.0, 38.0), Pos2::new(880.0, 760.0));
        assert_eq!(tabs_avail(full, 122.0, term, lights), 880.0 - 78.0 - 34.0);
        // Primer cuadro, sin terminal medida aún.
        assert_eq!(
            tabs_avail(full, 122.0, Rect::NOTHING, lights),
            1180.0 - 78.0 - 122.0 - 34.0
        );
        // Panel de agentes a la izquierda: los semáforos van en él y la barra no los reserva.
        assert_eq!(tabs_avail(full, 122.0, term, 0.0), term.right() - 34.0);
        let bar = Rect::from_min_max(Pos2::new(260.0, 0.0), Pos2::new(1180.0, 38.0));
        let term = Rect::from_min_max(Pos2::new(260.0, 38.0), Pos2::new(1180.0, 760.0));
        assert_eq!(
            tabs_avail(bar, 122.0, term, 0.0),
            1180.0 - 260.0 - 122.0 - 34.0
        );
    }

    #[test]
    fn hueco_a_la_izquierda_de_las_pestanas() {
        let lights = TRAFFIC_LIGHTS_W;
        let toggle = agents::TOGGLE_W;
        // Panel de agentes a la izquierda: los semáforos van en él.
        assert_eq!(tabs_left_reserved(true, false, 0.0), 0.0);
        // A la derecha: semáforos en la barra; con Puertos/Procesos a la izquierda (300 pt
        // bajo la barra), las pestañas empiezan donde acaba ese panel.
        assert_eq!(tabs_left_reserved(true, true, 0.0), lights);
        assert_eq!(tabs_left_reserved(true, true, 300.0), 300.0);
        // Oculto: semáforos + el botón que lo muestra.
        assert_eq!(tabs_left_reserved(false, false, 0.0), lights + toggle);
    }

    #[test]
    fn separadores_entre_pestanas() {
        // Cuatro pestañas, la activa es la 1: no hay raya pegada a ella (ni a su izquierda,
        // tras la 0, ni a su derecha); sí tras la 2 y tras la última.
        assert!(!divider_after(0, 1));
        assert!(!divider_after(1, 1));
        assert!(divider_after(2, 1));
        assert!(divider_after(3, 1));
        // Activa la primera: la raya empieza tras la segunda.
        assert!(!divider_after(0, 0));
        assert!(divider_after(1, 0));
    }

    #[test]
    fn pestana_tapada_se_recorta() {
        let r = |l: f32, rr: f32| Rect::from_min_max(Pos2::new(l, 5.0), Pos2::new(rr, 34.0));
        let tab = r(100.0, 200.0);
        // Nada encima, o la activa lejos: entera.
        assert_eq!(uncovered(tab, None), tab);
        assert_eq!(uncovered(tab, Some(r(300.0, 400.0))), tab);
        // La activa la tapa por la derecha o por la izquierda: queda el resto.
        assert_eq!(uncovered(tab, Some(r(150.0, 250.0))), r(100.0, 150.0));
        assert_eq!(uncovered(tab, Some(r(50.0, 130.0))), r(130.0, 200.0));
        // Tapada del todo: vacía.
        assert!(!uncovered(tab, Some(r(100.0, 200.0))).is_positive());
    }

    /// Con la ventana al frente la cabeza gira una vuelta por segundo en 20 pasos (uno por
    /// cuadro, a la vista sin saltos). Sin el foco salta de punto en punto (basta un cuadro
    /// por salto), una vuelta cada dos segundos.
    #[test]
    fn la_cabeza_del_indicador_gira_al_frente_y_salta_sin_foco() {
        use std::f32::consts::{FRAC_PI_4, TAU};
        assert_eq!((ring_period(true), ring_period(false)), (1.0, 2.0));
        assert_eq!(ring_angle(0.0, true), 0.0);
        assert!(
            (ring_angle(0.1, true) - TAU * 0.1).abs() < 1e-4,
            "al frente, un paso cada 50 ms"
        );
        assert!((ring_angle(1.25, true) - TAU * 0.25).abs() < 1e-4);
        assert!(ring_angle(0.999, true) < TAU);
        // Una pasada de más entre dos pasos (salida de la terminal) pinta el mismo ángulo:
        // el giro no depende de cuándo toquen las pasadas.
        assert_eq!(ring_angle(0.112, true), ring_angle(0.149, true));
        assert!(ring_angle(0.151, true) > ring_angle(0.149, true));
        assert_eq!(
            ring_angle(0.2, false),
            0.0,
            "sin foco, entre saltos no se mueve"
        );
        assert!((ring_angle(0.26, false) - FRAC_PI_4).abs() < 1e-4);
        assert!(ring_angle(1.999, false) < TAU);
    }

    #[test]
    fn puntos_del_indicador_se_apagan_tras_la_cabeza() {
        use std::f32::consts::FRAC_PI_4;
        // La cabeza (ángulo 0) brilla del todo; justo delante de ella, el más apagado; los
        // que la siguen se van apagando poco a poco.
        assert_eq!(dot_alpha(0, 8, 0.0), 1.0);
        assert!((dot_alpha(7, 8, 0.0) - 0.9).abs() < 1e-4);
        assert!((dot_alpha(1, 8, 0.0) - 0.3).abs() < 1e-4);
        // Al girar un octavo, la cabeza pasa al siguiente punto.
        assert!((dot_alpha(1, 8, FRAC_PI_4) - 1.0).abs() < 1e-4);
        // A medio camino, el de delante ya va por la mitad y el que deja se apaga despacio.
        let half = FRAC_PI_4 / 2.0;
        assert!((dot_alpha(1, 8, half) - 0.6).abs() < 1e-4);
        assert!((dot_alpha(0, 8, half) - (1.0 - 0.8 / 16.0)).abs() < 1e-4);
    }

    /// Girando sin saltos, ningún punto cambia de golpe: el brillo pasa de uno a otro.
    #[test]
    fn puntos_del_indicador_sin_saltos_de_brillo() {
        use std::f32::consts::TAU;
        let steps = 10_000;
        for k in 0..8 {
            let mut prev = dot_alpha(k, 8, 0.0);
            for i in 1..=steps {
                let now = dot_alpha(k, 8, TAU * i as f32 / steps as f32);
                assert!(
                    (now - prev).abs() < 0.01,
                    "punto {k}, paso {i}: {prev} → {now}"
                );
                prev = now;
            }
        }
    }

    #[test]
    fn ritmo_de_repintado() {
        let cerca =
            |d: Option<Duration>, s: f64| d.is_some_and(|d| (d.as_secs_f64() - s).abs() < 1e-6);
        // Con un agente trabajando, el cuadro siguiente justo después del próximo paso (más
        // 1 ms, para que el reloj de esa pasada ya lo haya cruzado): al frente, en la rejilla
        // de 50 ms del reloj, aunque la pasada caiga a destiempo; sin el foco, el próximo salto.
        let t = tick_for(true, true, 0.05, true);
        assert!(cerca(t, 0.051), "{t:?}");
        let t = tick_for(true, true, 0.12, true);
        assert!(cerca(t, 0.031), "pasada a destiempo: {t:?}");
        let t = tick_for(true, true, 0.05, false);
        assert!(cerca(t, 0.201), "{t:?}");
        assert_eq!(
            tick_for(false, true, 0.05, true),
            Some(Duration::from_millis(250))
        );
        assert_eq!(tick_for(false, false, 0.05, true), None);
    }

    #[test]
    fn barra_sin_hueco_bajo_la_pestana() {
        assert_eq!(bottom_line(0.0, 100.0, None), vec![(0.0, 100.0)]);
        assert_eq!(
            bottom_line(0.0, 100.0, Some((20.0, 60.0))),
            vec![(0.0, 20.0), (60.0, 100.0)]
        );
        assert_eq!(
            bottom_line(0.0, 100.0, Some((0.0, 40.0))),
            vec![(40.0, 100.0)]
        );
    }

    #[test]
    fn fondo_de_barra_no_tapa_la_pestana() {
        let full = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(300.0, 38.0));
        let tab = Rect::from_min_max(Pos2::new(100.0, 5.0), Pos2::new(200.0, 38.0));
        let m = bar_background(full, Some(tab), 7.0, Color32::WHITE);
        assert!(!cubre(&m, Pos2::new(150.0, 20.0))); // dentro de la pestaña
        assert!(!cubre(&m, Pos2::new(108.0, 8.0))); // dentro, cerca de la esquina redondeada
        assert!(cubre(&m, Pos2::new(101.0, 5.5))); // fuera del arco de la esquina
        assert!(cubre(&m, Pos2::new(50.0, 20.0)));
        assert!(cubre(&m, Pos2::new(150.0, 2.0))); // encima de la pestaña
        assert!(cubre(&m, Pos2::new(250.0, 30.0)));
        // Sin pestaña activa (ninguna) cubre todo.
        let m = bar_background(full, None, 7.0, Color32::WHITE);
        assert!(cubre(&m, Pos2::new(150.0, 20.0)));
    }
}
