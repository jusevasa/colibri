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
python3 scripts/icon.py assets # regenera assets/colibri.svg y colibri-1024.png
```

- `scripts/bundle.sh` firma con una identidad fija para que macOS recuerde los permisos entre compilaciones; se cambia con `COLIBRI_SIGN_ID`. Si falla, cae a firma ad-hoc.
- Las notificaciones nativas (con clic que lleva a la pestaña) solo funcionan desde `Colibri.app`; con `cargo run` se usa `osascript` como respaldo sin clic. Para probar notificaciones hay que pasar por `bundle.sh`.

## Probar la interfaz sin tocarla a mano

No hay pruebas de interfaz; en su lugar existe un guion de depuración (`src/debug.rs`) que maneja la app y guarda capturas de su propio render:

```sh
COLIBRI_SCRIPT=pasos.txt COLIBRI_CONFIG=/ruta/tmp/config.toml cargo run
```

Un paso por línea: `type <texto>` (`\n` = Enter), `wait <ms>`, `tab`, `select <n>`, `move <de> <a>`, `name <nombre>`, `shot <ruta.bmp>`, `procs`, `ports`, `hover`, `killport <puerto>`, `open palette|prefs|history`, `key up|down|enter|esc` (pulsa la tecla como si viniera del teclado), `press|moveto|release <x> <y>` (puntero simulado, en puntos de la ventana), `pref <id> <+1|-1>`, `update` (fuerza el botón «Actualizar»), `quit`. Las líneas que no se reconocen se ignoran en silencio. Los `id` de `pref` son los de `PREF_ROWS` en `src/overlay.rs`.

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
- **`state.rs`** — pestañas abiertas persistidas en `session.toml` (carpeta, nombre y agente; no los procesos).
- **`proc.rs`** — consultas puntuales por pid vía libc/libproc (cwd, argv, padre, nombre). **`procs.rs`** y **`ports.rs`** — hilos de fondo que ejecutan `ps`/`vm_stat` y `lsof` y publican en un `Arc<Mutex<…>>`.
- **`notify.rs`**, **`blur.rs`** — FFI de macOS (UNUserNotificationCenter; API privada de CoreGraphics para el desenfoque).

### Cosas que no se ven leyendo un solo archivo

- **Una acción nueva** toca varios sitios: variante en `Cmd`, rama en `App::run`, campo en `config::Keys` (con su valor por defecto) y fila en la tabla de `Keys::new` en `main.rs`; además la lista de la paleta en `overlay.rs`. `cmd+1..9` es fijo y no pasa por la config.
- **Una preferencia nueva**: campo en `Config` + `Default`, fila en `PREF_ROWS` y ramas en el valor mostrado y en `pref_change` (`overlay.rs`). Todos los structs de config llevan `#[serde(default)]`, de modo que los archivos antiguos siguen cargando.
- **Detección de agente y de «trabajando»** (`session.rs`): el agente sale del nombre del proceso en primer plano y, si no basta, de `argv` (Claude Code se llama como su número de versión). `busy` usa primero el título de la ventana (spinner braille = trabajando, `✳` = esperando) y, si el agente no lo anuncia, una heurística de ráfagas de salida que descarta el eco de lo que tecleas. Sin agente, `busy` = hay un proceso en primer plano que no es el shell. Una tarea solo cuenta como «terminada» (punto verde, notificación, `⌘J`) si duró ≥ 2 s con agente o ≥ 8 s sin él.
- **Repintado bajo demanda**: en reposo la app no repinta. Lo que cambie fuera del hilo de la interfaz debe llamar a `ctx.request_repaint()` (así lo hacen `Listener`, `Ports` y `Monitor`), y los hilos de fondo solo trabajan mientras su apartado está visible (`enabled`). No añadas repintados continuos.
- **Config en caliente**: `logic()` mira el mtime de `config.toml` cada segundo y recarga. `commit_cfg` actualiza `cfg_mtime` tras guardar para no recargar su propia escritura.
- **Restaurar pestañas**: donde corría Claude Code se deja escrito `claude --continue` sin ejecutarlo; es intencional.
- **Privacidad** como requisito de diseño: la app no abre conexiones de red propias (la única sonda es HTTP a `127.0.0.1` para saber si un puerto es web) y `ClipboardLoad` se ignora a propósito para que ningún programa lea el portapapeles.
- **Botón «Actualizar»**: aparece en la barra de pestañas cuando el mtime del ejecutable en disco cambia respecto al del arranque (tras `cargo build` o `scripts/bundle.sh`). `App::restart` guarda las pestañas, lanza la versión nueva (`open -n` sobre el bundle, o el binario suelto) y sale; también está en la paleta como «Reiniciar Colibrí…».
- **Historial con `↑`**: `App::history_key` intercepta `↑` solo si `Session::at_prompt()` (el shell en primer plano, sin pantalla alterna ni modo ratón) y abre `Overlay::History`; `Enter` llama a `Session::insert_command` (`ctrl+u` + pegado, sin ejecutar). `Session::track` reconstruye lo tecleado en el prompt para ofrecerlo aunque zsh no lo haya escrito aún en su archivo; lo que se escribe a otros programas no se registra. Al probar, apunta `HISTFILE` a un archivo falso para no leer el historial real.
- **Arrastre de pestañas y barra de título**: la ventana usa contenido a ventana completa con barra de título transparente, así que macOS sigue tratando la franja superior como barra de título y se queda con los arrastres (mueve la ventana) antes de que lleguen a egui. `tab_bar` llama a `blur::set_movable(false)` mientras el puntero está sobre las pestañas. El puntero simulado del guion no pasa por macOS: no reproduce este tipo de fallo.
- **Detener procesos**: primer clic arma una confirmación (`CONFIRM_WINDOW`, 4 s); si el proceso sigue vivo 5 s después de pedírselo pasa a `stubborn` y el siguiente intento lo fuerza.
