//! Ventanas flotantes sobre la terminal: paleta de comandos, preferencias y renombrar pestaña.

use crate::config::{self, Palette};
use crate::{App, Cmd, elided};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Id, Key, LayerId, Modifiers, Order, Pos2, Rect,
    Sense, Stroke, Vec2,
};

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
    /// Comandos usados antes; `sel` 0 es el más reciente.
    History {
        query: String,
        sel: usize,
    },
}

const PREF_ROWS: [(&str, &str); 16] = [
    ("theme", "Tema"),
    ("font", "Fuente"),
    ("font_size", "Tamaño de fuente"),
    ("line_height", "Interlineado"),
    ("padding", "Margen"),
    ("cursor", "Cursor"),
    ("opacity", "Opacidad del fondo"),
    ("blur", "Desenfocar el fondo"),
    ("scrollback", "Historial por pestaña"),
    ("restore_tabs", "Recuperar pestañas al abrir"),
    ("notify", "Notificar cuando algo termina"),
    ("history_popup", "Historial de comandos con ↑"),
    ("show_agents", "Agentes"),
    ("show_ports", "Puertos"),
    ("show_processes", "Procesos"),
    ("open_file", "Abrir config.toml en el editor…"),
];

fn section_before(id: &str) -> Option<&'static str> {
    match id {
        "theme" => Some("APARIENCIA"),
        "scrollback" => Some("TERMINAL"),
        "show_agents" => Some("PANELES VISIBLES AL ABRIR"),
        "open_file" => Some("AVANZADO"),
        _ => None,
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
        if self.overlay == Overlay::None {
            return;
        }
        if pressed(ctx, Key::Escape) {
            self.overlay = Overlay::None;
            return;
        }
        match self.overlay.clone() {
            Overlay::None => {}
            Overlay::Palette { query, sel } => self.palette_ui(ctx, query, sel),
            Overlay::Prefs { sel } => self.prefs_ui(ctx, sel),
            Overlay::Rename { text } => self.rename_ui(ctx, text),
            Overlay::History { query, sel } => self.history_ui(ctx, query, sel),
        }
    }

    /// Lista de comandos anclada al cursor: el más reciente abajo, ↑ va hacia los antiguos.
    fn history_ui(&mut self, ctx: &egui::Context, mut query: String, mut sel: usize) {
        if type_into(ctx, &mut query) {
            sel = 0;
        }
        let words: Vec<String> = query
            .to_lowercase()
            .split_whitespace()
            .map(str::to_string)
            .collect();
        let items: Vec<&String> = self
            .history
            .iter()
            .filter(|c| {
                let l = c.to_lowercase();
                words.iter().all(|w| l.contains(w))
            })
            .collect();
        if pressed(ctx, Key::ArrowUp) && !items.is_empty() {
            sel = (sel + 1).min(items.len() - 1);
        }
        if pressed(ctx, Key::ArrowDown) {
            // Bajar desde el más reciente cierra la lista, como volver al prompt vacío.
            if sel == 0 {
                self.overlay = Overlay::None;
                return;
            }
            sel -= 1;
        }
        sel = sel.min(items.len().saturating_sub(1));
        let mut pick = pressed(ctx, Key::Enter)
            .then_some(sel)
            .filter(|_| !items.is_empty());

        const VISIBLE: usize = 8;
        const ROW: f32 = 26.0;
        const FOOT: f32 = 30.0;
        let pal = self.pal.clone();
        let mono = FontId::monospace(12.5);
        let small = FontId::proportional(11.0);
        let screen = ctx.content_rect();
        let width = 640.0_f32.min(screen.width() - 16.0);
        let height = items.len().clamp(1, VISIBLE) as f32 * ROW + FOOT + 8.0;
        let cursor = self
            .sessions
            .get(self.active)
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
        let max_x = (screen.right() - width - 8.0).max(screen.left() + 8.0);
        let x = cursor.left().clamp(screen.left() + 8.0, max_x);
        let first = sel.saturating_sub(VISIBLE - 1);
        let last = (first + VISIBLE).min(items.len());

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
                        if items.is_empty() {
                            let (r, _) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), ROW),
                                Sense::hover(),
                            );
                            ui.painter().text(
                                Pos2::new(r.left() + 14.0, r.center().y),
                                Align2::LEFT_CENTER,
                                "Ningún comando coincide",
                                mono.clone(),
                                pal.muted,
                            );
                        }
                        for i in (first..last).rev() {
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
                            let label = items[i].replace('\n', " ↵ ");
                            let color = if i == sel { pal.fg } else { pal.muted };
                            let g = elided(ui.ctx(), &label, &mono, color, r.width() - 28.0);
                            p.galley(
                                Pos2::new(r.left() + 14.0, r.center().y - g.size().y / 2.0),
                                g,
                                color,
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
                        let (shown, color) = if query.is_empty() {
                            ("Escribe para filtrar".to_string(), pal.muted)
                        } else {
                            (query.clone(), pal.fg)
                        };
                        p.text(
                            Pos2::new(foot.left() + 14.0, foot.center().y),
                            Align2::LEFT_CENTER,
                            format!("›  {shown}"),
                            mono.clone(),
                            color,
                        );
                        p.text(
                            Pos2::new(foot.right() - 14.0, foot.center().y),
                            Align2::RIGHT_CENTER,
                            "↑↓ elegir  ·  ↩ insertar  ·  esc cerrar",
                            small.clone(),
                            pal.muted,
                        );
                    });
            });

        if let Some(i) = pick {
            let cmd = items[i].clone();
            self.overlay = Overlay::None;
            if let Some(s) = self.sessions.get_mut(self.active) {
                s.insert_command(&cmd);
            }
        } else if clicked_outside(ctx, out.response.rect) {
            self.overlay = Overlay::None;
        } else {
            self.overlay = Overlay::History { query, sel };
        }
    }

    fn palette_items(&self) -> Vec<(String, String, Cmd)> {
        let commands = [
            ("Nueva pestaña", Cmd::NewTab),
            ("Cerrar pestaña", Cmd::CloseTab),
            ("Pestaña siguiente", Cmd::NextTab),
            ("Pestaña anterior", Cmd::PrevTab),
            ("Renombrar pestaña…", Cmd::RenameTab),
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
        for (i, s) in self.sessions.iter().enumerate() {
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
            self.overlay = Overlay::None;
            self.run(ctx, *cmd);
        } else if clicked_outside(ctx, rect) {
            self.overlay = Overlay::None;
        } else {
            self.overlay = Overlay::Palette { query, sel };
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
            "scrollback" => (format!("{} líneas", c.scrollback), None),
            "blur" => (String::new(), Some(c.blur)),
            "restore_tabs" => (String::new(), Some(c.restore_tabs)),
            "notify" => (String::new(), Some(c.notify)),
            "history_popup" => (String::new(), Some(c.history_popup)),
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
            "restore_tabs" => c.restore_tabs = !c.restore_tabs,
            "notify" => c.notify = !c.notify,
            "history_popup" => c.history_popup = !c.history_popup,
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
            self.commit_cfg(ctx, &old);
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
                    "Fuentes monoespaciadas instaladas en tu Mac. Otras: font_path en el archivo."
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
            self.overlay = Overlay::None;
            return;
        }
        self.overlay = Overlay::Prefs { sel };
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
            if let Some(s) = self.sessions.get_mut(self.active) {
                s.name = text.trim().to_string();
            }
            self.overlay = Overlay::None;
        } else if clicked_outside(ctx, rect) {
            self.overlay = Overlay::None;
        } else {
            self.overlay = Overlay::Rename { text };
        }
    }
}
