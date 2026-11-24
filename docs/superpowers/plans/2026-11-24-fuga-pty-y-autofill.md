# Pestañas cerradas que no mueren y «AutoFill (Colibrí)» — plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cerrar una pestaña termina su shell y su agente aunque haya otras abiertas, ningún hilo se queda esperando a un shell que no termina, y AppKit no lanza «AutoFill (Colibrí)».

**Architecture:** La copia del maestro que guarda `Session` pasa a hacerse con `F_DUPFD_CLOEXEC` (función en `proc.rs`). `Pty::drop` (vendor, marcado «Colibrí:») espera con plazo también tras el SIGKILL y, si el shell no termina, deja a un hilo aparte recogiéndolo. `main` registra `_NSEnableTextInputContextBasedAutoFill = NO` con `registerDefaults:`.

**Tech Stack:** Rust 2024, libc, objc2-foundation 0.3, alacritty_terminal parcheado en `vendor/`.

**Spec:** `docs/superpowers/specs/2026-11-24-fuga-pty-y-autofill-design.md`

## Global Constraints

- Código, comentarios y mensajes de commit en español; commits `fix: …` / `perf: …` con el trailer `Co-Authored-By` del agente.
- Todo cambio en `vendor/alacritty_terminal` lleva un comentario «Colibrí:».
- Al ejecutar la app para probar: `COLIBRI_CONFIG` temporal, `HISTFILE=/dev/null`, shell de prueba `zsh -f`; nada en `~/.config/colibri` ni en el historial real.
- Sin hilos que despierten en reposo: el hilo que recoge al shell solo existe en el caso raro y está bloqueado en `waitpid`.
- Rama: `fix/fuga-pty-y-autofill`. Sin push ni PR hasta que el usuario lo pida.

## Review Focus

- Cerrar una ventana con varias pestañas a la vez: cada `Pty` se suelta en su hilo; ninguno debe retener a los demás ni la interfaz.
- `fcntl` falla (sin descriptores libres): `master_fd` queda en -1 como antes con `dup`; `foreground_pid` y `password_mode` ya lo toleran y `Session::drop` no cierra nada.
- Mover una pestaña viva a otra ventana: la copia del maestro va con la `Session`, sin cambios.
- La app empaquetada (`target/Colibri.app`) lee el valor de AutoFill igual que el binario suelto: se comprueba con el bundle en la tarea 3.
- Menú Edición → Autorrelleno: sigue existiendo (no se toca `_NSAutoFillOrSystemInsertMenuEnabled`); si alguien lo usa a mano puede lanzar el helper, y es correcto.

---

### Task 1: la copia del maestro no se hereda

**Files:**
- Modify: `src/proc.rs` (función nueva junto a `foreground_pid`; prueba en `mod tests`)
- Modify: `src/session.rs:850` (usarla en `Session::new`)
- Add: este plan y su spec al commit

**Interfaces:**
- Produces: `pub fn dup_cloexec(fd: c_int) -> c_int` — copia con `fcntl(fd, F_DUPFD_CLOEXEC, 0)`; -1 si falla.

- [ ] **Step 1: Prueba**

```rust
#[test]
fn copia_del_maestro_no_se_hereda() {
    use std::os::unix::io::AsRawFd;
    let file = std::fs::File::open("/dev/null").unwrap();
    let fd = dup_cloexec(file.as_raw_fd());
    assert!(fd >= 0);
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    let heredado = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(format!("[ -e /dev/fd/{fd} ]"))
        .status()
        .unwrap()
        .success();
    unsafe { libc::close(fd) };
    assert_ne!(flags & libc::FD_CLOEXEC, 0, "la copia no lleva FD_CLOEXEC");
    assert!(!heredado, "un proceso lanzado después heredó la copia");
}
```

- [ ] **Step 2:** `cargo test copia_del_maestro_no_se_hereda` → no compila (no existe `dup_cloexec`).
- [ ] **Step 3:** Escribir `dup_cloexec` primero con `libc::dup` y comprobar que la prueba falla por `heredado` (demuestra que detecta el fallo); después cambiarla a `F_DUPFD_CLOEXEC` con su comentario (por qué: lo que se lanza después heredaría el maestro y cerrar la pestaña no colgaría su terminal).
- [ ] **Step 4:** `cargo test copia_del_maestro_no_se_hereda` → pasa.
- [ ] **Step 5:** En `Session::new`, `let master_fd = proc::dup_cloexec(pty.file().as_raw_fd());`.
- [ ] **Step 6: Reproducción de punta a punta** (`cargo build --release`). Config temporal con `shell = "<tmp>/testsh"` (`testsh`: `exec /bin/zsh -f -i` con `HISTFILE=/dev/null`), agente falso `fake-agent.sh` (`trap '' HUP; exec yes COLIBRI_FUGA_TEST`) y este guion (generado con `printf` o heredoc):

```text
wait 1500
type <tmp>/fake-agent.sh\n
wait 1200
tab
wait 1500
select 0
wait 400
menu Cerrar pestaña
wait 4500
type { ps -axo pid,stat,command | grep COLIBRI_FUGA | grep -v grep; echo ---; lsof -p $$ | grep -c ptmx; sample $PPID 1 2>/dev/null | grep -c __wait4; } > <tmp>/tras-cerrar.txt 2>&1\n
wait 3500
quit
```

Esperado en `tras-cerrar.txt`: ninguna línea con `COLIBRI_FUGA`, `0` descriptores `ptmx` y `0` hilos en `__wait4`. (Antes del cambio: `yes` vivo, `1` y `2`.)

- [ ] **Step 7: Commit**

```bash
git add src/proc.rs src/session.rs docs/superpowers/specs/2026-11-24-fuga-pty-y-autofill-design.md docs/superpowers/plans/2026-11-24-fuga-pty-y-autofill.md
git commit -m "fix: los procesos de otras pestañas ya no heredan el PTY de las anteriores"
```

### Task 2: soltar una terminal no espera sin plazo

**Files:**
- Modify: `vendor/alacritty_terminal/src/tty/unix.rs` (`impl Drop for Pty`, constante nueva junto a `SHELL_EXIT_WAIT`, prueba en `mod colibri_tests`)
- Modify: `AGENTS.md` («Reabrir lo cerrado»: la copia con `CLOEXEC`, el plazo tras el SIGKILL y el hilo que recoge, con lo medido)

**Interfaces:**
- Produces: `const KILL_WAIT: Duration = Duration::from_secs(1);` — lo que se espera tras el SIGKILL antes de dejar al shell a un hilo `PTY reaper` (`libc::waitpid(pid, null_mut(), 0)`). Un único bucle de espera con plazo para los dos tramos (función privada `wait_until(child: &mut Child, plazo: Duration) -> bool`, cierto si terminó).

- [ ] **Step 1: Prueba** (usa los ayudantes `abrir`, `soltar`, `vivo` y `esperar` del módulo)

```rust
/// Si otro proceso conserva abierto el maestro (lo heredó), cerrarlo aquí no cuelga la
/// terminal: el shell se queda saliendo y no termina ni con SIGKILL. Soltar no debe esperarlo
/// sin plazo, y cuando por fin termine no debe quedar como zombi.
#[test]
fn colibri_soltar_con_el_maestro_abierto_en_otro_proceso() {
    let pty = abrir("/bin/zsh", &["-f", "-i", "-c", "sh -c 'trap \"\" HUP; exec yes'; :"]);
    let shell = pty.child().id() as i32;
    let maestro = pty.file().as_raw_fd();
    let mut grupo = 0;
    assert!(esperar(Duration::from_secs(5), || {
        grupo = unsafe { libc::tcgetpgrp(maestro) };
        grupo > 0 && grupo != shell
    }));
    thread::sleep(Duration::from_millis(300));
    // Otro proceso se queda con una copia del maestro, como los shells de otras pestañas.
    // La copia se hace en el hijo (`dup2` sin `CLOEXEC`): este proceso nunca tiene una que
    // pudieran heredar las pruebas que corren a la vez.
    let mut otro = unsafe {
        std::process::Command::new("/bin/sleep")
            .arg("30")
            .pre_exec(move || {
                libc::dup2(maestro, 10);
                Ok(())
            })
            .spawn()
            .unwrap()
    };

    let tardo = soltar(pty, Duration::from_secs(8));
    let _ = otro.kill();
    let _ = otro.wait();
    let termino = esperar(Duration::from_secs(5), || !vivo(-grupo) && !vivo(shell));
    if !termino {
        unsafe { libc::killpg(grupo, libc::SIGKILL) };
    }

    assert!(tardo.is_some(), "soltar la terminal esperó sin plazo al shell");
    assert!(termino, "al cerrarse el maestro quedó algo vivo o un zombi");
}
```

- [ ] **Step 2:** `cargo test --lib colibri_soltar_con_el_maestro` (desde `vendor/alacritty_terminal`) → falla por `tardo` (hoy `Pty::drop` se queda en `wait`).
- [ ] **Step 3:** En `Pty::drop`, tras `child.kill()`: `wait_until(KILL_WAIT)` y, si no terminó, el hilo `PTY reaper` con `waitpid`; el primer tramo pasa a usar también `wait_until`. Comentario «Colibrí:» con el porqué (un proceso que sale no recibe señales; con el maestro retenido, `wait` no volvería y el hilo retendría el `Term`).
- [ ] **Step 4:** `cargo test --lib colibri_` (desde `vendor/alacritty_terminal`) → pasan las tres.
- [ ] **Step 5:** AGENTS.md, «Reabrir lo cerrado».
- [ ] **Step 6: Commit**

```bash
git add vendor/alacritty_terminal/src/tty/unix.rs AGENTS.md
git commit -m "fix: soltar una terminal no espera sin plazo a un shell que no termina"
```

### Task 3: sin «AutoFill (Colibrí)»

**Files:**
- Modify: `src/main.rs` (al principio de `main`, junto a `ApplePressAndHoldEnabled`)
- Modify: `AGENTS.md` («Memoria»: el helper, el valor y lo medido)

**Interfaces:**
- Consumes: `objc2_foundation::{NSUserDefaults, NSDictionary, NSNumber, NSString, ns_string}`.

- [ ] **Step 1:** Comprobar que hoy se lanza: instancia de prueba (`cargo build --release`) con guion `wait 2500` / `type x\n` / `wait 2500` / `type launchctl print pid/$PPID | grep -c SafariPlatformSupport.Helper > <tmp>/autofill.txt\n` / `wait 2000` / `quit` → `autofill.txt` ≥ 1.
- [ ] **Step 2:** `registerDefaults:` con `{"_NSEnableTextInputContextBasedAutoFill": NO}` y comentario (qué lanza AppKit, cuánto ocupa, que es un valor privado y que no se guarda).
- [ ] **Step 3:** Mismo guion con el binario nuevo → `autofill.txt` = `0`. Repetir con `target/Colibri.app/Contents/MacOS/colibri` tras `scripts/bundle.sh` → `0`.
- [ ] **Step 4:** AGENTS.md, «Memoria».
- [ ] **Step 5: Comprobaciones finales:** `cargo fmt`, `cargo clippy --all-targets` (sin avisos nuevos), `cargo test`, `cargo test --lib` en `vendor/alacritty_terminal`.
- [ ] **Step 6: Commit**

```bash
git add src/main.rs AGENTS.md
git commit -m "perf: sin el proceso «AutoFill (Colibrí)» de AppKit (18 MB)"
```
