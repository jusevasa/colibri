//! Dibuja la rejilla de la terminal y traduce teclado/ratón a bytes para el PTY.

use crate::config::Palette;
use crate::session::Session;
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor};
use eframe::egui::{
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
) {
    let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
    let origin = rect.min + Vec2::new(m.pad, m.pad / 2.0);
    let cols = ((rect.width() - m.pad * 2.0) / m.cell_w).floor().max(2.0) as usize;
    let lines = ((rect.height() - m.pad) / m.cell_h).floor().max(1.0) as usize;
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

    handle_input(ui, s, &resp, rect, origin, m, keyboard);

    let painter = ui.painter().with_clip_rect(rect);
    let mut bgs: Vec<BgRun> = Vec::new();
    let mut texts: Vec<TextRun> = Vec::new();
    let mut lines_deco: Vec<(i32, usize, usize, Color32, bool)> = Vec::new();
    let cursor;
    let cursor_cell_fg;
    {
        let term = s.term.lock();
        let content = term.renderable_content();
        let offset = content.display_offset as i32;
        let colors = content.colors;
        let selection = content.selection;
        cursor = (
            content.cursor.shape,
            content.cursor.point.line.0 + offset,
            content.cursor.point.column.0,
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
            let width = if flags.contains(Flags::WIDE_CHAR) {
                2
            } else {
                1
            };

            let mut fg = resolve(cell.fg, colors, pal);
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
            if flags.intersects(Flags::ALL_UNDERLINES | Flags::STRIKEOUT) {
                lines_deco.push((row, col, width, fg, flags.contains(Flags::STRIKEOUT)));
            }
            let c = cell.c;
            if c == ' ' || c == '\0' || flags.contains(Flags::HIDDEN) {
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
                        text.extend(z.iter());
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
    if shape != CursorShape::Hidden && crow >= 0 && (crow as usize) < lines {
        let min = cell_pos(crow, ccol);
        let cell = Rect::from_min_size(min, Vec2::new(m.cell_w, m.cell_h));
        if !focused {
            painter.rect_stroke(cell, 0.0, Stroke::new(1.0, pal.cursor), StrokeKind::Inside);
        } else {
            match shape {
                CursorShape::Beam => {
                    painter.rect_filled(
                        Rect::from_min_size(min, Vec2::new(2.0, m.cell_h)),
                        0.0,
                        pal.cursor,
                    );
                }
                CursorShape::Underline => {
                    let r = Rect::from_min_max(Pos2::new(min.x, min.y + m.cell_h - 2.0), cell.max);
                    painter.rect_filled(r, 0.0, pal.cursor);
                }
                CursorShape::HollowBlock => {
                    painter.rect_stroke(
                        cell,
                        0.0,
                        Stroke::new(1.0, pal.cursor),
                        StrokeKind::Inside,
                    );
                }
                _ => {
                    painter.rect_filled(cell, 0.0, pal.cursor);
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
    let col = (x.floor() as usize).min(s.size.cols - 1);
    let row = (y.floor() as usize).min(s.size.lines - 1);
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
) {
    let ctx = ui.ctx().clone();
    let mode = *s.term.lock().mode();
    let (events, scroll_y, mods) =
        ui.input(|i| (i.events.clone(), i.smooth_scroll_delta.y, i.modifiers));

    for ev in events {
        if !keyboard {
            break;
        }
        match ev {
            egui::Event::Text(t) => s.input(t.into_bytes()),
            egui::Event::Ime(egui::ImeEvent::Commit(t)) => s.input(t.into_bytes()),
            egui::Event::Paste(t) => s.paste(&t),
            // ⌘V con una imagen en el portapapeles (sin texto): se envía Ctrl+V para que
            // el programa (Claude Code, etc.) lea la imagen él mismo, como hace Warp.
            egui::Event::PasteImage(_) => s.input(vec![0x16]),
            egui::Event::Copy | egui::Event::Cut => {
                if let Some(text) = s.term.lock().selection_to_string() {
                    if !text.is_empty() {
                        ctx.copy_text(text);
                    }
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
                if let Some(bytes) = key_bytes(key, modifiers, mode) {
                    s.input(bytes);
                }
            }
            _ => {}
        }
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
    let reporting = mode.intersects(TermMode::MOUSE_MODE) && !mods.shift;
    if reporting {
        report_mouse(ui, s, rect, origin, m, mode, mods);
        return;
    }
    s.mouse_cell = None;
    s.mouse_down = None;
    let Some(pos) = resp.interact_pointer_pos() else {
        return;
    };
    let mut term = s.term.lock();
    let offset = term.grid().display_offset();
    let (point, side) = grid_point(pos, origin, m, s, offset);
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
    use super::mouse_seq;

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
