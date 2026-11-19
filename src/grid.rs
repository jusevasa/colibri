//! Rejilla con instancias: los fondos y las letras de la terminal se dibujan en Metal como
//! rectángulos instanciados (uno por fondo o por glifo), sin pasar por la maquetación de
//! texto ni por el teselado de egui. Es lo que hace Warp; el código es propio.
//!
//! - [`Instance`]: un rectángulo (en puntos), su trozo del atlas de fuentes (en texeles) y
//!   su color. Los fondos usan el texel blanco del atlas (`epaint::WHITE_UV`).
//! - [`GridBatch`]: las instancias de un cuadro. Viaja dentro de un
//!   `egui::Shape::Callback` y lo dibuja `metal::Renderer` con su segunda tubería.
//! - [`GlyphCache`]: qué rectángulos produciría egui para cada letra. Se rellena maquetando
//!   cada texto **una vez** con la propia fuente de egui y leyendo la malla del galley, así
//!   que el resultado es el mismo píxel a píxel que con `painter.text` (mismos glifos del
//!   atlas, mismo redondeo, mismo posicionamiento subpíxel).
//!
//! **Cuándo se tira la caché.** Guarda coordenadas del atlas de egui, que solo valen
//! mientras el atlas no se rehaga. egui lo rehace (vacío) en tres casos: al llenarse más
//! del 80 % (`Fonts::begin_pass`), al cambiar las `TextOptions` y al cambiar las fuentes
//! (crea un `Fonts` nuevo). En los tres vacía **también** su caché de galleys, y nunca la
//! vacía a medias de una pasada salvo para estrenar fuentes antes de maquetar nada. Así que
//! al principio de cada rejilla se maqueta una sonda («M», que sale de la caché de egui y no
//! cuesta nada) y se compara con la de la pasada anterior por puntero (`Arc::ptr_eq`): si
//! no es el mismo galley, la caché de egui se vació (o lo desalojó por no usarlo) y se tira
//! la nuestra antes de dibujar nada. Como guardamos el galley viejo, su dirección no se
//! puede reutilizar. Además la clave incluye las fuentes (`FontId`, con su tamaño) y
//! `pixels_per_point`: un cambio de tamaño o de pantalla (Retina o no) también la tira.
//! El atlas crece en alto sin mover lo que ya tiene; por eso las coordenadas se guardan en
//! texeles y el pintor las normaliza con el tamaño del atlas del cuadro que dibuja.
//! (El único caso que no se detecta es el que tampoco aguanta egui: un atlas desbordado,
//! de 16384 texeles de alto, que empieza a pisar glifos a media pasada.)

use std::collections::HashMap;
use std::sync::Arc;

use egui::emath::GuiRounding as _;
use egui::epaint::text::Galley;
use egui::{Color32, FontId, Pos2, Vec2};

/// Un rectángulo de la rejilla, tal cual lo lee el sombreador (`GridInstance` en
/// `metal.rs`): 40 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Instance {
    /// Esquinas en puntos.
    pub min: [f32; 2],
    pub max: [f32; 2],
    /// Esquinas en texeles del atlas de fuentes (`TextureId::default()`).
    pub uv_min: [f32; 2],
    pub uv_max: [f32; 2],
    /// Alfa premultiplicado, como en `epaint::Vertex`.
    pub color: Color32,
    pub _pad: u32,
}

const _: () = assert!(size_of::<Instance>() == 40);

impl Instance {
    /// Un fondo: el texel blanco del atlas, como `painter.rect_filled`.
    pub fn solid(min: Pos2, max: Pos2, color: Color32) -> Self {
        Self {
            min: [min.x, min.y],
            max: [max.x, max.y],
            uv_min: [0.0; 2],
            uv_max: [0.0; 2],
            color,
            _pad: 0,
        }
    }
}

/// Lo que dibuja el pintor de Metal en el sitio del `Shape::Callback`.
pub struct GridBatch {
    pub instances: Vec<Instance>,
}

/// ¿Rejilla con instancias (por defecto) o con el camino de siempre de egui
/// (`COLIBRI_GRID=egui`, para comparar)?
pub fn instanced() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("COLIBRI_GRID").map_or(true, |v| v != "egui"))
}

/// El rectángulo de un glifo tal como lo deja egui en la malla del galley.
#[derive(Clone, Copy, Debug)]
struct Quad {
    /// Esquina de arriba a la izquierda, en puntos. En `other` es relativa al origen del
    /// galley; en `ascii`, al origen del galley más la x del glifo (`x_px / ppp`).
    min: Vec2,
    size: Vec2,
    uv_min: [u16; 2],
    uv_max: [u16; 2],
    /// Emoji de color: no se tiñe (egui lo pinta en blanco con el alfa del texto).
    is_color: bool,
}

/// Una letra ASCII: su avance y su glifo en cada una de las cuatro posiciones subpíxel.
#[derive(Clone, Copy, Default)]
struct Ascii {
    adv_px: f32,
    /// `None`: esa posición aún no se ha maquetado. `Some(None)`: no tiene píxeles.
    bins: [Option<Option<Quad>>; 4],
}

/// Las posiciones subpíxel de epaint (`SubpixelBin::new` en `glyph_atlas.rs`, que es
/// privada): la x entera del glifo y en cuál de los cuatro cuartos de píxel cae.
fn subpixel_bin(pos: f32) -> (i32, usize) {
    let trunc = pos as i32;
    let fract = pos - trunc as f32;
    if pos.is_sign_negative() {
        if fract > -0.125 {
            (trunc, 0)
        } else if fract > -0.375 {
            (trunc - 1, 3)
        } else if fract > -0.625 {
            (trunc - 1, 2)
        } else if fract > -0.875 {
            (trunc - 1, 1)
        } else {
            (trunc - 1, 0)
        }
    } else if fract < 0.125 {
        (trunc, 0)
    } else if fract < 0.375 {
        (trunc, 1)
    } else if fract < 0.625 {
        (trunc, 2)
    } else if fract < 0.875 {
        (trunc, 3)
    } else {
        (trunc + 1, 0)
    }
}

/// Textos que no son ASCII que se guardan como mucho (CJK, emoji, combinaciones…).
const OTHER_MAX: usize = 4096;

/// Los glifos ya maquetados. Ver al principio del módulo cuándo se tira.
#[derive(Default)]
pub struct GlyphCache {
    /// Fuente normal, negrita y `pixels_per_point` de lo guardado.
    key: Option<(FontId, FontId, u32)>,
    font: Option<(FontId, FontId)>,
    ppp: f32,
    /// La sonda de la pasada anterior (ver arriba).
    probe: Option<Arc<Galley>>,
    /// `[negrita][código]`.
    ascii: Vec<[Option<Ascii>; 128]>,
    space_px: [Option<f32>; 2],
    /// Cualquier otro texto de una celda (con sus caracteres de ancho cero): sus glifos,
    /// relativos al origen del galley.
    other: HashMap<(String, bool), Vec<Quad>>,
}

impl GlyphCache {
    /// Al principio de cada rejilla, antes de usar nada: tira lo guardado si ya no vale.
    pub fn begin(&mut self, ctx: &egui::Context, font: &FontId, bold: &FontId) {
        let ppp = ctx.pixels_per_point();
        let key = (font.clone(), bold.clone(), ppp.to_bits());
        let probe = ctx.fonts_mut(|f| f.layout_no_wrap("M".into(), font.clone(), Color32::WHITE));
        let same = self.key.as_ref() == Some(&key)
            && self.probe.as_ref().is_some_and(|p| Arc::ptr_eq(p, &probe));
        if !same {
            self.ascii = vec![[None; 128]; 2];
            self.space_px = [None; 2];
            self.other.clear();
            self.key = Some(key);
        }
        self.probe = Some(probe);
        self.font = Some((font.clone(), bold.clone()));
        self.ppp = ppp;
    }

    fn layout(&self, ctx: &egui::Context, text: String, bold: bool) -> Arc<Galley> {
        let (font, b) = self.font.clone().expect("GlyphCache::begin va antes");
        let font = if bold { b } else { font };
        ctx.fonts_mut(|f| f.layout_no_wrap(text, font, Color32::WHITE))
    }

    /// Los glifos de un texto como `painter.text(pos, LEFT_TOP, text, fuente, color)`.
    /// `ascii`: solo caracteres ASCII de una fuente monoespaciada, como los tramos de
    /// `view.rs` (se resuelven letra a letra); si no, el texto de una sola celda.
    #[allow(clippy::too_many_arguments)]
    pub fn push_text(
        &mut self,
        ctx: &egui::Context,
        out: &mut Vec<Instance>,
        pos: Pos2,
        text: &str,
        ascii: bool,
        bold: bool,
        color: Color32,
    ) {
        let ppp = self.ppp;
        // El teselador lleva el galley al principio de un píxel físico.
        let origin = pos.round_to_pixels(ppp);
        let mut emit = |at: Pos2, q: &Quad| {
            let min = at + q.min;
            let max = min + q.size;
            out.push(Instance {
                min: [min.x, min.y],
                max: [max.x, max.y],
                uv_min: [q.uv_min[0] as f32, q.uv_min[1] as f32],
                uv_max: [q.uv_max[0] as f32, q.uv_max[1] as f32],
                color: if q.is_color {
                    Color32::from_white_alpha(color.a())
                } else {
                    color
                },
                _pad: 0,
            });
        };
        if ascii {
            // Como en la maquetación de epaint: cada letra en la x acumulada del tramo, con
            // su posición subpíxel.
            let mut cursor = 0.0f32;
            for c in text.bytes() {
                let (x_px, bin) = subpixel_bin(cursor);
                let (adv, quad) = self.ascii(ctx, c, bold, bin);
                if let Some(q) = quad {
                    emit(origin + Vec2::new(x_px as f32 / ppp, 0.0), &q);
                }
                cursor += adv;
            }
        } else {
            let key = (text.to_owned(), bold);
            if !self.other.contains_key(&key) {
                if self.other.len() >= OTHER_MAX {
                    self.other.clear();
                }
                let galley = self.layout(ctx, text.to_owned(), bold);
                let mut quads = Vec::new();
                for row in &galley.rows {
                    for g in &row.glyphs {
                        if let Some(q) = quad_of(row, g) {
                            quads.push(Quad {
                                min: q.min + row.pos.to_vec2(),
                                ..q
                            });
                        }
                    }
                }
                self.other.insert(key.clone(), quads);
            }
            for q in &self.other[&key] {
                emit(origin, q);
            }
        }
    }

    /// Avance (px) y glifo de una letra ASCII en la posición subpíxel `bin`.
    fn ascii(&mut self, ctx: &egui::Context, c: u8, bold: bool, bin: usize) -> (f32, Option<Quad>) {
        let b = bold as usize;
        let c = c & 0x7f;
        if self.ascii[b][c as usize].is_none() {
            // Sola, la letra cae en la posición 0.
            let galley = self.layout(ctx, (c as char).to_string(), bold);
            let (adv, quad) = last_glyph(&galley, self.ppp).unwrap_or((0.0, None));
            let mut e = Ascii {
                adv_px: adv,
                ..Default::default()
            };
            e.bins[0] = Some(quad);
            self.ascii[b][c as usize] = Some(e);
        }
        let e = self.ascii[b][c as usize].unwrap_or_default();
        if let Some(q) = e.bins[bin] {
            return (e.adv_px, q);
        }
        // Otra posición subpíxel: se maqueta la letra detrás de unos cuantos espacios (o de
        // sí misma) elegidos para que caiga justo en esa posición, y se lee su glifo.
        let space = self.space_px(ctx, bold);
        let mut quad = None;
        'search: for (pre, adv) in [(' ', space), (c as char, e.adv_px)] {
            let mut cursor = 0.0f32;
            for k in 1..64 {
                cursor += adv;
                if subpixel_bin(cursor).1 == bin {
                    let mut text: String = std::iter::repeat_n(pre, k).collect();
                    text.push(c as char);
                    let galley = self.layout(ctx, text, bold);
                    quad = last_glyph(&galley, self.ppp).map(|(_, q)| q);
                    break 'search;
                }
            }
        }
        // Sin forma de llegar a esa posición (no pasa con fuentes monoespaciadas): la 0.
        let quad = quad.unwrap_or_else(|| e.bins[0].flatten());
        if let Some(e) = self.ascii[b][c as usize].as_mut() {
            e.bins[bin] = Some(quad);
        }
        (e.adv_px, quad)
    }

    fn space_px(&mut self, ctx: &egui::Context, bold: bool) -> f32 {
        if let Some(px) = self.space_px[bold as usize] {
            return px;
        }
        let galley = self.layout(ctx, " ".into(), bold);
        let px = galley
            .rows
            .first()
            .and_then(|r| r.glyphs.first())
            .map_or(0.0, |g| g.advance_width * self.ppp);
        self.space_px[bold as usize] = Some(px);
        px
    }
}

/// El rectángulo del glifo en la malla de su fila (coordenadas de la fila), si tiene píxeles.
fn quad_of(row: &egui::epaint::text::PlacedRow, g: &egui::epaint::text::Glyph) -> Option<Quad> {
    if g.uv_rect.is_nothing() {
        return None;
    }
    let v = row.visuals.mesh.vertices.get(g.first_vertex as usize)?;
    Some(Quad {
        min: v.pos.to_vec2(),
        size: g.uv_rect.size,
        uv_min: g.uv_rect.min,
        uv_max: g.uv_rect.max,
        is_color: g.is_color,
    })
}

/// Avance (px) y glifo del último carácter de un galley de una línea, con la x relativa a
/// la del propio glifo (para colocarlo luego en cualquier `x_px`).
fn last_glyph(galley: &Galley, ppp: f32) -> Option<(f32, Option<Quad>)> {
    let row = galley.rows.first()?;
    let g = row.glyphs.last()?;
    let quad = quad_of(row, g).map(|q| Quad {
        min: q.min + row.pos.to_vec2() - Vec2::new(g.pos.x, 0.0),
        ..q
    });
    Some((g.advance_width * ppp, quad))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_posiciones_subpixel() {
        assert_eq!(subpixel_bin(0.0), (0, 0));
        assert_eq!(subpixel_bin(15.66), (15, 3));
        assert_eq!(subpixel_bin(31.32), (31, 1));
        assert_eq!(subpixel_bin(46.98), (47, 0));
        assert_eq!(subpixel_bin(7.5), (7, 2));
    }

    /// Los glifos de la caché caen exactamente donde los pone el teselador de egui al
    /// pintar el mismo texto con `painter.text`.
    #[test]
    fn grid_mismos_rectangulos_que_egui() {
        let ctx = egui::Context::default();
        // La misma caché para todo: al cambiar de pantalla (ppp) tiene que tirarse sola.
        let mut cache = GlyphCache::default();
        for ppp in [1.0f32, 2.0, 1.0] {
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    Pos2::ZERO,
                    Vec2::new(700.0, 60.0),
                )),
                ..Default::default()
            };
            input
                .viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .native_pixels_per_point = Some(ppp);
            let font = FontId::monospace(13.0);
            let texts = [
                ("echo hola mundo; ls -la", true),
                ("ñ", false),
                ("漢", false),
            ];
            for _ in 0..2 {
                let mut ours = Vec::new();
                let out = ctx.run_ui(input.clone(), |ui| {
                    let ctx = ui.ctx().clone();
                    cache.begin(&ctx, &font, &font);
                    let painter = ui.painter();
                    let mut x = 3.3;
                    for (text, ascii) in texts {
                        let pos = Pos2::new(x, 7.7);
                        painter.text(
                            pos,
                            egui::Align2::LEFT_TOP,
                            text,
                            font.clone(),
                            Color32::RED,
                        );
                        cache.push_text(&ctx, &mut ours, pos, text, ascii, false, Color32::RED);
                        x += 200.0;
                    }
                });
                let mut out = out;
                out.textures_delta.clear();
                let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
                let mut theirs = Vec::new();
                for p in prims {
                    if let egui::epaint::Primitive::Mesh(m) = p.primitive {
                        for q in m.vertices.chunks(4) {
                            theirs.push((q[0].pos, q[3].pos, q[0].uv, q[3].uv, q[0].color));
                        }
                    }
                }
                let [w, h] = ctx.fonts_mut(|f| f.font_image_size());
                let uv = |t: [f32; 2]| Pos2::new(t[0] * (1.0 / w as f32), t[1] * (1.0 / h as f32));
                let ours: Vec<_> = ours
                    .iter()
                    .map(|i| {
                        let (min, max) = (Pos2::from(i.min), Pos2::from(i.max));
                        (min, max, uv(i.uv_min), uv(i.uv_max), i.color)
                    })
                    .collect();
                assert_eq!(ours.len(), theirs.len(), "ppp {ppp}: {ours:?}\n{theirs:?}");
                for (a, b) in ours.iter().zip(&theirs) {
                    assert_eq!(a, b, "ppp {ppp}");
                }
            }
        }
    }
}
