//! Panel de agentes: la parte pura (filas, estados, filtros, búsqueda, grupos, contadores,
//! agentes instalados) y el dibujo del panel (`impl App`, como `overlay.rs`).

use crate::session::Agent;
use crate::{App, Cmd, TAB_BAR_H, TRAFFIC_LIGHTS_W, proc};
use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// En qué está un agente, según el panel.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum State {
    Working,
    /// Acabó una tarea y aún no se ha mirado su pestaña.
    Done,
    Waiting,
}

pub fn state_of(busy: bool, unseen: bool) -> State {
    if busy {
        State::Working
    } else if unseen {
        State::Done
    } else {
        State::Waiting
    }
}

/// Filtros de arriba de la lista.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum Filter {
    #[default]
    All,
    /// Los que esperan un mensaje, hayan terminado sin verse o no.
    Waiting,
    Working,
}

impl Filter {
    pub fn admits(self, s: State) -> bool {
        match self {
            Filter::All => true,
            Filter::Waiting => s != State::Working,
            Filter::Working => s == State::Working,
        }
    }
}

/// Una fila de la lista: una pestaña con agente, de cualquier ventana.
pub struct Row {
    pub id: u64,
    pub dir: PathBuf,
    pub title: String,
    pub agent: Agent,
    pub state: State,
    /// Cuánto lleva en su estado actual.
    pub since: Duration,
    /// Es la pestaña activa de la ventana en curso.
    pub current: bool,
}

/// Título de la fila: el nombre puesto a mano; si no, el que publica el agente; si no hay
/// (o es su propio nombre), el nombre del agente.
pub fn row_title(name: &str, title: &str, agent: Agent) -> String {
    if !name.is_empty() {
        name.to_string()
    } else if !title.is_empty() && title != agent.label() {
        title.to_string()
    } else {
        agent.label().to_string()
    }
}

/// Minúsculas y sin tildes, para buscar.
pub fn fold(s: &str) -> String {
    s.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'ä' | 'â' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            c => c,
        })
        .collect()
}

/// La fila coincide con la búsqueda por título, carpeta o nombre del agente.
pub fn matches(row: &Row, query: &str) -> bool {
    let q = fold(query.trim());
    if q.is_empty() {
        return true;
    }
    [
        row.title.as_str(),
        &proc::base_name(&row.dir),
        row.agent.label(),
    ]
    .iter()
    .any(|s| fold(s).contains(&q))
}

/// Cuántos hay en cada filtro (Todos, Esperando, Trabajando), sin aplicar la búsqueda.
pub fn counts(rows: &[Row]) -> [usize; 3] {
    let n = |f: Filter| rows.iter().filter(|r| f.admits(r.state)).count();
    [n(Filter::All), n(Filter::Waiting), n(Filter::Working)]
}

/// Filas de una misma carpeta.
pub struct Group {
    pub dir: PathBuf,
    pub label: String,
    /// Índices en las filas, en su orden.
    pub rows: Vec<usize>,
}

/// Agrupa por carpeta completa, en orden de primera aparición. El encabezado es el nombre
/// de la carpeta, o la ruta abreviada si otra carpeta se llama igual. Los grupos sin filas
/// visibles no salen.
pub fn groups(rows: &[Row], visible: impl Fn(&Row) -> bool) -> Vec<Group> {
    let mut out: Vec<Group> = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        if !visible(r) {
            continue;
        }
        match out.iter_mut().find(|g| g.dir == r.dir) {
            Some(g) => g.rows.push(i),
            None => out.push(Group {
                dir: r.dir.clone(),
                label: proc::base_name(&r.dir),
                rows: vec![i],
            }),
        }
    }
    let names: Vec<String> = out.iter().map(|g| g.label.clone()).collect();
    for g in &mut out {
        if names.iter().filter(|n| **n == g.label).count() > 1 {
            g.label = proc::pretty_path(&g.dir);
        }
    }
    out
}

/// Lo que la lista enseña de una pestaña con agente: id, agente, trabajando, terminó sin
/// verse y título.
pub type Mark = (u64, Agent, bool, bool, String);

/// Las marcas de unas pestañas (`id`, agente, `busy`, `unseen`, título limpio). Si cambian
/// en una ventana, las demás repintan su lista (ver `logic()`).
pub fn marks<'a>(
    tabs: impl IntoIterator<Item = (u64, Option<Agent>, bool, bool, &'a str)>,
) -> Vec<Mark> {
    tabs.into_iter()
        .filter_map(|(id, agent, busy, unseen, title)| {
            Some((id, agent?, busy, unseen, title.to_string()))
        })
        .collect()
}

/// Agentes que se pueden lanzar desde abajo del panel, en este orden.
pub const LAUNCHERS: [Agent; 5] = [
    Agent::Claude,
    Agent::Codex,
    Agent::Gemini,
    Agent::Aider,
    Agent::OpenCode,
];

/// La orden que arranca cada agente.
pub fn command(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude => "claude",
        Agent::Codex => "codex",
        Agent::Gemini => "gemini",
        Agent::Aider => "aider",
        Agent::OpenCode => "opencode",
        Agent::Other => "",
    }
}

/// Los de `LAUNCHERS` que aparecen en la salida de `command -v …`, en el orden de
/// `LAUNCHERS`: una ruta absoluta por orden encontrada, o `alias nombre=…` si es un alias
/// (la instalación local de Claude Code). Cualquier otra línea es ruido del `.zshrc`.
pub fn parse_installed(out: &str) -> Vec<Agent> {
    let found: Vec<&str> = out
        .lines()
        .map(str::trim)
        .filter_map(|l| {
            if l.starts_with('/') {
                l.rsplit('/').next()
            } else {
                l.strip_prefix("alias ")?
                    .split_once('=')
                    .map(|(name, _)| name)
            }
        })
        .collect();
    LAUNCHERS
        .into_iter()
        .filter(|a| found.contains(&command(*a)))
        .collect()
}

/// Con qué se pregunta qué agentes hay: el shell de las pestañas (`cfg_shell`, si no
/// `$SHELL`, si no zsh), interactivo y de inicio de sesión como ellas, para ver su mismo
/// PATH (el de una app abierta desde Finder no trae `~/.local/bin` ni lo de npm, y en zsh
/// eso suele ponerse en el `.zshrc`).
pub fn detect_command(cfg_shell: &str, env_shell: Option<&str>) -> (String, Vec<String>) {
    let shell = if !cfg_shell.is_empty() {
        cfg_shell.to_string()
    } else {
        env_shell.unwrap_or("/bin/zsh").to_string()
    };
    let names: Vec<&str> = LAUNCHERS.iter().map(|a| command(*a)).collect();
    (
        shell,
        vec!["-ilc".into(), format!("command -v {}", names.join(" "))],
    )
}

/// Averigua en un hilo qué agentes están instalados (`detect_command`). `None` hasta que
/// responda; al hacerlo despierta todas las ventanas. Si el shell no acaba en 5 s (un
/// `.zshrc` que espera algo) se mata y no sale ningún lanzador.
pub fn find_installed(cfg_shell: &str) -> Arc<Mutex<Option<Vec<Agent>>>> {
    let slot = Arc::new(Mutex::new(None));
    let out = slot.clone();
    let (shell, args) = detect_command(cfg_shell, std::env::var("SHELL").ok().as_deref());
    std::thread::spawn(move || {
        use std::io::Read;
        use std::process::{Command, Stdio};
        let found = Command::new(shell)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()
            .and_then(|mut child| {
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                while matches!(child.try_wait(), Ok(None)) {
                    if std::time::Instant::now() > deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        return None;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                let mut text = String::new();
                child.stdout.take()?.read_to_string(&mut text).ok()?;
                Some(parse_installed(&text))
            })
            .unwrap_or_default();
        *out.lock().unwrap_or_else(|e| e.into_inner()) = Some(found);
        crate::shell::wake_all();
    });
    slot
}

/// Id del buscador del panel (igual en todas las ventanas: cada una tiene su contexto).
pub fn search_id() -> egui::Id {
    egui::Id::unique("agents-search")
}

/// Ancho del botón que oculta o muestra el panel (en la barra, tras los semáforos).
pub const TOGGLE_W: f32 = 28.0;

impl App {
    /// Las filas de la lista: las pestañas con agente de todas las ventanas, por orden de
    /// apertura de las ventanas y, dentro de cada una, de las pestañas.
    fn agent_rows(&self) -> Vec<Row> {
        let mut wins: Vec<(&crate::Win, bool)> = Vec::new();
        if self.cur.is_none_or(|c| !self.order.contains(&c)) {
            wins.push((&self.w, true));
        }
        for id in &self.order {
            if Some(*id) == self.cur {
                wins.push((&self.w, true));
            } else if let Some(w) = self.parked.get(id) {
                wins.push((w, false));
            }
        }
        let mut rows = Vec::new();
        for (w, here) in wins {
            for (i, s) in w.sessions.iter().enumerate() {
                let Some(agent) = s.agent else { continue };
                rows.push(Row {
                    id: s.id,
                    dir: s.cwd.clone(),
                    title: row_title(&s.name, s.clean_title(), agent),
                    agent,
                    state: state_of(s.busy, s.unseen),
                    since: s.state_since.elapsed(),
                    current: here && i == w.active,
                });
            }
        }
        rows
    }

    /// El panel de agentes, de arriba abajo de la ventana.
    pub fn agents_panel(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal.clone();
        let full = ui.max_rect();
        let strip = Rect::from_min_size(full.min, Vec2::new(full.width(), TAB_BAR_H));
        self.panel_strip(ui, strip);
        let ctx = ui.ctx().clone();
        let body = 13.0;
        let small = FontId::proportional(11.5);
        let pad = 10.0;
        let (left, right) = (full.left() + pad, full.right() - pad);
        let mut y = strip.bottom() + 4.0;

        // Nueva sesión y carpeta.
        let folder = Rect::from_min_size(Pos2::new(right - 30.0, y), Vec2::new(30.0, 30.0));
        let new = Rect::from_min_max(Pos2::new(left, y), Pos2::new(folder.left() - 6.0, y + 30.0));
        let resp = ui.interact(new, ui.scope_id().with("agents-new"), Sense::click());
        let p = ui.painter();
        if resp.hovered() {
            p.rect_filled(new, CornerRadius::same(6), pal.border);
        }
        let plus = Pos2::new(new.left() + 14.0, new.center().y);
        let stroke = Stroke::new(1.4, pal.fg);
        p.line_segment(
            [plus - Vec2::new(5.0, 0.0), plus + Vec2::new(5.0, 0.0)],
            stroke,
        );
        p.line_segment(
            [plus - Vec2::new(0.0, 5.0), plus + Vec2::new(0.0, 5.0)],
            stroke,
        );
        p.text(
            Pos2::new(new.left() + 28.0, new.center().y),
            Align2::LEFT_CENTER,
            "Nueva sesión",
            FontId::proportional(body),
            pal.fg,
        );
        p.text(
            Pos2::new(new.right() - 8.0, new.center().y),
            Align2::RIGHT_CENTER,
            self.keys.label(Cmd::NewTab),
            small.clone(),
            pal.muted,
        );
        let new_clicked = resp.clicked();
        let resp = ui.interact(folder, ui.scope_id().with("agents-folder"), Sense::click());
        let p = ui.painter();
        if resp.hovered() {
            p.rect_filled(folder, CornerRadius::same(6), pal.border);
        }
        folder_icon(
            p,
            folder.center(),
            if resp.hovered() { pal.fg } else { pal.muted },
        );
        let folder_clicked = resp.clicked();
        resp.on_hover_text("Nueva sesión en otra carpeta…");
        y = new.bottom() + 8.0;

        // Buscador: solo recibe teclas tras un clic; Esc lo vacía y devuelve el teclado.
        let field = Rect::from_min_max(Pos2::new(left, y), Pos2::new(right, y + 28.0));
        let p = ui.painter();
        p.rect_filled(field, CornerRadius::same(6), pal.bg);
        p.rect_stroke(
            field,
            CornerRadius::same(6),
            Stroke::new(1.0, pal.border),
            StrokeKind::Inside,
        );
        let lens = Pos2::new(field.left() + 14.0, field.center().y - 1.0);
        let ls = Stroke::new(1.3, pal.muted);
        p.circle_stroke(lens, 4.0, ls);
        p.line_segment([lens + Vec2::splat(3.0), lens + Vec2::splat(6.0)], ls);
        let text_rect = Rect::from_min_max(
            Pos2::new(field.left() + 26.0, field.top() + 2.0),
            Pos2::new(field.right() - 8.0, field.bottom() - 2.0),
        );
        let edit = ui.put(
            text_rect,
            egui::TextEdit::singleline(&mut self.agents_query)
                .id(search_id())
                .hint_text("Buscar agentes")
                .font(FontId::proportional(12.5))
                .frame(egui::Frame::NONE)
                .align(egui::Align2::LEFT_CENTER)
                .desired_width(text_rect.width()),
        );
        // El foco se mira en la memoria de egui y no con `edit.has_focus()`, que además
        // pide que la ventana tenga el foco de macOS: en el cuadro en que vuelve (⌘Tab) la
        // terminal lo reclamaría y le quitaría el teclado al buscador. Y en el cuadro en que
        // lo pierde (Enter y Esc lo sueltan solos, Tab lo pasa a otro) la tecla aún es del
        // buscador: sin `lost_focus` llegaría a la terminal y Enter ejecutaría lo escrito.
        let focused = ui.ctx().memory(|m| m.has_focus(edit.id));
        self.w.agents_search_focus = focused || edit.lost_focus();
        if self.w.agents_search_focus && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.agents_query.clear();
            edit.surrender_focus();
        }
        y = field.bottom() + 8.0;

        // Filtros con su contador.
        let rows = self.agent_rows();
        let n = counts(&rows);
        let mut x = left;
        for (k, (filter, label)) in [
            (Filter::All, "Todos"),
            (Filter::Waiting, "Esperando"),
            (Filter::Working, "Trabajando"),
        ]
        .into_iter()
        .enumerate()
        {
            let text = format!("{label} {}", n[k]);
            let on = self.agents_filter == filter;
            let color = if on { pal.fg } else { pal.muted };
            let g = ctx.fonts_mut(|f| f.layout_no_wrap(text, small.clone(), color));
            let chip = Rect::from_min_size(Pos2::new(x, y), Vec2::new(g.size().x + 16.0, 22.0));
            let resp = ui.interact(
                chip,
                ui.scope_id().with(("agents-filter", k)),
                Sense::click(),
            );
            let p = ui.painter();
            if on {
                p.rect_filled(chip, CornerRadius::same(11), pal.border);
            } else if resp.hovered() {
                p.rect_filled(chip, CornerRadius::same(11), pal.border.gamma_multiply(0.5));
            }
            p.galley(chip.center() - g.size() / 2.0, g, color);
            if resp.clicked() {
                self.agents_filter = filter;
            }
            x = chip.right() + 4.0;
        }
        y += 22.0 + 8.0;

        // Abajo, los agentes que se pueden lanzar.
        let installed = self
            .installed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .unwrap_or_default();
        let foot = Rect::from_min_max(Pos2::new(full.left(), full.bottom() - 44.0), full.max);
        let mut launch = None;
        if !installed.is_empty() {
            let p = ui.painter();
            p.line_segment(
                [foot.left_top(), foot.right_top()],
                Stroke::new(1.0, pal.border),
            );
            for (k, agent) in installed.iter().enumerate() {
                let c = Pos2::new(left + 14.0 + k as f32 * 34.0, foot.center().y);
                let r = Rect::from_center_size(c, Vec2::splat(28.0));
                let resp = ui.interact(r, ui.scope_id().with(("agents-launch", k)), Sense::click());
                if resp.hovered() {
                    ui.painter()
                        .rect_filled(r, CornerRadius::same(6), pal.border);
                }
                crate::agent_icon(ui.painter(), c, 7.5, *agent);
                if resp.clicked() {
                    launch = Some(*agent);
                }
                resp.on_hover_text(format!("Nueva sesión con {}", agent.label()));
            }
        }

        // La lista, agrupada por carpeta.
        let list = Rect::from_min_max(
            Pos2::new(full.left(), y),
            Pos2::new(
                full.right(),
                if installed.is_empty() {
                    full.bottom()
                } else {
                    foot.top()
                },
            ),
        );
        let (filter, query) = (self.agents_filter, self.agents_query.clone());
        let visible = |r: &Row| filter.admits(r.state) && matches(r, &query);
        let grouped = groups(&rows, visible);
        let mut go = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    if grouped.is_empty() {
                        let msg = if rows.is_empty() {
                            "Sin agentes. Abre uno con los iconos de abajo."
                        } else {
                            "Nada coincide."
                        };
                        let (r, _) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), 40.0),
                            Sense::hover(),
                        );
                        ui.painter().text(
                            Pos2::new(r.left() + pad + 4.0, r.center().y),
                            Align2::LEFT_CENTER,
                            msg,
                            small.clone(),
                            pal.muted,
                        );
                    }
                    for g in &grouped {
                        let (r, _) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), 26.0),
                            Sense::hover(),
                        );
                        let label = crate::elided(
                            ui.ctx(),
                            &g.label,
                            &small,
                            pal.muted,
                            r.width() - 2.0 * pad - 8.0,
                        );
                        ui.painter().galley(
                            Pos2::new(r.left() + pad + 4.0, r.bottom() - 6.0 - label.size().y),
                            label,
                            pal.muted,
                        );
                        for &k in &g.rows {
                            if self.agent_row(ui, &rows[k]) {
                                go = Some(rows[k].id);
                            }
                        }
                    }
                });
        });

        let cwd = self.w.sessions.get(self.w.active).map(|s| s.cwd.clone());
        if let Some(id) = go {
            self.focus_session(id);
        }
        if new_clicked {
            self.new_tab(&ctx, cwd.clone());
        }
        if folder_clicked {
            self.open_dir_picker(&ctx, folder.left_bottom() + Vec2::new(0.0, 4.0));
        }
        if let Some(agent) = launch {
            self.new_tab(&ctx, cwd);
            if let Some(s) = self.w.sessions.last_mut() {
                s.run_when_ready(command(agent).to_string());
            }
        }
    }

    /// Una fila de la lista. Devuelve si se hizo clic en ella.
    fn agent_row(&self, ui: &mut egui::Ui, row: &Row) -> bool {
        let pal = &self.pal;
        let (rect, resp) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 44.0), Sense::click());
        let rect = rect.shrink2(Vec2::new(6.0, 1.0));
        let p = ui.painter();
        if row.current {
            p.rect_filled(rect, CornerRadius::same(6), pal.border.gamma_multiply(0.8));
        } else if resp.hovered() {
            p.rect_filled(rect, CornerRadius::same(6), pal.border.gamma_multiply(0.45));
        }
        let surface = if row.current {
            pal.panel.lerp_to_gamma(pal.border, 0.8)
        } else {
            pal.panel
        };
        let icon = Pos2::new(rect.left() + 16.0, rect.top() + 15.0);
        match row.state {
            State::Working => crate::badged_icon(
                p,
                icon,
                7.0,
                row.agent,
                crate::Badge::Working(pal.fg),
                surface,
            ),
            State::Done => crate::badged_icon(
                p,
                icon,
                7.0,
                row.agent,
                crate::Badge::Done(pal.done),
                surface,
            ),
            State::Waiting => crate::agent_icon(p, icon, 7.0, row.agent),
        }
        let small = FontId::proportional(11.5);
        let time = ui
            .ctx()
            .fonts_mut(|f| f.layout_no_wrap(crate::ago(row.since), small.clone(), pal.muted));
        let tx = rect.left() + 32.0;
        let title_w = rect.right() - 8.0 - time.size().x - 8.0 - tx;
        let title = elided_end(
            ui.ctx(),
            &row.title,
            &FontId::proportional(13.0),
            pal.fg,
            title_w,
        );
        p.galley(Pos2::new(tx, rect.top() + 6.0), title, pal.fg);
        p.galley(
            Pos2::new(rect.right() - 8.0 - time.size().x, rect.top() + 8.0),
            time,
            pal.muted,
        );
        let (status, color) = match row.state {
            State::Working => ("Trabajando…", pal.fg),
            State::Done => ("Terminó", pal.done),
            State::Waiting => ("Esperando tu mensaje", pal.muted),
        };
        p.text(
            Pos2::new(tx, rect.top() + 31.0),
            Align2::LEFT_CENTER,
            status,
            small,
            color,
        );
        let clicked = resp.clicked();
        resp.on_hover_text(proc::pretty_path(&row.dir));
        clicked
    }

    /// Franja superior del panel, alta como la barra de pestañas: a la izquierda deja
    /// sitio a los semáforos; el botón de ocultar va tras ellos (o al principio, con el
    /// panel a la derecha). El resto arrastra la ventana, como la barra de título.
    fn panel_strip(&mut self, ui: &mut egui::Ui, strip: Rect) {
        let lead = if self.cfg.agents_on_right() {
            6.0
        } else {
            TRAFFIC_LIGHTS_W
        };
        let c = Pos2::new(strip.left() + lead + TOGGLE_W / 2.0, strip.center().y);
        self.toggle_button(ui, c);
        let drag_rect = Rect::from_min_max(
            Pos2::new(c.x + TOGGLE_W / 2.0 + 4.0, strip.top()),
            strip.right_bottom(),
        );
        if drag_rect.width() > 4.0 {
            let drag = ui.interact(
                drag_rect,
                ui.scope_id().with("agents-drag"),
                Sense::click_and_drag(),
            );
            let ctx = ui.ctx();
            if drag.drag_started() {
                ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            if drag.double_clicked() {
                let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
            }
        }
    }

    /// Botón con el icono de panel lateral que oculta o muestra el panel de agentes. Con el
    /// panel oculto lleva el número de agentes que terminaron sin verse.
    pub fn toggle_button(&mut self, ui: &mut egui::Ui, center: Pos2) {
        let pal = &self.pal;
        let rect = Rect::from_center_size(center, Vec2::new(TOGGLE_W, 24.0));
        let resp = ui.interact(rect, ui.scope_id().with("agents-toggle"), Sense::click());
        let p = ui.painter();
        if resp.hovered() {
            p.rect_filled(rect, CornerRadius::same(6), pal.border);
        }
        let color = if resp.hovered() { pal.fg } else { pal.muted };
        // Ventana con una franja al lado del panel.
        let frame = Rect::from_center_size(center, Vec2::new(15.0, 12.0));
        p.rect_stroke(
            frame,
            CornerRadius::same(2),
            Stroke::new(1.2, color),
            StrokeKind::Inside,
        );
        let x = if self.cfg.agents_on_right() {
            frame.right() - 5.0
        } else {
            frame.left() + 5.0
        };
        p.line_segment(
            [
                Pos2::new(x, frame.top() + 1.0),
                Pos2::new(x, frame.bottom() - 1.0),
            ],
            Stroke::new(1.2, color),
        );
        if !self.show_agents {
            let unseen = self
                .w
                .sessions
                .iter()
                .chain(self.parked.values().flat_map(|w| w.sessions.iter()))
                .filter(|s| s.unseen)
                .count();
            if unseen > 0 {
                let c = Pos2::new(rect.right() - 2.0, rect.top() + 3.0);
                p.circle_filled(c, 6.5, pal.done);
                p.text(
                    c,
                    Align2::CENTER_CENTER,
                    unseen.to_string(),
                    FontId::proportional(9.5),
                    Color32::BLACK,
                );
            }
        }
        let hint = format!(
            "{} panel de agentes  {}",
            if self.show_agents {
                "Ocultar"
            } else {
                "Mostrar"
            },
            self.keys.label(Cmd::ToggleAgents)
        );
        if resp.clicked() {
            self.show_agents = !self.show_agents;
        }
        resp.on_hover_text(hint);
    }
}

/// Como `elided`, pero recorta por el final (en un título lo que importa es el principio).
fn elided_end(
    ctx: &egui::Context,
    text: &str,
    font: &FontId,
    color: Color32,
    max_w: f32,
) -> std::sync::Arc<egui::Galley> {
    let layout = |t: String| ctx.fonts_mut(|f| f.layout_no_wrap(t, font.clone(), color));
    let galley = layout(text.to_string());
    if galley.size().x <= max_w || max_w <= 0.0 {
        return galley;
    }
    let chars: Vec<char> = text.chars().collect();
    let mut keep = ((chars.len() as f32) * max_w / galley.size().x) as usize;
    loop {
        keep = keep.saturating_sub(1);
        let head: String = chars[..keep.min(chars.len())].iter().collect();
        let g = layout(format!("{}…", head.trim_end()));
        if g.size().x <= max_w || keep == 0 {
            return g;
        }
    }
}

/// Carpeta dibujada a mano.
fn folder_icon(p: &egui::Painter, c: Pos2, color: Color32) {
    let stroke = Stroke::new(1.3, color);
    let body = Rect::from_center_size(c + Vec2::new(0.0, 1.0), Vec2::new(14.0, 10.0));
    p.rect_stroke(body, CornerRadius::same(2), stroke, StrokeKind::Inside);
    p.line_segment(
        [
            Pos2::new(body.left() + 1.0, body.top()),
            Pos2::new(body.left() + 2.0, body.top() - 2.5),
        ],
        stroke,
    );
    p.line_segment(
        [
            Pos2::new(body.left() + 2.0, body.top() - 2.5),
            Pos2::new(body.left() + 6.0, body.top() - 2.5),
        ],
        stroke,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fila(id: u64, dir: &str, title: &str, agent: Agent, state: State) -> Row {
        Row {
            id,
            dir: PathBuf::from(dir),
            title: title.to_string(),
            agent,
            state,
            since: Duration::ZERO,
            current: false,
        }
    }

    #[test]
    fn marcas_de_lo_que_ensena_la_lista() {
        let a = marks([
            (1, Some(Agent::Claude), false, false, "Logos"),
            (2, None, true, false, "cargo build"),
        ]);
        // Las pestañas sin agente no salen en la lista: no cuentan.
        assert_eq!(a.len(), 1);
        // Cambia algo de lo que se ve en la fila: estado, título, que aparezca el agente.
        let b = marks([(1, Some(Agent::Claude), true, false, "Logos")]);
        let c = marks([(1, Some(Agent::Claude), false, false, "Otra")]);
        let d = marks([
            (1, Some(Agent::Claude), false, false, "Logos"),
            (3, Some(Agent::Codex), false, false, ""),
        ]);
        assert!(a != b && a != c && a != d);
        // Lo que no se ve en la lista (la pestaña sin agente) no despierta a nadie.
        let e = marks([
            (1, Some(Agent::Claude), false, false, "Logos"),
            (2, None, false, false, "zsh"),
        ]);
        assert_eq!(a, e);
    }

    #[test]
    fn deteccion_con_el_shell_de_las_pestanas() {
        // El de la config manda; si no, $SHELL; si no, zsh. Interactivo y de inicio de
        // sesión, como las pestañas: así lee también el .zshrc (nvm, npm, ~/.local/bin).
        let (sh, args) = detect_command("/bin/bash", Some("/bin/zsh"));
        assert_eq!(sh, "/bin/bash");
        assert_eq!(args[0], "-ilc");
        assert!(args[1].starts_with("command -v claude codex gemini aider opencode"));
        assert_eq!(
            detect_command("", Some("/opt/homebrew/bin/fish")).0,
            "/opt/homebrew/bin/fish"
        );
        assert_eq!(detect_command("", None).0, "/bin/zsh");
    }

    #[test]
    fn un_alias_cuenta_como_instalado() {
        // `command -v` de zsh y bash con un alias (la instalación local de Claude Code).
        let out = "alias claude='/Users/x/.claude/local/claude'\n/usr/local/bin/codex\n";
        assert_eq!(parse_installed(out), vec![Agent::Claude, Agent::Codex]);
        // Ruido del .zshrc en la salida: no son rutas ni alias de un agente.
        assert_eq!(
            parse_installed("Bienvenido\nalias ll='ls -l'\n"),
            Vec::<Agent>::new()
        );
    }

    #[test]
    fn agentes_instalados_en_su_orden() {
        let out = "/Users/x/.local/bin/claude\n/opt/homebrew/bin/gemini\n/usr/local/bin/codex\n";
        assert_eq!(
            parse_installed(out),
            vec![Agent::Claude, Agent::Codex, Agent::Gemini]
        );
        assert_eq!(parse_installed(""), Vec::<Agent>::new());
        // Un alias o una función de zsh («claude: aliased to …») no es una ruta: no cuenta.
        assert_eq!(
            parse_installed("claude: aliased to foo\n"),
            Vec::<Agent>::new()
        );
    }

    #[test]
    fn estado_de_cada_fila() {
        assert_eq!(state_of(true, true), State::Working);
        assert_eq!(state_of(false, true), State::Done);
        assert_eq!(state_of(false, false), State::Waiting);
        assert!(Filter::Waiting.admits(State::Done) && Filter::Waiting.admits(State::Waiting));
        assert!(!Filter::Waiting.admits(State::Working));
        assert!(Filter::Working.admits(State::Working) && !Filter::Working.admits(State::Done));
        assert!(Filter::All.admits(State::Working) && Filter::All.admits(State::Waiting));
    }

    #[test]
    fn titulo_de_la_fila() {
        assert_eq!(row_title("mío", "Logos", Agent::Claude), "mío");
        assert_eq!(
            row_title("", "Logos que no se muestran", Agent::Claude),
            "Logos que no se muestran"
        );
        assert_eq!(row_title("", "", Agent::Codex), "Codex");
        assert_eq!(row_title("", "Claude Code", Agent::Claude), "Claude Code");
    }

    #[test]
    fn busqueda_sin_mayusculas_ni_tildes() {
        let r = fila(
            1,
            "/p/midulive",
            "Logos que no se muestran en línea",
            Agent::Claude,
            State::Done,
        );
        assert!(matches(&r, ""));
        assert!(matches(&r, "LINEA"));
        assert!(matches(&r, "midu"));
        assert!(matches(&r, "claude"));
        assert!(!matches(&r, "codex"));
    }

    #[test]
    fn contadores() {
        let rows = [
            fila(1, "/a", "", Agent::Claude, State::Working),
            fila(2, "/a", "", Agent::Codex, State::Done),
            fila(3, "/b", "", Agent::Claude, State::Waiting),
        ];
        assert_eq!(counts(&rows), [3, 2, 1]);
    }

    #[test]
    fn grupos_por_carpeta_completa() {
        let rows = [
            fila(1, "/x/app", "", Agent::Claude, State::Working),
            fila(2, "/y/web", "", Agent::Claude, State::Waiting),
            fila(3, "/x/app", "", Agent::Codex, State::Waiting),
            fila(4, "/z/app", "", Agent::Claude, State::Waiting),
        ];
        let g = groups(&rows, |_| true);
        assert_eq!(g.len(), 3);
        assert_eq!(g[0].rows, vec![0, 2]);
        assert_eq!(g[1].label, "web");
        assert!(g[0].label.contains("/x/app") && g[2].label.contains("/z/app")); // nombres repetidos
        let solo_trabajando = groups(&rows, |r| r.state == State::Working);
        assert_eq!(solo_trabajando.len(), 1);
    }
}
