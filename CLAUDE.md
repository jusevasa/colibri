# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Qué es

Colibrí es una terminal para macOS escrita en Rust (edición 2024), pensada para correr varios agentes de código (Claude Code, Codex, Gemini…) en pestañas y saber cuál está trabajando y cuál terminó. Es un único binario: `eframe`/`egui` sobre wgpu para la interfaz y `alacritty_terminal` como emulador + PTY. Solo macOS: usa `libproc`, `lsof`, `ps`, `vm_stat`, `osascript`, `open` y APIs de Cocoa vía `objc2`.

El código, los comentarios, los textos de la interfaz y los mensajes de commit están en español; mantén ese idioma.

## Comandos

```sh
cargo run                      # desarrollo (perfil dev con opt-level 1)
cargo build --release
cargo test                     # las únicas pruebas están en src/view.rs
cargo test sgr_press_release   # una sola prueba, por nombre
cargo fmt && cargo clippy
scripts/bundle.sh              # release + target/Colibri.app firmada
scripts/rebundle.sh            # lo mismo, pero solo si el código cambió desde el último bundle
python3 scripts/icon.py assets # regenera assets/colibri.svg y colibri-1024.png
```

- **Rebundle automático**: `.claude/settings.json` (a nivel de proyecto, versionado) tiene un hook `Stop` que corre `scripts/rebundle.sh` cuando Claude Code termina un turno. El script solo llama a `bundle.sh` si algo de `src/`, `assets/`, `Cargo.toml`, `Cargo.lock` o `scripts/bundle.sh` es más nuevo que el ejecutable del bundle; si no, no hace nada. Así el Colibrí abierto muestra «Actualizar» tras cada cambio sin correr nada a mano. Si la compilación falla solo avisa, no bloquea. Se desactiva desde `/hooks` o borrando la entrada.
- `scripts/bundle.sh` firma con una identidad fija para que macOS recuerde los permisos entre compilaciones; se cambia con `COLIBRI_SIGN_ID`. Si falla, cae a firma ad-hoc.
- Las notificaciones nativas (con clic que lleva a la pestaña) solo funcionan desde `Colibri.app`; con `cargo run` se usa `osascript` como respaldo sin clic. Para probar notificaciones hay que pasar por `bundle.sh`.

## Probar la interfaz sin tocarla a mano

No hay pruebas de interfaz; en su lugar existe un guion de depuración (`src/debug.rs`) que maneja la app y guarda capturas de su propio render:

```sh
COLIBRI_SCRIPT=pasos.txt COLIBRI_CONFIG=/ruta/tmp/config.toml cargo run
```

Un paso por línea: `type <texto>` (`\n` = Enter), `wait <ms>`, `tab`, `select <n>`, `move <de> <a>`, `agent next|prev`, `name <nombre>`, `shot <ruta.bmp>`, `procs`, `ports`, `hover`, `killport <puerto>`, `open palette|prefs|history|branches|dirs`, `key up|down|enter|esc|tab|paste` (pulsa la tecla como si viniera del teclado; `paste` equivale a ⌘V), `press|moveto|release <x> <y>` (puntero simulado, en puntos de la ventana), `rclick <x> <y>` (clic derecho), `pref <id> <+1|-1>`, `update` (fuerza el botón «Actualizar»), `quit`. Las líneas que no se reconocen se ignoran en silencio. Los `id` de `pref` son los de `PREF_ROWS` en `src/overlay.rs`.

Variables de entorno para no pisar los archivos reales del usuario:

- `COLIBRI_CONFIG` — otro `config.toml` (por defecto `~/.config/colibri/config.toml`). Ojo: `pref` y Preferencias **escriben** en ese archivo, así que úsala siempre al probar.
- `COLIBRI_STATE` — otro `session.toml`. Con `COLIBRI_SCRIPT` activo no se restauran ni guardan pestañas salvo que se defina esta variable.

Al añadir una función a la interfaz que deba poder verificarse, añade también su paso al guion (`Step` en `debug.rs` + el `match` de `App::run_script` en `main.rs`).

## Arquitectura

`src/main.rs` contiene `App` (todo el estado de la ventana) y casi todo el dibujo del cromo: barra de pestañas, panel lateral (Agentes / Puertos / Procesos) e iconos, que se dibujan a mano con `Painter` para no depender de los glifos de la fuente. `eframe::App` se parte en dos: `logic()` (recarga de config, guion, sondeo de sesiones, notificaciones, guardado de pestañas) y `ui()` (atajos, overlays, paneles).

- **`session.rs`** — una pestaña: `Term` de alacritty + PTY + el hilo `EventLoop` de alacritty. Los eventos del emulador llegan por un canal `mpsc` y se procesan en `Session::drain` una vez por cuadro. `Session::poll` averigua el proceso en primer plano del PTY, detecta el agente y calcula `busy`.
- **`view.rs`** — pinta la rejilla de la sesión activa y traduce teclado/ratón a bytes para el PTY (`key_bytes`, `mouse_seq`).
- **`overlay.rs`** — paleta de comandos, Preferencias y renombrar pestaña. Es un segundo `impl App` (`overlay_ui`, `pref_change`).
- **`config.rs`** — `Config`/`Theme`/`Keys` (serde + TOML), temas incluidos, `Palette` (colores ya resueltos) y el parser de `Shortcut`.
- **`history.rs`** — lee el historial del shell (zsh, bash, fish; respeta `$HISTFILE`) para la lista que abre `↑` en el prompt.
- **`complete.rs`** — sugerencias de carpetas y archivos para la palabra que se teclea en el prompt (Tab); solo lee el disco.
- **`git.rs`** — rama (`.git/HEAD`) y archivos con cambios (`git status` en un hilo) para las cajas del prompt.
- **`clip.rs`** — ⌘C/⌘X/⌘V: eframe arranca con `clipboard_shortcuts: false` y `clip::translate` (en `raw_input_hook`) convierte esas teclas en `Copy`/`Cut`/`Paste`/`PasteImage` leyendo NSPasteboard. Orden al pegar: archivos (`public.file-url`: lo que deja Finder y lo que llega del iPhone por Continuity) → se pega la ruta escapada; texto → se pega; imagen suelta (captura) → Ctrl+V para que el programa la lea él mismo, sin que Colibrí la toque. egui por sí solo ignoraba el ⌘V con archivos.
- **`hotkey.rs`**, **`menu.rs`** — FFI de macOS: atajo global (Carbon) y «Reiniciar Colibrí» en el menú de la aplicación.
- **`state.rs`** — pestañas abiertas persistidas en `session.toml` (carpeta, nombre y agente; no los procesos).
- **`proc.rs`** — consultas puntuales por pid vía libc/libproc (cwd, argv, padre, nombre). **`procs.rs`** y **`ports.rs`** — hilos de fondo que ejecutan `ps`/`vm_stat` y `lsof` y publican en un `Arc<Mutex<…>>`.
- **`notify.rs`**, **`blur.rs`** — FFI de macOS (UNUserNotificationCenter; API privada de CoreGraphics para el desenfoque).

### Cosas que no se ven leyendo un solo archivo

- **Una acción nueva** toca varios sitios: variante en `Cmd`, rama en `App::run`, campo en `config::Keys` (con su valor por defecto) y fila en la tabla de `Keys::new` en `main.rs`; además la lista de la paleta en `overlay.rs`. `cmd+1..9` es fijo y no pasa por la config.
- **Una preferencia nueva**: campo en `Config` + `Default`, fila en `PREF_ROWS` y ramas en el valor mostrado y en `pref_change` (`overlay.rs`). Todos los structs de config llevan `#[serde(default)]`, de modo que los archivos antiguos siguen cargando.
- **Detección de agente y de «trabajando»** (`session.rs`): el agente sale del nombre del proceso en primer plano y, si no basta, de `argv` (Claude Code se llama como su número de versión). `busy` usa primero el título de la ventana (spinner braille = trabajando, `✳` = esperando) y, si el agente no lo anuncia, una heurística de ráfagas de salida que descarta el eco de lo que tecleas. Sin agente, `busy` = hay un proceso en primer plano que no es el shell. Una tarea solo cuenta como «terminada» (punto verde, notificación, `⌘J`) si duró ≥ 2 s con agente o ≥ 8 s sin él.
- **Repintado bajo demanda**: en reposo la app no repinta. Lo que cambie fuera del hilo de la interfaz debe llamar a `ctx.request_repaint()` (así lo hacen `Listener`, `Ports` y `Monitor`), y los hilos de fondo solo trabajan mientras su apartado está visible (`enabled`). No añadas repintados continuos.
- **Config en caliente**: `logic()` mira el mtime de `config.toml` cada segundo y recarga. `commit_cfg` actualiza `cfg_mtime` tras guardar para no recargar su propia escritura.
- **Restaurar pestañas**: donde corría Claude Code queda `claude --continue` como sugerencia en gris (`Session::suggestion`), que Colibrí pinta junto al cursor pero **no escribe en el shell** (escribirlo antes de que el shell arrancara descuadraba el prompt). Enter la ejecuta, Tab o → la escriben, cualquier otra tecla la descarta (`App::suggest_key`).
- **Privacidad** como requisito de diseño: la app no abre conexiones de red propias (la única sonda es HTTP a `127.0.0.1` para saber si un puerto es web) y `ClipboardLoad` se ignora a propósito para que ningún programa lea el portapapeles.
- **Botón «Actualizar»**: aparece en la barra de pestañas cuando el mtime del ejecutable en disco cambia respecto al del arranque (tras `cargo build` o `scripts/bundle.sh`). `App::restart` guarda las pestañas, lanza la versión nueva (`open -n` sobre el bundle, o el binario suelto) y sale; también está en la paleta como «Reiniciar Colibrí…».
- **Historial con `↑`**: `App::history_key` intercepta `↑` solo si `Session::at_prompt()` (el shell en primer plano, sin pantalla alterna ni modo ratón) y abre `Overlay::History`; `Enter` llama a `Session::insert_command` (`ctrl+u` + pegado, sin ejecutar). `Session::track` reconstruye lo tecleado en el prompt para ofrecerlo aunque zsh no lo haya escrito aún en su archivo; lo que se escribe a otros programas no se registra. Al probar, apunta `HISTFILE` a un archivo falso para no leer el historial real.
- **Sugerencias con Tab**: `App::complete_key` solo intercepta Tab si `Session::prompt_line()` sabe qué hay escrito (se pierde la pista con flechas, Tab al shell, etc.) y ya se pasó del nombre del comando; si no, o si nada coincide, el Tab llega al shell. Con una sola coincidencia se inserta directa; con varias abre `Overlay::Complete`. Teclear con la lista abierta la cierra y la tecla pasa al shell.
- **Cajas en el prompt** (carpeta, rama, cambios): no hay integración con el shell. `Session::stamp_prompt` localiza la fila de encima de donde se escribe (prompts de dos líneas como powerlevel10k) y, si nombra la carpeta actual, **reescribe sus celdas en la rejilla** con «carpeta rama cambios», marcando la primera celda de cada dato con un carácter de ancho cero (`CHIP_DIR`/`CHIP_BRANCH`/`CHIP_CHANGES`). Por estar en la rejilla, las cajas persisten en el historial con el valor que tenían al usar ese prompt; el prompt actual se resella cada cuadro (el shell puede repintarlo). Al sellar se borra lo que el shell ponga a la derecha de esa línea (el reloj). Las cajas del prompt actual son botones que abren `Overlay::Picker` (`App::open_picker`): la de la rama lista las ramas locales (`git for-each-ref`) y al elegir una escribe y ejecuta `git checkout <rama>` en el shell; la de la carpeta lista `..`, carpetas y archivos, y ejecuta `cd` (un archivo solo se pega en el prompt). Bajo cada línea de cajas tampoco se pinta el símbolo del prompt («❯ »): esa fila se dibuja corrida a la izquierda (`Session::row_shifts`, que `grid_point` deshace para el ratón) y, en el prompt, el cursor va del color de acento (la forma y el parpadeo salen de las preferencias `cursor` y `cursor_blink`; el parpadeo repinta cada 500 ms solo con la ventana al frente). `view::show` no pinta como texto las filas marcadas (`Session::chip_rows`) y `prompt_chips` (`main.rs`) dibuja las cajas. Si el prompt no tiene esa forma se ve tal cual lo pinta el shell. `git::Watch` solo vuelve a correr `git status` cuando cambia `Session::activity` (hubo salida en la pestaña) y como mucho cada 3 s; no sondea en reposo.
- **Fuente Hack**: va dentro del ejecutable (`assets/fonts`, rutas `builtin:hack` / `builtin:hack-bold` que resuelve `config::font_bytes`).
- **Arrastre de pestañas y barra de título**: la ventana usa contenido a ventana completa con barra de título transparente, así que macOS sigue tratando la franja superior como barra de título y se queda con los arrastres (mueve la ventana) antes de que lleguen a egui. `tab_bar` llama a `blur::set_movable(false)` mientras el puntero está sobre las pestañas. El puntero simulado del guion no pasa por macOS: no reproduce este tipo de fallo.
- **Detener procesos**: primer clic arma una confirmación (`CONFIRM_WINDOW`, 4 s); si el proceso sigue vivo 5 s después de pedírselo pasa a `stubborn` y el siguiente intento lo fuerza.
