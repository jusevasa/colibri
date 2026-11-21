# Arrastrar una pestaña fuera de la ventana — plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Arrastrar una pestaña fuera de la barra la pasa, con su proceso vivo, a una ventana nueva (soltada fuera de Colibrí) o a otra ventana de Colibrí (soltada encima).

**Architecture:** `tab_bar` marca la pestaña como despegada cuando el puntero se aleja de la barra; al soltar, `macos.rs` da la posición global del puntero y el número de la ventana debajo, `shell.rs` traduce ese número a `WindowId`, y `App` mueve la `Session` entre `Win` (o la manda a una ventana nueva con `Fill::Move`). El cálculo puro (cuándo se despega, en qué hueco cae, voltear coordenadas) va en un módulo nuevo `src/drag.rs` con sus pruebas.

**Tech Stack:** Rust 2024, egui 0.37 sobre winit 0.30, objc2 0.6 (`msg_send!` como en `macos.rs`).

**Spec:** `docs/superpowers/specs/2026-11-21-arrastrar-pestana-fuera-design.md`

## Global Constraints

- Código, comentarios, textos y commits en español; commits terminan con `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- La pestaña se mueve con su proceso vivo: nunca se suelta ni se recrea la `Session` (nada de `close_tab`/`revive`).
- Mover una pestaña no apunta nada en ⌘⇧T (`App::closed`).
- Margen de despegue: media altura de la barra (`TAB_BAR_H / 2`, `TAB_BAR_H = 38.0`) por debajo de ella, o el puntero fuera de la ventana.
- Única pestaña soltada fuera de toda ventana → no pasa nada.
- No añadir repintados continuos (ver «Repintado bajo demanda» en CLAUDE.md).
- `cargo fmt && cargo clippy` limpios; `cargo test` en verde.

## Review Focus

1. Arrastrar para reordenar y salirse un poco por debajo de la barra sin querer: dentro del margen debe seguir reordenando, no despegarse (prueba `detaching_dentro_del_margen` en la tarea 1).
2. Despegar con un overlay abierto en la ventana de origen (historial, renombrar, desplegable de cajas) que apunta a la pestaña activa: el overlay se cierra al mover (paso de guion `open history` + `detach` en la tarea 3).
3. Soltar sobre una ventana de Colibrí tapada por otra app: la ventana que se ve es la de la otra app, así que sale una ventana nueva (cubierto por `windowNumberAtPoint`; se comprueba a mano en la tarea 4).
4. Mover la última pestaña de una ventana a otra: la de origen se cierra y ⌘⇧T no la reabre (paso de guion en la tarea 3).
5. La pestaña movida sigue pintando su salida y su insignia de agente en la ventana de destino (contexto cambiado y caché de rejilla vaciada; guion con `claude` falso en la tarea 3).

---

### Task 1: Cálculo puro del arrastre (`src/drag.rs`)

**Files:**
- Create: `src/drag.rs`
- Modify: `src/main.rs` (añadir `mod drag;` en la lista de módulos, orden alfabético)

**Interfaces:**
- Produces:
  - `pub fn detaching(pointer: Option<egui::Pos2>, bar: egui::Rect, window: egui::Rect) -> bool` — `true` si `pointer` es `None`, si está fuera de `window`, o si `pointer.y > bar.bottom() + bar.height() / 2.0`.
  - `pub fn drop_slot(x: f32, tabs_left: f32, tab_w: f32, n: usize) -> usize` — hueco donde insertar en una barra con `n` pestañas: `((x - tabs_left) / tab_w).round()` acotado a `0..=n`; con `tab_w <= 0.0` devuelve `n`.
  - `pub fn flip_y(y: f64, primary_height: f64) -> f64` — de coordenadas de Cocoa (origen abajo a la izquierda de la pantalla principal) a las de winit/egui (origen arriba): `primary_height - y`. Es su propia inversa.

- [ ] **Step 1: Escribir las pruebas que fallan** (en `src/drag.rs`, `#[cfg(test)] mod tests`)

```rust
use egui::{pos2, Rect};
fn bar() -> Rect { Rect::from_min_max(pos2(0.0, 0.0), pos2(800.0, 38.0)) }
fn window() -> Rect { Rect::from_min_max(pos2(0.0, 0.0), pos2(800.0, 600.0)) }

#[test] fn detaching_sin_puntero() { assert!(detaching(None, bar(), window())); }
#[test] fn detaching_dentro_del_margen() {
    assert!(!detaching(Some(pos2(100.0, 20.0)), bar(), window()));
    assert!(!detaching(Some(pos2(100.0, 57.0)), bar(), window())); // 38 + 19
}
#[test] fn detaching_bajo_el_margen() { assert!(detaching(Some(pos2(100.0, 57.5)), bar(), window())); }
#[test] fn detaching_fuera_de_la_ventana() {
    assert!(detaching(Some(pos2(-5.0, 20.0)), bar(), window()));
    assert!(detaching(Some(pos2(100.0, -3.0)), bar(), window()));
}
#[test] fn drop_slot_huecos() {
    assert_eq!(drop_slot(70.0, 70.0, 100.0, 3), 0);
    assert_eq!(drop_slot(130.0, 70.0, 100.0, 3), 1);
    assert_eq!(drop_slot(10_000.0, 70.0, 100.0, 3), 3);
    assert_eq!(drop_slot(-50.0, 70.0, 100.0, 3), 0);
    assert_eq!(drop_slot(130.0, 70.0, 0.0, 3), 3);
}
#[test] fn flip_y_ida_y_vuelta() {
    assert_eq!(flip_y(100.0, 1117.0), 1017.0);
    assert_eq!(flip_y(flip_y(42.5, 900.0), 900.0), 42.5);
}
```

- [ ] **Step 2: Comprobar que fallan**

Run: `cargo test drag::`
Expected: error de compilación (funciones sin definir).

- [ ] **Step 3: Implementar las tres funciones en `src/drag.rs`** con un comentario de módulo en español que diga para qué es.

- [ ] **Step 4: Comprobar que pasan**

Run: `cargo test drag::`
Expected: 6 pruebas OK.

- [ ] **Step 5: Commit**

```bash
git add src/drag.rs src/main.rs docs/superpowers/specs/2026-11-21-arrastrar-pestana-fuera-design.md docs/superpowers/plans/2026-11-21-arrastrar-pestana-fuera.md
git commit -m "feat: cálculo puro para despegar pestañas arrastrándolas"
```

---

### Task 2: Qué ventana hay bajo el puntero (macOS + registro de ventanas)

**Files:**
- Modify: `src/macos.rs` (dos funciones nuevas)
- Modify: `src/shell.rs` (registro `WindowId → NSWindow`, altas en `open_window`, bajas donde se quita la ventana de `self.windows`)

**Interfaces:**
- Consumes: `drag::flip_y`.
- Produces:
  - `pub fn macos::pointer_on_screen() -> Option<egui::Pos2>` — `NSEvent.mouseLocation` volteado con `flip_y` y la altura de `NSScreen.screens[0].frame` (la principal, no `mainScreen`), en puntos con origen arriba a la izquierda (el mismo sistema que `Win::frame` y `outer_position` de winit). `None` si no hay pantallas.
  - `pub fn macos::window_number_at(p: egui::Pos2) -> Option<isize>` — vuelve a voltear `p` y llama a `[NSWindow windowNumberAtPoint:belowWindowWithWindowNumber:0]`; `None` si devuelve `<= 0`.
  - `pub fn shell::window_with_number(number: isize) -> Option<WindowId>` — busca en un `static NS_WINDOWS: Mutex<Vec<(WindowId, usize)>>` (dirección del `NSWindow`) la ventana cuyo `windowNumber` (consultado en el momento, no guardado: antes de mostrarse aún no tiene) es `number`. Solo desde el hilo principal; documentarlo.
- Alta: en `open_window`, tras `create_window`, sacar el `NSWindow` del `NSView` del `raw_window_handle` (como `metal.rs:1113`, `[view window]`) y guardarlo. Baja: en el mismo sitio donde la ventana sale de `self.windows` al cerrarse.

- [ ] **Step 1: Implementar las tres funciones y el registro.**

- [ ] **Step 2: Compilar y pasar clippy**

Run: `cargo build && cargo clippy`
Expected: sin errores ni avisos nuevos (puede quedar `dead_code` hasta la tarea 4; márcalo con `#[allow(dead_code)]` y un comentario «se usa al soltar una pestaña despegada (siguiente paso)», como ya se hace con `Session::set_context`).

- [ ] **Step 3: Comprobar a mano con una sonda temporal** (no se commitea): en `logic()`, con `COLIBRI_FPS` definida, escribir en stderr cada segundo `pointer_on_screen()` y `window_number_at(..).and_then(shell::window_with_number)`. Con `cargo run`, poner el ratón sobre la ventana (sale `Some(WindowId)`), sobre el escritorio u otra app (`None`) y en la esquina superior izquierda de la pantalla principal (≈ `(0, 0)`). Quitar la sonda.

- [ ] **Step 4: Commit**

```bash
git add src/macos.rs src/shell.rs
git commit -m "feat: saber qué ventana de Colibrí hay bajo el puntero"
```

---

### Task 3: Mover una sesión viva entre ventanas (+ paso de guion `detach`)

**Files:**
- Modify: `src/main.rs` — `Fill::Move`, `App::move_to_window`, `App::detach_to_new_window`, rama en `window_opened`, rama `Step::Detach` en `run_script`
- Modify: `src/session.rs:992` — quitar el `#[allow(dead_code)]` de `set_context`
- Modify: `src/debug.rs` — `Step::Detach(usize, String)` y su parseo
- Modify: `CLAUDE.md` — añadir `detach <n> new|<m>` a la lista de pasos del guion

**Interfaces:**
- Produces (en `impl App`, `src/main.rs`):
  - `Fill::Move(Session)` — nueva variante: una pestaña que llega viva de otra ventana.
  - `fn move_to_window(&mut self, i: usize, win: WindowId, slot: Option<usize>)` — saca la pestaña `i` de `self.w` (ajustando `active` igual que `close_tab`, sin `remember`), cierra el overlay de la ventana de origen (`cancel_overlay()` si no es `Overlay::None`), vacía su caché de rejilla (`s.grid = view::GridCache::default()`), `s.set_context(&destino.ctx)`, la inserta en `parked[win]` en `slot.unwrap_or(len).min(len)`, la hace activa allí (`unseen = false`), pone `self.front = Some(win)` y manda `ViewportCommand::Focus` a ese contexto. Si `win == self.cur` o `i` no existe, no hace nada. La ventana de origen vacía la cierra `logic()` (ya lo hace con `sessions.is_empty()`), y `window_closed` no la apunta en ⌘⇧T porque no tiene pestañas.
  - `fn detach_to_new_window(&mut self, i: usize, at: Pos2)` — si `self.w.sessions.len() <= 1`, no hace nada; si no, saca la pestaña igual que arriba (overlay, caché), empuja `Fill::Move(s)` a `opening` y `Some([at.x, at.y, w, h])` a `want_windows`, con `w, h` del `frame` de la ventana de origen (o `None` en `want_windows` si no se conoce).
  - En `window_opened`: `Fill::Move(s) => { s.set_context(ctx); self.w.sessions.push(s); self.w.active = 0; }`.
  - Guion: `detach <n> new` → `detach_to_new_window(n, origen del frame + (40, 40))`; `detach <n> <m>` → `move_to_window(n, self.order[m], None)`. `n` y `m` desde 0, como `select` y `window`.

- [ ] **Step 1: Escribir el guion de prueba** en `$SCRATCH/detach.txt` (con `printf` o heredoc, no `echo`), con config temporal y `HISTFILE` falso:

```
type echo uno\n
tab
type echo dos\n
tab
type echo tres\n
wait 500
detach 1 new
wait 800
shot /…/scratch/a-origen.bmp
window 1
shot /…/scratch/b-nueva.bmp
window 0
open history
detach 1 1
wait 800
shot /…/scratch/c-origen.bmp
window 1
shot /…/scratch/d-destino.bmp
window 0
detach 0 1
wait 800
shot /…/scratch/e-tras-ultima.bmp
quit
```

Run: `COLIBRI_LAZY=1 COLIBRI_SCRIPT=$SCRATCH/detach.txt COLIBRI_CONFIG=$SCRATCH/config.toml HISTFILE=$SCRATCH/hist cargo run`
Expected antes de implementar: las líneas `detach` se ignoran (pasos desconocidos); las capturas muestran todo en una ventana.

- [ ] **Step 2: Implementar `Step::Detach`, `Fill::Move`, `move_to_window`, `detach_to_new_window` y la rama de `window_opened`.**

- [ ] **Step 3: Repetir el guion y mirar las capturas.**
Expected:
- `a-origen`: dos pestañas («uno», «tres»).
- `b-nueva`: una pestaña con «echo dos» y su salida (pantalla conservada).
- `c-origen`: sin overlay de historial, solo con la pestaña «uno».
- `d-destino`: dos pestañas, la activa es «tres», con su salida.
- `e-tras-ultima`: la ventana 0 se cerró; la que queda tiene tres pestañas.
- Ningún error en stderr.

- [ ] **Step 4: Proceso vivo y repintado en el destino.** Con un `claude` falso (`#!/bin/sh` + `printf '\033]0;⠋ claude\007'; sleep 30`) delante en el PATH, abrir una pestaña con él, `detach 1 new`, `window 1`, `wait 1200`, `shot`. Expected: la insignia de «trabajando» se ve en la ventana nueva y `ps -p <pid del sleep>` sigue vivo tras el `detach` (apuntar el pid con `procs` antes y después). Con `COLIBRI_FPS`, la ventana nueva pinta ~8 cuadros/s con el foco o 4 sin él, no más.

- [ ] **Step 5: `cargo fmt && cargo clippy && cargo test`** → todo limpio y en verde.

- [ ] **Step 6: Commit**

```bash
git add src/main.rs src/session.rs src/debug.rs CLAUDE.md
git commit -m "feat: mover una pestaña viva a otra ventana o a una nueva"
```

---

### Task 4: Despegar arrastrando en la barra de pestañas

**Files:**
- Modify: `src/main.rs` — `Win` (campo `detached: bool`), `tab_bar` (`src/main.rs:1520-1960`)
- Modify: `CLAUDE.md` — quitar «Pendiente: mover pestañas entre ventanas» y describir el despegue en «Arrastre de pestañas y barra de título»

**Interfaces:**
- Consumes: `drag::detaching`, `drag::drop_slot`, `macos::pointer_on_screen`, `macos::window_number_at`, `shell::window_with_number`, `App::move_to_window`, `App::detach_to_new_window`, `Win::tab_layout` (ancho de pestaña, origen) y `Win::frame` de la ventana de destino, `TAB_BAR_H`.
- Produces: `Win::detached: bool` — la pestaña de `drag_tab` está despegada en este cuadro.

Comportamiento:
- Mientras `drag_tab` es `Some`: `self.w.detached = drag::detaching(latest_pos, full /* rect de la barra */, ctx.screen_rect())`. Si está despegada, la pestaña se pinta en su hueco (no sigue al puntero) con `painter.multiply_opacity(0.4)` y no produce `moved`.
- Al soltar (el bloque que hoy hace `drag_tab = None` cuando `!primary_down()`): si `detached` era `true`, antes de limpiar, `drop_detached(i)` con el índice de la pestaña de `drag_tab`:
  - `p = macos::pointer_on_screen()`; sin `p`, nada.
  - `target = macos::window_number_at(p).and_then(shell::window_with_number)`.
  - `target == self.cur` → nada.
  - `Some(win)` → hueco: con `[fx, fy, ..] = frame` y `(tab_w, tabs_left) = tab_layout` de esa `Win`, si `0.0 <= p.y - fy <= TAB_BAR_H` → `Some(drag::drop_slot(p.x - fx, tabs_left, tab_w, n))`, si no `None`; luego `move_to_window(i, win, slot)`.
  - `None` → `detach_to_new_window(i, p)`.
- Luego `drag_tab = None` y `detached = false` como siempre.

- [ ] **Step 1: Implementar el campo, el pintado atenuado y `drop_detached`.**

- [ ] **Step 2: Comprobar que el reordenado no cambió** con el guion existente de arrastre (`press`/`moveto`/`release` dentro de la barra) y `move 0 2`: capturas iguales a las de antes del cambio.

- [ ] **Step 3: Prueba a mano con `scripts/bundle.sh`** (el puntero simulado no pasa por macOS) y config/estado temporales (`open -n target/Colibri.app --env COLIBRI_CONFIG=… --env COLIBRI_STATE=…`). Comprobar:
  - Arrastrar dentro de la barra y un poco por debajo (menos de ~19 pt) reordena, sin atenuar.
  - Arrastrar más abajo atenúa la pestaña; volver a la barra la devuelve al arrastre normal.
  - Soltar sobre el escritorio → ventana nueva con la esquina bajo el puntero, con la pestaña viva.
  - Soltar sobre la barra de otra ventana de Colibrí → entra en el hueco bajo el puntero; soltar en su terminal → al final. Esa ventana pasa al frente.
  - Soltar sobre una ventana de otra app que tapa una de Colibrí → ventana nueva.
  - Única pestaña soltada en el escritorio → vuelve a su sitio; única pestaña soltada en otra ventana → se mueve y la de origen se cierra; ⌘⇧T no la reabre.
  - Soltar sobre la propia ventana, en la terminal → vuelve a su sitio.

- [ ] **Step 4: `cargo fmt && cargo clippy && cargo test`** → limpio y en verde.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs CLAUDE.md
git commit -m "feat: arrastrar una pestaña fuera de la barra la lleva a otra ventana"
```
