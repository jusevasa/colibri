# Pestaña unida y arco de carga — plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** La pestaña activa se une a la terminal sin costura, y mientras un agente trabaja su icono se queda y gira un arco alrededor, con animación fluida.

**Architecture:** Todo en `src/main.rs`: `tab_bar` (forma de la pestaña activa, fondo de la barra y línea inferior), una función nueva `busy_ring` junto a `spinner`, y el refresco al final de `ui()`. Las funciones de geometría y de ritmo son puras y llevan pruebas en el `mod tests` de `main.rs`.

**Tech Stack:** Rust 2024, egui (`Painter`, `Mesh`, `CornerRadius`), guion de depuración (`src/debug.rs`).

**Spec:** `docs/superpowers/specs/2026-11-20-pestanas-unidas-y-spinner-design.md`

## Global Constraints

- Código, comentarios y textos en español.
- Repintado bajo demanda: en reposo 0 cuadros/s; el refresco rápido solo con un agente `busy` en esa ventana, pedido desde `ui()` con `request_repaint_after`.
- Giro fluido: ~15 cuadros/s → `RING_TICK = 66 ms`; una vuelta por segundo (`RING_PERIOD = 1.0 s`).
- Sin agente se mantiene `spinner` (la luna a saltos) y el refresco de 250 ms de hoy.
- La base de la pestaña activa y la terminal deben dar el mismo píxel, también con `opacity < 1` (la barra no se pinta debajo de la pestaña activa).
- Sin commits: el usuario decide cuándo guardar.
- Pruebas manuales con `COLIBRI_SCRIPT` + `COLIBRI_CONFIG`/`COLIBRI_STATE` en una carpeta temporal; nunca los archivos reales.

## Review Focus

1. Ventana translúcida (`opacity = 0.85`): sin costura entre pestaña activa y terminal → captura en la tarea 1.
2. Pestaña activa deslizándose o arrastrada: el hueco en la línea inferior y el recorte del fondo siguen a la pestaña → captura a mitad de `move` en la tarea 1.
3. Tema claro: el contorno se ve y no queda una banda más oscura → captura en la tarea 1.
4. Pestaña sin agente con un servidor en marcha: sigue a 4 cuadros/s, no a 15 → `COLIBRI_FPS` en la tarea 2.
5. Agente que termina: el refresco rápido para en cuanto `busy` cae (0 cuadros/s en reposo) → `COLIBRI_FPS` en la tarea 2.

---

### Task 1: Pestaña activa unida a la terminal

**Files:**
- Modify: `src/main.rs` (`tab_bar` ~1546–1700; `ui()` donde se crea `Panel::top("tabs")` ~2805)

**Interfaces:**
- Produces: `fn bar_background(full: Rect, tab: Option<Rect>, r: f32, color: Color32) -> egui::Mesh`: rellena `full` salvo la forma de la pestaña (rectángulo con las dos esquinas superiores redondeadas de radio `r`). Las esquinas se cubren con abanicos de triángulos desde el vértice exterior del rectángulo hacia los puntos del arco.
- Produces: `fn bottom_line(left: f32, right: f32, gap: Option<(f32, f32)>) -> Vec<(f32, f32)>`: tramos de la línea inferior de la barra, sin el hueco.

- [ ] **Step 1: Pruebas que fallan** (en `mod tests` de `main.rs`)

```rust
#[test]
fn barra_sin_hueco_bajo_la_pestana() {
    assert_eq!(bottom_line(0.0, 100.0, None), vec![(0.0, 100.0)]);
    assert_eq!(bottom_line(0.0, 100.0, Some((20.0, 60.0))), vec![(0.0, 20.0), (60.0, 100.0)]);
    assert_eq!(bottom_line(0.0, 100.0, Some((0.0, 40.0))), vec![(40.0, 100.0)]);
}

#[test]
fn fondo_de_barra_no_tapa_la_pestana() {
    let full = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(300.0, 38.0));
    let tab = Rect::from_min_max(Pos2::new(100.0, 5.0), Pos2::new(200.0, 38.0));
    let m = bar_background(full, Some(tab), 7.0, Color32::WHITE);
    // `cubre(m, p)`: algún triángulo de la malla contiene p (ayudante de la prueba).
    assert!(!cubre(&m, Pos2::new(150.0, 20.0)));   // dentro de la pestaña
    assert!(!cubre(&m, Pos2::new(108.0, 8.0)));    // dentro, cerca de la esquina redondeada
    assert!(cubre(&m, Pos2::new(101.0, 5.5)));     // fuera del arco de la esquina
    assert!(cubre(&m, Pos2::new(50.0, 20.0)));
    assert!(cubre(&m, Pos2::new(150.0, 2.0)));     // encima de la pestaña
    assert!(cubre(&m, Pos2::new(250.0, 30.0)));
}
```

- [ ] **Step 2:** `cargo test barra_sin_hueco fondo_de_barra` → falla (no existen).
- [ ] **Step 3: Implementar** `bottom_line` y `bar_background`.
- [ ] **Step 4: Usarlas en `tab_bar`:**
  - `Panel::top("tabs")` pasa a un `Frame` sin relleno; `tab_bar` pinta primero `bar_background(full, rect_activa, 7.0, fade(pal.tab_bar, opacity))`. `rect_activa` es la que se ve (con deslizamiento o arrastre), así que hay que calcularla antes de pintar, o pintar el fondo en una capa inferior (`LayerId::background()`) tras recorrer las pestañas.
  - Pestaña activa: `rect` desde `full.top() + 5.0` hasta `full.bottom()`; relleno `fade(pal.bg, opacity)` con `CornerRadius { nw: 7, ne: 7, sw: 0, se: 0 }`; contorno `pal.border` de 1 px solo arriba y a los lados (camino abierto, no `rect_stroke`).
  - Línea inferior de la barra (`pal.border`, 1 px, en `full.bottom() - 0.5`) con los tramos de `bottom_line`, con hueco bajo la activa.
  - Activa con color: sin tinte en el relleno; raya de 2 px del color en el borde superior, entre las esquinas. Inactivas: como hoy.
- [ ] **Step 5:** `cargo test` todo verde; `cargo clippy` sin avisos nuevos.
- [ ] **Step 6: Capturas** (`COLIBRI_SCRIPT`, carpeta temporal): una pestaña; tres pestañas con la del medio activa; activa con color (`rclick` sobre la pestaña → color); `pref opacity -1` dos veces; tema claro (`pref theme +1` hasta el claro); a mitad de `move 1 3` (captura 60 ms después). En cada captura, mirar que la base de la activa y la terminal se ven como una sola pieza y que la línea inferior tiene su hueco justo debajo.

### Task 2: Arco de carga alrededor del icono del agente

**Files:**
- Modify: `src/main.rs` (junto a `spinner` ~3242; indicador en `tab_bar` ~1662; estado en `side_panel` ~2032; final de `ui()` ~2905)

**Interfaces:**
- Produces: `fn ring_angle(t: f64) -> f32`: ángulo de inicio del arco en radianes, `2π·(t mod RING_PERIOD)/RING_PERIOD`, en `[0, 2π)`.
- Produces: `fn busy_ring(p: &egui::Painter, c: Pos2, r: f32, agent: session::Agent)`: `agent_icon(p, c, r, agent)` + anillo tenue (`agent.color()` al 25 %, radio `r + 3.0`, 1,5 px) + arco de 100° desde `ring_angle(tiempo)` del mismo radio y grosor en `agent.color()`.
- Produces: `fn tick_for(agent_busy: bool, any_busy: bool) -> Option<Duration>`: `Some(RING_TICK)` si `agent_busy`; si no, `Some(250 ms)` si `any_busy`; si no, `None`.

- [ ] **Step 1: Pruebas que fallan**

```rust
#[test]
fn arco_da_una_vuelta_por_segundo() {
    assert_eq!(ring_angle(0.0), 0.0);
    assert!((ring_angle(0.25) - std::f32::consts::FRAC_PI_2).abs() < 1e-4);
    assert!((ring_angle(1.25) - std::f32::consts::FRAC_PI_2).abs() < 1e-4);
    assert!(ring_angle(0.999) < std::f32::consts::TAU);
}

#[test]
fn ritmo_de_repintado() {
    assert_eq!(tick_for(true, true), Some(Duration::from_millis(66)));
    assert_eq!(tick_for(false, true), Some(Duration::from_millis(250)));
    assert_eq!(tick_for(false, false), None);
}
```

- [ ] **Step 2:** `cargo test arco_da ritmo_de` → falla.
- [ ] **Step 3: Implementar** `ring_angle`, `tick_for` (constantes `RING_PERIOD`, `RING_TICK`) y `busy_ring`.
- [ ] **Step 4: Usarlas:**
  - `tab_bar`: con `s.busy` y agente → `busy_ring(p, ind, 5.5, a)`; con `s.busy` sin agente → `spinner` como hoy. El resto de casos no cambia.
  - `side_panel`: el icono de arriba de cada agente (`agent_icon(..., 6.5, agent)`) pasa a `busy_ring` cuando `s.busy`; el punto de «Trabajando…» de abajo deja de dibujar `spinner` (queda solo el texto).
  - Final de `ui()`: sustituir el `if any busy { 250 ms }` por `tick_for(any agente busy, any busy)` sobre `self.w.sessions`; el comentario «no se anima» se actualiza.
- [ ] **Step 5:** `cargo test` verde; `cargo clippy` sin avisos nuevos.
- [ ] **Step 6: Comprobación** con un `claude` falso que se anuncie trabajando por el título (`printf '\033]0;⠋ claude\007'; sleep 8; printf '\033]0;✳ claude\007'; sleep 30`) y `COLIBRI_FPS=1 COLIBRI_LAZY=1`: captura del arco en la pestaña y en el panel (`open` del panel de agentes con `cmd+shift+a` si hace falta); ~15 cuadros/s mientras trabaja, 0/s cuando pasa a `✳`. Con `sleep 30` lanzado en una pestaña sin agente: ~4/s y luna a saltos.
- [ ] **Step 7:** Actualizar `CLAUDE.md`: la frase de «La luna de “trabajando”…» (sección de repintado) para decir que con agente se dibuja `busy_ring` con refresco de 66 ms y sin agente sigue la luna a 250 ms.
