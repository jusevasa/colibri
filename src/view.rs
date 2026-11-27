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
    /// Un clic normal abre los enlaces web (preferencia `link_click`); si no, solo con ⌘.
    pub link_click: bool,
}

/// Glifos ya maquetados para la rejilla con instancias. Uno por `egui::Context` (por
/// ventana), guardado en el propio contexto: cada ventana tiene su atlas y su sonda, y una
/// caché compartida se tiraría entera cada vez que se pinta otra ventana.
type Glyphs = std::sync::Arc<std::sync::Mutex<crate::grid::GlyphCache>>;

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
    link_card(ui, s, link.as_ref(), rect, origin, pal, m);

    // Las líneas de cajas no se pintan como texto: en su sitio van las cajas, que dibuja
    // quien llama a partir de `s.chips_px`.
    let chip_rows = if chips { s.chip_rows() } else { Vec::new() };
    // Las rutas que existen en el comando de cada prompt, de otro color (como en Warp).
    let paths = s.path_tokens();
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
            // Con la fila de encima vacía, las cajas suben hacia ella y dejan aire sobre
            // el comando (12 px entre el borde de la caja y su fila, como en Warp).
            let lift = chip_lift(c.room_above, m.cell_h);
            let top = origin.y + c.row as f32 * m.cell_h - lift;
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
    let prompt_bar = !shifts.is_empty() && s.at_prompt();
    let painter = ui.painter().with_clip_rect(rect);
    // Una raya fina separa cada bloque del anterior, arriba de la línea en blanco sobre
    // sus cajas (con la primera fila de la pantalla no hay bloque anterior que separar).
    let rule = pal.bg.lerp_to_gamma(pal.fg, 0.10);
    for c in chip_rows.iter().filter(|c| c.rule_above) {
        let y = origin.y + (c.row - 1) as f32 * m.cell_h + 0.5;
        painter.hline(rect.x_range(), y, Stroke::new(1.0, rule));
    }
    // Cada prompt marcado sabe cómo acabó el comando anterior: si falló, una barra roja
    // en el margen recorre ese comando y su salida, y junto a sus cajas van el código de
    // salida y lo que tardó (la duración sola, si acabó bien pero tardó).
    for (i, c) in chip_rows.iter().enumerate() {
        let Some((status, ms)) = c.result else {
            continue;
        };
        let prev = i.checked_sub(1).map(|j| chip_rows[j].row);
        // La etiqueta va junto a las cajas del prompt anterior: sube con ellas.
        let prev_lift = i
            .checked_sub(1)
            .map_or(0.0, |j| chip_lift(chip_rows[j].room_above, m.cell_h));
        let failed = status != 0;
        let y = |row: i32| origin.y + row as f32 * m.cell_h;
        if failed {
            let bar = Rect::from_min_max(
                Pos2::new(rect.left(), y(prev.unwrap_or(0)) - 3.0 - prev_lift),
                Pos2::new(rect.left() + 3.0, y(c.row) - m.cell_h * 0.6),
            );
            painter.rect_filled(bar, 1.0, pal.ansi[1]);
        }
        let tx = &crate::i18n::t().term;
        let time = (tx.duration)(ms);
        let label = match (failed, ms >= 1000) {
            (true, _) => (tx.failed)(status, &time),
            (false, true) => time,
            (false, false) => continue,
        };
        if let Some(prev) = prev {
            painter.text(
                Pos2::new(
                    rect.right() - m.pad - 4.0,
                    y(prev) + m.cell_h / 2.0 - 4.0 - prev_lift,
                ),
                Align2::RIGHT_CENTER,
                label,
                egui::FontId::proportional(m.font.size * 0.85),
                if failed { pal.ansi[1] } else { pal.muted },
            );
        }
    }
    // Fondos, letras, subrayados y viñetas, fila a fila (`walk_row`). Con instancias, solo
    // se rehacen las filas que cambiaron (`grid_pass`). Lo que no son fondos ni letras
    // (viñetas, subrayados, cursor, cajas) va encima, con egui.
    let cursor;
    let cursor_cell_fg;
    let out = {
        let mut term = s.term.lock();
        let out = if crate::grid::instanced() {
            let ctx = ui.ctx().clone();
            let glyphs: Glyphs = ctx.data_mut(|d| {
                d.get_temp_mut_or_default::<Glyphs>(egui::Id::unique("colibri-glyphs"))
                    .clone()
            });
            let mut glyphs = glyphs.lock().unwrap_or_else(|e| e.into_inner());
            let mut pass = GridPass {
                pal,
                m,
                origin,
                link: link.as_ref(),
                chip_rows: &chip_rows,
                paths: &paths,
                shifts: &shifts,
            };
            pass.run(&mut term, &mut s.grid, &mut glyphs, &ctx, no_grid_cache())
        } else {
            GridOut::default()
        };
        let content = term.renderable_content();
        let offset = content.display_offset as i32;
        let look = Look {
            pal,
            colors: content.colors,
            selection: content.selection,
            link: link.as_ref(),
            chip_rows: &chip_rows,
            paths: &paths,
            shifts: &shifts,
        };
        let cursor_row = content.cursor.point.line.0 + offset;
        cursor = (
            content.cursor.shape,
            cursor_row,
            (content.cursor.point.column.0 as i32 - shift_of(&shifts, cursor_row)).max(0) as usize,
        );
        cursor_cell_fg = under_cursor(&term, cursor.1, cursor.2, offset, &look);
        if crate::grid::instanced() {
            out
        } else {
            // Camino de siempre de egui (`COLIBRI_GRID=egui`, para comparar): sin caché.
            let rows: Vec<RowRuns> = (0..term.screen_lines() as i32)
                .map(|row| walk_row(&term, row, offset, &look))
                .collect();
            GridOut::from_runs(rows)
        }
    };

    let cell_pos = |row: i32, col: usize| {
        Pos2::new(
            origin.x + col as f32 * m.cell_w,
            origin.y + row as f32 * m.cell_h,
        )
    };
    if let Some(batch) = &out.batch {
        crate::debug::timing::INSTANCES.fetch_add(
            batch.instances.len() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        // Fondos y letras como instancias de Metal (`grid.rs`), en un solo `Callback` en
        // el mismo sitio del orden de pintado: lo que va después queda encima, como antes.
        painter.add(egui::Shape::Callback(egui::PaintCallback {
            rect,
            callback: batch.clone(),
        }));
    }
    for b in out.runs.iter().flat_map(|r| &r.bgs) {
        let min = cell_pos(b.row, b.col);
        let max = Pos2::new(origin.x + b.end as f32 * m.cell_w, min.y + m.cell_h);
        painter.rect_filled(Rect::from_min_max(min, max), 0.0, b.color);
    }
    for t in out.runs.iter().flat_map(|r| &r.texts) {
        let pos = cell_pos(t.row, t.col) + Vec2::new(0.0, m.text_dy);
        let font = if t.bold {
            m.bold.clone()
        } else {
            m.font.clone()
        };
        painter.text(pos, Align2::LEFT_TOP, &t.text, font, t.fg);
    }
    for &(row, col, color) in &out.dots {
        let center = cell_pos(row, col) + Vec2::new(m.cell_w, m.cell_h) / 2.0;
        painter.circle_filled(center, (m.cell_w * 0.45).min(m.cell_h * 0.3), color);
    }
    for &(row, col, width, color, strike) in &out.deco {
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
    // Tras la sugerencia gris de zsh-autosuggestions, una cajita «→» dice con qué tecla se
    // acepta (como en Warp): a 10 px de su última letra, del alto del cursor.
    if focused
        && shape != CursorShape::Hidden
        && let Some(end) = s.autosuggestion_end()
    {
        let offset = s.term.lock().grid().display_offset() as i32;
        let row = end.line.0 + offset;
        let col = (end.column.0 as i32 - shift_of(&shifts, row)).max(0) as usize;
        if row >= 0 && (row as usize) < lines {
            let last = cell_pos(row, col);
            accept_hint(&painter, last + Vec2::new(m.cell_w, 0.0), m, pal);
        }
    }
    // Dónde colocar la ventana de composición si el cursor no está en pantalla.
    let mut ime_cell = Rect::from_min_size(rect.min, Vec2::new(m.cell_w, m.cell_h));
    // Aunque el programa oculte el cursor (Claude Code lo oculta y pinta el suyo), su
    // posición sigue siendo donde se escribe: ahí va lo que se compone.
    if crow >= 0 && (crow as usize) < lines {
        let hidden = shape == CursorShape::Hidden;
        let base = cell_pos(crow, ccol);
        if !hidden {
            s.cursor_px = Some(Rect::from_min_size(base, Vec2::new(m.cell_w, m.cell_h)));
        }
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
        ime_cell = cell;
        if hidden {
            // El programa pinta su propio cursor.
        } else if !focused {
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
    }
    // La entrada de texto del sistema va siempre activa, también con el cursor oculto o
    // fuera de la pantalla: sin ella macOS no compone las teclas muertas (⌥N + N daba
    // «˜n» en Claude Code en vez de «ñ»). `ime_cell` dice dónde poner la ventana de
    // composición (acentos, emoji).
    ui.output_mut(|o| {
        o.ime = Some(egui::output::IMEOutput {
            purpose: egui::IMEPurpose::Terminal,
            rect,
            cursor_rect: ime_cell,
            should_interrupt_composition: false,
        })
    });
}

/// La pista «→» de la sugerencia gris, empezando 10 px a la derecha de `after` (la esquina
/// de arriba de la celda que sigue a su última letra): alto del cursor (mín. 12 px), borde
/// de 1 px, radio del 25 % del alto y el texto del tema al 40 %, sin relleno.
fn accept_hint(painter: &egui::Painter, after: Pos2, m: &Metrics, pal: &Palette) {
    let h = m.cell_h.max(12.0);
    let color = pal.bg.lerp_to_gamma(pal.fg, 0.40);
    let font = egui::FontId::proportional((m.font.size - 2.0).max(8.0));
    let galley = painter.layout_no_wrap("→".into(), font, color);
    let w = (galley.size().x + 8.0).max(h);
    let rect = Rect::from_min_size(after + Vec2::new(10.0, 0.0), Vec2::new(w, h));
    let radius = (h * 0.25).round() as u8;
    painter.rect_stroke(
        rect,
        egui::CornerRadius::same(radius),
        Stroke::new(1.0, color),
        StrokeKind::Inside,
    );
    painter.galley(rect.center() - galley.size() / 2.0, galley, color);
}

/// Cuánto suben las cajas del prompt cuando encima hay una fila vacía. Con lo que
/// `prompt_chips` ya recorta (3 px arriba, 5 abajo) su borde inferior queda 12 px por
/// encima de la fila del comando.
const CHIP_LIFT: f32 = 7.0;

/// Lo que suben las cajas de una fila: `CHIP_LIFT` si hay hueco encima y cabe. Con filas
/// bajas, solo hasta 5 px del borde de arriba de la fila vacía (los 3 que `prompt_chips`
/// ya sube y 2 de margen para la raya), para no tocar la fila de más arriba.
fn chip_lift(room_above: bool, cell_h: f32) -> f32 {
    if room_above {
        CHIP_LIFT.min((cell_h - 5.0).max(0.0))
    } else {
        0.0
    }
}

/// `COLIBRI_NO_GRID_CACHE`: rehacer la rejilla entera en cada cuadro, para comparar.
fn no_grid_cache() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("COLIBRI_NO_GRID_CACHE").is_some())
}

/// Lo que sale de una fila: fondos y letras como instancias, subrayados (columna, ancho,
/// color, tachado) y viñetas ⏺ (columna, color).
#[derive(Default, PartialEq)]
struct RowOut {
    bgs: Vec<crate::grid::Instance>,
    letters: Vec<crate::grid::Instance>,
    deco: Vec<(usize, usize, Color32, bool)>,
    dots: Vec<(usize, Color32)>,
}

/// Todo lo que decide la rejilla y no está en las celdas: si cambia, se rehace entera. Las
/// celdas las sigue alacritty (`Term::damage`), salvo lo que Colibrí escribe por su cuenta
/// (las cajas del prompt), que cambia `chip_rows`.
#[derive(PartialEq)]
struct GridKey {
    origin: Pos2,
    cell: [f32; 3],
    fonts: (FontId, FontId),
    ppp: f32,
    size: (usize, usize),
    offset: usize,
    selection: Option<alacritty_terminal::selection::SelectionRange>,
    link: Option<Span>,
    chip_rows: Vec<crate::session::ChipRow>,
    shifts: Vec<(i32, i32)>,
    palette: Palette,
    /// `GlyphCache::generation`: si el atlas se rehízo, las coordenadas de los glifos ya no
    /// valen.
    glyphs: u64,
}

/// La rejilla del cuadro anterior, fila a fila, para rehacer en el siguiente solo las que
/// cambian. Una por pestaña (`Session::grid`).
#[derive(Default)]
pub struct GridCache {
    key: Option<GridKey>,
    rows: Vec<RowOut>,
    batch: Option<std::sync::Arc<crate::grid::GridBatch>>,
    /// Filas rehechas en el último cuadro.
    rebuilt: usize,
    /// Las rutas coloreadas en el último cuadro: una fila cuyas rutas cambian se rehace
    /// aunque alacritty no la dé por dañada (un archivo que se creó, otro prompt).
    paths: Vec<(i32, usize, usize, crate::session::Tok)>,
}

/// Lo que sale de la rejilla en un cuadro. Con instancias: el lote (fondos y letras) y, con
/// su fila, subrayados y viñetas. Por el camino de egui, los tramos (`runs`).
#[derive(Default)]
struct GridOut {
    batch: Option<std::sync::Arc<crate::grid::GridBatch>>,
    runs: Vec<RowRuns>,
    deco: Vec<(i32, usize, usize, Color32, bool)>,
    dots: Vec<(i32, usize, Color32)>,
}

impl GridOut {
    fn from_runs(mut runs: Vec<RowRuns>) -> Self {
        let mut out = GridOut::default();
        for (row, r) in runs.iter_mut().enumerate() {
            let row = row as i32;
            out.deco
                .extend(r.deco.drain(..).map(|(c, w, k, s)| (row, c, w, k, s)));
            out.dots.extend(r.dots.drain(..).map(|(c, k)| (row, c, k)));
        }
        out.runs = runs;
        out
    }
}

/// Una pasada de la rejilla con instancias: lo que no está en las celdas.
struct GridPass<'a> {
    pal: &'a Palette,
    m: &'a Metrics,
    origin: Pos2,
    link: Option<&'a Link>,
    chip_rows: &'a [crate::session::ChipRow],
    /// Rutas del comando a colorear (`Session::path_tokens`).
    paths: &'a [(i32, usize, usize, crate::session::Tok)],
    shifts: &'a [(i32, i32)],
}

impl GridPass<'_> {
    /// Rehace en `cache` las filas que cambiaron desde el cuadro anterior (todas con `full`
    /// o si cambió algo de `GridKey`) y devuelve el lote del cuadro: el mismo `Arc` si
    /// ninguna fila salió distinta.
    fn run<T>(
        &mut self,
        term: &mut Term<T>,
        cache: &mut GridCache,
        glyphs: &mut crate::grid::GlyphCache,
        ctx: &egui::Context,
        full: bool,
    ) -> GridOut {
        use alacritty_terminal::term::TermDamage;
        // Lo que cambió en las celdas desde la última vez (`None` = todo). Se consume aquí:
        // esta caché es la única que lo usa.
        let damaged: Option<Vec<usize>> = match term.damage() {
            TermDamage::Full => None,
            TermDamage::Partial(lines) => Some(lines.map(|l| l.line).collect()),
        };
        term.reset_damage();

        glyphs.begin(ctx, &self.m.font, &self.m.bold);
        let ppp = ctx.pixels_per_point();
        let offset = term.grid().display_offset() as i32;
        let selection = term.selection.as_ref().and_then(|s| s.to_range(term));
        let lines = term.screen_lines();
        let key = GridKey {
            origin: self.origin,
            cell: [self.m.cell_w, self.m.cell_h, self.m.text_dy],
            fonts: (self.m.font.clone(), self.m.bold.clone()),
            ppp,
            size: (term.columns(), lines),
            offset: offset as usize,
            selection,
            link: self.link.map(|l| l.span.clone()),
            chip_rows: self.chip_rows.to_vec(),
            shifts: self.shifts.to_vec(),
            palette: self.pal.clone(),
            glyphs: glyphs.generation(),
        };
        let full = full
            || damaged.is_none()
            || cache.key.as_ref() != Some(&key)
            || cache.rows.len() != lines;
        let mut rows: Vec<usize> = match &damaged {
            Some(rows) if !full => rows.iter().copied().filter(|&r| r < lines).collect(),
            _ => (0..lines).collect(),
        };
        if !full {
            let of_row = |list: &[(i32, usize, usize, crate::session::Tok)], row: i32| {
                list.iter()
                    .filter(|t| t.0 == row)
                    .copied()
                    .collect::<Vec<_>>()
            };
            for row in cache.paths.iter().chain(self.paths).map(|t| t.0) {
                let fresh = row >= 0
                    && (row as usize) < lines
                    && !rows.contains(&(row as usize))
                    && of_row(&cache.paths, row) != of_row(self.paths, row);
                if fresh {
                    rows.push(row as usize);
                }
            }
        }
        cache.paths = self.paths.to_vec();
        if full {
            cache.rows.clear();
            cache.rows.resize_with(lines, RowOut::default);
        }

        let look = Look {
            pal: self.pal,
            colors: term.colors(),
            selection,
            link: self.link,
            chip_rows: self.chip_rows,
            paths: self.paths,
            shifts: self.shifts,
        };
        let mut changed = full;
        for &row in &rows {
            let runs = walk_row(term, row as i32, offset, &look);
            let mut out = RowOut::default();
            row_instances(
                &runs,
                ctx,
                glyphs,
                self.origin,
                self.m,
                ppp,
                &mut out.bgs,
                &mut out.letters,
            );
            out.deco = runs.deco;
            out.dots = runs.dots;
            if cache.rows[row] != out {
                cache.rows[row] = out;
                changed = true;
            }
        }
        cache.rebuilt = rows.len();
        cache.key = Some(key);
        if changed {
            let mut instances: Vec<_> = cache
                .rows
                .iter()
                .flat_map(|r| r.bgs.iter().copied())
                .collect();
            instances.extend(cache.rows.iter().flat_map(|r| r.letters.iter().copied()));
            cache.batch = (!instances.is_empty())
                .then(|| std::sync::Arc::new(crate::grid::GridBatch { instances }));
        }
        crate::debug::timing::GRID_ROWS
            .fetch_add(rows.len() as u64, std::sync::atomic::Ordering::Relaxed);
        cache.out()
    }
}

impl GridCache {
    /// El cuadro tal como está guardado.
    fn out(&self) -> GridOut {
        let mut out = GridOut {
            batch: self.batch.clone(),
            ..Default::default()
        };
        for (row, r) in self.rows.iter().enumerate() {
            let row = row as i32;
            out.deco
                .extend(r.deco.iter().map(|&(c, w, k, s)| (row, c, w, k, s)));
            out.dots.extend(r.dots.iter().map(|&(c, k)| (row, c, k)));
        }
        out
    }
}

/// Desplazamiento en celdas de la fila `row` (ver `Session::row_shifts`).
fn shift_of(shifts: &[(i32, i32)], row: i32) -> i32 {
    shifts
        .iter()
        .find(|(r, _)| *r == row)
        .map_or(0, |(_, n)| *n)
}

/// Color de un trozo de comando: el que se escribe en una máquina remota (allí no hay
/// resaltado) o una ruta que existe en el comando de un prompt local.
fn token_at(
    chip_rows: &[crate::session::ChipRow],
    paths: &[(i32, usize, usize, crate::session::Tok)],
    row: i32,
    col: usize,
) -> Option<crate::session::Tok> {
    chip_rows
        .iter()
        .flat_map(|c| c.tokens.iter())
        .chain(paths)
        .find(|(r, from, to, _)| *r == row && (*from..*to).contains(&col))
        .map(|(.., tok)| *tok)
}

/// Lo que decide cómo se pintan las celdas y no está en ellas.
struct Look<'a> {
    pal: &'a Palette,
    colors: &'a Colors,
    selection: Option<alacritty_terminal::selection::SelectionRange>,
    link: Option<&'a Link>,
    chip_rows: &'a [crate::session::ChipRow],
    paths: &'a [(i32, usize, usize, crate::session::Tok)],
    shifts: &'a [(i32, i32)],
}

/// Fondos, letras, subrayados (columna, ancho, color, tachado) y viñetas ⏺ (columna, color)
/// de una fila de la pantalla.
#[derive(Default)]
struct RowRuns {
    bgs: Vec<BgRun>,
    texts: Vec<TextRun>,
    deco: Vec<(usize, usize, Color32, bool)>,
    dots: Vec<(usize, Color32)>,
}

/// Los tramos de la fila `row` de la pantalla (`offset`: desplazamiento por el historial).
fn walk_row<T>(term: &Term<T>, row: i32, offset: i32, look: &Look) -> RowRuns {
    let pal = look.pal;
    let mut out = RowRuns::default();
    let line = Line(row - offset);
    let cells = &term.grid()[line];
    let shift = shift_of(look.shifts, row);
    for col in 0..term.columns() {
        let point = Point::new(line, Column(col));
        let cell = &cells[Column(col)];
        let flags = cell.flags;
        if flags.contains(Flags::WIDE_CHAR_SPACER) {
            continue;
        }
        if look.chip_rows.iter().any(|c| c.row == row && col < c.end) {
            continue;
        }
        let token = token_at(look.chip_rows, look.paths, row, col);
        let shifted = col as i32 - shift;
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
            Some(crate::session::Tok::Path) => pal.ansi[5],
            None => resolve(cell.fg, look.colors, pal),
        };
        let mut bg = resolve(cell.bg, look.colors, pal);
        if flags.contains(Flags::DIM) {
            fg = dim(fg);
        }
        if flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if look.selection.is_some_and(|sel| sel.contains(point)) {
            bg = pal.selection;
        }
        // El enlace bajo el puntero (con ⌘): del color de acento y subrayado.
        let linked = look.link.is_some_and(|l| l.contains(point, cell));
        if linked {
            fg = pal.accent;
        }
        if bg != pal.bg {
            match out.bgs.last_mut() {
                Some(last) if last.end == col && last.color == bg => last.end = col + width,
                _ => out.bgs.push(BgRun {
                    row,
                    col,
                    end: col + width,
                    color: bg,
                }),
            }
        }
        // Una ruta coloreada va sin el subrayado con que la marca zsh-syntax-highlighting.
        let underline =
            flags.intersects(Flags::ALL_UNDERLINES) && token != Some(crate::session::Tok::Path);
        if linked {
            out.deco.push((col, width, fg, false));
        } else if underline || flags.contains(Flags::STRIKEOUT) {
            out.deco
                .push((col, width, fg, flags.contains(Flags::STRIKEOUT)));
        }
        let c = cell.c;
        if c == ' ' || c == '\0' || flags.contains(Flags::HIDDEN) {
            continue;
        }
        // ⏺ (la viñeta de Claude Code) solo existe como emoji de color en macOS:
        // se dibuja a mano para que salga como un punto del color del texto.
        if c == '\u{23FA}' {
            out.dots.push((col, fg));
            continue;
        }
        let bold = flags.contains(Flags::BOLD);
        let zero = cell.zerowidth();
        let ascii = c.is_ascii() && zero.is_none();
        match out.texts.last_mut() {
            Some(last)
                if ascii
                    && last.ascii
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
                out.texts.push(TextRun {
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
    out
}

/// La letra bajo el cursor (`row`, `col` ya desplazada), que el cursor de bloque pinta del
/// color del fondo: la celda que el recorrido de esa fila pondría en esa columna.
fn under_cursor<T>(
    term: &Term<T>,
    row: i32,
    col: usize,
    offset: i32,
    look: &Look,
) -> Option<(char, bool)> {
    if row < 0 || row as usize >= term.screen_lines() {
        return None;
    }
    let orig = col as i32 + shift_of(look.shifts, row);
    if orig < 0 || orig as usize >= term.columns() {
        return None;
    }
    let orig = orig as usize;
    if look.chip_rows.iter().any(|c| c.row == row && orig < c.end) {
        return None;
    }
    let cell = &term.grid()[Line(row - offset)][Column(orig)];
    if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
        return None;
    }
    Some((cell.c, cell.flags.contains(Flags::BOLD)))
}

/// Las instancias de una fila: sus fondos en `bgs` y sus letras en `letters` (van detrás de
/// todos los fondos de la pantalla).
#[allow(clippy::too_many_arguments)]
fn row_instances(
    runs: &RowRuns,
    ctx: &egui::Context,
    cache: &mut crate::grid::GlyphCache,
    origin: Pos2,
    m: &Metrics,
    ppp: f32,
    bgs: &mut Vec<crate::grid::Instance>,
    letters: &mut Vec<crate::grid::Instance>,
) {
    let cell_pos = |row: i32, col: usize| {
        Pos2::new(
            origin.x + col as f32 * m.cell_w,
            origin.y + row as f32 * m.cell_h,
        )
    };
    for b in &runs.bgs {
        // Como `rect_filled`: el teselador lleva el rectángulo a píxeles enteros.
        let min = cell_pos(b.row, b.col);
        let max = Pos2::new(origin.x + b.end as f32 * m.cell_w, min.y + m.cell_h);
        let r = egui::emath::GuiRounding::round_to_pixels(Rect::from_min_max(min, max), ppp);
        bgs.push(crate::grid::Instance::solid(r.min, r.max, b.color));
    }
    for t in &runs.texts {
        let pos = cell_pos(t.row, t.col) + Vec2::new(0.0, m.text_dy);
        cache.push_text(ctx, letters, pos, &t.text, t.ascii, t.bold, t.fg);
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
    // Sin modificadores (preferencia `link_click`) pasa lo mismo, pero solo con enlaces web
    // fuera del comando que se escribe (`opens_on_click`): las rutas siguen siendo del
    // programa (Quick Peek) y un clic en el comando coloca el cursor.
    let plain = m.link_click && mods.is_none();
    let found = ((mods.command || plain) && hovered)
        .then(|| ctx.pointer_hover_pos())
        .flatten()
        .and_then(|pos| {
            let term = s.term.lock();
            let offset = term.grid().display_offset();
            let (point, _) = grid_point(pos, origin, m, s, offset);
            // Las rutas son de esta máquina: dentro de una sesión ssh no se buscan. Sin ⌘
            // tampoco: solo cuentan las direcciones web (y no se mira el disco al pasar).
            let cwd = (mods.command && !s.remote()).then_some(s.cwd.as_path());
            link_at(&term, point, cwd).map(|link| (link, point))
        });
    let link = match found {
        Some((link, _)) if mods.command => Some(link),
        Some((link, point)) => {
            let in_command = s
                .input_cells()
                .is_some_and(|(cells, _)| cells.contains(&point));
            opens_on_click(&link.target, in_command).then_some(link)
        }
        None => None,
    };
    // El enlace bajo el puntero sin ⌘ y desde cuándo (para el recuadro con la dirección).
    let hover_span = link
        .as_ref()
        .filter(|_| !mods.command)
        .map(|l| l.span.clone());
    s.link_hover = match (hover_span, s.link_hover.take()) {
        (Some(span), Some((old, since))) if span == old => Some((old, since)),
        (Some(span), _) => Some((span, std::time::Instant::now())),
        (None, _) => None,
    };
    if let Some(link) = &link {
        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        if mods.command {
            if resp.clicked() {
                open_once(s, link, &m.editor);
            }
            return link_some(link);
        }
    }

    let reporting = mode.intersects(TermMode::MOUSE_MODE) && !mods.shift;
    if reporting {
        report_mouse(ui, s, rect, origin, m, mode, mods, link.as_ref());
        return link;
    }
    s.link_press = None;
    // Sin informes de ratón: un clic (sin arrastrar) sobre el enlace lo abre.
    if let Some(link) = &link
        && resp.clicked()
    {
        open_once(s, link, &m.editor);
        return link_some(link);
    }
    s.mouse_cell = None;
    s.mouse_down = None;
    let app_cursor = mode.contains(TermMode::APP_CURSOR);
    if resp.drag_stopped() {
        adopt_selection(s, app_cursor);
    }
    let Some(pos) = resp.interact_pointer_pos() else {
        return link;
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
    link
}

/// Un enlace que se puede abrir: el de un hiperenlace OSC 8 (lo que emite Claude Code),
/// una dirección escrita tal cual en la pantalla o la ruta de un archivo que existe.
#[derive(Clone)]
struct Link {
    target: Target,
    span: Span,
}

/// Cuánto tiene que quedarse quieto el puntero sobre un enlace para que salga su recuadro.
const LINK_CARD_DELAY: std::time::Duration = std::time::Duration::from_millis(400);

/// El recuadro con la dirección completa del enlace web bajo el puntero (sin ⌘), cuando lleva
/// `LINK_CARD_DELAY` encima: debajo de su fila o, si no cabe, encima. Antes de tiempo pide
/// una sola pasada para entonces; quieto después, no repinta.
fn link_card(
    ui: &egui::Ui,
    s: &Session,
    link: Option<&Link>,
    rect: Rect,
    origin: Pos2,
    pal: &Palette,
    m: &Metrics,
) {
    let (Some(link), Some((span, since))) = (link, s.link_hover.as_ref()) else {
        return;
    };
    let Target::Url(url) = &link.target else {
        return;
    };
    if link.span != *span {
        return;
    }
    let waited = since.elapsed();
    if waited < LINK_CARD_DELAY {
        ui.ctx().request_repaint_after(LINK_CARD_DELAY - waited);
        return;
    }
    let Some(pointer) = ui.ctx().pointer_hover_pos() else {
        return;
    };
    let row = ((pointer.y - origin.y) / m.cell_h).floor();
    let row_top = origin.y + row * m.cell_h;
    const HEIGHT: f32 = 44.0;
    let below = row_top + m.cell_h + 4.0;
    let y = if below + HEIGHT <= rect.bottom() {
        below
    } else {
        row_top - HEIGHT - 4.0
    };
    let text = middle_truncate(url, 80);
    let mono = FontId::monospace(11.5);
    let width = ui
        .ctx()
        .fonts_mut(|f| f.layout_no_wrap(text.clone(), mono.clone(), pal.fg))
        .size()
        .x
        + 24.0;
    let x = (pointer.x - 12.0)
        .min(rect.right() - width - 4.0)
        .max(rect.left() + 4.0);
    egui::Area::new(egui::Id::unique("colibri-link-card"))
        .order(egui::Order::Tooltip)
        .fixed_pos(Pos2::new(x, y))
        .interactable(false)
        .show(ui.ctx(), |ui| {
            egui::Frame::new()
                .fill(pal.panel)
                .stroke(Stroke::new(1.0, pal.border))
                .corner_radius(egui::CornerRadius::same(6))
                .inner_margin(egui::Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(0.0, 2.0);
                    ui.label(egui::RichText::new(text).font(mono).color(pal.fg));
                    ui.label(
                        egui::RichText::new(crate::i18n::t().term.link_open)
                            .font(FontId::proportional(11.0))
                            .color(pal.muted),
                    );
                });
        });
}

/// `text` con como mucho `max` caracteres: si no cabe, se quita del medio (queda más del
/// principio, que dice a dónde va el enlace) y en su lugar va «…».
fn middle_truncate(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max || max < 2 {
        return text.to_string();
    }
    let head = (max - 1) * 2 / 3;
    let tail = max - 1 - head;
    let mut out: String = text.chars().take(head).collect();
    out.push('…');
    out.extend(text.chars().skip(count - tail));
    out
}

/// Tiempo en el que un segundo clic sobre el mismo enlace es el doble clic del primero.
const DOUBLE_CLICK: std::time::Duration = std::time::Duration::from_millis(500);

/// Abrir `span` ahora repetiría lo que abrió el clic anterior: es el segundo clic de un doble
/// clic sobre el mismo enlace (sin esto, el navegador abría dos pestañas).
fn repeated_open(
    last: Option<&(Span, std::time::Instant)>,
    span: &Span,
    now: std::time::Instant,
) -> bool {
    last.is_some_and(|(seen, at)| seen == span && now.duration_since(*at) < DOUBLE_CLICK)
}

/// Abre el enlace, salvo que sea el segundo clic de un doble clic sobre él.
fn open_once(s: &mut Session, link: &Link, editor: &str) {
    let now = std::time::Instant::now();
    if !repeated_open(s.last_link_open.as_ref(), &link.span, now) {
        open_link(&link.target, editor);
    }
    s.last_link_open = Some((link.span.clone(), now));
}

fn link_some(link: &Link) -> Option<Link> {
    Some(link.clone())
}

/// Un clic normal (sin ⌘) abre el enlace: solo las direcciones web, y no dentro del comando
/// que se escribe (ahí el clic coloca el cursor).
fn opens_on_click(target: &Target, in_command: bool) -> bool {
    matches!(target, Target::Url(_)) && !in_command
}

/// Pulsación sobre un enlace web en un programa que lee el ratón: se guarda hasta saber si
/// es un clic (se abre el enlace y el programa no recibe nada) o el principio de un
/// arrastre (se le manda y el arrastre sigue como siempre).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Withheld {
    pub button: u8,
    /// Celda de la pulsación (columna, fila), como en los informes de ratón.
    pub cell: (usize, usize),
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum PointerEv {
    Move((usize, usize)),
    Release,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum WithheldOut {
    /// Sigue sin saberse.
    Wait,
    /// Es un arrastre: mandar al programa la pulsación guardada.
    SendPress,
    /// Es un clic: abrir el enlace.
    Open,
}

fn withheld_step(w: Withheld, ev: PointerEv) -> (Option<Withheld>, WithheldOut) {
    match ev {
        PointerEv::Move(cell) if cell == w.cell => (Some(w), WithheldOut::Wait),
        PointerEv::Move(_) => (None, WithheldOut::SendPress),
        PointerEv::Release => (None, WithheldOut::Open),
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Target {
    Url(String),
    /// Archivo o carpeta de esta máquina, con la línea y la columna si el texto las decía
    /// (`src/view.rs:120:8`).
    File(PathBuf, Option<u32>, Option<u32>),
}

/// Qué celdas ocupa el enlace.
#[derive(Clone, PartialEq, Debug)]
pub enum Span {
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

#[allow(clippy::too_many_arguments)]
fn report_mouse(
    ui: &egui::Ui,
    s: &mut Session,
    rect: Rect,
    origin: Pos2,
    m: &Metrics,
    mode: TermMode,
    mods: Modifiers,
    link: Option<&Link>,
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
            // Botón izquierdo sobre un enlace web: se guarda hasta saber si es un clic.
            if b == 0 && link.is_some() {
                s.link_press = Some(Withheld { button: 0, cell });
                continue;
            }
            s.mouse_down = Some(b as u8);
            out.extend(
                mouse_seq(b as u8 + mod_bits, cell.0, cell.1, true, sgr).unwrap_or_default(),
            );
        }
    }
    if let Some(w) = s.link_press
        && cell != w.cell
    {
        let (keep, step) = withheld_step(w, PointerEv::Move(cell));
        s.link_press = keep;
        if step == WithheldOut::SendPress {
            // Era un arrastre: el programa recibe la pulsación donde fue y el arrastre sigue.
            s.mouse_down = Some(w.button);
            s.mouse_cell = Some(w.cell);
            out.extend(
                mouse_seq(w.button + mod_bits, w.cell.0, w.cell.1, true, sgr).unwrap_or_default(),
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
        if let Some(w) = s.link_press.filter(|w| is_released && w.button == b as u8) {
            let (keep, step) = withheld_step(w, PointerEv::Release);
            s.link_press = keep;
            if let (WithheldOut::Open, Some(link)) = (step, link) {
                open_once(s, link, &m.editor);
            }
            continue;
        }
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
    /// Las cajas suben 7 px si cabe; con filas bajas (letra pequeña) solo lo que deja la
    /// fila vacía de encima, sin tocar la de más arriba ni la raya.
    #[test]
    fn las_cajas_suben_lo_que_cabe() {
        assert_eq!(super::chip_lift(false, 20.0), 0.0);
        assert_eq!(super::chip_lift(true, 20.0), 7.0);
        assert_eq!(super::chip_lift(true, 10.0), 5.0);
        assert_eq!(super::chip_lift(true, 4.0), 0.0);
    }

    use super::{PointerEv, Target, Withheld, WithheldOut, opens_on_click, withheld_step};
    use super::{file_command, mouse_seq, path_span, split_position, url_span};

    #[test]
    fn direccion_cortada_por_el_medio() {
        assert_eq!(super::middle_truncate("https://a.b/c", 40), "https://a.b/c");
        let s = super::middle_truncate(&format!("https://example.com/{}", "x".repeat(100)), 30);
        assert_eq!(s.chars().count(), 30);
        assert!(s.starts_with("https://example") && s.contains('…') && s.ends_with("xxx"));
    }
    use std::path::PathBuf;

    #[test]
    fn clic_normal_solo_en_enlaces_web() {
        let url = Target::Url("https://example.com/pr/13".into());
        let file = Target::File(PathBuf::from("/tmp/x"), None, None);
        assert!(opens_on_click(&url, false));
        // Dentro del comando que se escribe, el clic coloca el cursor.
        assert!(!opens_on_click(&url, true));
        // Las rutas siguen siendo de Quick Peek (clic) y de Colibrí con ⌘.
        assert!(!opens_on_click(&file, false));
    }

    #[test]
    fn doble_clic_abre_una_vez() {
        use super::{Span, repeated_open};
        use std::time::{Duration, Instant};
        let t0 = Instant::now();
        let url = Span::Tag("https://example.com".into());
        let otra = Span::Tag("https://example.org".into());
        let last = Some((url.clone(), t0));
        assert!(repeated_open(
            last.as_ref(),
            &url,
            t0 + Duration::from_millis(200)
        ));
        assert!(!repeated_open(
            last.as_ref(),
            &url,
            t0 + Duration::from_millis(900)
        ));
        assert!(!repeated_open(
            last.as_ref(),
            &otra,
            t0 + Duration::from_millis(200)
        ));
        assert!(!repeated_open(None, &url, t0));
    }

    #[test]
    fn pulsacion_aplazada_sobre_un_enlace() {
        let w = Withheld {
            button: 0,
            cell: (24, 1),
        };
        assert_eq!(
            withheld_step(w, PointerEv::Move((24, 1))),
            (Some(w), WithheldOut::Wait)
        );
        assert_eq!(
            withheld_step(w, PointerEv::Move((25, 1))),
            (None, WithheldOut::SendPress)
        );
        assert_eq!(
            withheld_step(w, PointerEv::Release),
            (None, WithheldOut::Open)
        );
    }
    use std::path::Path;

    /// La rejilla con caché sale igual que rehecha entera y rehace solo las filas que
    /// cambian (`Term::damage`), o todas si cambia algo que no está en las celdas.
    #[test]
    fn rejilla_incremental_igual_que_entera() {
        use super::{GridCache, GridOut, GridPass, Metrics};
        use alacritty_terminal::event::VoidListener;
        use alacritty_terminal::index::{Column, Line, Point, Side};
        use alacritty_terminal::selection::{Selection, SelectionType};
        use alacritty_terminal::term::{Config, Term};
        use alacritty_terminal::vte::ansi::{CursorShape, Processor};
        use egui::{FontId, Pos2};
        let size = crate::session::TermSize {
            cols: 40,
            lines: 10,
        };
        let mut term = Term::new(Config::default(), &size, VoidListener);
        let mut parser: Processor = Processor::new();
        let text = "\x1b[41mrojo\x1b[0m normal \x1b[1mnegrita\x1b[0m\r\n\x1b[4msub\x1b[0m \u{23FA} 漢字 ñ\r\nfin";
        parser.advance(&mut term, text.as_bytes());
        let ctx = egui::Context::default();
        let pal = crate::config::Palette::new(&crate::config::Theme::default());
        let m = Metrics {
            font: FontId::monospace(13.0),
            bold: FontId::monospace(13.0),
            cell_w: 8.0,
            cell_h: 16.0,
            text_dy: 1.0,
            pad: 4.0,
            cursor: CursorShape::Block,
            blink: false,
            editor: String::new(),
            link_click: true,
        };
        let mut glyphs = crate::grid::GlyphCache::default();
        let mut pass = |term: &mut Term<VoidListener>, cache: &mut GridCache, full: bool| {
            let mut out = GridOut::default();
            let mut frame = ctx.run_ui(egui::RawInput::default(), |ui| {
                let ctx = ui.ctx().clone();
                let mut p = GridPass {
                    pal: &pal,
                    m: &m,
                    origin: Pos2::new(4.0, 4.0),
                    link: None,
                    chip_rows: &[],
                    paths: &[],
                    shifts: &[],
                };
                out = p.run(term, cache, &mut glyphs, &ctx, full);
            });
            frame.textures_delta.clear();
            out
        };
        let same = |a: &GridOut, b: &GridOut| {
            let inst = |o: &GridOut| o.batch.as_ref().map(|b| b.instances.clone());
            assert_eq!(inst(a), inst(b), "instancias");
            assert_eq!(a.deco, b.deco, "subrayados");
            assert_eq!(a.dots, b.dots, "viñetas");
        };
        let mut cache = GridCache::default();
        pass(&mut term, &mut cache, false);
        assert_eq!(cache.rebuilt, 10, "la primera vez, todas");

        // Cambia una fila: solo esa y las del cursor (la de antes y la de ahora).
        parser.advance(&mut term, b"\x1b[2;3HXYZ");
        let inc = pass(&mut term, &mut cache, false);
        assert!(cache.rebuilt <= 3, "filas rehechas: {}", cache.rebuilt);
        same(&inc, &pass(&mut term, &mut GridCache::default(), true));

        // La selección no está en las celdas: se rehace entera.
        let mut sel = Selection::new(
            SelectionType::Simple,
            Point::new(Line(0), Column(1)),
            Side::Left,
        );
        sel.update(Point::new(Line(1), Column(5)), Side::Right);
        term.selection = Some(sel);
        let inc = pass(&mut term, &mut cache, false);
        assert_eq!(cache.rebuilt, 10, "con la selección nueva, todas");
        same(&inc, &pass(&mut term, &mut GridCache::default(), true));

        // Sin cambios: solo la fila del cursor, y el mismo lote de antes.
        let before = cache.batch.clone().expect("hay lote");
        pass(&mut term, &mut cache, false);
        assert!(cache.rebuilt <= 1, "filas rehechas: {}", cache.rebuilt);
        let after = cache.batch.clone().expect("hay lote");
        assert!(
            std::sync::Arc::ptr_eq(&before, &after),
            "el mismo lote, sin copiarlo"
        );
    }

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
