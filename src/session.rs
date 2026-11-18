//! Una pestaña: el emulador (alacritty_terminal), su PTY y el estado del agente que corre dentro.

use crate::config::{Config, Palette};
use crate::proc;
use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config as TermConfig, Term};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::Rgb;
use eframe::egui;
use std::borrow::Cow;
use std::collections::HashMap;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct Listener {
    tx: Sender<Event>,
    ctx: egui::Context,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let _ = self.tx.send(event);
        self.ctx.request_repaint();
    }
}

#[derive(Clone, Copy)]
pub struct TermSize {
    pub cols: usize,
    pub lines: usize,
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.lines
    }
    fn screen_lines(&self) -> usize {
        self.lines
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Agent {
    Claude,
    Codex,
    Gemini,
    Aider,
    OpenCode,
    Other,
}

impl Agent {
    pub fn label(self) -> &'static str {
        match self {
            Agent::Claude => "Claude Code",
            Agent::Codex => "Codex",
            Agent::Gemini => "Gemini",
            Agent::Aider => "Aider",
            Agent::OpenCode => "OpenCode",
            Agent::Other => "Agente",
        }
    }
    pub fn color(self) -> egui::Color32 {
        match self {
            Agent::Claude => egui::Color32::from_rgb(0xd9, 0x77, 0x57),
            Agent::Codex => egui::Color32::from_rgb(0xe6, 0xe6, 0xe6),
            Agent::Gemini => egui::Color32::from_rgb(0x7a, 0xa2, 0xf7),
            _ => egui::Color32::from_rgb(0xb3, 0x9d, 0xdb),
        }
    }

    fn from_word(w: &str) -> Option<Agent> {
        let base = w.rsplit('/').next().unwrap_or(w).to_lowercase();
        let base = base.trim_end_matches(".js").trim_end_matches(".mjs");
        Some(match base {
            "claude" | "claude-code" => Agent::Claude,
            "codex" => Agent::Codex,
            "gemini" => Agent::Gemini,
            "aider" => Agent::Aider,
            "opencode" => Agent::OpenCode,
            "amp" | "cursor-agent" | "copilot" | "goose" | "crush" | "droid" | "qwen" => {
                Agent::Other
            }
            _ => return None,
        })
    }

    fn detect(pid: i32, name: &str) -> Option<Agent> {
        if let Some(a) = Agent::from_word(name) {
            return Some(a);
        }
        // El nombre del proceso engaña: Claude Code se llama como su versión ("2.1.327")
        // y otros CLIs corren sobre node/python. argv dice la verdad.
        let args = proc::args(pid);
        for a in args.iter().take(4) {
            if let Some(agent) = Agent::from_word(a) {
                return Some(agent);
            }
            if a.contains("claude-code") || a.contains("/claude/versions/") {
                return Some(Agent::Claude);
            }
        }
        None
    }
}

pub struct Session {
    pub id: u64,
    pub term: Arc<FairMutex<Term<Listener>>>,
    sender: EventLoopSender,
    rx: Receiver<Event>,
    master_fd: i32,
    shell_pid: i32,
    pub size: TermSize,
    pub cell_px: (u16, u16),
    pub exited: bool,

    /// Nombre que le pusiste a la pestaña; vacío = automático.
    pub name: String,
    /// Título que puso el programa (OSC 0/2).
    pub title: String,
    pub cwd: PathBuf,
    pub fg_pid: i32,
    pub fg_name: String,
    pub agent: Option<Agent>,

    /// Hay algo corriendo: un agente trabajando o un comando en primer plano.
    pub busy: bool,
    busy_since: Instant,
    last_input: Instant,
    last_active_output: Instant,
    streak_start: Instant,
    /// Cuándo terminó la última tarea (para el panel y el salto).
    pub finished_at: Option<Instant>,
    /// Terminó mientras no la estabas viendo.
    pub unseen: bool,
    dirty: bool,
    last_poll: Instant,
    pub scroll_acc: f32,
    /// Última celda informada al programa y botón que se mantiene pulsado.
    pub mouse_cell: Option<(usize, usize)>,
    pub mouse_down: Option<u8>,
}

impl Session {
    pub fn new(
        id: u64,
        cfg: &Config,
        cwd: Option<PathBuf>,
        ctx: &egui::Context,
    ) -> std::io::Result<Self> {
        let size = TermSize {
            cols: 80,
            lines: 24,
        };
        let (tx, rx) = channel();
        let listener = Listener {
            tx,
            ctx: ctx.clone(),
        };

        let shell = if !cfg.shell.is_empty() {
            cfg.shell.clone()
        } else {
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into())
        };
        let mut env = HashMap::new();
        env.insert("TERM".to_string(), "xterm-256color".to_string());
        env.insert("COLORTERM".to_string(), "truecolor".to_string());
        env.insert("TERM_PROGRAM".to_string(), "Colibri".to_string());
        env.insert(
            "TERM_PROGRAM_VERSION".to_string(),
            env!("CARGO_PKG_VERSION").to_string(),
        );
        if std::env::var_os("LANG").is_none() {
            env.insert("LANG".to_string(), "en_US.UTF-8".to_string());
        }
        let cwd = cwd.filter(|p| p.is_dir()).or_else(dirs::home_dir);
        let options = tty::Options {
            shell: Some(tty::Shell::new(shell, vec!["-l".into()])),
            working_directory: cwd.clone(),
            drain_on_exit: false,
            env,
        };
        let window_size = WindowSize {
            num_lines: 24,
            num_cols: 80,
            cell_width: 8,
            cell_height: 16,
        };
        let pty = tty::new(&options, window_size, id)?;
        let master_fd = unsafe { libc::dup(pty.file().as_raw_fd()) };
        let shell_pid = pty.child().id() as i32;

        let term_cfg = TermConfig {
            scrolling_history: cfg.scrollback,
            ..Default::default()
        };
        let term = Arc::new(FairMutex::new(Term::new(term_cfg, &size, listener.clone())));
        let event_loop = EventLoop::new(term.clone(), listener, pty, false, false)?;
        let sender = event_loop.channel();
        event_loop.spawn();

        let now = Instant::now();
        let old = now - Duration::from_secs(60);
        Ok(Self {
            id,
            term,
            sender,
            rx,
            master_fd,
            shell_pid,
            size,
            cell_px: (8, 16),
            exited: false,
            name: String::new(),
            title: String::new(),
            cwd: cwd.unwrap_or_default(),
            fg_pid: shell_pid,
            fg_name: String::new(),
            agent: None,
            busy: false,
            busy_since: now,
            last_input: old,
            last_active_output: old,
            streak_start: old,
            finished_at: None,
            unseen: false,
            dirty: true,
            last_poll: old,
            scroll_acc: 0.0,
            mouse_cell: None,
            mouse_down: None,
        })
    }

    fn send(&self, bytes: Vec<u8>) {
        if !bytes.is_empty() {
            let _ = self.sender.send(Msg::Input(Cow::Owned(bytes)));
        }
    }

    /// Entrada del usuario (teclado o pegar).
    pub fn input(&mut self, bytes: Vec<u8>) {
        self.last_input = Instant::now();
        self.term.lock().scroll_display(Scroll::Bottom);
        self.send(bytes);
    }

    /// Informes de ratón: van al programa sin mover el scroll.
    pub fn report(&mut self, bytes: Vec<u8>) {
        self.last_input = Instant::now();
        self.send(bytes);
    }

    pub fn paste(&mut self, text: &str) {
        let bracketed = self
            .term
            .lock()
            .mode()
            .contains(alacritty_terminal::term::TermMode::BRACKETED_PASTE);
        if bracketed {
            let clean = text.replace('\x1b', "");
            self.input(format!("\x1b[200~{clean}\x1b[201~").into_bytes());
        } else {
            self.input(text.replace("\r\n", "\r").replace('\n', "\r").into_bytes());
        }
    }

    pub fn resize(&mut self, cols: usize, lines: usize, cell_w: f32, cell_h: f32) {
        let (cols, lines) = (cols.max(2), lines.max(1));
        if cols == self.size.cols && lines == self.size.lines {
            return;
        }
        self.size = TermSize { cols, lines };
        self.cell_px = (
            cell_w.round().max(1.0) as u16,
            cell_h.round().max(1.0) as u16,
        );
        self.term.lock().resize(self.size);
        let _ = self.sender.send(Msg::Resize(self.window_size()));
    }

    fn window_size(&self) -> WindowSize {
        WindowSize {
            num_lines: self.size.lines as u16,
            num_cols: self.size.cols as u16,
            cell_width: self.cell_px.0,
            cell_height: self.cell_px.1,
        }
    }

    /// Procesa lo que mandó el emulador desde el último cuadro.
    pub fn drain(&mut self, ctx: &egui::Context, pal: &Palette) {
        let now = Instant::now();
        while let Ok(event) = self.rx.try_recv() {
            self.dirty = true;
            match event {
                Event::Wakeup => {
                    // Salida que no es eco de lo que tecleas = actividad real.
                    if now.duration_since(self.last_input) > Duration::from_millis(350) {
                        if now.duration_since(self.last_active_output) > Duration::from_millis(1500)
                        {
                            self.streak_start = now;
                        }
                        self.last_active_output = now;
                    }
                }
                Event::Title(t) => self.title = t,
                Event::ResetTitle => self.title.clear(),
                Event::PtyWrite(s) => self.send(s.into_bytes()),
                Event::ClipboardStore(_, s) => ctx.copy_text(s),
                Event::ColorRequest(index, format) => {
                    let known = self.term.lock().colors()[index];
                    let rgb = known.unwrap_or_else(|| {
                        let c = match index {
                            0..=255 => pal.indexed(index as u8),
                            257 => pal.bg,
                            258 => pal.cursor,
                            _ => pal.fg,
                        };
                        Rgb {
                            r: c.r(),
                            g: c.g(),
                            b: c.b(),
                        }
                    });
                    self.send(format(rgb).into_bytes());
                }
                Event::TextAreaSizeRequest(format) => {
                    self.send(format(self.window_size()).into_bytes())
                }
                Event::Exit | Event::ChildExit(_) => self.exited = true,
                // ClipboardLoad se ignora a propósito: ningún programa lee tu portapapeles.
                _ => {}
            }
        }
    }

    /// Actualiza proceso en primer plano y estado. Devuelve true si algo acaba de terminar
    /// sin que lo estuvieras viendo.
    pub fn poll(&mut self, watching: bool) -> bool {
        let now = Instant::now();
        let interval = if self.dirty { 200 } else { 1000 };
        let due = now.duration_since(self.last_poll) >= Duration::from_millis(interval);
        if due && (self.dirty || self.busy) {
            self.last_poll = now;
            self.dirty = false;
            let fg = proc::foreground_pid(self.master_fd).unwrap_or(self.shell_pid);
            if fg != self.fg_pid || self.fg_name.is_empty() {
                self.fg_pid = fg;
                self.fg_name = proc::name(fg);
                if self
                    .fg_name
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_digit())
                {
                    if let Some(arg0) = proc::args(fg).first() {
                        self.fg_name = arg0.rsplit('/').next().unwrap_or(arg0).to_string();
                    }
                }
                self.agent = if fg == self.shell_pid {
                    None
                } else {
                    Agent::detect(fg, &self.fg_name)
                };
            }
            if let Some(dir) = proc::cwd(fg).or_else(|| proc::cwd(self.shell_pid)) {
                self.cwd = dir;
            }
        }

        let busy = match self.agent {
            Some(_) => match self.title_hint() {
                Some(hint) => hint,
                None => {
                    now.duration_since(self.last_active_output) < Duration::from_millis(2000)
                        && self.last_active_output.duration_since(self.streak_start)
                            > Duration::from_millis(1000)
                }
            },
            None => self.fg_pid != self.shell_pid,
        };

        let mut notify = false;
        if busy && !self.busy {
            self.busy_since = now;
            self.finished_at = None;
        } else if !busy && self.busy {
            let ran = now.duration_since(self.busy_since);
            let min = if self.agent.is_some() { 2 } else { 8 };
            if ran >= Duration::from_secs(min) {
                self.finished_at = Some(now);
                if !watching {
                    self.unseen = true;
                    notify = true;
                }
            }
        }
        self.busy = busy;
        notify
    }

    /// Algunos agentes (Claude Code) anuncian su estado en el título:
    /// spinner braille = trabajando, ✳ = esperando.
    fn title_hint(&self) -> Option<bool> {
        let first = self.title.chars().next()?;
        if ('\u{2800}'..='\u{28FF}').contains(&first) {
            Some(true)
        } else if first == '✳' {
            Some(false)
        } else {
            None
        }
    }

    /// Título del programa sin el símbolo de estado inicial.
    pub fn clean_title(&self) -> &str {
        let t = self.title.trim();
        match t.chars().next() {
            Some(c) if !c.is_alphanumeric() && c != '~' && c != '/' => t[c.len_utf8()..].trim(),
            _ => t,
        }
    }

    pub fn shell_pid(&self) -> i32 {
        self.shell_pid
    }

    pub fn project(&self) -> String {
        proc::base_name(&self.cwd)
    }

    /// Texto de la pestaña: siempre dice dónde estás.
    pub fn tab_label(&self) -> String {
        if !self.name.is_empty() {
            return self.name.clone();
        }
        match self.agent {
            Some(_) => self.project(),
            None if self.fg_pid != self.shell_pid && !self.fg_name.is_empty() => {
                format!("{} · {}", self.fg_name, self.project())
            }
            None => proc::pretty_path(&self.cwd),
        }
    }

    pub fn wants_ticks(&self) -> bool {
        self.busy || self.dirty
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.sender.send(Msg::Shutdown);
        if self.master_fd >= 0 {
            unsafe { libc::close(self.master_fd) };
        }
    }
}
