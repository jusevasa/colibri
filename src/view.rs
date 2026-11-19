//! Dibuja la rejilla de la terminal y traduce teclado/ratón a bytes para el PTY.

use crate::config::Palette;
use crate::session::Session;
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor};
use std::path::{Path, PathBuf};

use egui::{
    self, Align2, Color32, FontId, Key, Modifiers, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2,
};

pub struct Metrics {
    pub font: FontId,
    pub bold: FontId,
    pub cell_w: f32,
    pub cell_h: f32,
    pub text_dy: f32,
    pub pad: f32,
    /// Forma del cursor cuando el programa no pide una.
    pub cursor: CursorShape,
    /// El cursor parpadea mientras no se escribe.
    pub blink: bool,
    /// Con qué se abre un archivo al hacer ⌘+clic en su ruta (vacío = lo que diga macOS).
    pub editor: String,
}

thread_local! {
    /// Glifos ya maquetados para la rejilla con instancias (solo el hilo de la interfaz).
    static GLYPHS: std::cell::RefCell<crate::grid::GlyphCache> = Default::default();
}

struct TextRun {
    row: i32,
    col: usize,
    next_col: usize,
    text: String,
    fg: Color32,
    bold: bool,
    ascii: bool,
}

struct BgRun {
    row: i32,
    col: usize,
    end: usize,
    color: Color32,
}

fn dim(c: Color32) -> Color32 {
    Color32::from_rgb(
        (c.r() as u16 * 2 / 3) as u8,
        (c.g() as u16 * 2 / 3) as u8,
        (c.b() as u16 * 2 / 3) as u8,
    )
}

fn named(n: NamedColor, pal: &Palette) -> Color32 {
    let i = n as usize;
    match i {
        0..=15 => pal.ansi[i],
        257 => pal.bg,
        258 => pal.cursor,
        259..=266 => dim(pal.ansi[i - 259]),
        268 => dim(pal.fg),
        _ => pal.fg,
    }
}

fn resolve(c: Color, colors: &Colors, pal: &Palette) -> Color32 {
    let rgb = |r: alacritty_terminal::vte::ansi::Rgb| Color32::from_rgb(r.r, r.g, r.b);
    match c {
        Color::Spec(r) => rgb(r),
        Color::Indexed(i) => colors[i as usize]
            .map(rgb)
            .unwrap_or_else(|| pal.indexed(i)),
        Color::Named(n) => colors[n].map(rgb).unwrap_or_else(|| named(n, pal)),
    }
}

pub fn show(
    ui: &mut egui::Ui,
    s: &mut Session,
    pal: &Palette,
    m: &Metrics,
    focused: bool,
    keyboard: bool,
    chips: bool,
) {
    let t = std::time::Instant::now();
    show_grid(ui, s, pal, m, focused, keyboard, chips);
    crate::debug::timing::add(&crate::debug::timing::GRID, t.elapsed());
}

fn show_grid(
    ui: &mut egui::Ui,
    s: &mut Session,
    pal: &Palette,
    m: &Metrics,
    focused: bool,
    keyboard: bool,
    chips: bool,
) {
    let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
    // Arriba va el margen entero: la primera fila (a menudo las cajas del prompt) no debe
    // quedar pegada a la barra de pestañas. Abajo basta con la mitad.
    let origin = rect.min + Vec2::new(m.pad, m.pad);
    let cols = ((rect.width() - m.pad * 2.0) / m.cell_w).floor().max(2.0) as usize;
    let lines = ((rect.height() - m.pad * 1.5) / m.cell_h).floor().max(1.0) as usize;
    s.resize(cols, lines, m.cell_w, m.cell_h);

    // El foco de teclado es de la terminal (Tab, flechas y Esc no navegan la interfaz),
    // salvo mientras escribes en un campo del panel; un clic en la terminal lo recupera.
    let id = resp.id;
    let keyboard = keyboard || resp.clicked() || resp.drag_started();
    ui.memory_mut(|mem| {
        if keyboard {
            mem.request_focus(id);
        }
        mem.set_focus_lock_filter(
            id,
            egui::EventFilter {
                tab: true,
                horizontal_arrows: true,
                vertical_arrows: true,
                escape: true,
            },
        );
    });

    let link = handle_input(ui, s, &resp, rect, origin, m, keyboard);

    // Las líneas de cajas no se pintan como texto: en su sitio van las cajas, que dibuja
    // quien llama a partir de `s.chips_px`.
    let chip_rows = if chips { s.chip_rows() } else { Vec::new() };
    // Un prompt remoto lleva sus cajas en la misma fila que el comando. Las cajas ocupan
    // más que el texto que sustituyen, así que el comando se pinta corrido a la derecha
    // lo que haga falta (en celdas enteras, para que siga en la rejilla).
    let lead = |c: &crate::session::ChipRow| {
        // Mismas medidas que `prompt_chips`: 36 px de relleno e icono por caja, 6 entre
        // cajas, y la letra algo más pequeña que la de la rejilla si la fila es baja.
        let scale = (m.font.size.min((m.cell_h - 4.0) * 0.8) / m.font.size).min(1.0);
        let px: f32 = c
            .parts
            .iter()
            .map(|(_, text)| 42.0 + text.chars().count() as f32 * m.cell_w * scale)
            .sum();
        (px / m.cell_w).ceil() as usize + 1
    };
    s.chips_px = chip_rows
        .iter()
        .map(|c| {
            let top = origin.y + c.row as f32 * m.cell_h;
            // (Las cajas de un prompt remoto se miden solas; el comando empieza en `lead`.)
            let right = if c.inline || c.next >= cols {
                cols
            } else {
                c.next - 2
            };
            let area = Rect::from_min_max(
                Pos2::new(origin.x, top),
                Pos2::new(origin.x + right as f32 * m.cell_w, top + m.cell_h),
            );
            (area, c.parts.clone())
        })
        .collect();
    // Bajo una línea de cajas no se pinta el símbolo del prompt («❯ »): la fila se corre a
    // la izquierda para que se escriba desde el borde, y el cursor ahí es una barra. El
    // desplazamiento es en celdas; negativo = a la derecha (prompt remoto).
    s.row_shifts = chip_rows
        .iter()
        .filter_map(|c| {
            if c.inline {
                Some((c.row, c.end as i32 - lead(c) as i32))
            } else {
                (c.symbol > 0).then_some((c.row + 1, c.symbol as i32))
            }
        })
        .collect();
    let shifts = s.row_shifts.clone();
    let shift_of = |row: i32| {
        shifts
            .iter()
            .find(|(r, _)| *r == row)
            .map_or(0, |(_, n)| *n)
    };
    // Colores del comando que se escribe en una máquina remota (allí no hay resaltado).
    let token_at = |row: i32, col: usize| {
        chip_rows
            .iter()
            .flat_map(|c| c.tokens.iter())
            .find(|(r, from, to, _)| *r == row && (*from..*to).contains(&col))
            .map(|(.., tok)| *tok)
    };
    let prompt_bar = !shifts.is_empty() && s.at_prompt();
    let painter = ui.painter().with_clip_rect(rect);
    // Cada prompt marcado sabe cómo acabó el comando anterior: si falló, una barra roja
    // en el margen recorre ese comando y su salida, y junto a sus cajas van el código de
    // salida y lo que tardó (la duración sola, si acabó bien pero tardó).
    for (i, c) in chip_rows.iter().enumerate() {
        let Some((status, ms)) = c.result else {
            continue;
        };
        let prev = i.checked_sub(1).map(|j| chip_rows[j].row);
        let failed = status != 0;
        let y = |row: i32| origin.y + row as f32 * m.cell_h;
        if failed {
            let bar = Rect::from_min_max(
                Pos2::new(rect.left(), y(prev.unwrap_or(0)) - 3.0),
                Pos2::new(rect.left() + 3.0, y(c.row) - m.cell_h * 0.6),
            );
            painter.rect_filled(bar, 1.0, pal.ansi[1]);
        }
        let time = if ms < 1000 {
            format!("{ms} ms")
        } else if ms < 60_000 {
            format!("{:.1} s", ms as f32 / 1000.0)
        } else {
            format!("{} min {} s", ms / 60_000, ms % 60_000 / 1000)
        };
        let label = match (failed, ms >= 1000) {
            (true, _) => format!("falló ({status})  ·  {time}"),
            (false, true) => time,
            (false, false) => continue,
        };
        if let Some(prev) = prev {
            painter.text(
                Pos2::new(rect.right() - m.pad - 4.0, y(prev) + m.cell_h / 2.0 - 4.0),
                Align2::RIGHT_CENTER,
                label,
                egui::FontId::proportional(m.font.size * 0.85),
                if failed { pal.ansi[1] } else { pal.muted },
            );
        }
    }
    let mut bgs: Vec<BgRun> = Vec::new();
    let mut texts: Vec<TextRun> = Vec::new();
    let mut lines_deco: Vec<(i32, usize, usize, Color32, bool)> = Vec::new();
    let mut dots: Vec<(i32, usize, Color32)> = Vec::new();
    let cursor;
    let cursor_cell_fg;
    {
        let term = s.term.lock();
        let content = term.renderable_content();
        let offset = content.display_offset as i32;
        let colors = content.colors;
        let selection = content.selection;
        let cursor_row = content.cursor.point.line.0 + offset;
        cursor = (
            content.cursor.shape,
            cursor_row,
            (content.cursor.point.column.0 as i32 - shift_of(cursor_row)).max(0) as usize,
        );
        let mut under_cursor = None;

        for ind in content.display_iter {
            let cell = ind.cell;
            let flags = cell.flags;
            if flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            let row = ind.point.line.0 + offset;
            let col = ind.point.column.0;
            if chip_rows.iter().any(|c| c.row == row && col < c.end) {
                continue;
            }
            let token = token_at(row, col);
            let shifted = col as i32 - shift_of(row);
            if shifted < 0 {
                continue;
            }
            let col = shifted as usize;
            let width = if flags.contains(Flags::WIDE_CHAR) {
                2
            } else {
                1
            };

            let mut fg = match token {
                Some(crate::session::Tok::Command) => pal.ansi[2],
                Some(crate::session::Tok::Flag) => pal.ansi[4],
                Some(crate::session::Tok::Str) => pal.ansi[3],
                None => resolve(cell.fg, colors, pal),
            };
            let mut bg = resolve(cell.bg, colors, pal);
            if flags.contains(Flags::DIM) {
                fg = dim(fg);
            }
            if flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            if selection.is_some_and(|sel| sel.contains(ind.point)) {
                bg = pal.selection;
            }
            // El enlace bajo el puntero (con ⌘): del color de acento y subrayado.
            let linked = link.as_ref().is_some_and(|l| l.contains(ind.point, cell));
            if linked {
                fg = pal.accent;
            }
            if bg != pal.bg {
                match bgs.last_mut() {
                    Some(last) if last.row == row && last.end == col && last.color == bg => {
                        last.end = col + width
                    }
                    _ => bgs.push(BgRun {
                        row,
                        col,
                        end: col + width,
                        color: bg,
                    }),
                }
            }
            if row == cursor.1 && col == cursor.2 {
                under_cursor = Some((cell.c, flags.contains(Flags::BOLD)));
            }
            if linked {
                lines_deco.push((row, col, width, fg, false));
            } else if flags.intersects(Flags::ALL_UNDERLINES | Flags::STRIKEOUT) {
                lines_deco.push((row, col, width, fg, flags.contains(Flags::STRIKEOUT)));
            }
            let c = cell.c;
            if c == ' ' || c == '\0' || flags.contains(Flags::HIDDEN) {
                continue;
            }
            // ⏺ (la viñeta de Claude Code) solo existe como emoji de color en macOS:
            // se dibuja a mano para que salga como un punto del color del texto.
            if c == '\u{23FA}' {
                dots.push((row, col, fg));
                continue;
            }
            let bold = flags.contains(Flags::BOLD);
            let zero = cell.zerowidth();
            let ascii = c.is_ascii() && zero.is_none();
            match texts.last_mut() {
                Some(last)
                    if ascii
                        && last.ascii
                        && last.row == row
                        && last.next_col == col
                        && last.fg == fg
                        && last.bold == bold =>
                {
                    last.text.push(c);
                    last.next_col = col + 1;
                }
                _ => {
                    let mut text = String::from(c);
                    if let Some(z) = zero {
                        // Las marcas de las cajas no son texto (se ven si las cajas están apagadas).
                        text.extend(z.iter().filter(|c| !('\u{E0D0}'..='\u{E0D5}').contains(*c)));
                    }
                    texts.push(TextRun {
                        row,
                        col,
                        next_col: col + width,
                        text,
                        fg,
                        bold,
                        ascii,
                    });
                }
            }
        }
        cursor_cell_fg = under_cursor;
    }

    let cell_pos = |row: i32, col: usize| {
        Pos2::new(
            origin.x + col as f32 * m.cell_w,
            origin.y + row as f32 * m.cell_h,
        )
    };
    if crate::grid::instanced() {
        // Fondos y letras como instancias de Metal (`grid.rs`), en un solo `Callback` en
        // el mismo sitio del orden de pintado: lo que va después (puntos, subrayados,
        // cursor, cajas) queda encima, como antes.
        let ctx = ui.ctx().clone();
        let ppp = ctx.pixels_per_point();
        let mut instances = Vec::with_capacity(bgs.len() + texts.len() * 8);
        for b in &bgs {
            // Como `rect_filled`: el teselador lleva el rectángulo a píxeles enteros.
            let min = cell_pos(b.row, b.col);
            let max = Pos2::new(origin.x + b.end as f32 * m.cell_w, min.y + m.cell_h);
            let r = egui::emath::GuiRounding::round_to_pixels(Rect::from_min_max(min, max), ppp);
            instances.push(crate::grid::Instance::solid(r.min, r.max, b.color));
        }
        GLYPHS.with_borrow_mut(|cache| {
            cache.begin(&ctx, &m.font, &m.bold);
            for t in &texts {
                let pos = cell_pos(t.row, t.col) + Vec2::new(0.0, m.text_dy);
                cache.push_text(&ctx, &mut instances, pos, &t.text, t.ascii, t.bold, t.fg);
            }
        });
        crate::debug::timing::INSTANCES
            .fetch_add(instances.len() as u64, std::sync::atomic::Ordering::Relaxed);
        if !instances.is_empty() {
            painter.add(egui::Shape::Callback(egui::PaintCallback {
                rect,
                callback: std::sync::Arc::new(crate::grid::GridBatch { instances }),
            }));
        }
    } else {
        for b in &bgs {
            let min = cell_pos(b.row, b.col);
            let max = Pos2::new(origin.x + b.end as f32 * m.cell_w, min.y + m.cell_h);
            painter.rect_filled(Rect::from_min_max(min, max), 0.0, b.color);
        }
        for t in texts {
            let pos = cell_pos(t.row, t.col) + Vec2::new(0.0, m.text_dy);
            let font = if t.bold {
                m.bold.clone()
            } else {
                m.font.clone()
            };
            painter.text(pos, Align2::LEFT_TOP, t.text, font, t.fg);
        }
    }
    for (row, col, color) in dots {
        let center = cell_pos(row, col) + Vec2::new(m.cell_w, m.cell_h) / 2.0;
        painter.circle_filled(center, (m.cell_w * 0.45).min(m.cell_h * 0.3), color);
    }
    for (row, col, width, color, strike) in lines_deco {
        let p = cell_pos(row, col);
        let y = if strike {
            p.y + m.cell_h * 0.5
        } else {
            p.y + m.cell_h - 1.5
        };
        painter.line_segment(
            [
                Pos2::new(p.x, y),
                Pos2::new(p.x + width as f32 * m.cell_w, y),
            ],
            Stroke::new(1.0, color),
        );
    }

    // Cursor.
    let (shape, crow, ccol) = cursor;
    let shape = if shape == CursorShape::Block {
        m.cursor
    } else {
        shape
    };
    // En el prompt con cajas el cursor va del color de acento, como el resto del prompt.
    let cursor_color = if prompt_bar { pal.accent } else { pal.cursor };
    // Parpadeo: fijo mientras se escribe y medio segundo después; luego alterna cada 500 ms.
    let blink_on = !m.blink || !focused || {
        let ms = s.since_input().as_millis() as u64;
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(500 - ms % 500));
        (ms / 500) % 2 == 0
    };
    if shape != CursorShape::Hidden && crow >= 0 && (crow as usize) < lines {
        let base = cell_pos(crow, ccol);
        s.cursor_px = Some(Rect::from_min_size(base, Vec2::new(m.cell_w, m.cell_h)));
        // Lo que se está componiendo va en el sitio del cursor, subrayado, y el cursor detrás.
        let composing = s.preedit.chars().count() as f32 * m.cell_w;
        if composing > 0.0 {
            let area = Rect::from_min_size(base, Vec2::new(composing, m.cell_h));
            painter.rect_filled(area, 0.0, pal.bg);
            painter.text(
                base + Vec2::new(0.0, m.text_dy),
                Align2::LEFT_TOP,
                &s.preedit,
                m.font.clone(),
                pal.fg,
            );
            let line = Rect::from_min_max(Pos2::new(area.left(), area.bottom() - 2.0), area.max);
            painter.rect_filled(line, 0.0, pal.accent);
        }
        let min = base + Vec2::new(composing, 0.0);
        let cell = Rect::from_min_size(min, Vec2::new(m.cell_w, m.cell_h));
        if !focused {
            painter.rect_stroke(cell, 0.0, Stroke::new(1.0, pal.cursor), StrokeKind::Inside);
        } else if blink_on {
            match shape {
                CursorShape::Beam => {
                    painter.rect_filled(
                        Rect::from_min_size(min, Vec2::new(2.0, m.cell_h)),
                        1.0,
                        cursor_color,
                    );
                }
                CursorShape::Underline => {
                    let r = Rect::from_min_max(Pos2::new(min.x, min.y + m.cell_h - 2.0), cell.max);
                    painter.rect_filled(r, 0.0, cursor_color);
                }
                CursorShape::HollowBlock => {
                    painter.rect_stroke(
                        cell,
                        0.0,
                        Stroke::new(1.0, cursor_color),
                        StrokeKind::Inside,
                    );
                }
                _ => {
                    painter.rect_filled(cell, 0.0, cursor_color);
                    if let Some((c, bold)) = cursor_cell_fg {
                        if c != ' ' && c != '\0' {
                            let font = if bold { m.bold.clone() } else { m.font.clone() };
                            painter.text(
                                min + Vec2::new(0.0, m.text_dy),
                                Align2::LEFT_TOP,
                                c,
                                font,
                                pal.bg,
                            );
                        }
                    }
                }
            }
        }
        // Dónde colocar la ventana de composición (acentos, emoji).
        ui.output_mut(|o| {
            o.ime = Some(egui::output::IMEOutput {
                purpose: egui::IMEPurpose::Terminal,
                rect,
                cursor_rect: cell,
                should_interrupt_composition: false,
            })
        });
    }
}

fn grid_point(pos: Pos2, origin: Pos2, m: &Metrics, s: &Session, offset: usize) -> (Point, Side) {
    let x = ((pos.x - origin.x) / m.cell_w).max(0.0);
    let y = ((pos.y - origin.y) / m.cell_h).max(0.0);
    let row = (y.floor() as usize).min(s.size.lines - 1);
    // Las filas pintadas sin el símbolo del prompt están corridas a la izquierda.
    let shift = s
        .row_shifts
        .iter()
        .find(|(r, _)| *r == row as i32)
        .map_or(0, |(_, n)| *n);
    let col = ((x.floor() as i32 + shift).max(0) as usize).min(s.size.cols - 1);
    let side = if x.fract() > 0.5 {
        Side::Right
    } else {
        Side::Left
    };
    (
        Point::new(Line(row as i32 - offset as i32), Column(col)),
        side,
    )
}

fn handle_input(
    ui: &mut egui::Ui,
    s: &mut Session,
    resp: &egui::Response,
    rect: Rect,
    origin: Pos2,
    m: &Metrics,
    keyboard: bool,
) -> Option<Link> {
    let ctx = ui.ctx().clone();
    let mode = *s.term.lock().mode();
    let (events, scroll_y, mods) =
        ui.input(|i| (i.events.clone(), i.smooth_scroll_delta.y, i.modifiers));

    // Lo que se está componiendo deja de enseñarse en cuanto llega texto de verdad. macOS
    // no siempre avisa de que la composición terminó (la «ñ» de ⌥N + N llega como texto
    // normal, sin un `Preedit` vacío antes), y el «˜» se quedaba pegado al cursor.
    let frame = ctx.cumulative_frame_nr();
    let committed = events.iter().any(|e| {
        matches!(
            e,
            egui::Event::Text(_)
                | egui::Event::Paste(_)
                | egui::Event::Ime(egui::ImeEvent::Commit(_))
        )
    });
    // También si se pulsa otra tecla en un cuadro posterior (Esc, flechas…) o se va el foco.
    let later_key = frame > s.preedit_frame
        && events
            .iter()
            .any(|e| matches!(e, egui::Event::Key { pressed: true, .. }));
    let focused = ui.input(|i| i.viewport().focused.unwrap_or(true));
    if later_key || !focused {
        s.preedit.clear();
    }

    for ev in events {
        if !keyboard {
            break;
        }
        match ev {
            egui::Event::Text(t) => s.input(t.into_bytes()),
            egui::Event::Ime(egui::ImeEvent::Commit(t)) => {
                s.preedit.clear();
                s.input(t.into_bytes());
            }
            // Composición en curso: todavía no es texto, solo se enseña.
            egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => {
                s.preedit = text;
                s.preedit_frame = frame;
            }
            egui::Event::Paste(t) => s.paste(&t),
            // ⌘V con una imagen en el portapapeles (sin texto): se envía Ctrl+V para que
            // el programa (Claude Code, etc.) lea la imagen él mismo, como hace Warp.
            egui::Event::PasteImage(_) => s.input(vec![0x16]),
            egui::Event::Copy | egui::Event::Cut => {
                let text = s
                    .term
                    .lock()
                    .selection_to_string()
                    .filter(|t| !t.is_empty());
                match text {
                    Some(text) => ctx.copy_text(text),
                    // Sin selección de pantalla, ⌘C/⌘X son para lo seleccionado dentro
                    // del comando, que lo lleva zle (ver `assets/shell/zshenv.zsh`).
                    None if s.input_cells().is_some() => {
                        let cut = matches!(ev, egui::Event::Cut);
                        s.input(if cut { SEL_CUT } else { SEL_COPY }.to_vec());
                    }
                    None => {}
                }
            }
            egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => {
                if modifiers.shift && !mode.contains(TermMode::ALT_SCREEN) {
                    let scroll = match key {
                        Key::PageUp => Some(Scroll::PageUp),
                        Key::PageDown => Some(Scroll::PageDown),
                        Key::Home => Some(Scroll::Top),
                        Key::End => Some(Scroll::Bottom),
                        _ => None,
                    };
                    if let Some(sc) = scroll {
                        s.term.lock().scroll_display(sc);
                        continue;
                    }
                }
                // ⌘A en el prompt selecciona el comando entero; y al seleccionar con el
                // teclado se suelta lo que hubiera seleccionado con el ratón.
                let arrow = matches!(key, Key::ArrowLeft | Key::ArrowRight);
                let select_all = key == Key::A && modifiers.mac_cmd && !modifiers.shift;
                if (select_all || arrow && modifiers.shift) && s.in_command() {
                    s.term.lock().selection = None;
                    if select_all {
                        s.input(SEL_ALL.to_vec());
                        continue;
                    }
                }
                if let Some(bytes) = key_bytes(key, modifiers, mode) {
                    s.input(bytes);
                }
            }
            _ => {}
        }
    }

    if committed {
        s.preedit.clear();
    }

    // Rueda / trackpad.
    let hovered = ui.rect_contains_pointer(rect);
    if hovered && scroll_y != 0.0 {
        s.scroll_acc += scroll_y / m.cell_h;
        let n = s.scroll_acc.trunc() as i32;
        if n != 0 {
            s.scroll_acc -= n as f32;
            if mode.intersects(TermMode::MOUSE_MODE)
                && mode.contains(TermMode::SGR_MOUSE)
                && !mods.shift
            {
                if let Some(pos) = ctx.pointer_hover_pos() {
                    let (p, _) = grid_point(pos, origin, m, s, 0);
                    let button = if n > 0 { 64 } else { 65 };
                    let seq = format!("\x1b[<{};{};{}M", button, p.column.0 + 1, p.line.0 + 1);
                    s.input(seq.repeat(n.unsigned_abs() as usize).into_bytes());
                }
            } else if mode.contains(TermMode::ALT_SCREEN)
                && mode.contains(TermMode::ALTERNATE_SCROLL)
            {
                let key = if n > 0 { "\x1bOA" } else { "\x1bOB" };
                s.input(key.repeat(n.unsigned_abs() as usize).into_bytes());
            } else {
                s.term.lock().scroll_display(Scroll::Delta(n));
            }
        }
    }

    // Ratón: si el programa pidió informes (y no mantienes Shift), se le envían clics,
    // arrastres y movimiento; si no, el ratón selecciona texto.
    // Con ⌘ pulsado, el enlace bajo el puntero se resalta y un clic lo abre. Va antes que
    // los informes de ratón: ese clic no es para el programa ni empieza una selección.
    let link = (mods.command && hovered)
        .then(|| ctx.pointer_hover_pos())
        .flatten()
        .and_then(|pos| {
            let term = s.term.lock();
            let offset = term.grid().display_offset();
            let (point, _) = grid_point(pos, origin, m, s, offset);
            // Las rutas son de esta máquina: dentro de una sesión ssh no se buscan.
            let cwd = (!s.remote()).then_some(s.cwd.as_path());
            link_at(&term, point, cwd)
        });
    if let Some(link) = link {
        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        if resp.clicked() {
            open_link(&link.target, &m.editor);
        }
        return Some(link);
    }

    let reporting = mode.intersects(TermMode::MOUSE_MODE) && !mods.shift;
    if reporting {
        report_mouse(ui, s, rect, origin, m, mode, mods);
        return None;
    }
    s.mouse_cell = None;
    s.mouse_down = None;
    let app_cursor = mode.contains(TermMode::APP_CURSOR);
    if resp.drag_stopped() {
        adopt_selection(s, app_cursor);
    }
    let Some(pos) = resp.interact_pointer_pos() else {
        return None;
    };
    let mut term = s.term.lock();
    let offset = term.grid().display_offset();
    let (point, side) = grid_point(pos, origin, m, s, offset);
    let mut place = None;
    if resp.triple_clicked() {
        term.selection = Some(Selection::new(SelectionType::Lines, point, side));
    } else if resp.double_clicked() {
        term.selection = Some(Selection::new(SelectionType::Semantic, point, side));
    } else if resp.drag_started() {
        term.selection = Some(Selection::new(SelectionType::Simple, point, side));
    } else if resp.dragged() {
        if let Some(sel) = term.selection.as_mut() {
            sel.update(point, side);
        }
    } else if resp.clicked() {
        term.selection = None;
        place = Some((point, side));
    }
    drop(term);
    if resp.double_clicked() {
        adopt_selection(s, app_cursor);
    } else if let Some((point, side)) = place {
        place_cursor(s, point, side, app_cursor);
    }
    None
}

/// Un enlace que se puede abrir: el de un hiperenlace OSC 8 (lo que emite Claude Code),
/// una dirección escrita tal cual en la pantalla o la ruta de un archivo que existe.
struct Link {
    target: Target,
    span: Span,
}

#[derive(Debug, PartialEq)]
enum Target {
    Url(String),
    /// Archivo o carpeta de esta máquina, con la línea y la columna si el texto las decía
    /// (`src/view.rs:120:8`).
    File(PathBuf, Option<u32>, Option<u32>),
}

/// Qué celdas ocupa el enlace.
enum Span {
    /// Las del hiperenlace OSC 8 con esta dirección. Se compara la dirección y no el id:
    /// un enlace partido en varias líneas llega como varios hiperenlaces.
    Tag(String),
    /// De esta celda a esta otra, si salió del texto.
    Range(Point, Point),
}

impl Link {
    fn contains(&self, point: Point, cell: &Cell) -> bool {
        match &self.span {
            Span::Range(from, to) => (*from..=*to).contains(&point),
            Span::Tag(uri) => cell.hyperlink().is_some_and(|h| h.uri() == uri),
        }
    }
}

/// Solo se abren direcciones web y de correo: un hiperenlace lo escribe el programa que
/// corre en la terminal, y `open` con otro esquema podría lanzar archivos o aplicaciones.
fn openable(uri: &str) -> bool {
    ["http://", "https://", "mailto:"]
        .iter()
        .any(|scheme| uri.starts_with(scheme))
}

/// `cwd` es la carpeta contra la que se resuelven las rutas relativas; sin ella (sesión
/// remota) solo se buscan direcciones.
fn link_at<T>(term: &Term<T>, point: Point, cwd: Option<&Path>) -> Option<Link> {
    let grid = term.grid();
    if let Some(tag) = grid[point].hyperlink() {
        let target = if openable(tag.uri()) {
            Some(Target::Url(tag.uri().to_string()))
        } else {
            // `file://máquina/ruta`, lo que emiten `ls --hyperlink` y algunos agentes.
            cwd.and(tag.uri().strip_prefix("file://"))
                .and_then(|rest| rest.find('/').map(|i| PathBuf::from(unescape(&rest[i..]))))
                .filter(|path| path.exists())
                .map(|path| Target::File(path, None, None))
        };
        if let Some(target) = target {
            return Some(Link {
                target,
                span: Span::Tag(tag.uri().to_string()),
            });
        }
    }
    // Sin hiperenlace se busca una dirección en el texto de la línea, entera aunque la
    // terminal la haya partido en varias filas (hasta 20 hacia cada lado).
    let last = grid.last_column();
    let wraps = |line: i32| grid[Line(line)][last].flags.contains(Flags::WRAPLINE);
    let mut first = point.line.0;
    while first > grid.topmost_line().0 && point.line.0 - first < 20 && wraps(first - 1) {
        first -= 1;
    }
    let mut end = point.line.0;
    while end < grid.bottommost_line().0 && end - point.line.0 < 20 && wraps(end) {
        end += 1;
    }
    let mut chars = Vec::new();
    let mut points = Vec::new();
    for line in first..=end {
        for col in 0..grid.columns() {
            let at = Point::new(Line(line), Column(col));
            let cell = &grid[at];
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            chars.push(if cell.c == '\0' { ' ' } else { cell.c });
            points.push(at);
        }
    }
    let idx = points.iter().position(|p| *p == point)?;
    if let Some((from, to)) = url_span(&chars, idx) {
        return Some(Link {
            target: Target::Url(chars[from..to].iter().collect()),
            span: Span::Range(points[from], points[to - 1]),
        });
    }
    let (from, to) = path_span(&chars, idx)?;
    let token: String = chars[from..to].iter().collect();
    let (name, line, col) = split_position(&token);
    let path = find_path(name, cwd?)?;
    Some(Link {
        target: Target::File(path, line, col),
        span: Span::Range(points[from], points[to - 1]),
    })
}

/// `%XX` → byte, para las rutas de los hiperenlaces `file://`.
fn unescape(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = (bytes[i] == b'%')
            .then(|| text.get(i + 1..i + 3))
            .flatten()
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match hex {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// La palabra con forma de ruta que pasa por `idx`, como rango `[desde, hasta)`: llega
/// hasta los espacios, comillas o paréntesis que la rodean, sin la puntuación final.
fn path_span(chars: &[char], idx: usize) -> Option<(usize, usize)> {
    let part = |c: char| {
        if c.is_ascii() {
            c.is_ascii_graphic() && !"\"'`<>()[]{}|*?;,=".contains(c)
        } else {
            c.is_alphanumeric()
        }
    };
    if !part(*chars.get(idx)?) {
        return None;
    }
    let mut from = idx;
    while from > 0 && part(chars[from - 1]) {
        from -= 1;
    }
    let mut to = idx + 1;
    while to < chars.len() && part(chars[to]) {
        to += 1;
    }
    while to > from && matches!(chars[to - 1], '.' | ':') {
        to -= 1;
    }
    (idx < to && chars[from..to].iter().any(|c| c.is_alphanumeric())).then_some((from, to))
}

/// Separa `ruta:línea:columna` (lo que escriben compiladores y agentes) en sus partes.
fn split_position(token: &str) -> (&str, Option<u32>, Option<u32>) {
    fn number(text: &str) -> Option<(&str, u32)> {
        let (rest, n) = text.rsplit_once(':')?;
        (!rest.is_empty()).then_some((rest, n.parse().ok()?))
    }
    match number(token) {
        Some((rest, last)) => match number(rest) {
            Some((name, line)) => (name, Some(line), Some(last)),
            None => (rest, Some(last), None),
        },
        None => (token, None, None),
    }
}

/// La ruta a la que se refiere `name` vista desde `cwd`, si existe en el disco. Los
/// `a/` y `b/` que `git diff` pone delante no son parte de la ruta.
fn find_path(name: &str, cwd: &Path) -> Option<PathBuf> {
    let from_home = name
        .strip_prefix("~/")
        .and_then(|rest| dirs::home_dir().map(|home| home.join(rest)));
    let plain = from_home.unwrap_or_else(|| cwd.join(name));
    let diff = ["a/", "b/"]
        .iter()
        .find_map(|prefix| name.strip_prefix(prefix))
        .map(|rest| cwd.join(rest));
    [Some(plain), diff]
        .into_iter()
        .flatten()
        .find(|p| p.exists())
}

/// Con qué orden se abre un archivo. Lo que al abrirse **se ejecuta** o se instala (una
/// aplicación, un `.command`, un binario) solo se enseña en Finder: la ruta la escribió el
/// programa que corre en la terminal, no quien hace clic.
fn file_command(
    path: &Path,
    line: Option<u32>,
    col: Option<u32>,
    editor: &str,
    dir: bool,
    executable: bool,
) -> Vec<String> {
    const RUNS: [&str; 22] = [
        "app",
        "command",
        "tool",
        "terminal",
        "workflow",
        "action",
        "pkg",
        "mpkg",
        "dmg",
        "jar",
        "scpt",
        "scptd",
        "applescript",
        "webloc",
        "inetloc",
        "fileloc",
        "prefpane",
        "zip",
        "gz",
        "tgz",
        "tar",
        "xip",
    ];
    const MEDIA: [&str; 16] = [
        "png", "jpg", "jpeg", "gif", "webp", "heic", "tiff", "bmp", "ico", "icns", "pdf", "mp4",
        "mov", "mp3", "wav", "m4a",
    ];
    let file = path.to_string_lossy().into_owned();
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let open = |flags: &[&str]| {
        let mut cmd = vec!["open".to_string()];
        cmd.extend(flags.iter().map(|f| f.to_string()));
        cmd.push(file.clone());
        cmd
    };
    if RUNS.contains(&ext.as_str()) {
        return open(&["-R"]);
    }
    if dir || MEDIA.contains(&ext.as_str()) {
        return open(&[]);
    }
    let mut words = editor.split_whitespace().map(str::to_string);
    let Some(program) = words.next() else {
        return if executable { open(&["-R"]) } else { open(&[]) };
    };
    let at = |sep: &str| match (line, col) {
        (Some(l), Some(c)) => format!("{file}{sep}{l}{sep}{c}"),
        (Some(l), None) => format!("{file}{sep}{l}"),
        _ => file.clone(),
    };
    let name = program.rsplit('/').next().unwrap_or_default().to_string();
    let args = match name.as_str() {
        "code" | "code-insiders" | "codium" | "cursor" | "windsurf" => vec!["-g".into(), at(":")],
        "zed" | "subl" | "nova" => vec![at(":")],
        "idea" | "webstorm" | "pycharm" | "rustrover" | "goland" | "clion" | "phpstorm"
        | "rubymine" | "studio" | "xed" => match line {
            Some(l) => vec!["--line".into(), l.to_string(), file.clone()],
            None => vec![file.clone()],
        },
        _ => vec![file.clone()],
    };
    // El editor se busca con el PATH de un shell de inicio de sesión: abierta desde Finder,
    // la app no hereda el del usuario (Homebrew, `~/.local/bin`…).
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let mut cmd = vec![
        shell,
        "-lc".into(),
        "exec \"$@\"".into(),
        "colibri".into(),
        program,
    ];
    cmd.extend(words);
    cmd.extend(args);
    cmd
}

/// La dirección web que pasa por `idx`, como rango `[desde, hasta)` de `chars`. No incluye
/// la puntuación con la que acaba la frase ni el paréntesis que la encierra.
fn url_span(chars: &[char], idx: usize) -> Option<(usize, usize)> {
    let starts = |i: usize, scheme: &str| {
        chars[i..].len() > scheme.len()
            && chars[i..].iter().zip(scheme.chars()).all(|(a, b)| *a == b)
    };
    let part = |c: char| {
        if c.is_ascii() {
            c.is_ascii_graphic()
                && !matches!(c, '"' | '<' | '>' | '`' | '\\' | '^' | '{' | '}' | '|')
        } else {
            c.is_alphanumeric()
        }
    };
    let mut i = 0;
    while i <= idx {
        let after_word = i > 0 && chars[i - 1].is_alphanumeric();
        if after_word || !(starts(i, "https://") || starts(i, "http://")) {
            i += 1;
            continue;
        }
        let mut end = i;
        while end < chars.len() && part(chars[end]) {
            end += 1;
        }
        loop {
            let count = |c: char| chars[i..end].iter().filter(|x| **x == c).count();
            let drop = match chars[end - 1] {
                '.' | ',' | ';' | ':' | '!' | '?' | '\'' => true,
                ')' => count(')') > count('('),
                ']' => count(']') > count('['),
                _ => false,
            };
            if !drop {
                break;
            }
            end -= 1;
        }
        let scheme = if starts(i, "https://") { 8 } else { 7 };
        if end - i > scheme && (i..end).contains(&idx) {
            return Some((i, end));
        }
        i = end.max(i + 1);
    }
    None
}

/// Abre el enlace: una dirección con la aplicación del sistema, un archivo con el editor
/// elegido. Con el guion de depuración solo se anuncia en stderr, para poder comprobarlo
/// sin abrir nada.
fn open_link(target: &Target, editor: &str) {
    let cmd = match target {
        Target::Url(uri) => vec!["open".to_string(), uri.clone()],
        Target::File(path, line, col) => {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(path).ok();
            let dir = meta.as_ref().is_some_and(|m| m.is_dir());
            let executable = meta.is_some_and(|m| m.permissions().mode() & 0o111 != 0);
            file_command(path, *line, *col, editor, dir, executable)
        }
    };
    if std::env::var_os("COLIBRI_SCRIPT").is_some() {
        match target {
            Target::Url(uri) => eprintln!("enlace: {uri}"),
            Target::File(..) => eprintln!("enlace: {}", cmd.join(" ")),
        }
        return;
    }
    std::thread::spawn(move || {
        let _ = std::process::Command::new(&cmd[0])
            .args(&cmd[1..])
            .stdin(std::process::Stdio::null())
            .status();
    });
}

/// Teclas privadas que la integración de zsh enlaza a sus widgets de selección.
const SEL_ALL: &[u8] = b"\x1b[77;a~";
const SEL_COPY: &[u8] = b"\x1b[77;c~";
const SEL_CUT: &[u8] = b"\x1b[77;x~";

/// Flechas (sin modificadores) para llevar el cursor de zle de un carácter a otro.
fn cursor_moves(from: usize, to: usize, app_cursor: bool) -> Vec<u8> {
    let key: &[u8] = match (to < from, app_cursor) {
        (true, true) => b"\x1bOD",
        (true, false) => b"\x1b[D",
        (false, true) => b"\x1bOC",
        (false, false) => b"\x1b[C",
    };
    key.repeat(from.abs_diff(to))
}

/// Una selección hecha con el ratón que cae entera dentro del comando que se está
/// escribiendo pasa a ser selección de zle (cursor al principio + Shift+→ hasta el final):
/// así se puede sustituir escribiendo, borrar o cortar, igual que la hecha con el teclado.
fn adopt_selection(s: &mut Session, app_cursor: bool) {
    let Some((cells, cursor)) = s.input_cells() else {
        return;
    };
    let range = {
        let term = s.term.lock();
        term.selection.as_ref().and_then(|sel| sel.to_range(&term))
    };
    let (Some(range), Some(first), Some(last)) = (range, cells.first(), cells.last()) else {
        return;
    };
    if range.is_block || range.start < *first || range.end.line > last.line {
        return;
    }
    let from = cells.partition_point(|c| *c < range.start);
    let to = cells.partition_point(|c| *c <= range.end);
    if to <= from {
        return;
    }
    let at = cells.partition_point(|c| *c < cursor);
    let mut bytes = cursor_moves(at, from, app_cursor);
    bytes.extend(b"\x1b[1;2C".repeat(to - from));
    s.term.lock().selection = None;
    s.input(bytes);
}

/// Un clic dentro del comando que se está escribiendo lleva ahí el cursor.
fn place_cursor(s: &mut Session, point: Point, side: Side, app_cursor: bool) {
    let Some((cells, cursor)) = s.input_cells() else {
        return;
    };
    let (Some(first), Some(last)) = (cells.first(), cells.last()) else {
        return;
    };
    if point < *first || point.line > last.line {
        return;
    }
    let after = side == Side::Right && cells.contains(&point);
    let to = cells.partition_point(|c| *c < point) + after as usize;
    let at = cells.partition_point(|c| *c < cursor);
    if to != at {
        s.input(cursor_moves(at, to, app_cursor));
    }
}

/// Secuencia de informe de ratón. `code` ya incluye botón, movimiento y modificadores.
/// Columna y fila empiezan en 1.
fn mouse_seq(code: u8, col: usize, row: usize, press: bool, sgr: bool) -> Option<Vec<u8>> {
    if sgr {
        let end = if press { 'M' } else { 'm' };
        return Some(format!("\x1b[<{code};{col};{row}{end}").into_bytes());
    }
    // Formato clásico: un byte por coordenada, así que no llega más allá de la columna 223.
    if col > 223 || row > 223 {
        return None;
    }
    let code = if press { code } else { (code & !0b11) | 3 };
    Some(vec![
        0x1b,
        b'[',
        b'M',
        32 + code,
        32 + col as u8,
        32 + row as u8,
    ])
}

fn report_mouse(
    ui: &egui::Ui,
    s: &mut Session,
    rect: Rect,
    origin: Pos2,
    m: &Metrics,
    mode: TermMode,
    mods: Modifiers,
) {
    use egui::PointerButton::{Middle, Primary, Secondary};
    let buttons = [Primary, Middle, Secondary];
    let (pos, pressed, released) = ui.input(|i| {
        (
            i.pointer.latest_pos(),
            buttons.map(|b| i.pointer.button_pressed(b)),
            buttons.map(|b| i.pointer.button_released(b)),
        )
    });
    let Some(pos) = pos else {
        return;
    };
    let inside = rect.contains(pos) && ui.rect_contains_pointer(rect);
    let sgr = mode.contains(TermMode::SGR_MOUSE);
    let (p, _) = grid_point(pos, origin, m, s, 0);
    let cell = (p.column.0 + 1, p.line.0 as usize + 1);
    let mod_bits = 8 * mods.alt as u8 + 16 * mods.ctrl as u8;
    let mut out: Vec<u8> = Vec::new();

    for (b, &is_pressed) in pressed.iter().enumerate() {
        if is_pressed && inside {
            s.mouse_down = Some(b as u8);
            out.extend(
                mouse_seq(b as u8 + mod_bits, cell.0, cell.1, true, sgr).unwrap_or_default(),
            );
        }
    }
    if s.mouse_cell != Some(cell) {
        match s.mouse_down {
            // Arrastre: movimiento con el botón pulsado.
            Some(b) if mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION) => {
                out.extend(
                    mouse_seq(32 + b + mod_bits, cell.0, cell.1, true, sgr).unwrap_or_default(),
                );
            }
            // Movimiento libre (hover): solo si el programa lo pidió.
            None if inside && mode.contains(TermMode::MOUSE_MOTION) => {
                out.extend(mouse_seq(35 + mod_bits, cell.0, cell.1, true, sgr).unwrap_or_default());
            }
            _ => {}
        }
        s.mouse_cell = inside.then_some(cell).or(s.mouse_down.map(|_| cell));
    }
    for (b, &is_released) in released.iter().enumerate() {
        if is_released && s.mouse_down == Some(b as u8) {
            s.mouse_down = None;
            out.extend(
                mouse_seq(b as u8 + mod_bits, cell.0, cell.1, false, sgr).unwrap_or_default(),
            );
        }
    }
    if !out.is_empty() {
        s.report(out);
    }
}

#[cfg(test)]
mod tests {
    use super::{file_command, mouse_seq, path_span, split_position, url_span};
    use std::path::Path;

    fn url(text: &str, at: &str) -> Option<String> {
        let chars: Vec<char> = text.chars().collect();
        let idx = text[..text.find(at)?].chars().count();
        url_span(&chars, idx).map(|(a, b)| chars[a..b].iter().collect())
    }

    #[test]
    fn path_in_text() {
        let word = |text: &str, at: &str| {
            let chars: Vec<char> = text.chars().collect();
            let idx = text[..text.find(at)?].chars().count();
            path_span(&chars, idx).map(|(a, b)| chars[a..b].iter().collect::<String>())
        };
        assert_eq!(
            word("  --> src/view.rs:120:8", "view").as_deref(),
            Some("src/view.rs:120:8")
        );
        assert_eq!(
            word("mira (src/main.rs).", "main").as_deref(),
            Some("src/main.rs")
        );
        assert_eq!(
            word("en `Cargo.toml`: ok", "Cargo").as_deref(),
            Some("Cargo.toml")
        );
        assert_eq!(word("uno  dos", " d"), None);
        assert_eq!(
            split_position("src/view.rs:120:8"),
            ("src/view.rs", Some(120), Some(8))
        );
        assert_eq!(split_position("a.rs:7"), ("a.rs", Some(7), None));
        assert_eq!(split_position("a.rs"), ("a.rs", None, None));
        assert_eq!(split_position("C:x"), ("C:x", None, None));
    }

    #[test]
    fn file_commands() {
        let cmd = |path: &str, editor: &str, dir: bool, exec: bool| {
            file_command(Path::new(path), Some(12), Some(3), editor, dir, exec).join(" ")
        };
        // Sin editor decide macOS, salvo lo que al abrirse se ejecuta: eso solo se enseña.
        assert_eq!(cmd("/x/a.rs", "", false, false), "open /x/a.rs");
        assert_eq!(cmd("/x/run", "", false, true), "open -R /x/run");
        assert_eq!(
            cmd("/x/Mala.app", "code", true, true),
            "open -R /x/Mala.app"
        );
        assert_eq!(cmd("/x/src", "code", true, true), "open /x/src");
        assert_eq!(cmd("/x/foto.PNG", "code", false, false), "open /x/foto.PNG");
        assert!(cmd("/x/a.rs", "code", false, false).ends_with(" colibri code -g /x/a.rs:12:3"));
        assert!(cmd("/x/a.rs", "zed", false, true).ends_with(" colibri zed /x/a.rs:12:3"));
        assert!(cmd("/x/a.rs", "idea", false, false).ends_with(" colibri idea --line 12 /x/a.rs"));
        assert!(
            cmd("/x/a.rs", "nvim-gui -f", false, false).ends_with(" colibri nvim-gui -f /x/a.rs")
        );
    }

    #[test]
    fn url_in_text() {
        let pr = "https://bitbucket.org/x/pull-requests/new?source=feat-1&t=1";
        assert_eq!(
            url(&format!("Para crear el PR: {pr}"), "bitbucket").as_deref(),
            Some(pr)
        );
        assert_eq!(url(&format!("({pr}).  otra"), "new?").as_deref(), Some(pr));
        assert_eq!(url(&format!("Para crear el PR: {pr}"), "crear"), None);
        assert_eq!(
            url(
                "ver https://es.wikipedia.org/wiki/Rust_(lenguaje), y",
                "wiki/"
            )
            .as_deref(),
            Some("https://es.wikipedia.org/wiki/Rust_(lenguaje)")
        );
        assert_eq!(
            url("http://a.com y https://b.com/x.", "b.com").as_deref(),
            Some("https://b.com/x")
        );
        assert_eq!(url("https:// nada", "https"), None);
    }

    #[test]
    fn sgr_press_release_and_hover() {
        assert_eq!(mouse_seq(0, 5, 3, true, true).unwrap(), b"\x1b[<0;5;3M");
        assert_eq!(mouse_seq(0, 5, 3, false, true).unwrap(), b"\x1b[<0;5;3m");
        assert_eq!(
            mouse_seq(35, 120, 40, true, true).unwrap(),
            b"\x1b[<35;120;40M"
        );
    }

    #[test]
    fn classic_encoding() {
        assert_eq!(
            mouse_seq(0, 1, 1, true, false).unwrap(),
            vec![0x1b, b'[', b'M', 32, 33, 33]
        );
        assert_eq!(
            mouse_seq(0, 1, 1, false, false).unwrap(),
            vec![0x1b, b'[', b'M', 35, 33, 33]
        );
        assert!(mouse_seq(0, 300, 1, true, false).is_none());
    }
}

fn key_bytes(key: Key, m: Modifiers, mode: TermMode) -> Option<Vec<u8>> {
    let app_cursor = mode.contains(TermMode::APP_CURSOR);
    let modified = m.shift || m.alt || m.ctrl;
    let mod_param = 1 + m.shift as u8 + 2 * m.alt as u8 + 4 * m.ctrl as u8;
    let csi = |letter: char| -> Vec<u8> {
        if modified {
            format!("\x1b[1;{mod_param}{letter}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{letter}").into_bytes()
        } else {
            format!("\x1b[{letter}").into_bytes()
        }
    };
    let tilde = |n: u8| -> Vec<u8> {
        if modified {
            format!("\x1b[{n};{mod_param}~").into_bytes()
        } else {
            format!("\x1b[{n}~").into_bytes()
        }
    };

    // Atajos estilo macOS para moverse en la línea.
    if m.mac_cmd {
        return match key {
            // Con Shift: seleccionar hasta el principio o el final (Shift+Inicio/Fin).
            Key::ArrowLeft if m.shift => Some(b"\x1b[1;2H".to_vec()),
            Key::ArrowRight if m.shift => Some(b"\x1b[1;2F".to_vec()),
            Key::ArrowLeft => Some(vec![0x01]),
            Key::ArrowRight => Some(vec![0x05]),
            Key::Backspace => Some(vec![0x15]),
            _ => None,
        };
    }
    if m.alt && !m.ctrl && !m.shift {
        match key {
            Key::ArrowLeft => return Some(b"\x1bb".to_vec()),
            Key::ArrowRight => return Some(b"\x1bf".to_vec()),
            Key::Backspace => return Some(b"\x1b\x7f".to_vec()),
            Key::Delete => return Some(b"\x1bd".to_vec()),
            _ => {}
        }
    }

    let out = match key {
        Key::Enter => {
            // Shift+Enter / Alt+Enter = salto de línea en Claude Code y similares.
            if m.shift || m.alt {
                b"\x1b\r".to_vec()
            } else {
                vec![b'\r']
            }
        }
        Key::Tab => {
            if m.shift {
                b"\x1b[Z".to_vec()
            } else {
                vec![b'\t']
            }
        }
        Key::Backspace => {
            if m.ctrl {
                vec![0x08]
            } else {
                vec![0x7f]
            }
        }
        Key::Escape => vec![0x1b],
        Key::ArrowUp => csi('A'),
        Key::ArrowDown => csi('B'),
        Key::ArrowRight => csi('C'),
        Key::ArrowLeft => csi('D'),
        Key::Home => csi('H'),
        Key::End => csi('F'),
        Key::Insert => tilde(2),
        Key::Delete => tilde(3),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        Key::F1 => b"\x1bOP".to_vec(),
        Key::F2 => b"\x1bOQ".to_vec(),
        Key::F3 => b"\x1bOR".to_vec(),
        Key::F4 => b"\x1bOS".to_vec(),
        Key::F5 => tilde(15),
        Key::F6 => tilde(17),
        Key::F7 => tilde(18),
        Key::F8 => tilde(19),
        Key::F9 => tilde(20),
        Key::F10 => tilde(21),
        Key::F11 => tilde(23),
        Key::F12 => tilde(24),
        _ if m.ctrl => {
            let byte = match key {
                Key::Space => 0,
                Key::OpenBracket => 27,
                Key::Backslash => 28,
                Key::CloseBracket => 29,
                Key::Slash | Key::Minus => 31,
                _ => {
                    let name = key.name();
                    let b = name.as_bytes();
                    if b.len() == 1 && b[0].is_ascii_uppercase() {
                        b[0] - b'A' + 1
                    } else {
                        return None;
                    }
                }
            };
            if m.alt { vec![0x1b, byte] } else { vec![byte] }
        }
        _ => return None,
    };
    Some(out)
}
