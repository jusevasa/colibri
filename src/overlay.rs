//! Ventanas flotantes sobre la terminal: paleta de comandos, preferencias y renombrar pestaña.

use crate::config::{self, Palette};
use crate::{App, Cmd, elided};
use egui::{
    self, Align2, Color32, CornerRadius, FontId, Id, Key, LayerId, Modifiers, Order, Pos2, Rect,
    Sense, Stroke, Vec2,
};

#[derive(Clone, Copy, PartialEq)]
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
    /// Carpetas y archivos que encajan con la palabra que se teclea en el prompt.
    /// Sin elegir nada (`sel` None) la lista solo se muestra; al elegir, la opción queda
    /// escrita en el prompt. `line` es la línea del prompt para la que vale la lista (si
    /// cambia es que se siguió escribiendo y se rehace) y `x`, dónde se ancla.
    Complete {
        target: crate::complete::Target,
        items: Vec<crate::complete::Item>,
        sel: Option<usize>,
        line: String,
        x: f32,
    },
}

const PREF_ROWS: [(&str, &str); 22] = [
    ("theme", "Tema"),
    ("font", "Fuente"),
    ("font_size", "Tamaño de fuente"),
    ("line_height", "Interlineado"),
    ("padding", "Margen"),
    ("cursor", "Cursor"),
    ("cursor_blink", "Cursor parpadeante"),
    ("opacity", "Opacidad del fondo"),
    ("blur", "Desenfocar el fondo"),
    ("scrollback", "Historial por pestaña"),
    ("restore_tabs", "Recuperar pestañas al abrir"),
    ("notify", "Notificar cuando algo termina"),
    ("history_popup", "Historial de comandos con ↑"),
    ("complete_popup", "Sugerir carpetas y archivos con ⇥"),
    ("status_bar", "Carpeta y rama en cajas, en el prompt"),
    ("shell_integration", "Integración con zsh (pestañas nuevas)"),
    ("editor", "Abrir archivos (⌘+clic) con"),
    ("secure_input", "Entrada segura de teclado"),
    ("show_agents", "Agentes"),
    ("show_ports", "Puertos"),
    ("show_processes", "Procesos"),
    ("open_file", "Abrir config.toml en el editor…"),
];

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

fn section_before(id: &str) -> Option<&'static str> {
    match id {
        "theme" => Some("APARIENCIA"),
        "scrollback" => Some("TERMINAL"),
        "show_agents" => Some("PANELES VISIBLES AL ABRIR"),
        "open_file" => Some("AVANZADO"),
        _ => None,
    }
}

/// «hace 5 min», «hace 3 h»… para una fecha en segundos Unix.
fn ago(at: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    match now.saturating_sub(at) {
        s if s < 60 => "ahora".to_string(),
        s if s < 3600 => format!("hace {} min", s / 60),
        s if s < 86_400 => format!("hace {} h", s / 3600),
        s => format!("hace {} d", s / 86_400),
    }
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

/// Dibuja un cuadro centrado arriba, con el fondo atenuado. Devuelve su rectángulo.
fn modal(
    ctx: &egui::Context,
    pal: &Palette,
    id: &str,
    width: f32,
    add: impl FnOnce(&mut egui::Ui),
) -> Rect {
    let screen = ctx.content_rect();
    ctx.layer_painter(LayerId::new(Order::Middle, Id::unique("overlay-dim")))
        .rect_filled(screen, 0.0, Color32::from_black_alpha(110));
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
            } => self.complete_ui(ctx, target, items, sel, line, x),
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
        if preview {
            let path = items.get(sel).and_then(|p| {
                let cwd = &self.w.sessions.get(self.w.active)?.cwd;
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
                            ("Buscar rama…".to_string(), pal.muted)
                        } else {
                            (
                                "Buscar carpeta o archivo…  ·  espacio: vista previa".to_string(),
                                pal.muted,
                            )
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
                                "Nada coincide",
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
                                PickKind::CurrentBranch => (item.name.clone(), pal.done, "actual"),
                                PickKind::Branch => (item.name.clone(), pal.fg, ""),
                                PickKind::Parent => {
                                    p.text(
                                        icon_at,
                                        Align2::CENTER_CENTER,
                                        "↑",
                                        mono.clone(),
                                        pal.muted,
                                    );
                                    (".. (carpeta superior)".to_string(), pal.muted, "")
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

        if let Some(i) = pick {
            let (kind, name) = (items[i].kind, items[i].name.clone());
            self.w.overlay = Overlay::None;
            if let Some(s) = self.w.sessions.get_mut(self.w.active) {
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
        } else {
            self.w.overlay = Overlay::Picker { query, sel };
        }
    }

    /// Cierra el overlay deshaciendo lo que hubiera dejado a medias en el prompt: el
    /// historial devuelve lo que estaba escrito y las sugerencias, la palabra original.
    pub fn cancel_overlay(&mut self) {
        let overlay = std::mem::replace(&mut self.w.overlay, Overlay::None);
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
                            format!("{} de {n}", sel + 1)
                        } else {
                            format!("{} de {n}  ·  empiezan por «{original}»", sel + 1)
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
                            "↑↓ elegir  ·  ↩ ejecutar  ·  ⇥ editar  ·  esc cancelar",
                            small.clone(),
                            pal.muted,
                        );
                    });
            });

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
    fn complete_ui(
        &mut self,
        ctx: &egui::Context,
        mut target: crate::complete::Target,
        mut items: Vec<crate::complete::Item>,
        mut sel: Option<usize>,
        mut line: String,
        x: f32,
    ) {
        use crate::complete;
        let Some(s) = self.w.sessions.get(self.w.active) else {
            self.w.overlay = Overlay::None;
            return;
        };
        if s.prompt_line() != Some(line.as_str()) {
            let fresh = s
                .prompt_line()
                .and_then(|l| complete::target(l).map(|t| (l.to_string(), t)))
                .map(|(l, t)| (complete::candidates(&s.cwd, &t), l, t))
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
                            if item.command {
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
                                crate::file_icon(p, icon, item.is_dir, icon_color);
                            }
                            // El trozo que coincide con lo tecleado va resaltado.
                            let name = if item.is_dir {
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
                            let kind = if item.command {
                                "Comando"
                            } else if item.is_dir {
                                "Carpeta"
                            } else {
                                "Archivo"
                            };
                            let kind_w = 64.0;
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
                            Some(i) => format!("{} de {n}", i + 1),
                            None => format!("{n} opciones"),
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
                            "⇥ ↑↓ elegir  ·  escribe para filtrar  ·  esc cerrar",
                            small.clone(),
                            pal.muted,
                        );
                    });
            });

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
            };
        }
    }

    fn palette_items(&self) -> Vec<(String, String, Cmd)> {
        let commands = [
            ("Nueva pestaña", Cmd::NewTab),
            ("Cerrar pestaña", Cmd::CloseTab),
            ("Nueva ventana", Cmd::NewWindow),
            ("Cerrar ventana", Cmd::CloseWindow),
            ("Reabrir lo último cerrado", Cmd::ReopenClosed),
            ("Pestaña siguiente", Cmd::NextTab),
            ("Pestaña anterior", Cmd::PrevTab),
            ("Mover pestaña a la izquierda", Cmd::MoveTabLeft),
            ("Mover pestaña a la derecha", Cmd::MoveTabRight),
            ("Renombrar pestaña…", Cmd::RenameTab),
            ("Agente siguiente", Cmd::NextAgent),
            ("Agente anterior", Cmd::PrevAgent),
            ("Ir al agente que terminó", Cmd::JumpFinished),
            ("Preferencias: tema, fuente, tamaño…", Cmd::Prefs),
            ("Panel de agentes", Cmd::ToggleAgents),
            ("Panel de puertos", Cmd::TogglePorts),
            ("Panel de procesos (CPU y memoria)", Cmd::ToggleProcs),
            ("Mostrar u ocultar paneles", Cmd::TogglePanel),
            ("Agrandar texto", Cmd::FontBigger),
            ("Reducir texto", Cmd::FontSmaller),
            ("Tamaño de texto original", Cmd::FontReset),
            ("Limpiar pantalla e historial", Cmd::Clear),
            ("Abrir config.toml en el editor", Cmd::OpenConfig),
            ("Reiniciar Colibrí con la última compilación", Cmd::Restart),
            ("Historial de comandos", Cmd::History),
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
            items.push((format!("Ir a: {}", s.tab_label()), key, Cmd::GoTab(i)));
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
                "Escribe un comando o el nombre de una pestaña".to_string()
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
                    "Sin resultados",
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
        match id {
            "theme" => {
                let name = config::presets()
                    .into_iter()
                    .find(|(_, t)| *t == c.theme)
                    .map(|(n, _)| n)
                    .unwrap_or("Personalizado");
                (name.to_string(), None)
            }
            "font" => {
                let name = self
                    .font_choices
                    .iter()
                    .find(|(_, path, _)| *path == c.font_path)
                    .map(|(n, _, _)| n.clone())
                    .unwrap_or_else(|| "Personalizada".into());
                (name, None)
            }
            "font_size" => (format!("{:.0} pt", c.font_size), None),
            "line_height" => (format!("{:.2}", c.line_height), None),
            "padding" => (format!("{:.0} pt", c.padding), None),
            "opacity" => (format!("{:.0} %", c.opacity * 100.0), None),
            "cursor" => (
                match c.cursor.as_str() {
                    "bar" | "beam" => "Barra",
                    "underline" => "Subrayado",
                    _ => "Bloque",
                }
                .to_string(),
                None,
            ),
            "cursor_blink" => (String::new(), Some(c.cursor_blink)),
            "scrollback" => (format!("{} líneas", c.scrollback), None),
            "blur" => (String::new(), Some(c.blur)),
            "restore_tabs" => (String::new(), Some(c.restore_tabs)),
            "notify" => (String::new(), Some(c.notify)),
            "history_popup" => (String::new(), Some(c.history_popup)),
            "complete_popup" => (String::new(), Some(c.complete_popup)),
            "status_bar" => (String::new(), Some(c.status_bar)),
            "shell_integration" => (String::new(), Some(c.shell_integration)),
            "editor" if c.editor.is_empty() => ("Lo que diga macOS".to_string(), None),
            "editor" => (c.editor.clone(), None),
            "secure_input" => (String::new(), Some(c.secure_input)),
            "show_agents" => (String::new(), Some(c.show_agents)),
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
            "secure_input" => c.secure_input = !c.secure_input,
            "show_agents" => c.show_agents = !c.show_agents,
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
        let mono = FontId::monospace(13.0);
        let small = FontId::monospace(11.0);
        let values: Vec<(String, Option<bool>)> = PREF_ROWS
            .iter()
            .map(|(id, _)| self.pref_value(id))
            .collect();
        let rect = modal(ctx, &pal, "prefs", 620.0, |ui| {
            let (head, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 44.0), Sense::hover());
            let p = ui.painter();
            p.text(
                Pos2::new(head.left() + 18.0, head.center().y),
                Align2::LEFT_CENTER,
                "Preferencias",
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

            for (i, (id, label)) in PREF_ROWS.iter().enumerate() {
                if let Some(title) = section_before(id) {
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
                    *label,
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
            let hint = match PREF_ROWS[sel].0 {
                "font" => {
                    "Menlo, Hack (incluida) y las monoespaciadas de tu Mac. Otras: font_path en el archivo."
                }
                "opacity" => "Menos de 100 % deja ver lo que hay detrás de la ventana.",
                "blur" => "Solo se nota con la opacidad por debajo de 100 %.",
                "scrollback" => "Se aplica a las pestañas que abras a partir de ahora.",
                "theme" => "Colores a medida: sección [theme] del archivo.",
                "show_agents" | "show_ports" | "show_processes" => {
                    "Qué apartados aparecen abiertos al iniciar Colibrí."
                }
                _ => "Los cambios se guardan al momento.",
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
            self.pref_change(ctx, PREF_ROWS[i].0, dir);
        }
    }

    fn rename_ui(&mut self, ctx: &egui::Context, mut text: String) {
        type_into(ctx, &mut text);
        let pal = self.pal.clone();
        let mono = FontId::monospace(13.0);
        let small = FontId::monospace(11.0);
        let rect = modal(ctx, &pal, "rename", 460.0, |ui| {
            let (head, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::hover());
            ui.painter().text(
                Pos2::new(head.left() + 18.0, head.center().y + 3.0),
                Align2::LEFT_CENTER,
                "Nombre de la pestaña",
                small.clone(),
                pal.muted,
            );
            let (row, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 38.0), Sense::hover());
            let p = ui.painter();
            let shown = if text.is_empty() {
                "automático (carpeta o agente)".to_string()
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
                "↩ guardar  ·  vacío = automático  ·  esc cancelar",
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
