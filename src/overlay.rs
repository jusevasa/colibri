//! Ventanas flotantes sobre la terminal: paleta de comandos, preferencias y renombrar pestaña.

use crate::config::{self, Palette, SecureInput};
use crate::i18n;
use crate::{App, Cmd, elided};
use egui::{
    self, Align2, Color32, CornerRadius, FontId, Id, Key, LayerId, Modifiers, Order, Pos2, Rect,
    Sense, Stroke, Vec2,
};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PickKind {
    Branch,
    CurrentBranch,
    Parent,
    Dir,
    File,
}

/// Una fila del desplegable de una caja del prompt.
pub struct Pick {
    pub kind: PickKind,
    pub name: String,
}

/// Lo que ofrece el selector de carpetas en `dir`: `..` (si hay carpeta superior), las
/// carpetas y, con `files`, los archivos; cada grupo por nombre sin mirar mayúsculas.
pub fn dir_picks(dir: &std::path::Path, files: bool) -> Vec<Pick> {
    let mut picks = Vec::new();
    if dir.parent().is_some() {
        picks.push(Pick {
            kind: PickKind::Parent,
            name: "..".into(),
        });
    }
    let mut entries: Vec<(bool, String)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            Some((!e.path().is_dir(), name))
        })
        .filter(|(file, _)| files || !file)
        .collect();
    entries.sort_by(|a, b| (a.0, a.1.to_lowercase()).cmp(&(b.0, b.1.to_lowercase())));
    picks.extend(entries.into_iter().take(2000).map(|(file, name)| Pick {
        kind: if file { PickKind::File } else { PickKind::Dir },
        name,
    }));
    picks
}

/// Carpeta en la que abrir una pestaña nueva al elegir `name` en el selector de carpetas
/// (botón de carpeta del panel de agentes). Un archivo no abre nada.
pub fn pick_target(
    cwd: &std::path::Path,
    kind: PickKind,
    name: &str,
) -> Option<std::path::PathBuf> {
    match kind {
        PickKind::Dir => Some(cwd.join(name)),
        PickKind::Parent => cwd.parent().map(|p| p.to_path_buf()),
        _ => None,
    }
}

/// Argumento listo para el shell: entre comillas simples si hace falta.
fn shell_arg(text: &str) -> String {
    let plain = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_alphanumeric() || "-_./@+".contains(c));
    if plain {
        text.to_string()
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

#[derive(Clone, PartialEq)]
pub enum Overlay {
    None,
    Palette {
        query: String,
        sel: usize,
    },
    Prefs {
        sel: usize,
    },
    Rename {
        text: String,
    },
    /// Comandos usados antes (`App::history`, ya filtrados); `sel` 0 es el más reciente y
    /// el elegido está escrito en el prompt. `original` es lo que había antes de abrir.
    History {
        sel: usize,
        original: String,
    },
    /// Desplegable de una caja del prompt: lo que ofrece está en `App::picks`.
    Picker {
        query: String,
        sel: usize,
    },
    /// Carpetas, archivos, comandos o ramas que encajan con la palabra que se teclea en el
    /// prompt. Sin elegir nada (`sel` None) la lista solo se muestra; al elegir, la opción
    /// queda escrita en el prompt. `line` es la línea del prompt para la que vale la lista
    /// (si cambia es que se siguió escribiendo y se rehace), `x`, dónde se ancla, y `git`,
    /// lo ya leído de git (al rehacerla no se vuelve a llamar).
    Complete {
        target: crate::complete::Target,
        items: Vec<crate::complete::Item>,
        sel: Option<usize>,
        line: String,
        x: f32,
        git: crate::complete::GitData,
    },
}

/// Rótulo del tipo de una sugerencia de Tab, en el idioma de la interfaz.
fn kind_label(kind: crate::complete::Kind) -> &'static str {
    use crate::complete::Kind;
    let o = &i18n::t().overlay;
    match kind {
        Kind::Dir => o.kind_folder,
        Kind::File => o.kind_file,
        Kind::Command => o.kind_command,
        Kind::Branch => o.kind_branch,
        Kind::CurrentBranch => o.kind_current_branch,
        Kind::RemoteBranch => o.kind_remote_branch,
    }
}

/// Filas de Preferencias, por id (el que usa el guion: `pref <id>`); la etiqueta sale de
/// `pref_label` en el idioma de la interfaz.
const PREF_ROWS: [&str; 24] = [
    "theme",
    "font",
    "font_size",
    "line_height",
    "padding",
    "cursor",
    "cursor_blink",
    "opacity",
    "blur",
    "scrollback",
    "restore_tabs",
    "notify",
    "history_popup",
    "complete_popup",
    "status_bar",
    "shell_integration",
    "editor",
    "secure_input",
    "check_updates",
    "show_agents",
    "agents_side",
    "show_ports",
    "show_processes",
    "open_file",
];

fn pref_label(o: &'static i18n::overlay::T, id: &str) -> &'static str {
    match id {
        "theme" => o.pref_theme,
        "font" => o.pref_font,
        "font_size" => o.pref_font_size,
        "line_height" => o.pref_line_height,
        "padding" => o.pref_padding,
        "cursor" => o.pref_cursor,
        "cursor_blink" => o.pref_cursor_blink,
        "opacity" => o.pref_opacity,
        "blur" => o.pref_blur,
        "scrollback" => o.pref_scrollback,
        "restore_tabs" => o.pref_restore_tabs,
        "notify" => o.pref_notify,
        "history_popup" => o.pref_history_popup,
        "complete_popup" => o.pref_complete_popup,
        "status_bar" => o.pref_status_bar,
        "shell_integration" => o.pref_shell_integration,
        "editor" => o.pref_editor,
        "secure_input" => o.pref_secure_input,
        "check_updates" => o.pref_check_updates,
        "show_agents" => o.pref_show_agents,
        "agents_side" => o.pref_agents_side,
        "show_ports" => o.pref_show_ports,
        "show_processes" => o.pref_show_processes,
        "open_file" => o.pref_open_file,
        _ => "",
    }
}

/// Vista previa de macOS (Quick Look) de un archivo o carpeta. Si ya hay una abierta
/// se cierra: espacio la abre y la cierra, como en Finder.
fn quick_look(preview: &mut Option<std::process::Child>, path: Option<std::path::PathBuf>) {
    if let Some(mut open) = preview.take() {
        let running = matches!(open.try_wait(), Ok(None));
        let _ = open.kill();
        let _ = open.wait();
        if running {
            return;
        }
    }
    let Some(path) = path else {
        return;
    };
    if std::env::var_os("COLIBRI_SCRIPT").is_some() {
        eprintln!("vista previa: {}", path.display());
        return;
    }
    use std::process::Stdio;
    *preview = std::process::Command::new("qlmanage")
        .arg("-p")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok();
}

fn section_before(o: &'static i18n::overlay::T, id: &str) -> Option<&'static str> {
    match id {
        "theme" => Some(o.sec_appearance),
        "scrollback" => Some(o.sec_terminal),
        "show_agents" => Some(o.sec_panels),
        "open_file" => Some(o.sec_advanced),
        _ => None,
    }
}

/// «hace 5 min», «hace 3 h»… para una fecha en segundos Unix.
fn ago(at: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    (i18n::t().overlay.ago)(now.saturating_sub(at))
}

fn pressed(ctx: &egui::Context, key: Key) -> bool {
    ctx.input_mut(|i| i.consume_key(Modifiers::NONE, key))
}

/// Escribe en `text` lo tecleado o pegado en este cuadro.
fn type_into(ctx: &egui::Context, text: &mut String) -> bool {
    let before = text.clone();
    ctx.input(|i| {
        for event in &i.events {
            match event {
                egui::Event::Text(t) => text.push_str(t),
                egui::Event::Paste(t) => text.push_str(t.lines().next().unwrap_or("")),
                egui::Event::Key {
                    key: Key::Backspace,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    if modifiers.mac_cmd || modifiers.alt {
                        text.clear();
                    } else {
                        text.pop();
                    }
                }
                _ => {}
            }
        }
    });
    *text != before
}

/// Cuánto se ve ya un cuadro que acaba de aparecer, de 0 a 1: el fundido de entrada que egui
/// da a sus `Area` (`animation_time`, curva `quadratic_out`), para que lo que va con él (el
/// fondo atenuado) lo siga. Mientras dura, el pintor va en modo fluido.
fn fade_in(ctx: &egui::Context, area: Id) -> f32 {
    let Some(since) = egui::AreaState::load(ctx, area).and_then(|s| s.last_became_visible_at)
    else {
        return 1.0;
    };
    let (age, dt) = ctx.input(|i| ((i.time - since) as f32, i.predicted_dt));
    let duration = ctx.global_style().animation_time;
    let t = egui::emath::easing::quadratic_out(egui::remap_clamp(
        age + dt * 0.5,
        0.0..=duration,
        0.0..=1.0,
    ));
    if t < 1.0 {
        crate::shell::animating(ctx);
    }
    t
}

/// Dibuja un cuadro centrado arriba, con el fondo atenuado. Devuelve su rectángulo.
fn modal(
    ctx: &egui::Context,
    pal: &Palette,
    id: &str,
    width: f32,
    add: impl FnOnce(&mut egui::Ui),
) -> Rect {
    let screen = ctx.content_rect();
    let width = width.min(screen.width() - 40.0);
    let pos = Pos2::new(screen.center().x - width / 2.0, screen.top() + 64.0);
    let out = egui::Area::new(Id::unique(id))
        .order(Order::Foreground)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(pal.panel)
                .stroke(Stroke::new(1.0, pal.border))
                .corner_radius(CornerRadius::same(10))
                .show(ui, |ui| {
                    ui.set_width(width);
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    add(ui);
                });
        });
    // Después del cuadro (va en otra capa, debajo): su fundido empieza al mostrarlo y el
    // fondo se oscurece a la vez, en vez de aparecer de golpe.
    let shown = fade_in(ctx, Id::unique(id));
    ctx.layer_painter(LayerId::new(Order::Middle, Id::unique("overlay-dim")))
        .rect_filled(
            screen,
            0.0,
            Color32::from_black_alpha((110.0 * shown) as u8),
        );
    out.response.rect
}

fn clicked_outside(ctx: &egui::Context, rect: Rect) -> bool {
    ctx.input(|i| {
        i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !rect.contains(p))
    })
}

impl App {
    pub fn overlay_ui(&mut self, ctx: &egui::Context) {
        if self.w.overlay == Overlay::None {
            return;
        }
        if pressed(ctx, Key::Escape) {
            self.cancel_overlay();
            return;
        }
        match self.w.overlay.clone() {
            Overlay::None => {}
            Overlay::Palette { query, sel } => self.palette_ui(ctx, query, sel),
            Overlay::Prefs { sel } => self.prefs_ui(ctx, sel),
            Overlay::Rename { text } => self.rename_ui(ctx, text),
            Overlay::History { sel, original } => self.history_ui(ctx, sel, original),
            Overlay::Picker { query, sel } => self.picker_ui(ctx, query, sel),
            Overlay::Complete {
                target,
                items,
                sel,
                line,
                x,
                git,
            } => self.complete_ui(ctx, target, items, sel, line, x, git),
        }
    }

    /// Lista de comandos anclada al cursor: el más reciente abajo, ↑ va hacia los antiguos.
    /// Desplegable bajo una caja del prompt: buscador arriba y las opciones debajo. Lo que
    /// se elige se escribe y ejecuta en el shell, a la vista (`git checkout`, `cd`), para
    /// que cualquier error se lea ahí. Un archivo solo se pega en el prompt.
    fn picker_ui(&mut self, ctx: &egui::Context, mut query: String, mut sel: usize) {
        // Espacio, en la lista de carpetas y archivos, es la vista previa de Finder (y no
        // un carácter de la búsqueda).
        let files = self
            .w
            .picks
            .iter()
            .any(|p| matches!(p.kind, PickKind::Dir | PickKind::File));
        let preview = files
            && ctx.input_mut(|i| {
                let key = i.consume_key(Modifiers::NONE, Key::Space);
                let before = i.events.len();
                i.events
                    .retain(|e| !matches!(e, egui::Event::Text(t) if t == " "));
                key || before != i.events.len()
            });
        if type_into(ctx, &mut query) {
            sel = 0;
        }
        let q = query.trim().to_lowercase();
        let branches = self
            .w
            .picks
            .iter()
            .any(|p| matches!(p.kind, PickKind::Branch | PickKind::CurrentBranch));
        // Los ocultos (.git, .env…) solo salen si se buscan con el punto por delante.
        let items: Vec<&Pick> = self
            .w
            .picks
            .iter()
            .filter(|p| match p.kind {
                PickKind::Parent => q.is_empty() || "..".starts_with(&q),
                PickKind::Dir | PickKind::File
                    if p.name.starts_with('.') && !q.starts_with('.') =>
                {
                    false
                }
                _ => p.name.to_lowercase().contains(&q),
            })
            .collect();
        if pressed(ctx, Key::ArrowDown) && !items.is_empty() {
            sel = (sel + 1) % items.len();
        }
        if pressed(ctx, Key::ArrowUp) && !items.is_empty() {
            sel = (sel + items.len() - 1) % items.len();
        }
        sel = sel.min(items.len().saturating_sub(1));
        let mut pick = pressed(ctx, Key::Enter)
            .then_some(sel)
            .filter(|_| !items.is_empty());
        // Desde el panel de agentes, → entra en la carpeta elegida (o sube con `..`) en vez de
        // abrir ya la pestaña: así se llega a una carpeta hermana.
        let enter = self.w.pick_new_tab && pressed(ctx, Key::ArrowRight);
        if enter
            && let Some(p) = items.get(sel)
            && let Some(dir) = self
                .pick_dir()
                .and_then(|base| pick_target(&base, p.kind, &p.name))
        {
            self.w.picks = dir_picks(&dir, false);
            self.w.pick_base = Some(dir);
            self.w.overlay = Overlay::Picker {
                query: String::new(),
                sel: 0,
            };
            ctx.request_repaint();
            return;
        }
        if preview {
            let base = self.pick_dir();
            let path = items.get(sel).and_then(|p| {
                let cwd = base.as_ref()?;
                Some(match p.kind {
                    PickKind::Parent => cwd.parent()?.to_path_buf(),
                    _ => cwd.join(&p.name),
                })
            });
            quick_look(&mut self.w.preview, path);
        }

        const VISIBLE: usize = 10;
        const ROW: f32 = 28.0;
        const HEAD: f32 = 34.0;
        let pal = self.pal.clone();
        let o = &i18n::t().overlay;
        let mono = FontId::monospace(12.5);
        let screen = ctx.content_rect();
        let width = 440.0_f32.min(screen.width() - 16.0);
        let height = HEAD + items.len().clamp(1, VISIBLE) as f32 * ROW + 8.0;
        let max_x = (screen.right() - width - 8.0).max(screen.left() + 8.0);
        let max_y = (screen.bottom() - height - 8.0).max(screen.top() + 8.0);
        let pos = Pos2::new(
            self.w.pick_anchor.x.clamp(screen.left() + 8.0, max_x),
            (self.w.pick_anchor.y + 4.0).min(max_y),
        );
        let first = sel.saturating_sub(VISIBLE - 1);

        let out = egui::Area::new(Id::unique("picker"))
            .order(Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(pal.panel)
                    .stroke(Stroke::new(1.0, pal.border))
                    .corner_radius(CornerRadius::same(8))
                    .show(ui, |ui| {
                        ui.set_width(width);
                        ui.spacing_mut().item_spacing = Vec2::ZERO;
                        let (head, _) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), HEAD),
                            Sense::hover(),
                        );
                        let p = ui.painter();
                        let (shown, color) = if !query.is_empty() {
                            (query.clone(), pal.fg)
                        } else if branches {
                            (o.search_branch.to_string(), pal.muted)
                        } else if self.w.pick_new_tab {
                            (o.new_tab_folder.to_string(), pal.muted)
                        } else {
                            (o.search_files.to_string(), pal.muted)
                        };
                        let r = p.text(
                            Pos2::new(head.left() + 14.0, head.center().y),
                            Align2::LEFT_CENTER,
                            shown,
                            mono.clone(),
                            color,
                        );
                        let caret_x = if query.is_empty() {
                            head.left() + 13.0
                        } else {
                            r.right() + 1.0
                        };
                        p.line_segment(
                            [
                                Pos2::new(caret_x, head.center().y - 7.0),
                                Pos2::new(caret_x, head.center().y + 7.0),
                            ],
                            Stroke::new(1.5, pal.accent),
                        );
                        p.line_segment(
                            [head.left_bottom(), head.right_bottom()],
                            Stroke::new(1.0, pal.border),
                        );
                        ui.add_space(4.0);
                        if items.is_empty() {
                            let (r, _) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), ROW),
                                Sense::hover(),
                            );
                            ui.painter().text(
                                Pos2::new(r.left() + 14.0, r.center().y),
                                Align2::LEFT_CENTER,
                                o.no_matches,
                                mono.clone(),
                                pal.muted,
                            );
                        }
                        for (i, item) in items.iter().enumerate().skip(first).take(VISIBLE) {
                            let (r, resp) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), ROW),
                                Sense::click(),
                            );
                            if resp.hovered() && ui.input(|inp| inp.pointer.delta() != Vec2::ZERO) {
                                sel = i;
                            }
                            if resp.clicked() {
                                pick = Some(i);
                            }
                            let p = ui.painter();
                            if i == sel {
                                p.rect_filled(
                                    r.shrink2(Vec2::new(4.0, 1.0)),
                                    CornerRadius::same(5),
                                    pal.accent.gamma_multiply(0.16),
                                );
                            }
                            let icon_at = Pos2::new(r.left() + 20.0, r.center().y);
                            let (label, color, note) = match item.kind {
                                PickKind::CurrentBranch => {
                                    (item.name.clone(), pal.done, o.current_branch)
                                }
                                PickKind::Branch => (item.name.clone(), pal.fg, ""),
                                PickKind::Parent => {
                                    p.text(
                                        icon_at,
                                        Align2::CENTER_CENTER,
                                        "↑",
                                        mono.clone(),
                                        pal.muted,
                                    );
                                    (o.parent_folder.to_string(), pal.muted, "")
                                }
                                PickKind::Dir => {
                                    crate::file_icon(p, icon_at, true, pal.accent);
                                    (item.name.clone(), pal.fg, "")
                                }
                                PickKind::File => {
                                    crate::file_icon(p, icon_at, false, pal.muted);
                                    (item.name.clone(), pal.fg, "")
                                }
                            };
                            let text_x = if branches {
                                r.left() + 14.0
                            } else {
                                r.left() + 38.0
                            };
                            let g =
                                elided(ui.ctx(), &label, &mono, color, r.right() - text_x - 70.0);
                            p.galley(Pos2::new(text_x, r.center().y - g.size().y / 2.0), g, color);
                            if !note.is_empty() {
                                p.text(
                                    Pos2::new(r.right() - 14.0, r.center().y),
                                    Align2::RIGHT_CENTER,
                                    note,
                                    FontId::proportional(11.0),
                                    pal.muted,
                                );
                            }
                        }
                        ui.add_space(4.0);
                    });
            });
        // Su fundido de entrada (el de egui) sale fluido.
        fade_in(ctx, Id::unique("picker"));

        if let Some(i) = pick {
            let (kind, name) = (items[i].kind, items[i].name.clone());
            self.w.overlay = Overlay::None;
            if self.w.pick_new_tab {
                // Desde el panel de agentes: pestaña nueva en esa carpeta, sin `cd`.
                let target = self
                    .pick_dir()
                    .and_then(|base| pick_target(&base, kind, &name));
                self.w.pick_new_tab = false;
                self.w.pick_base = None;
                if let Some(dir) = target {
                    self.new_tab(ctx, Some(dir));
                }
            } else if let Some(s) = self.w.sessions.get_mut(self.w.active) {
                let run = match kind {
                    PickKind::CurrentBranch => None,
                    PickKind::Branch => Some(format!("git checkout {}", shell_arg(&name))),
                    PickKind::Parent => Some("cd ..".to_string()),
                    PickKind::Dir => Some(format!("cd {}", shell_arg(&name))),
                    PickKind::File => {
                        s.paste(&format!("{} ", shell_arg(&name)));
                        None
                    }
                };
                if let Some(cmd) = run {
                    s.insert_command(&cmd);
                    s.input(b"\r".to_vec());
                }
            }
        } else if clicked_outside(ctx, out.response.rect) {
            self.w.overlay = Overlay::None;
            self.w.pick_new_tab = false;
            self.w.pick_base = None;
        } else {
            self.w.overlay = Overlay::Picker { query, sel };
        }
    }

    /// Cierra el overlay deshaciendo lo que hubiera dejado a medias en el prompt: el
    /// historial devuelve lo que estaba escrito y las sugerencias, la palabra original.
    pub fn cancel_overlay(&mut self) {
        let overlay = std::mem::replace(&mut self.w.overlay, Overlay::None);
        self.w.pick_new_tab = false;
        self.w.pick_base = None;
        let Some(s) = self.w.sessions.get_mut(self.w.active) else {
            return;
        };
        match overlay {
            Overlay::History { original, .. } if original.is_empty() => s.input(vec![0x15]),
            Overlay::History { original, .. } => s.insert_command(&original),
            Overlay::Complete {
                target,
                items,
                sel: Some(i),
                ..
            } => {
                let written = crate::complete::replacement(&target, &items[i]);
                s.replace_word(written.chars().count(), &target.token);
            }
            _ => {}
        }
    }

    /// Historial sobre el prompt, como ↑ en el shell pero viendo la lista: el comando
    /// elegido está siempre escrito en el prompt, así que Enter lo ejecuta y cualquier otra
    /// tecla cierra la lista y lo edita. El más reciente va abajo; ↑ va hacia los antiguos.
    fn history_ui(&mut self, ctx: &egui::Context, mut sel: usize, original: String) {
        let n = self.w.history.len();
        if n == 0 {
            self.w.overlay = Overlay::None;
            return;
        }
        let before = sel;
        if pressed(ctx, Key::ArrowUp) {
            sel = (sel + 1).min(n - 1);
        }
        if pressed(ctx, Key::ArrowDown) {
            // Bajar desde el más reciente vuelve a lo que había escrito.
            if sel == 0 {
                self.cancel_overlay();
                return;
            }
            sel -= 1;
        }
        let run = pressed(ctx, Key::Enter);
        let keep = pressed(ctx, Key::Tab) || pressed(ctx, Key::ArrowRight);
        // Lo que quede sin consumir es para el shell: se cierra y la tecla sigue su camino.
        let other = ctx.input(|i| {
            i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Text(_)
                        | egui::Event::Paste(_)
                        | egui::Event::Key { pressed: true, .. }
                )
            })
        });
        let mut pick = None;

        const VISIBLE: usize = 9;
        const ROW: f32 = 26.0;
        const FOOT: f32 = 28.0;
        let pal = self.pal.clone();
        let o = &i18n::t().overlay;
        let mono = FontId::monospace(12.5);
        let small = FontId::proportional(11.0);
        let screen = ctx.content_rect();
        // Todo el ancho de la terminal: los comandos largos se ven enteros.
        let area = if self.w.term_rect.is_positive() {
            self.w.term_rect
        } else {
            screen
        };
        let width = (area.width() - 16.0).max(200.0);
        let height = n.min(VISIBLE) as f32 * ROW + FOOT + 8.0;
        let cursor = self
            .w
            .sessions
            .get(self.w.active)
            .and_then(|s| s.cursor_px)
            .unwrap_or_else(|| {
                Rect::from_min_size(
                    Pos2::new(screen.left() + 16.0, screen.bottom() - 40.0),
                    Vec2::new(8.0, 18.0),
                )
            });
        // Encima del cursor; si no cabe (cursor en lo alto de la pantalla), debajo.
        let above = cursor.top() - height - 6.0;
        let y = if above >= screen.top() + crate::TAB_BAR_H {
            above
        } else {
            cursor.bottom() + 6.0
        };
        let x = area.left() + 8.0;
        let first = sel.saturating_sub(VISIBLE - 1);
        let last = (first + VISIBLE).min(n);

        let out = egui::Area::new(Id::unique("history"))
            .order(Order::Foreground)
            .fixed_pos(Pos2::new(x, y))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(pal.panel)
                    .stroke(Stroke::new(1.0, pal.border))
                    .corner_radius(CornerRadius::same(8))
                    .show(ui, |ui| {
                        ui.set_width(width);
                        ui.spacing_mut().item_spacing = Vec2::ZERO;
                        ui.add_space(4.0);
                        for i in (first..last).rev() {
                            let (r, resp) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), ROW),
                                Sense::click(),
                            );
                            if resp.clicked() {
                                pick = Some(i);
                            }
                            let p = ui.painter();
                            if i == sel {
                                p.rect_filled(
                                    r.shrink2(Vec2::new(4.0, 1.0)),
                                    CornerRadius::same(5),
                                    pal.accent.gamma_multiply(0.16),
                                );
                            } else if resp.hovered() {
                                p.rect_filled(
                                    r.shrink2(Vec2::new(4.0, 1.0)),
                                    CornerRadius::same(5),
                                    pal.fg.gamma_multiply(0.04),
                                );
                            }
                            let entry = &self.w.history[i];
                            let when = entry.at.map(ago).unwrap_or_default();
                            let when_w = if when.is_empty() { 0.0 } else { 86.0 };
                            let label = entry.cmd.replace('\n', " ↵ ");
                            let color = if i == sel { pal.fg } else { pal.muted };
                            let g =
                                elided(ui.ctx(), &label, &mono, color, r.width() - 28.0 - when_w);
                            p.galley(
                                Pos2::new(r.left() + 14.0, r.center().y - g.size().y / 2.0),
                                g,
                                color,
                            );
                            p.text(
                                Pos2::new(r.right() - 14.0, r.center().y),
                                Align2::RIGHT_CENTER,
                                when,
                                small.clone(),
                                pal.muted.gamma_multiply(0.8),
                            );
                        }
                        ui.add_space(4.0);
                        let (foot, _) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), FOOT),
                            Sense::hover(),
                        );
                        let p = ui.painter();
                        p.line_segment(
                            [foot.left_top(), foot.right_top()],
                            Stroke::new(1.0, pal.border),
                        );
                        let left = if original.is_empty() {
                            (o.of)(sel + 1, n)
                        } else {
                            format!(
                                "{}  ·  {}",
                                (o.of)(sel + 1, n),
                                (o.starting_with)(&original)
                            )
                        };
                        let g = elided(ui.ctx(), &left, &small, pal.muted, foot.width() - 330.0);
                        p.galley(
                            Pos2::new(foot.left() + 14.0, foot.center().y - g.size().y / 2.0),
                            g,
                            pal.muted,
                        );
                        p.text(
                            Pos2::new(foot.right() - 14.0, foot.center().y),
                            Align2::RIGHT_CENTER,
                            o.history_keys,
                            small.clone(),
                            pal.muted,
                        );
                    });
            });
        // Su fundido de entrada (el de egui) sale fluido.
        fade_in(ctx, Id::unique("history"));

        if let Some(i) = pick {
            sel = i;
        }
        if sel != before
            && let Some(s) = self.w.sessions.get_mut(self.w.active)
        {
            s.insert_command(&self.w.history[sel].cmd);
        }
        if run {
            self.w.overlay = Overlay::None;
            if let Some(s) = self.w.sessions.get_mut(self.w.active) {
                s.input(b"\r".to_vec());
            }
        } else if keep || other || pick.is_some() || clicked_outside(ctx, out.response.rect) {
            self.w.overlay = Overlay::None;
        } else {
            self.w.overlay = Overlay::History { sel, original };
        }
    }

    /// Sugerencias de carpetas y archivos bajo la palabra que se teclea. Abierta, el teclado
    /// sigue llegando al shell: escribir o borrar rehace la lista con lo nuevo. Tab y las
    /// flechas eligen, y lo elegido queda escrito en el prompt al momento; Enter lo acepta
    /// (sin nada elegido, Enter es del shell) y Esc devuelve la palabra original.
    #[allow(clippy::too_many_arguments)]
    fn complete_ui(
        &mut self,
        ctx: &egui::Context,
        mut target: crate::complete::Target,
        mut items: Vec<crate::complete::Item>,
        mut sel: Option<usize>,
        mut line: String,
        x: f32,
        mut git: crate::complete::GitData,
    ) {
        use crate::complete;
        let Some(s) = self.w.sessions.get(self.w.active) else {
            self.w.overlay = Overlay::None;
            return;
        };
        if s.prompt_line() != Some(line.as_str()) {
            let aliases = complete::shell_aliases();
            let fresh = s
                .prompt_line()
                .and_then(|l| {
                    complete::offer(l, &s.cwd, &|n| aliases.get(n).cloned(), &mut git)
                        .map(|(t, found)| (found, l.to_string(), t))
                })
                .filter(|(found, ..)| !found.is_empty());
            let Some((found, l, t)) = fresh else {
                self.w.overlay = Overlay::None;
                return;
            };
            (items, line, target, sel) = (found, l, t, None);
        }
        let n = items.len();
        let mut next = sel;
        if pressed(ctx, Key::ArrowDown) || pressed(ctx, Key::Tab) {
            next = Some(sel.map_or(0, |i| (i + 1) % n));
        }
        if pressed(ctx, Key::ArrowUp)
            || ctx.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::Tab))
        {
            next = Some(sel.map_or(n - 1, |i| (i + n - 1) % n));
        }
        let mut accept = sel.is_some() && pressed(ctx, Key::Enter);

        const VISIBLE: usize = 8;
        const ROW: f32 = 28.0;
        const FOOT: f32 = 26.0;
        let pal = self.pal.clone();
        let o = &i18n::t().overlay;
        let mono = FontId::monospace(12.5);
        let small = FontId::proportional(11.0);
        let screen = ctx.content_rect();
        let width = 440.0_f32.min(screen.width() - 16.0);
        let height = n.min(VISIBLE) as f32 * ROW + FOOT + 8.0;
        let cursor = s.cursor_px.unwrap_or_else(|| {
            Rect::from_min_size(
                Pos2::new(screen.left() + 16.0, screen.bottom() - 40.0),
                Vec2::new(8.0, 18.0),
            )
        });
        // Debajo de lo que se escribe; si no cabe (prompt al fondo de la ventana), encima.
        let below = cursor.bottom() + 6.0;
        let y = if below + height <= screen.bottom() - 4.0 {
            below
        } else {
            (cursor.top() - height - 6.0).max(screen.top() + crate::TAB_BAR_H)
        };
        let max_x = (screen.right() - width - 8.0).max(screen.left() + 8.0);
        let x_at = x.clamp(screen.left() + 8.0, max_x);
        let shown = next.unwrap_or(0);
        let first = (shown + 1).saturating_sub(VISIBLE);
        let last = (first + VISIBLE).min(n);
        let needle = target.prefix.to_lowercase();

        let out = egui::Area::new(Id::unique("complete"))
            .order(Order::Foreground)
            .fixed_pos(Pos2::new(x_at, y))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(pal.panel)
                    .stroke(Stroke::new(1.0, pal.border))
                    .corner_radius(CornerRadius::same(8))
                    .show(ui, |ui| {
                        ui.set_width(width);
                        ui.spacing_mut().item_spacing = Vec2::ZERO;
                        ui.add_space(4.0);
                        for (i, item) in items.iter().enumerate().take(last).skip(first) {
                            let (r, resp) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), ROW),
                                Sense::click(),
                            );
                            if resp.clicked() {
                                next = Some(i);
                                accept = true;
                            }
                            let chosen = next == Some(i);
                            let p = ui.painter();
                            if chosen {
                                p.rect_filled(
                                    r.shrink2(Vec2::new(4.0, 1.0)),
                                    CornerRadius::same(5),
                                    pal.accent.gamma_multiply(0.22),
                                );
                            } else if resp.hovered() {
                                p.rect_filled(
                                    r.shrink2(Vec2::new(4.0, 1.0)),
                                    CornerRadius::same(5),
                                    pal.fg.gamma_multiply(0.04),
                                );
                            }
                            let icon = Pos2::new(r.left() + 20.0, r.center().y);
                            let icon_color = if chosen { pal.fg } else { pal.muted };
                            if item.is_branch() {
                                crate::branch_icon(p, icon, icon_color);
                            } else if item.kind == crate::complete::Kind::Command {
                                // «>_» de terminal.
                                let st = Stroke::new(1.2, icon_color);
                                let c = icon + Vec2::new(-3.0, 0.0);
                                p.line_segment([c + Vec2::new(-2.5, -3.5), c], st);
                                p.line_segment([c, c + Vec2::new(-2.5, 3.5)], st);
                                p.line_segment(
                                    [c + Vec2::new(2.0, 4.0), c + Vec2::new(7.0, 4.0)],
                                    st,
                                );
                            } else {
                                crate::file_icon(p, icon, item.is_dir(), icon_color);
                            }
                            // El trozo que coincide con lo tecleado va resaltado.
                            let name = if item.is_dir() {
                                format!("{}/", item.name)
                            } else {
                                item.name.clone()
                            };
                            let hit = name
                                .to_lowercase()
                                .find(&needle)
                                .filter(|at| !needle.is_empty() && name.is_char_boundary(*at))
                                .filter(|at| name.is_char_boundary(at + needle.len()));
                            let mut job = egui::text::LayoutJob::default();
                            let mut add = |text: &str, color: Color32| {
                                job.append(
                                    text,
                                    0.0,
                                    egui::TextFormat {
                                        font_id: mono.clone(),
                                        color,
                                        ..Default::default()
                                    },
                                );
                            };
                            let dim = if chosen {
                                pal.fg
                            } else {
                                pal.fg.gamma_multiply(0.7)
                            };
                            match hit {
                                Some(at) => {
                                    add(&name[..at], dim);
                                    add(&name[at..at + needle.len()], pal.accent);
                                    add(&name[at + needle.len()..], dim);
                                }
                                None => add(&name, dim),
                            }
                            let kind = kind_label(item.kind);
                            let kind_w = ui
                                .ctx()
                                .fonts_mut(|f| {
                                    f.layout_no_wrap(kind.to_string(), small.clone(), pal.muted)
                                })
                                .size()
                                .x
                                .max(64.0);
                            job.wrap = egui::text::TextWrapping::truncate_at_width(
                                r.width() - 40.0 - kind_w - 12.0,
                            );
                            let g = ui.ctx().fonts_mut(|f| f.layout_job(job));
                            p.galley(
                                Pos2::new(r.left() + 36.0, r.center().y - g.size().y / 2.0),
                                g,
                                pal.fg,
                            );
                            p.text(
                                Pos2::new(r.right() - 14.0, r.center().y),
                                Align2::RIGHT_CENTER,
                                kind,
                                small.clone(),
                                pal.muted,
                            );
                        }
                        ui.add_space(4.0);
                        let (foot, _) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), FOOT),
                            Sense::hover(),
                        );
                        let p = ui.painter();
                        p.line_segment(
                            [foot.left_top(), foot.right_top()],
                            Stroke::new(1.0, pal.border),
                        );
                        let count = match next {
                            Some(i) => (o.of)(i + 1, n),
                            None => (o.options)(n),
                        };
                        p.text(
                            Pos2::new(foot.left() + 14.0, foot.center().y),
                            Align2::LEFT_CENTER,
                            count,
                            small.clone(),
                            pal.muted,
                        );
                        p.text(
                            Pos2::new(foot.right() - 14.0, foot.center().y),
                            Align2::RIGHT_CENTER,
                            o.complete_keys,
                            small.clone(),
                            pal.muted,
                        );
                    });
            });
        // Su fundido de entrada (el de egui) sale fluido.
        fade_in(ctx, Id::unique("complete"));

        // Lo elegido se escribe en el prompt en lugar de lo que hubiera (la palabra
        // original o la opción anterior).
        if next != sel
            && let (Some(i), Some(s)) = (next, self.w.sessions.get_mut(self.w.active))
        {
            let written = match sel {
                Some(j) => complete::replacement(&target, &items[j]),
                None => target.token.clone(),
            };
            s.replace_word(
                written.chars().count(),
                &complete::replacement(&target, &items[i]),
            );
            match s.prompt_line() {
                Some(l) => line = l.to_string(),
                None => accept = true,
            }
            sel = next;
        }
        if accept || clicked_outside(ctx, out.response.rect) {
            self.w.overlay = Overlay::None;
        } else {
            self.w.overlay = Overlay::Complete {
                target,
                items,
                sel,
                line,
                x,
                git,
            };
        }
    }

    fn palette_items(&self) -> Vec<(String, String, Cmd)> {
        let o = &i18n::t().overlay;
        let commands = [
            (o.new_tab, Cmd::NewTab),
            (o.close_tab, Cmd::CloseTab),
            (o.new_window, Cmd::NewWindow),
            (o.close_window, Cmd::CloseWindow),
            (o.reopen_closed, Cmd::ReopenClosed),
            (o.next_tab, Cmd::NextTab),
            (o.prev_tab, Cmd::PrevTab),
            (o.move_tab_left, Cmd::MoveTabLeft),
            (o.move_tab_right, Cmd::MoveTabRight),
            (o.rename_tab, Cmd::RenameTab),
            (o.next_agent, Cmd::NextAgent),
            (o.prev_agent, Cmd::PrevAgent),
            (o.jump_finished, Cmd::JumpFinished),
            (o.prefs, Cmd::Prefs),
            (o.agents_panel, Cmd::ToggleAgents),
            (o.search_agents, Cmd::SearchAgents),
            (o.ports_panel, Cmd::TogglePorts),
            (o.procs_panel, Cmd::ToggleProcs),
            (o.toggle_panel, Cmd::TogglePanel),
            (o.font_bigger, Cmd::FontBigger),
            (o.font_smaller, Cmd::FontSmaller),
            (o.font_reset, Cmd::FontReset),
            (o.clear, Cmd::Clear),
            (o.open_config, Cmd::OpenConfig),
            (o.restart, Cmd::Restart),
            (o.history, Cmd::History),
        ];
        let mut items: Vec<(String, String, Cmd)> = commands
            .iter()
            .map(|(label, cmd)| (label.to_string(), self.keys.label(*cmd), *cmd))
            .collect();
        for (i, s) in self.w.sessions.iter().enumerate() {
            let key = if i < 9 {
                format!("⌘{}", i + 1)
            } else {
                String::new()
            };
            items.push(((o.go_to)(&s.tab_label()), key, Cmd::GoTab(i)));
        }
        items
    }

    fn palette_ui(&mut self, ctx: &egui::Context, mut query: String, mut sel: usize) {
        if type_into(ctx, &mut query) {
            sel = 0;
        }
        let words: Vec<String> = query
            .to_lowercase()
            .split_whitespace()
            .map(str::to_string)
            .collect();
        let items: Vec<(String, String, Cmd)> = self
            .palette_items()
            .into_iter()
            .filter(|(label, _, _)| {
                let l = label.to_lowercase();
                words.iter().all(|w| l.contains(w))
            })
            .collect();
        if pressed(ctx, Key::ArrowDown) && !items.is_empty() {
            sel = (sel + 1) % items.len();
        }
        if pressed(ctx, Key::ArrowUp) && !items.is_empty() {
            sel = (sel + items.len() - 1) % items.len();
        }
        sel = sel.min(items.len().saturating_sub(1));
        let mut run = pressed(ctx, Key::Enter).then_some(sel);

        let pal = self.pal.clone();
        let o = &i18n::t().overlay;
        let mono = FontId::monospace(13.0);
        let small = FontId::monospace(11.5);
        const VISIBLE: usize = 11;
        let first = sel.saturating_sub(VISIBLE - 1);
        let rect = modal(ctx, &pal, "palette", 600.0, |ui| {
            let (head, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 44.0), Sense::hover());
            let p = ui.painter();
            p.text(
                Pos2::new(head.left() + 18.0, head.center().y),
                Align2::LEFT_CENTER,
                "›",
                mono.clone(),
                pal.accent,
            );
            let shown = if query.is_empty() {
                o.palette_placeholder.to_string()
            } else {
                query.clone()
            };
            let color = if query.is_empty() { pal.muted } else { pal.fg };
            let r = p.text(
                Pos2::new(head.left() + 36.0, head.center().y),
                Align2::LEFT_CENTER,
                shown,
                mono.clone(),
                color,
            );
            let caret_x = if query.is_empty() {
                head.left() + 36.0
            } else {
                r.right() + 1.0
            };
            p.line_segment(
                [
                    Pos2::new(caret_x, head.center().y - 8.0),
                    Pos2::new(caret_x, head.center().y + 8.0),
                ],
                Stroke::new(1.5, pal.accent),
            );
            p.line_segment(
                [head.left_bottom(), head.right_bottom()],
                Stroke::new(1.0, pal.border),
            );

            if items.is_empty() {
                let (r, _) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), Sense::hover());
                ui.painter().text(
                    Pos2::new(r.left() + 36.0, r.center().y),
                    Align2::LEFT_CENTER,
                    o.no_results,
                    mono.clone(),
                    pal.muted,
                );
            }
            for (i, (label, key, _)) in items.iter().enumerate().skip(first).take(VISIBLE) {
                let (r, resp) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::click());
                if resp.hovered() && ui.input(|inp| inp.pointer.delta() != Vec2::ZERO) {
                    sel = i;
                }
                let p = ui.painter();
                if i == sel {
                    p.rect_filled(
                        r.shrink2(Vec2::new(4.0, 1.0)),
                        CornerRadius::same(5),
                        pal.accent.gamma_multiply(0.16),
                    );
                }
                let g = elided(ui.ctx(), label, &mono, pal.fg, r.width() - 150.0);
                p.galley(
                    Pos2::new(r.left() + 36.0, r.center().y - g.size().y / 2.0),
                    g,
                    pal.fg,
                );
                p.text(
                    Pos2::new(r.right() - 18.0, r.center().y),
                    Align2::RIGHT_CENTER,
                    key,
                    small.clone(),
                    pal.muted,
                );
                if resp.clicked() {
                    run = Some(i);
                }
            }
            ui.add_space(6.0);
        });

        if let Some((_, _, cmd)) = run.and_then(|i| items.get(i)) {
            self.w.overlay = Overlay::None;
            self.run(ctx, *cmd);
        } else if clicked_outside(ctx, rect) {
            self.w.overlay = Overlay::None;
        } else {
            self.w.overlay = Overlay::Palette { query, sel };
        }
    }

    /// Valor mostrado de una preferencia y, si es un interruptor, su estado.
    fn pref_value(&self, id: &str) -> (String, Option<bool>) {
        let c = &self.cfg;
        let o = &i18n::t().overlay;
        match id {
            "theme" => {
                let name = config::presets()
                    .into_iter()
                    .find(|(_, t)| *t == c.theme)
                    .map(|(n, _)| n)
                    .unwrap_or(o.custom_theme);
                (name.to_string(), None)
            }
            "font" => {
                let name = self
                    .font_choices
                    .iter()
                    .find(|(_, path, _)| *path == c.font_path)
                    .map(|(n, _, _)| n.clone())
                    .unwrap_or_else(|| o.custom_font.into());
                (name, None)
            }
            "font_size" => (format!("{:.0} pt", c.font_size), None),
            "line_height" => (format!("{:.2}", c.line_height), None),
            "padding" => (format!("{:.0} pt", c.padding), None),
            "opacity" => ((o.percent)(c.opacity * 100.0), None),
            "cursor" => (
                match c.cursor.as_str() {
                    "bar" | "beam" => o.cursor_bar,
                    "underline" => o.cursor_underline,
                    _ => o.cursor_block,
                }
                .to_string(),
                None,
            ),
            "cursor_blink" => (String::new(), Some(c.cursor_blink)),
            "scrollback" => ((o.lines)(c.scrollback), None),
            "blur" => (String::new(), Some(c.blur)),
            "restore_tabs" => (String::new(), Some(c.restore_tabs)),
            "notify" => (String::new(), Some(c.notify)),
            "history_popup" => (String::new(), Some(c.history_popup)),
            "complete_popup" => (String::new(), Some(c.complete_popup)),
            "status_bar" => (String::new(), Some(c.status_bar)),
            "shell_integration" => (String::new(), Some(c.shell_integration)),
            "editor" if c.editor.is_empty() => (o.editor_default.to_string(), None),
            "editor" => (c.editor.clone(), None),
            "secure_input" => (
                match c.secure_mode() {
                    SecureInput::Off => o.secure_off,
                    SecureInput::Passwords => o.secure_passwords,
                    SecureInput::Always => o.secure_always,
                }
                .to_string(),
                None,
            ),
            "check_updates" => (String::new(), Some(c.check_updates)),
            "show_agents" => (String::new(), Some(c.show_agents)),
            "agents_side" if c.agents_on_right() => (o.side_right.to_string(), None),
            "agents_side" => (o.side_left.to_string(), None),
            "show_ports" => (String::new(), Some(c.show_ports)),
            "show_processes" => (String::new(), Some(c.show_processes)),
            _ => (String::new(), None),
        }
    }

    pub fn pref_change(&mut self, ctx: &egui::Context, id: &str, dir: i32) {
        let old = self.cfg.clone();
        let cycle = |len: usize, current: Option<usize>| -> usize {
            match current {
                Some(i) => (i as i32 + dir).rem_euclid(len as i32) as usize,
                None => 0,
            }
        };
        let c = &mut self.cfg;
        match id {
            "theme" => {
                let presets = config::presets();
                let i = cycle(
                    presets.len(),
                    presets.iter().position(|(_, t)| *t == c.theme),
                );
                c.theme = presets[i].1.clone();
            }
            "font" if !self.font_choices.is_empty() => {
                let fonts = &self.font_choices;
                let i = cycle(
                    fonts.len(),
                    fonts.iter().position(|(_, path, _)| *path == c.font_path),
                );
                c.font_path = fonts[i].1.clone();
                c.font_bold_path = fonts[i].2.clone();
            }
            "font_size" => c.font_size = (c.font_size + dir as f32).clamp(8.0, 32.0),
            "line_height" => {
                c.line_height = ((c.line_height + 0.05 * dir as f32) * 100.0)
                    .round()
                    .clamp(90.0, 200.0)
                    / 100.0
            }
            "padding" => c.padding = (c.padding + 2.0 * dir as f32).clamp(0.0, 32.0),
            "opacity" => {
                c.opacity = (c.opacity * 100.0 + 5.0 * dir as f32)
                    .round()
                    .clamp(30.0, 100.0)
                    / 100.0
            }
            "cursor" => {
                let kinds = ["block", "bar", "underline"];
                let i = cycle(kinds.len(), kinds.iter().position(|k| *k == c.cursor));
                c.cursor = kinds[i].to_string();
            }
            "scrollback" => {
                c.scrollback =
                    (c.scrollback as i64 + 1000 * dir as i64).clamp(1000, 100_000) as usize
            }
            "blur" => c.blur = !c.blur,
            "cursor_blink" => c.cursor_blink = !c.cursor_blink,
            "restore_tabs" => c.restore_tabs = !c.restore_tabs,
            "notify" => c.notify = !c.notify,
            "history_popup" => c.history_popup = !c.history_popup,
            "complete_popup" => c.complete_popup = !c.complete_popup,
            "status_bar" => c.status_bar = !c.status_bar,
            "shell_integration" => c.shell_integration = !c.shell_integration,
            "editor" => {
                let editors = ["", "code", "cursor", "zed", "subl", "idea", "xed"];
                let i = cycle(editors.len(), editors.iter().position(|e| *e == c.editor));
                c.editor = editors[i].to_string();
            }
            "secure_input" => {
                let modes = [
                    SecureInput::Off,
                    SecureInput::Passwords,
                    SecureInput::Always,
                ];
                let now = c.secure_mode();
                let i = cycle(modes.len(), modes.iter().position(|m| *m == now));
                c.set_secure_mode(modes[i]);
            }
            "check_updates" => c.check_updates = !c.check_updates,
            "show_agents" => c.show_agents = !c.show_agents,
            "agents_side" => {
                c.agents_side = if c.agents_on_right() {
                    "izquierda"
                } else {
                    "derecha"
                }
                .into()
            }
            "show_ports" => c.show_ports = !c.show_ports,
            "show_processes" => c.show_processes = !c.show_processes,
            "open_file" => {
                self.run(ctx, Cmd::OpenConfig);
                return;
            }
            _ => {}
        }
        if self.cfg != old {
            self.commit_cfg(&old);
        }
    }

    fn prefs_ui(&mut self, ctx: &egui::Context, mut sel: usize) {
        let n = PREF_ROWS.len();
        if pressed(ctx, Key::ArrowDown) {
            sel = (sel + 1) % n;
        }
        if pressed(ctx, Key::ArrowUp) {
            sel = (sel + n - 1) % n;
        }
        let mut change: Option<(usize, i32)> = None;
        if pressed(ctx, Key::ArrowRight) || pressed(ctx, Key::Enter) || pressed(ctx, Key::Space) {
            change = Some((sel, 1));
        }
        if pressed(ctx, Key::ArrowLeft) {
            change = Some((sel, -1));
        }

        let pal = self.pal.clone();
        let o = &i18n::t().overlay;
        let mono = FontId::monospace(13.0);
        let small = FontId::monospace(11.0);
        let values: Vec<(String, Option<bool>)> =
            PREF_ROWS.iter().map(|id| self.pref_value(id)).collect();
        let rect = modal(ctx, &pal, "prefs", 620.0, |ui| {
            let (head, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 44.0), Sense::hover());
            let p = ui.painter();
            p.text(
                Pos2::new(head.left() + 18.0, head.center().y),
                Align2::LEFT_CENTER,
                o.prefs_title,
                FontId::proportional(14.5),
                pal.fg,
            );
            p.text(
                Pos2::new(head.right() - 18.0, head.center().y),
                Align2::RIGHT_CENTER,
                "~/.config/colibri/config.toml",
                small.clone(),
                pal.muted,
            );
            p.line_segment(
                [head.left_bottom(), head.right_bottom()],
                Stroke::new(1.0, pal.border),
            );

            for (i, id) in PREF_ROWS.iter().enumerate() {
                if let Some(title) = section_before(o, id) {
                    let (r, _) = ui
                        .allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::hover());
                    ui.painter().text(
                        Pos2::new(r.left() + 18.0, r.bottom() - 9.0),
                        Align2::LEFT_CENTER,
                        title,
                        small.clone(),
                        pal.muted,
                    );
                }
                let (r, resp) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::click());
                if resp.hovered() && ui.input(|inp| inp.pointer.delta() != Vec2::ZERO) {
                    sel = i;
                }
                let p = ui.painter();
                if i == sel {
                    p.rect_filled(
                        r.shrink2(Vec2::new(4.0, 1.0)),
                        CornerRadius::same(5),
                        pal.accent.gamma_multiply(0.16),
                    );
                }
                p.text(
                    Pos2::new(r.left() + 28.0, r.center().y),
                    Align2::LEFT_CENTER,
                    pref_label(o, id),
                    mono.clone(),
                    pal.fg,
                );
                let (value, toggle) = &values[i];
                let right = r.right() - 18.0;
                if let Some(on) = toggle {
                    let sw = Rect::from_min_size(
                        Pos2::new(right - 30.0, r.center().y - 8.0),
                        Vec2::new(30.0, 16.0),
                    );
                    p.rect_filled(
                        sw,
                        CornerRadius::same(8),
                        if *on { pal.accent } else { pal.border },
                    );
                    let knob = if *on {
                        sw.right() - 8.0
                    } else {
                        sw.left() + 8.0
                    };
                    p.circle_filled(Pos2::new(knob, sw.center().y), 6.0, Color32::WHITE);
                    if resp.clicked() {
                        change = Some((i, 1));
                    }
                } else if value.is_empty() {
                    p.text(
                        Pos2::new(right, r.center().y),
                        Align2::RIGHT_CENTER,
                        "↩",
                        mono.clone(),
                        pal.muted,
                    );
                    if resp.clicked() {
                        change = Some((i, 1));
                    }
                } else {
                    p.text(
                        Pos2::new(right, r.center().y),
                        Align2::RIGHT_CENTER,
                        "›",
                        mono.clone(),
                        pal.muted,
                    );
                    let v = p.text(
                        Pos2::new(right - 18.0, r.center().y),
                        Align2::RIGHT_CENTER,
                        value,
                        mono.clone(),
                        pal.fg,
                    );
                    p.text(
                        Pos2::new(v.left() - 12.0, r.center().y),
                        Align2::RIGHT_CENTER,
                        "‹",
                        mono.clone(),
                        pal.muted,
                    );
                    if *id == "theme" {
                        for (k, color) in [1usize, 2, 3, 4, 5, 6].iter().enumerate() {
                            let x = v.left() - 40.0 - (5 - k) as f32 * 11.0;
                            let sq = Rect::from_center_size(
                                Pos2::new(x, r.center().y),
                                Vec2::splat(8.0),
                            );
                            p.rect_filled(sq, 1.0, pal.ansi[*color]);
                        }
                    }
                    if resp.clicked() {
                        // Clic en la mitad izquierda del valor retrocede; en el resto avanza.
                        let back = resp
                            .interact_pointer_pos()
                            .is_some_and(|pos| pos.x < v.left() && pos.x > v.left() - 30.0);
                        change = Some((i, if back { -1 } else { 1 }));
                    }
                }
            }

            let (foot, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 38.0), Sense::hover());
            let p = ui.painter();
            p.line_segment(
                [foot.left_top(), foot.right_top()],
                Stroke::new(1.0, pal.border),
            );
            let hint = match PREF_ROWS[sel] {
                "font" => o.hint_font,
                "opacity" => o.hint_opacity,
                "blur" => o.hint_blur,
                "scrollback" => o.hint_scrollback,
                "theme" => o.hint_theme,
                "show_agents" | "show_ports" | "show_processes" => o.hint_panels,
                "agents_side" => o.hint_agents_side,
                "secure_input" => o.hint_secure_input,
                _ => o.hint_default,
            };
            p.text(
                Pos2::new(foot.left() + 18.0, foot.center().y),
                Align2::LEFT_CENTER,
                hint,
                small.clone(),
                pal.muted,
            );
            p.text(
                Pos2::new(foot.right() - 18.0, foot.center().y),
                Align2::RIGHT_CENTER,
                "↑↓  ←→  esc",
                small.clone(),
                pal.muted,
            );
        });

        if clicked_outside(ctx, rect) {
            self.w.overlay = Overlay::None;
            return;
        }
        self.w.overlay = Overlay::Prefs { sel };
        if let Some((i, dir)) = change {
            self.pref_change(ctx, PREF_ROWS[i], dir);
        }
    }

    fn rename_ui(&mut self, ctx: &egui::Context, mut text: String) {
        type_into(ctx, &mut text);
        let pal = self.pal.clone();
        let o = &i18n::t().overlay;
        let mono = FontId::monospace(13.0);
        let small = FontId::monospace(11.0);
        let rect = modal(ctx, &pal, "rename", 460.0, |ui| {
            let (head, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::hover());
            ui.painter().text(
                Pos2::new(head.left() + 18.0, head.center().y + 3.0),
                Align2::LEFT_CENTER,
                o.rename_title,
                small.clone(),
                pal.muted,
            );
            let (row, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 38.0), Sense::hover());
            let p = ui.painter();
            let shown = if text.is_empty() {
                o.rename_placeholder.to_string()
            } else {
                text.clone()
            };
            let color = if text.is_empty() { pal.muted } else { pal.fg };
            let r = p.text(
                Pos2::new(row.left() + 18.0, row.center().y),
                Align2::LEFT_CENTER,
                shown,
                mono.clone(),
                color,
            );
            let caret_x = if text.is_empty() {
                row.left() + 18.0
            } else {
                r.right() + 1.0
            };
            p.line_segment(
                [
                    Pos2::new(caret_x, row.center().y - 8.0),
                    Pos2::new(caret_x, row.center().y + 8.0),
                ],
                Stroke::new(1.5, pal.accent),
            );
            let (foot, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::hover());
            let p = ui.painter();
            p.line_segment(
                [foot.left_top(), foot.right_top()],
                Stroke::new(1.0, pal.border),
            );
            p.text(
                Pos2::new(foot.left() + 18.0, foot.center().y),
                Align2::LEFT_CENTER,
                o.rename_keys,
                small.clone(),
                pal.muted,
            );
        });
        if pressed(ctx, Key::Enter) {
            if let Some(s) = self.w.sessions.get_mut(self.w.active) {
                s.name = text.trim().to_string();
            }
            self.w.overlay = Overlay::None;
        } else if clicked_outside(ctx, rect) {
            self.w.overlay = Overlay::None;
        } else {
            self.w.overlay = Overlay::Rename { text };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn carpeta_elegida_para_pestana_nueva() {
        let cwd = Path::new("/p/colibri");
        assert_eq!(
            pick_target(cwd, PickKind::Dir, "src"),
            Some(PathBuf::from("/p/colibri/src"))
        );
        assert_eq!(
            pick_target(cwd, PickKind::Parent, ".."),
            Some(PathBuf::from("/p"))
        );
        assert_eq!(pick_target(cwd, PickKind::File, "a.rs"), None);
    }

    #[test]
    fn lista_de_una_carpeta() {
        let dir = std::env::temp_dir().join(format!("colibri-picks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Beta")).unwrap();
        std::fs::create_dir_all(dir.join("alfa")).unwrap();
        std::fs::write(dir.join("nota.txt"), "").unwrap();
        let names = |picks: Vec<Pick>| -> Vec<(PickKind, String)> {
            picks.into_iter().map(|p| (p.kind, p.name)).collect()
        };
        // `..` primero, carpetas por nombre sin mirar mayúsculas y luego archivos.
        assert_eq!(
            names(dir_picks(&dir, true)),
            vec![
                (PickKind::Parent, "..".to_string()),
                (PickKind::Dir, "alfa".to_string()),
                (PickKind::Dir, "Beta".to_string()),
                (PickKind::File, "nota.txt".to_string()),
            ]
        );
        // Para una pestaña nueva, solo carpetas.
        assert_eq!(names(dir_picks(&dir, false)).len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
