# Pestañas cerradas que no mueren y «AutoFill (Colibrí)»

## Por qué

Análisis de rendimiento del 2026-11-24 (medido sobre la instancia real y en instancias de
prueba aisladas):

- `Session::new` guarda una copia del maestro del PTY (para `tcgetpgrp` y `tcgetattr`) hecha
  con `libc::dup`, que no lleva `FD_CLOEXEC`. Todo lo que se lanza después la hereda: los
  shells de las pestañas siguientes, sus agentes, los procesos de p10k. Al cerrar una pestaña
  más vieja que otra abierta, su terminal no se cuelga: el shell se queda saliendo (`?Es`,
  esperando a que se vacíe la salida), el agente sigue vivo bloqueado escribiendo y el hilo
  del PTY se queda en `wait4` para siempre tras el SIGKILL de `Pty::drop`, con el `Term` de
  la pestaña. En la instancia real: 3 shells, 3 hilos y 2 `claude` de 384 y 342 MB. Se
  reprodujo aislado; cerrar la pestaña más nueva (que nadie hereda) no falla.
- AppKit lanza `com.apple.SafariPlatformSupport.Helper` («AutoFill (Colibrí)» en el Monitor
  de Actividad, 18 MB y 5 hilos) desde `NSAutoFillHeuristicController` en cuanto se activa la
  entrada de texto. Con el valor `_NSEnableTextInputContextBasedAutoFill = NO` no se lanza
  (probado como argumento de la línea de órdenes).

## Objetivos

1. La copia del maestro se hace con `F_DUPFD_CLOEXEC`: ningún proceso lanzado después la
   hereda, así que cerrar una pestaña cuelga su terminal aunque haya otras abiertas.
2. `Pty::drop` tampoco espera sin plazo después del SIGKILL. Si el shell no termina (está
   saliendo y otro proceso retiene el maestro), lo recoge un hilo aparte cuando acabe y el
   hilo del PTY termina, soltando el `Term` de la pestaña.
3. `main` pone `_NSEnableTextInputContextBasedAutoFill` a falso: no se lanza el helper.

## No objetivos

- Cerrar los descriptores de la caché de Metal que también heredan los shells: son archivos
  y no retienen nada.
- La carrera de `rustix-openpty`, que marca `CLOEXEC` después de `openpty` (no es atómico):
  la ventana es de microsegundos y, si alguien hereda ahí el maestro, el objetivo 2 evita
  que deje un hilo colgado.
- Terminar los procesos huérfanos de la instancia que ya está abierta.
- Mandar SIGHUP al grupo en primer plano: con el maestro cerrado de verdad, el kernel cuelga
  la terminal y lo hace él.

## Anti-objetivos

- Que cerrar una pestaña bloquee la interfaz: todo sigue en el hilo del PTY.
- Dejar zombis: lo que no termina en plazo lo recoge el hilo aparte.
- Tocar el menú Edición: `_NSAutoFillOrSystemInsertMenuEnabled` no se cambia.

## Enfoques

- Copia del maestro: **`fcntl(fd, F_DUPFD_CLOEXEC, 0)`** (elegido, una función en `proc.rs`
  con su prueba). `File::try_clone` hace lo mismo por dentro, pero cambia el tipo del campo y
  el `Drop` de `Session`. Usar el descriptor del `Pty` sin copia no vale: el `Pty` vive en el
  hilo del PTY y se suelta cuando él quiere.
- Shell que no termina tras SIGKILL: **un hilo que lo recoge con `waitpid`** (elegido) frente
  a dejarlo sin recoger (quedaría zombi hasta salir de Colibrí) o esperar sin plazo (lo de hoy).
- AutoFill: **`registerDefaults:`** al principio de `main`, junto a `ApplePressAndHoldEnabled`
  (elegido). A diferencia de `setBool:forKey:`, no se guarda en las preferencias de la app: si
  un día se quita, nadie se queda con el valor puesto, y `defaults write` lo puede anular. Es
  un valor privado: si desaparece en otra versión de macOS, no tiene efecto.

## Verificación

- `proc::dup_cloexec`: la copia lleva `FD_CLOEXEC` y un proceso lanzado después no la tiene
  (falla con `libc::dup`).
- Prueba nueva en `vendor/alacritty_terminal` (`colibri_tests`): soltar una terminal cuyo
  maestro tiene abierto otro proceso termina en plazo (hoy se queda esperando).
- Repetir el guion de reproducción (agente que ignora SIGHUP, segunda pestaña, cerrar la
  primera): sin agente vivo, sin `/dev/ptmx` en el shell de la segunda, 0 hilos en `wait4`.
- Instancia de prueba sin argumentos: `launchctl print pid/<pid>` sin
  `SafariPlatformSupport.Helper`.
- `cargo fmt`, `cargo clippy --all-targets`, `cargo test` y `cargo test --lib` en
  `vendor/alacritty_terminal`.
- AGENTS.md al día: «Reabrir lo cerrado» y «Memoria».
