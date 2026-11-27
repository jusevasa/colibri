# Aire en el prompt, raya entre bloques y pista «→» — plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Que la fila de cajas del prompt deje ~12 px de aire sobre el comando, que una raya de 1 px separe los bloques y que tras la sugerencia gris de zsh-autosuggestions salga una cajita «→».

**Architecture:** Todo se dibuja con egui encima de la rejilla, sin cambiar filas ni clics. `Session::chip_rows` dice si la fila de encima de cada línea de cajas está en blanco (`room_above`); `view::show` sube esas cajas y pinta la raya; una función pura reconoce la sugerencia gris por su color y `view.rs` pinta la pista tras ella.

**Tech Stack:** Rust 2024, egui, alacritty_terminal (vendor).

**Spec:** `docs/superpowers/specs/2026-11-27-prompt-aire-y-pista-design.md`

## Global Constraints

- Warp es AGPL: solo guía de medidas y comportamiento, nada de su código (`AGENTS.md`, regla 3).
- Código, comentarios y commits en español (regla 1); commits `feat: …`/`docs: …` con `Co-Authored-By`.
- Sin repintados nuevos: todo se pinta en la pasada que ya hay (regla 5).
- Al probar: `COLIBRI_CONFIG`, `COLIBRI_STATE`, `HISTFILE` y `COLIBRI_HISTFILE` temporales; nunca traer la ventana al frente ni mandar pulsaciones reales (regla 7).
- Medidas: aire 12 px entre el borde inferior de la caja y la fila del comando; raya 1 px, `pal.fg` al 10 % sobre `pal.bg`; caja con radio 4, borde `fg` al 15 %, fondo `fg` al 5 % (al 10 % con el puntero); pista a 10 px del final de la sugerencia, alto del cursor (mín. 12 px), borde 1 px, radio 25 % del alto, «→» en letra proporcional `font.size − 2`, color `fg` al 40 %, sin relleno.
- Sin hueco encima (primera fila de pantalla, fila con texto), las cajas quedan como hoy.
- Prompts remotos (`ChipRow::inline`): sin cambios.
- Al terminar: `cargo fmt`, `cargo clippy --all-targets` sin avisos nuevos (hoy hay 16), `cargo test` (regla 9); actualizar `AGENTS.md` (regla 10).

## Review Focus

- Prompt en la primera fila de la pantalla (tras `clear`): sin hueco encima → cajas como hoy y sin raya. Test en Task 1 (`fila_en_blanco`, fila 0).
- Línea de encima con solo espacios pero con fondo de color (salida de un programa): no cuenta como blanco. Test en Task 1.
- El cursor movido a la izquierda dentro de lo escrito: el texto de detrás no es sugerencia aunque exista. Test en Task 2 (`sugerencia_gris`, caso de color normal).
- Sugerencia gris que salta a la línea siguiente (comando largo): la pista va tras la última celda gris. Test en Task 2 (tramo que cruza el final de la fila).
- La duración y el código de salida que se pintan junto a las cajas del prompt anterior suben con ellas. Comprobado en la captura de Task 1.

---

### Task 1: Cajas con aire, raya entre bloques y estilo de las cajas

**Files:**
- Modify: `src/session.rs` (`ChipRow`, `chip_rows`, nueva `row_is_blank`, tests)
- Modify: `src/view.rs:140-225` (`chips_px`, raya, etiqueta de duración)
- Modify: `src/main.rs:3326-3440` (`prompt_chips`: colores y radio)
- Modify: `AGENTS.md` (sección «Cajas en el prompt»)

**Interfaces:**
- Produces: `ChipRow::room_above: bool`; `fn row_is_blank(cells: &[Cell]) -> bool` en `session.rs`; `const CHIP_LIFT: f32 = 7.0` en `view.rs`.

- [ ] **Step 1: Test `fila_en_blanco`** en `session.rs` (`mod tests`): una fila de `Cell::default()` → `true`; una con `c = 'x'` en una celda → `false`; una de espacios pero con `bg` distinto de `Color::Named(NamedColor::Background)` en una celda → `false`.
- [ ] **Step 2:** `cargo test fila_en_blanco` → no compila (falta `row_is_blank`).
- [ ] **Step 3:** Implementar `row_is_blank(cells: &[Cell]) -> bool`: todas las celdas con `is_blank(c)`, fondo por defecto y sin marca de prompt ni de cajas. En `chip_rows`, para filas no `inline`: `room_above = row > 0 && row_is_blank(&fila row-1)`.
- [ ] **Step 4:** `cargo test fila_en_blanco` → PASS.
- [ ] **Step 5:** En `view.rs`: si `c.room_above`, el `area` de `chips_px` se sube `CHIP_LIFT` (con `up = 3`, `down = 5` de `prompt_chips` deja el borde inferior de la caja 12 px por encima de la fila del comando); la etiqueta de duración/código del prompt anterior (`y(prev)`) sube lo mismo si ese prompt tenía hueco; por cada fila con `room_above`, raya de 1 px de `rect.left()` a `rect.right()` en `y = origin.y + (row − 1)·cell_h + 0.5`, color `pal.bg.lerp_to_gamma(pal.fg, 0.10)`.
- [ ] **Step 6:** En `prompt_chips`: radio 4; fondo `pal.bg.lerp_to_gamma(pal.fg, 0.05)` (0.10 con el puntero); borde `pal.bg.lerp_to_gamma(pal.fg, 0.15)`.
- [ ] **Step 7: Captura.** Guion con config temporal (copia de la del usuario), historial falso: `type cd ~/Desktop/dev\n`, `wait 1500`, `type false\n`, `wait 1500`, `shot antes/después`. Esperado: aire visible entre cajas y comando, raya sobre cada bloque salvo el de arriba, barra roja y «código» del `false` en su sitio.
- [ ] **Step 8:** Actualizar «Cajas en el prompt» en `AGENTS.md` (aire, `room_above`, raya, estilo). `cargo fmt && cargo clippy --all-targets && cargo test`.
- [ ] **Step 9: Commit** `feat: aire entre las cajas del prompt y el comando, y raya entre bloques`.

### Task 2: Pista «→» tras la sugerencia gris

**Files:**
- Modify: `src/session.rs` (funciones puras + `Session::autosuggestion_end`, tests)
- Modify: `src/view.rs` (`show_grid`: dibujo de la pista)
- Modify: `AGENTS.md` (nueva línea en «Cosas que no se ven…», junto a «Rutas de color»)

**Interfaces:**
- Consumes: `Session::input_cells() -> Option<(Vec<Point>, Point)>` (ya existe).
- Produces: `fn is_suggestion_color(fg: Color, flags: Flags) -> bool`; `fn suggestion_end(cells: &[(char, Color, Flags)], cursor: usize) -> Option<usize>` (índice de la última celda de la sugerencia); `pub fn autosuggestion_end(&self) -> Option<Point>` en `Session`.

- [ ] **Step 1: Test `sugerencia_gris`** en `session.rs`:
  - `is_suggestion_color(Color::Named(NamedColor::BrightBlack), Flags::empty())` → `true`; `Color::Indexed(8)` → `true`; cualquier color con `Flags::DIM` → `true`; `Color::Named(NamedColor::Foreground)` sin `DIM` → `false`.
  - `suggestion_end`: «cd s» normal + «rc/» gris, cursor en 4 → `Some(6)`; mismo texto todo en color normal → `None`; cursor al final sin nada detrás → `None`; gris con espacios en blanco al final → índice de la última no blanca; gris que sigue tras un salto de fila (las celdas vienen seguidas en el vector) → última gris.
- [ ] **Step 2:** `cargo test sugerencia_gris` → no compila.
- [ ] **Step 3:** Implementar ambas: `suggestion_end` exige que la celda del cursor sea gris y avanza mientras lo sean; devuelve la última no blanca. `autosuggestion_end` arma el vector desde `input_cells()` (las celdas desde el cursor) y traduce el índice a `Point`.
- [ ] **Step 4:** `cargo test sugerencia_gris` → PASS.
- [ ] **Step 5:** En `show_grid`, con `focused`, el cursor en pantalla y `s.autosuggestion_end()`: caja con las medidas de Global Constraints a 10 px a la derecha de esa celda (con `row_shifts` aplicado, como el resto de la fila), «→» centrado.
- [ ] **Step 6: Captura.** Historial falso con `cd ~/Desktop/dev`; guion `type cd ~/D`, `wait 1500`, `shot`. Esperado: «esktop/dev» en gris y la cajita «→» detrás. Y `key left` ×2 + `shot`: sin pista si el cursor ya no está sobre gris.
- [ ] **Step 7:** `AGENTS.md`: cómo se reconoce la sugerencia (color 8 / atenuada, heurística) y qué se pinta. `cargo fmt && cargo clippy --all-targets && cargo test`.
- [ ] **Step 8: Commit** `feat: pista «→» tras la sugerencia gris de zsh`.
