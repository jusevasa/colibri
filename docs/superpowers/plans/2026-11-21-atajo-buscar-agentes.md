# Atajo para buscar agentes — plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** ⌘⇧F lleva el teclado al buscador de agentes (abriendo el panel si hace falta), la caja enseña la tecla, y ↑/↓ + Enter saltan al agente elegido.

**Architecture:** Acción nueva `Cmd::SearchAgents` por el camino de siempre (config, tabla de `Keys`, `App::run`, paleta, menú). En `agents_panel`, una fila elegida (`App::agents_pick`) que mueven ↑/↓ (consumidas antes del `TextEdit`) y que Enter abre con `focus_session`. La aritmética de elegir va en una función pura con pruebas.

**Tech Stack:** Rust 2024, egui.

**Spec:** `docs/superpowers/specs/2026-11-21-atajo-buscar-agentes-design.md`

## Global Constraints

- Código, comentarios y textos en español; textos de interfaz en `ES` y `EN`.
- Tecla por defecto `cmd+shift+f`, campo `search_agents` en `config::Keys`.
- Sin repintados continuos.
- Probar con `COLIBRI_CONFIG` apuntando a un archivo temporal.

## Review Focus

- Enter con el buscador sin resultados (búsqueda sin coincidencias): no hace nada y no llega a la terminal.
- La fila elegida desaparece de la lista mientras se busca (el agente termina y el filtro «Trabajando» la oculta, o se cierra la pestaña): vuelve a la primera visible, sin quedarse sin elegir.
- ⌘⇧F desde la paleta (la paleta tenía el foco): el buscador debe quedarse con él.
- ⌘⇧F con el panel oculto: se abre y el buscador recibe ya la primera letra.
- ↑/↓ en la caja no mueven el cursor del texto ni llegan a la terminal.

---

### Task 1: `step_pick`, la fila elegida

**Files:**
- Modify: `src/agents.rs` (parte pura, junto a `groups`; pruebas en `mod tests`)

**Interfaces:**
- Produces: `pub fn step_pick(order: &[u64], pick: Option<u64>, step: i32) -> Option<u64>`: `order` son los ids visibles en el orden en que se ven. Si `pick` no está en `order` cuenta como la primera. `step` mueve con vuelta (como la paleta). `order` vacío → `None`.

- [ ] **Step 1: Pruebas**

```rust
#[test]
fn elegir_fila_con_flechas() {
    let o = [10, 20, 30];
    assert_eq!(step_pick(&o, None, 0), Some(10));
    assert_eq!(step_pick(&o, Some(99), 0), Some(10)); // ya no se ve
    assert_eq!(step_pick(&o, Some(10), 1), Some(20));
    assert_eq!(step_pick(&o, Some(30), 1), Some(10)); // da la vuelta
    assert_eq!(step_pick(&o, Some(10), -1), Some(30));
    assert_eq!(step_pick(&o, None, 1), Some(20)); // desde la primera
    assert_eq!(step_pick(&[], Some(10), 1), None);
}
```

- [ ] **Step 2:** `cargo test elegir_fila_con_flechas` → falla (no existe).
- [ ] **Step 3:** Implementar `step_pick` en `src/agents.rs`.
- [ ] **Step 4:** `cargo test elegir_fila_con_flechas` → pasa.

### Task 2: acción `SearchAgents`

**Files:**
- Modify: `src/config.rs` (`Keys` + `Default`), `src/main.rs` (`Cmd`, `Keys::new`, `App::run`, `App::menu_spec` tras `ToggleAgents`), `src/overlay.rs` (lista de la paleta tras `ToggleAgents`), `src/i18n/overlay.rs`, `src/i18n/menu.rs` (campo `search_agents`: «Buscar agentes» / «Search Agents»).

**Interfaces:**
- Produces: `Cmd::SearchAgents`; `self.keys.label(Cmd::SearchAgents)` → texto de la tecla.

- [ ] **Step 1:** Variante, campo de config, fila en la tabla, textos, paleta y menú.
- [ ] **Step 2:** `App::run`: `Cmd::SearchAgents` pone `show_agents = true`, `agents_pick = None` y `ctx.memory_mut(|m| m.request_focus(agents::search_id()))`.
- [ ] **Step 3:** Comprobar con el guion (config temporal con `show_agents = false`): `key cmd+shift+f`, `type abc`, `shot` → panel abierto con «abc» en el buscador y nada en el prompt. Repetir con `menu Buscar agentes` y con `open palette` + `type buscar agentes` + `key enter`.

### Task 3: tecla en la caja, fila elegida, ↑/↓ y Enter

**Files:**
- Modify: `src/main.rs` (`App::agents_pick: Option<u64>`, inicializado a `None`), `src/agents.rs` (`agents_panel`, `agent_row`), `CLAUDE.md` (párrafo «Panel de agentes»: el buscador ya no solo se activa con un clic).

**Interfaces:**
- Consumes: `step_pick` (Task 1), `Cmd::SearchAgents` (Task 2).

- [ ] **Step 1:** Antes del `TextEdit`, si `search_id()` tiene el foco en la memoria de egui, consumir `ArrowUp`/`ArrowDown` sin modificadores y guardar el paso (−1/+1).
- [ ] **Step 2:** Con la caja vacía y sin foco, pintar a la derecha `self.keys.label(Cmd::SearchAgents)` (fuente `small`, `pal.muted`, alineada a la derecha a 8 pt del borde); nada si la etiqueta está vacía.
- [ ] **Step 3:** Si `edit.changed()`, `agents_pick = None`. Tras `groups`, `order` = ids aplanando `g.rows` en orden; `agents_pick = step_pick(&order, agents_pick, paso)` mientras el buscador tiene el teclado.
- [ ] **Step 4:** `agent_row` recibe `picked: bool` (solo con el buscador activo): contorno de 1 pt `pal.accent`, y `scroll_to_me(None)` si el paso era distinto de 0.
- [ ] **Step 5:** Enter: si `edit.lost_focus()` y se pulsó Enter, `go = agents_pick`; Enter y Esc dejan `agents_pick = None`.
- [ ] **Step 6:** `cargo test`, `cargo clippy` y guion con dos `claude` falsos en carpetas distintas: `agents focus`, `type <carpeta 2>`, `shot` (elegida la de la carpeta 2), `key enter`, `shot` (activa esa pestaña, la caja sin foco con «⌘⇧F» visible tras Esc). Otra pasada con `key down`/`key up` dando la vuelta, y una con una búsqueda sin resultados + `key enter` + `type x` (la «x» no llega al prompt… porque Enter soltó el foco: llega a la terminal, y es correcto; lo que no debe pasar es que Enter llegue al shell).
