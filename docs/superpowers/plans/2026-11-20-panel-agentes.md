# Panel de agentes — plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Un panel de agentes de arriba abajo (izquierda por defecto, configurable a la derecha) con nueva sesión, carpeta, buscador, filtros con contadores, lista agrupada por carpeta y lanzadores de agentes, que sustituye a la sección «Agentes» del panel derecho.

**Architecture:** Módulo nuevo `src/agents.rs` con la parte pura (filas, estados, filtros, búsqueda, grupos, contadores, agentes instalados) y el dibujo del panel (`impl App`, como `overlay.rs`). `main.rs` reparte la ventana: el panel de agentes se declara **antes** que la barra de pestañas para que ocupe toda la altura; Puertos y Procesos siguen en `side_panel`, en el lado contrario. `Session` gana «en este estado desde» y una orden pendiente para lanzar el agente cuando el shell esté listo.

**Tech Stack:** Rust 2024, egui (`Panel::left/right`, `TextEdit`, `Painter`), guion de depuración (`src/debug.rs`).

**Spec:** `docs/superpowers/specs/2026-11-20-panel-agentes-design.md`

## Global Constraints

- Código, comentarios y textos de la interfaz en español.
- Repintado bajo demanda: nada nuevo repinta en reposo (lo único animado es la insignia, que ya tiene su refresco).
- Sin «Archivadas». ⌘T = nueva sesión (no se duplica en la barra de menús); ⇧⌘T sigue siendo «reabrir lo cerrado».
- `/` no es atajo global: el buscador solo recibe teclas tras un clic; `Esc` lo vacía y devuelve el teclado a la terminal.
- Estado del panel compartido por todas las ventanas (en `App`); la lista incluye las pestañas con agente de todas las ventanas (`self.w` + `parked`).
- Preferencia nueva `agents_side`: `"izquierda"` (por defecto) o `"derecha"`; `show_agents` pasa a `true` por defecto. `#[serde(default)]`.
- Ancho del panel 220–420, 260 por defecto; no se guarda.
- Lanzadores, en este orden, solo los instalados: Claude Code (`claude`), Codex (`codex`), Gemini (`gemini`), Aider (`aider`), OpenCode (`opencode`). Instalados = encontrados con el PATH de `$SHELL -lc`, una vez al arrancar, en un hilo.
- Pruebas manuales con `COLIBRI_SCRIPT` + `COLIBRI_CONFIG`/`COLIBRI_STATE` temporales.
- Sin commits hasta que el usuario lo pida.

## Review Focus

1. Panel a la izquierda visible: los semáforos no quedan tapados ni descolocados y la ventana se sigue arrastrando por la franja superior del panel y por el hueco libre de la barra de pestañas → captura en la tarea 3 + prueba a mano del usuario (el guion no pasa por macOS).
2. Escribir en el buscador no llega a la terminal y viceversa; `Esc` devuelve el teclado → paso del guion en la tarea 4 (escribir con el buscador enfocado y comprobar que la terminal no lo recibe).
3. Un agente en otra ventana sale en la lista y su clic trae esa ventana → paso del guion en la tarea 4 con `window new`.
4. Ocultar/mostrar el panel o cambiar de lado recoloca la barra de pestañas sin saltos raros ni pestañas bajo los semáforos → capturas en la tarea 3.
5. En reposo con el panel abierto: 0 cuadros/s; escribir en el buscador no deja repintado continuo → `COLIBRI_FPS` en la tarea 4.

---

### Task 1: Modelo del panel (puro, con pruebas)

**Files:**
- Create: `src/agents.rs` (declarar `mod agents;` en `main.rs`)
- Modify: `src/session.rs` (campo `pub state_since: Instant`)

**Interfaces:**
- Produces (en `agents.rs`):
  - `#[derive(Clone, Copy, PartialEq, Debug)] pub enum State { Working, Done, Waiting }` y `pub fn state_of(busy: bool, unseen: bool) -> State` (busy → Working; si no, unseen → Done; si no, Waiting).
  - `#[derive(Clone, Copy, PartialEq, Debug, Default)] pub enum Filter { #[default] All, Waiting, Working }` y `impl Filter { pub fn admits(self, s: State) -> bool }` (Waiting admite Done y Waiting).
  - `pub struct Row { pub id: u64, pub dir: PathBuf, pub title: String, pub agent: Agent, pub state: State, pub since: Duration, pub current: bool }`.
  - `pub fn row_title(name: &str, title: &str, agent: Agent) -> String`: `name` si no está vacío; si no, `title` salvo que esté vacío o sea `agent.label()`; si no, `agent.label()`.
  - `pub fn fold(s: &str) -> String`: minúsculas y sin tildes (á é í ó ú ü ñ → a e i o u u n).
  - `pub fn matches(row: &Row, query: &str) -> bool`: `query` vacío → true; si no, `fold(query)` contenido en `fold` del título, del nombre de la carpeta o de `agent.label()`.
  - `pub fn counts(rows: &[Row]) -> [usize; 3]` (Todos, Esperando, Trabajando; sin aplicar búsqueda).
  - `pub struct Group { pub dir: PathBuf, pub label: String, pub rows: Vec<usize> }` y `pub fn groups(rows: &[Row], visible: impl Fn(&Row) -> bool) -> Vec<Group>`: por ruta completa, en orden de primera aparición; `label` = nombre de la carpeta, o `proc::pretty_path(dir)` si otro grupo se llama igual; grupos sin filas visibles no salen.
- Produces (en `Session`): `pub state_since: Instant`, puesto en `Session::new`, cuando `agent` cambia en `poll` y en cada cambio de `busy` (junto a `busy_since`).

- [ ] **Step 1: Pruebas que fallan** en `agents.rs` (`mod tests`):

```rust
#[test] fn estado_de_cada_fila() {
    assert_eq!(state_of(true, true), State::Working);
    assert_eq!(state_of(false, true), State::Done);
    assert_eq!(state_of(false, false), State::Waiting);
    assert!(Filter::Waiting.admits(State::Done) && Filter::Waiting.admits(State::Waiting));
    assert!(!Filter::Waiting.admits(State::Working));
    assert!(Filter::Working.admits(State::Working) && !Filter::Working.admits(State::Done));
    assert!(Filter::All.admits(State::Working) && Filter::All.admits(State::Waiting));
}
#[test] fn titulo_de_la_fila() {
    assert_eq!(row_title("mío", "Logos", Agent::Claude), "mío");
    assert_eq!(row_title("", "Logos que no se muestran", Agent::Claude), "Logos que no se muestran");
    assert_eq!(row_title("", "", Agent::Codex), "Codex");
    assert_eq!(row_title("", "Claude Code", Agent::Claude), "Claude Code");
}
#[test] fn busqueda_sin_mayusculas_ni_tildes() {
    let r = fila(1, "/p/midulive", "Logos que no se muestran en línea", Agent::Claude, State::Done);
    assert!(matches(&r, ""));
    assert!(matches(&r, "LINEA"));
    assert!(matches(&r, "midu"));
    assert!(matches(&r, "claude"));
    assert!(!matches(&r, "codex"));
}
#[test] fn contadores() {
    let rows = [fila(1, "/a", "", Agent::Claude, State::Working),
                fila(2, "/a", "", Agent::Codex, State::Done),
                fila(3, "/b", "", Agent::Claude, State::Waiting)];
    assert_eq!(counts(&rows), [3, 2, 1]);
}
#[test] fn grupos_por_carpeta_completa() {
    let rows = [fila(1, "/x/app", "", Agent::Claude, State::Working),
                fila(2, "/y/web", "", Agent::Claude, State::Waiting),
                fila(3, "/x/app", "", Agent::Codex, State::Waiting),
                fila(4, "/z/app", "", Agent::Claude, State::Waiting)];
    let g = groups(&rows, |_| true);
    assert_eq!(g.len(), 3);
    assert_eq!(g[0].rows, vec![0, 2]);
    assert_eq!(g[1].label, "web");
    assert!(g[0].label.contains("/x/app") && g[2].label.contains("/z/app")); // nombres repetidos
    let solo_trabajando = groups(&rows, |r| r.state == State::Working);
    assert_eq!(solo_trabajando.len(), 1);
}
```
(`fila(id, dir, title, agent, state)` es un ayudante de la prueba que construye un `Row` con `since` 0 y `current` falso.)

- [ ] **Step 2:** `cargo test agents::` → falla (no existe).
- [ ] **Step 3:** Implementar lo del bloque Interfaces y el campo `state_since`.
- [ ] **Step 4:** `cargo test` → todo verde.

### Task 2: Agentes instalados y orden pendiente

**Files:**
- Modify: `src/agents.rs`, `src/session.rs`, `src/main.rs` (campo en `App`)

**Interfaces:**
- Produces:
  - `pub const LAUNCHERS: [Agent; 5] = [Agent::Claude, Agent::Codex, Agent::Gemini, Agent::Aider, Agent::OpenCode];`
  - `pub fn command(agent: Agent) -> &'static str` (`claude`, `codex`, `gemini`, `aider`, `opencode`; `Other` → `""`).
  - `pub fn parse_installed(out: &str) -> Vec<Agent>`: `out` es la salida de `command -v claude codex gemini aider opencode` (una ruta absoluta por línea de las que existen); devuelve los de `LAUNCHERS` cuya orden aparece como último componente de alguna línea, en el orden de `LAUNCHERS`.
  - `pub fn find_installed(ctx: egui::Context) -> Arc<Mutex<Option<Vec<Agent>>>>`: lanza un hilo con `$SHELL -lc 'command -v claude codex gemini aider opencode'` (o `/bin/zsh` si no hay `$SHELL`), guarda el resultado y llama a `shell::wake_all()`. `App` lo guarda en `installed` al arrancar.
  - `Session::pending: Option<String>` y `Session::run_when_ready(&mut self, cmd: String)`; en `poll`, si hay `pending` y `at_prompt()`, se hace `insert_command(&cmd)` + `input(b"\r")` y se vacía.

- [ ] **Step 1: Prueba que falla**

```rust
#[test] fn agentes_instalados_en_su_orden() {
    let out = "/Users/x/.local/bin/claude\n/opt/homebrew/bin/gemini\n/usr/local/bin/codex\n";
    assert_eq!(parse_installed(out), vec![Agent::Claude, Agent::Codex, Agent::Gemini]);
    assert_eq!(parse_installed(""), Vec::<Agent>::new());
    // Un alias o una función de zsh («claude: aliased to …») no es una ruta: no cuenta.
    assert_eq!(parse_installed("claude: aliased to foo\n"), Vec::<Agent>::new());
}
```
- [ ] **Step 2:** `cargo test agentes_instalados` → falla.
- [ ] **Step 3:** Implementar lo del bloque Interfaces.
- [ ] **Step 4:** `cargo test` → verde.

### Task 3: Reparto de la ventana (lado, semáforos, botón de ocultar)

**Files:**
- Modify: `src/config.rs` (`agents_side`, `show_agents` por defecto `true`), `src/overlay.rs` (fila en `PREF_ROWS`, valor mostrado «Izquierda»/«Derecha», rama en `pref_change` que alterna), `src/main.rs` (`ui()`, `tab_bar`, `side_panel`, `tabs_avail`)

**Interfaces:**
- Consumes: nada de las tareas 1–2 salvo que el contenido del panel puede ser, por ahora, un `ui.label("Agentes")`.
- Produces:
  - `App::agents_panel(&mut self, ui: &mut egui::Ui)` (en `agents.rs`, `impl App`), llamado desde `ui()` dentro de `egui::Panel::left("agents")` o `Panel::right("agents")` según `cfg.agents_side`, **antes** del `Panel::top("tabs")`; `default_size(260.0)`, `size_range(220.0..=420.0)`, `resizable(true)`, relleno `pal.panel`.
  - `fn toggle_button(&mut self, ui: &mut egui::Ui, center: Pos2)`: icono de panel lateral (rectángulo con una franja; dibujado a mano) que alterna `show_agents`; tooltip con el atajo `toggle_agents` (`pretty_shortcut`).
  - Con el panel a la izquierda y visible: su franja superior (alto `TAB_BAR_H`) deja libre `TRAFFIC_LIGHTS_W` para los semáforos y pinta el botón de ocultar justo a su derecha; la barra de pestañas empieza en `full.left()` sin reservar semáforos. Con el panel a la derecha (visible): el botón va a la izquierda de su franja superior; la barra reserva semáforos como hoy. Panel oculto: la barra reserva `TRAFFIC_LIGHTS_W` + el botón (28 pt) y pinta el botón tras los semáforos.
  - `tabs_avail(full, right_reserved, term, left_reserved: f32)` (parámetro nuevo; actualizar su prueba con el caso «panel a la izquierda»: `left_reserved = 0`).
  - Se quita el botón de Agentes (robot) del grupo de la derecha de la barra; `right_reserved` baja en lo que ocupaba (36 pt). Su contador de «terminados sin ver» pasa al botón de ocultar cuando el panel está oculto.
  - `side_panel` deja de dibujar la sección «Agentes»; su `Panel` va a `right` o `left` (el contrario de `agents_side`), solo si `show_ports || show_procs`.

- [ ] **Step 1: Prueba que falla** — ampliar `pestanas_no_pasan_sobre_el_panel` con `tabs_avail(full, 122.0, term, 0.0)` = `term.right() - 34.0` cuando el panel de agentes está a la izquierda (sin semáforos), y los casos existentes con `left_reserved = TRAFFIC_LIGHTS_W`.
- [ ] **Step 2:** `cargo test pestanas_no` → falla (firma nueva).
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** `cargo test` → verde.
- [ ] **Step 5: Capturas** (guion, config temporal con `show_agents = true`): panel izquierda visible con 3 pestañas; `pref agents_side +1` (derecha); `key cmd+shift+a` u otro camino que llame a `Cmd::ToggleAgents` (oculto). En cada una: semáforos a la vista (hueco libre arriba a la izquierda), botón de ocultar a su sitio, pestañas sin meterse bajo el panel ni bajo los semáforos, Puertos/Procesos (`procs`) en el lado contrario.

### Task 4: Contenido del panel

**Files:**
- Modify: `src/agents.rs` (`agents_panel`), `src/main.rs` (campos en `App`: `agents_filter: agents::Filter`, `agents_query: String`; `Win::agents_search_focus: bool`), `src/debug.rs` + `App::run_script` (pasos `agents filter all|waiting|working` y `agents search <texto>`)

**Interfaces:**
- Consumes: todo lo de las tareas 1–3; `badged_icon`, `agent_icon`, `ago`, `focus_session`, `new_tab`, `Session::run_when_ready`.
- Produces: el panel completo, de arriba abajo:
  1. Franja superior (tarea 3).
  2. «Nueva sesión» con su atajo a la derecha (el de `new_tab`, `pretty_shortcut`) → `Cmd::NewTab`; a su derecha, botón de carpeta → tarea 5 (`open_dir_picker`).
  3. Buscador (`TextEdit::singleline`, sugerencia «Buscar agentes»): mientras tiene el foco, `self.w.search_focus = true` (el mismo cerrojo que ya usa el buscador de procesos, para que la terminal no reciba teclas); `Esc` lo vacía y le quita el foco.
  4. Filtros «Todos N», «Esperando N», «Trabajando N» (`counts`), el elegido resaltado.
  5. Lista con `ScrollArea`: por cada `Group` visible (`filter.admits && matches`), su `label` en gris; cada fila: `badged_icon` (o `agent_icon` si `Waiting`) con superficie `pal.panel`, título (`row_title`, recortado con `elided`), segunda línea «Trabajando…» (`pal.fg`) / «Terminó» (`pal.done`) / «Esperando tu mensaje» (`pal.muted`), tiempo `ago(since)` a la derecha; fila de la pestaña activa de la ventana en curso resaltada; clic → `focus_session(id)`. Sin filas: «Sin agentes. Abre uno con los iconos de abajo.» (o «Nada coincide.» si hay búsqueda/filtro).
  6. Abajo, separada por una línea: un icono por agente de `installed` (`agent_icon`), tooltip con su nombre; clic → `new_tab` en la carpeta de la activa + `run_when_ready(command(agent))`.
- Las filas se construyen de `self.w.sessions` y de las de `self.parked` con `agent.is_some()`: `Row { state: state_of(busy, unseen), since: state_since.elapsed(), current: ventana en curso && i == active, … }`.

- [ ] **Step 1:** Implementar (la lógica ya tiene pruebas en la tarea 1; esta tarea es dibujo).
- [ ] **Step 2:** `cargo test` y `cargo clippy` sin avisos nuevos.
- [ ] **Step 3: Comprobaciones con el guion** (`claude` falso que anuncia «trabajando» por el título 6 s y luego espera; otro llamado `codex` igual pero sin título; `COLIBRI_LAZY=1`, `COLIBRI_FPS=1`):
  - Captura con dos carpetas y tres agentes: grupos, insignias, estados, tiempos, contadores.
  - `agents filter working` / `waiting`: captura de cada una; contadores no cambian.
  - `agents search linea`: solo la fila cuyo título tiene «línea».
  - Con el buscador enfocado (clic en él con `press/release`), `type hola`: la terminal no recibe «hola» (captura de la terminal); `key esc` vacía el buscador y la siguiente `type x` llega a la terminal.
  - `window new` + lanzar un agente falso en la ventana nueva: sale en la lista de la primera; clic en su fila → esa ventana al frente (`window` activa en stderr o captura).
  - Clic en el icono de Claude de abajo: pestaña nueva que arranca el `claude` falso en cuanto sale el prompt.
  - En reposo 6 s con el panel abierto: 0 cuadros/s.

### Task 5: Botón de carpeta → pestaña nueva

**Files:**
- Modify: `src/main.rs` (`open_dir_picker`), `src/overlay.rs` (`picker_ui`), `Win::pick_new_tab: bool`

**Interfaces:**
- Produces: `App::open_dir_picker(&mut self, ctx: &egui::Context, at: Pos2)`: como `open_picker` con la caja de carpeta, pero solo `Parent` + `Dir` (sin archivos) de la carpeta de la pestaña activa, y `pick_new_tab = true`. En `picker_ui`, con `pick_new_tab`, elegir `Dir` abre `new_tab(Some(cwd.join(name)))` y `Parent`, `new_tab(Some(cwd.parent()))`; no escribe `cd`. Se pone a `false` al cerrar el selector.

- [ ] **Step 1: Prueba que falla** — extraer `fn pick_target(cwd: &Path, kind: PickKind, name: &str) -> Option<PathBuf>` (Dir → `cwd/name`, Parent → padre, resto → `None`) con su prueba:

```rust
#[test] fn carpeta_elegida_para_pestana_nueva() {
    let cwd = Path::new("/p/colibri");
    assert_eq!(pick_target(cwd, PickKind::Dir, "src"), Some(PathBuf::from("/p/colibri/src")));
    assert_eq!(pick_target(cwd, PickKind::Parent, ".."), Some(PathBuf::from("/p")));
    assert_eq!(pick_target(cwd, PickKind::File, "a.rs"), None);
}
```
- [ ] **Step 2:** `cargo test carpeta_elegida` → falla.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** `cargo test` verde; captura del selector abierto desde el botón (clic simulado) y de la pestaña nueva tras `key down` + `key enter`.

### Task 6: Documentación

**Files:**
- Modify: `CLAUDE.md`

- [ ] **Step 1:** Añadir `agents.rs` a la lista de módulos; una entrada «Panel de agentes» en «Cosas que no se ven…» con: panel declarado antes que la barra para ocupar toda la altura, franja de semáforos y botón de ocultar según el lado, estado compartido en `App`, lista de todas las ventanas, cerrojo `search_focus` del buscador, lanzadores con `$SHELL -lc` y `run_when_ready`, selector de carpeta en modo pestaña nueva; los pasos nuevos del guion (`agents filter …`, `agents search …`) en la lista de pasos.
- [ ] **Step 2:** `cargo test` final.
