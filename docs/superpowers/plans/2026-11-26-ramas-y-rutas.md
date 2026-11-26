# Ramas de git con Tab y rutas de color — plan de implementación

> **Para agentes:** SUB-SKILL OBLIGATORIO: usar superpowers:subagent-driven-development
> (recomendado) o superpowers:executing-plans para ejecutar este plan tarea a tarea. Los pasos
> usan casillas (`- [ ]`) para seguir el avance.

**Objetivo:** que Tab ofrezca ramas de git (con alias del shell y de git) y un panel de
detalles junto a la lista de sugerencias; que Colibrí pinte de cian las rutas que existen
en el comando de cada prompt marcado, y que un clic normal abra los enlaces web (con un
recuadro con la dirección al pasar el puntero).

**Arquitectura:** la parte pura (qué pide cada palabra, qué ramas ofrecer, qué palabras son
rutas) va en `complete.rs`, `git.rs` y `session.rs` con sus pruebas. La integración de zsh
escribe dos archivos más (alias y rutas de órdenes) junto a `commands`. La lista
`Overlay::Complete` guarda lo leído de git mientras está abierta. Las rutas se pintan al
dibujar, como los tramos de ssh (`Tok`), y solo se rehacen las filas cuyos tramos cambian.

**Tecnología:** Rust 2024, egui, alacritty_terminal (vendor), zsh.

**Spec:** `docs/superpowers/specs/2026-11-26-ramas-y-rutas-design.md`

## Restricciones globales

- Código, comentarios, documentación y commits en español. Los textos de la interfaz, en
  español e inglés en `src/i18n/overlay.rs` (si falta uno, no compila).
- Sin red: solo `git for-each-ref` y `git config --get-regexp` locales, con
  `--no-optional-locks`, `-C <carpeta>`, stdin y stderr a null. Nada de `fetch`/`ls-remote`.
- Sin repintados continuos, temporizadores ni hilos nuevos. El color de las rutas se calcula
  al pintar.
- Git se consulta solo al pulsar Tab, una vez por lista abierta; al seguir escribiendo se
  filtra en memoria.
- Al ejecutar la app para probar: `COLIBRI_CONFIG` y `COLIBRI_STATE` temporales, `HISTFILE`
  falso y `COLIBRI_LANG=es`. Nada en `~/.config/colibri`, `~/.claude` ni el historial real.
- Color de las rutas: `pal.ansi[6]`, sin el subrayado que pone `zsh-syntax-highlighting`.
- Archivos de la integración: `~/Library/Caches/colibri/zsh/aliases` y `…/zsh/paths`, pares
  nombre/valor separados por NUL (`print -rN`).
- Toda afirmación de rendimiento dice cómo se midió (`COLIBRI_FPS`, columna de la rejilla).
- Antes de cerrar cada tarea: `cargo fmt`, `cargo clippy --all-targets` sin avisos nuevos y
  `cargo test`. No se toca `vendor/`.
- Se trabaja en la rama `feat/ramas-y-rutas` (se crea desde `main` antes de la tarea 1).
  Commits con el skill `commit:commit`, al estilo Conventional Commits en español. El primero
  lleva también el spec y este plan.
- Warp (AGPL) solo como referencia de técnica: no se copia código.

## Puntos de revisión

Los cinco casos que el spec implica y es más fácil que fallen. Cada uno tiene su prueba en la
tarea que lo hace:

1. **Sugerencia gris de zsh-autosuggestions detrás del cursor**: no se pinta como ruta
   (tarea 5, `palabras_de_ruta_hasta_el_cursor`).
2. **Comando que ocupa varias filas** (se parte con `WRAPLINE`): el tramo de una ruta partida
   se pinta en las dos filas (tarea 6, `tramos_partidos_por_filas`).
3. **Fuera de un repositorio, o sin git instalado**: Tab vuelve a ofrecer archivos y no
   falla (tarea 2, `oferta_sin_repo_da_archivos`).
4. **Ramas con `/` o con caracteres especiales** (`feat/x`, `fix(ui)`): se filtran enteras y
   se escriben escapadas (tarea 2, `oferta_rama_con_barra` y `reemplazo_de_rama_escapado`).
5. **Rutas relativas de un prompt del historial**: se resuelven contra la carpeta de su
   marca, no contra la de la pestaña; una carpeta borrada da «no existe», sin fallar
   (tarea 5, `existe_relativa_a_la_carpeta_dada`).
6. **Arrastrar para seleccionar empezando en un enlace, dentro de Claude Code**: el programa
   recibe la pulsación aplazada y el arrastre, y no se abre nada (tarea 7,
   `pulsacion_aplazada_sobre_un_enlace` y el paso del guion).

---

### Tarea 1: Contexto de git y alias del shell

**Archivos:**
- Modificar: `assets/shell/zshenv.zsh` (bloque `if [[ $PATH != ${_colibri_path-} ]]` de
  `_colibri_precmd`)
- Modificar: `src/complete.rs` (funciones nuevas y pruebas)

**Interfaces:**
- Produce (en `complete.rs`):
  - `pub enum GitArg { Branch { remotes: bool }, Shell, Files }` (`Clone, Copy, PartialEq, Debug`)
  - `pub fn last_command(line: &str) -> &str`: lo que sigue al último `|`, `||`, `&&` o `;`
    (sin recortar espacios por la derecha).
  - `pub fn expand_alias(command: &str, alias: &dyn Fn(&str) -> Option<String>) -> String`
  - `pub fn git_arg(line: &str, shell_alias: &dyn Fn(&str) -> Option<String>, git_alias: &mut dyn FnMut(&str) -> Option<String>) -> Option<GitArg>`
  - `pub fn shell_aliases() -> Arc<HashMap<String, String>>` y
    `pub fn command_paths() -> Arc<HashMap<String, String>>`: los dos archivos, leídos con
    la misma caché por mtime que `commands()` (generalizar esa caché en una función común).
  - `fn parse_pairs(bytes: &[u8]) -> HashMap<String, String>`

- [ ] **Paso 1: Pruebas que fallan** (en `complete.rs`, `mod tests`)

```rust
#[test]
fn contexto_de_git() {
    let sh = |n: &str| (n == "gco").then(|| "git checkout".to_string());
    let mut git = |n: &str| match n {
        "co" => Some("checkout".to_string()),
        "sync" => Some("!git pull".to_string()),
        _ => None,
    };
    let b = |remotes| Some(GitArg::Branch { remotes });
    assert_eq!(git_arg("gco f", &sh, &mut git), b(true));
    assert_eq!(git_arg("git checkout ", &sh, &mut git), b(true));
    assert_eq!(git_arg("git switch -c ", &sh, &mut git), Some(GitArg::Shell));
    assert_eq!(git_arg("git checkout -b nueva ", &sh, &mut git), b(true)); // punto de partida
    assert_eq!(git_arg("git merge ma", &sh, &mut git), b(false));
    assert_eq!(git_arg("git branch -D fe", &sh, &mut git), b(false));
    assert_eq!(git_arg("git branch ", &sh, &mut git), Some(GitArg::Shell));
    assert_eq!(git_arg("git branch nueva ", &sh, &mut git), b(false));
    assert_eq!(git_arg("git push ", &sh, &mut git), Some(GitArg::Shell));
    assert_eq!(git_arg("git push origin fe", &sh, &mut git), b(false));
    assert_eq!(git_arg("git checkout -- src/ma", &sh, &mut git), Some(GitArg::Files));
    assert_eq!(git_arg("git co fe", &sh, &mut git), b(true));
    assert_eq!(git_arg("git -C ../otro checkout f", &sh, &mut git), b(true));
    assert_eq!(git_arg("make && gco f", &sh, &mut git), b(true));
    assert_eq!(git_arg("git sync fe", &sh, &mut git), None); // alias con «!»
    assert_eq!(git_arg("git ch", &sh, &mut git), None); // se escribe la suborden
    assert_eq!(git_arg("git status ", &sh, &mut git), None);
    assert_eq!(git_arg("ls f", &sh, &mut git), None);
}

#[test]
fn alias_que_empieza_por_si_mismo() {
    let a = |n: &str| (n == "ls").then(|| "ls -G".to_string());
    assert_eq!(expand_alias("ls x", &a), "ls -G x");
    assert_eq!(last_command("make && gco f"), " gco f");
}

#[test]
fn pares_separados_por_nul() {
    let m = parse_pairs(b"gco\0git checkout\0ll\0ls -l\0");
    assert_eq!(m.get("gco").map(String::as_str), Some("git checkout"));
    assert_eq!(m.len(), 2);
    assert!(parse_pairs(b"suelto\0").is_empty());
}
```

- [ ] **Paso 2:** `cargo test contexto_de_git alias_que_empieza pares_separados` → no compila
  (no existen las funciones).

- [ ] **Paso 3: Implementar.** Reglas de `git_arg`:
  - Palabras separadas por espacios sin escapar. La última es la que se completa, y puede
    ser vacía si la línea acaba en espacio.
  - Opciones globales antes de la suborden: `-C` y `-c` se comen la palabra siguiente; las
    demás que empiezan por `-` se saltan.
  - Una suborden desconocida se expande una vez con `git_alias`. Si el alias empieza por `!`,
    devuelve `None`.
  - Argumentos ya escritos (`args`, sin la última palabra) y posicionales (los que no
    empiezan por `-`):
    - `--` en `args` → `Files`.
    - `checkout`/`switch`: si el último arg es `-b`/`-B`/`-c`/`-C`/`--orphan` → `Shell`; si
      no, `Branch{remotes: true}`.
    - `merge`, `rebase`, `diff`, `log`, `show`, `cherry-pick`, `reset` → `Branch{remotes: false}`.
    - `branch`: con `-d`/`-D`/`-m`/`-M`/`--delete`/`--move` → `Branch{false}`; si no, sin
      posicionales → `Shell` y con uno o más → `Branch{false}`.
    - `push`/`pull`/`fetch`: sin posicionales → `Shell`; con alguno → `Branch{false}`.
    - Otra suborden → `None`.
  - `expand_alias` para si la expansión empieza por el mismo nombre, y como mucho 3 veces.

- [ ] **Paso 4: Integración de zsh.** En el mismo bloque que escribe `commands`:
  `print -rN -- ${(kv)aliases} >| …/zsh/aliases` y `print -rN -- ${(kv)commands} >| …/zsh/paths`,
  con `2>/dev/null` como la línea de `commands`. Comprobar con `zsh -n assets/shell/zshenv.zsh`
  (sin salida).

- [ ] **Paso 5:** `cargo test` → todas pasan. `cargo fmt`, `cargo clippy --all-targets`.

- [ ] **Paso 6: Commit** (`commit:commit`): `feat: contexto de git y alias del shell para las sugerencias`
  (incluye spec y plan).

### Tarea 2: Ramas como sugerencias (modelo)

**Archivos:**
- Modificar: `src/git.rs` (`Ref`, `refs`, `parse_refs`, `aliases`, `parse_aliases`)
- Modificar: `src/complete.rs` (`Kind`, `Item`, `Target::whole`, `branch_items`, `GitData`, `offer`)
- Modificar: los usos de `Item::is_dir`/`Item::command` en `src/overlay.rs` y `src/main.rs`
  pasan a `kind` (sin cambio de comportamiento)

**Interfaces:**
- Consume: `git_arg`, `last_command`, `shell_aliases` (tarea 1).
- Produce:
  - `git::Ref { pub name: String, pub remote: bool, pub time: u64, pub subject: String }`
    (`Clone, PartialEq, Debug`). En las remotas, `name` = `origin/x`.
  - `git::refs(dir: &Path) -> Vec<Ref>`: `for-each-ref --sort=-committerdate
    --format=%(refname)%00%(committerdate:unix)%00%(subject) refs/heads refs/remotes`.
  - `git::parse_refs(out: &str) -> Vec<Ref>`: quita `refs/heads/` o `refs/remotes/` y salta
    las que acaban en `/HEAD`.
  - `git::aliases(dir: &Path) -> Vec<(String, String)>`: `config --get-regexp ^alias\.`.
    Con `git::parse_aliases(out: &str)` (línea `alias.co checkout` → `("co", "checkout")`).
  - `complete::Kind { Dir, File, Command, Branch, CurrentBranch, RemoteBranch }`
    (`Clone, Copy, PartialEq, Eq, Debug`).
  - `complete::Item { pub name: String, pub kind: Kind, pub commit: Option<(u64, String)> }`
    con `fn is_dir(&self) -> bool` y `fn is_branch(&self) -> bool`. `replacement` añade `/`
    a las carpetas y espacio a las órdenes y las ramas.
  - `Target::whole(self) -> Target`: `typed_dir` vacío y `prefix` = la palabra entera sin
    escapes.
  - `complete::branch_items(refs: &[git::Ref], current: Option<&str>, remotes: bool, prefix: &str) -> Vec<Item>`
  - `complete::GitData { pub refs: Option<Vec<git::Ref>>, pub current: Option<String>, pub aliases: Option<Vec<(String, String)>> }`
    (`Clone, Default, Debug`), con métodos que cargan con git la primera vez que hacen
    falta y luego reutilizan.
  - `complete::offer(line: &str, cwd: &Path, shell_alias: &dyn Fn(&str) -> Option<String>, git: &mut GitData) -> Option<(Target, Vec<Item>)>`.
    `None` o lista vacía = el Tab es del shell.

- [ ] **Paso 1: Pruebas que fallan**

```rust
// git.rs
#[test]
fn ramas_de_for_each_ref() {
    let out = "refs/heads/main\x00300\x00inicio\nrefs/remotes/origin/HEAD\x00300\x00inicio\n\
               refs/remotes/origin/feat-c\x00250\x00trabajo en c\n";
    let r = parse_refs(out);
    assert_eq!(r.len(), 2);
    assert_eq!((r[0].name.as_str(), r[0].remote), ("main", false));
    assert_eq!((r[1].name.as_str(), r[1].remote, r[1].time), ("origin/feat-c", true, 250));
    assert_eq!(parse_aliases("alias.co checkout\nalias.st status -sb\n")[1],
               ("st".to_string(), "status -sb".to_string()));
}

// complete.rs
fn refs() -> Vec<git::Ref> { /* main 300, feat-a 200, feat-b 100 (locales);
   origin/feat-a 200, origin/feat-c 250, upstream/feat-c 240 (remotas); subject "msg <nombre>" */ }

#[test]
fn ramas_filtradas_y_ordenadas() {
    let names = |v: Vec<Item>| v.into_iter().map(|i| (i.name, i.kind)).collect::<Vec<_>>();
    assert_eq!(names(branch_items(&refs(), Some("feat-a"), true, "F")), [
        ("feat-a".into(), Kind::CurrentBranch),
        ("feat-c".into(), Kind::RemoteBranch),
        ("feat-b".into(), Kind::Branch),
    ]);
    // «a» no empieza ninguna: todas las que la contienen, la actual primero y luego por fecha.
    let n: Vec<String> = branch_items(&refs(), Some("feat-a"), false, "a")
        .into_iter().map(|i| i.name).collect();
    assert_eq!(n, ["feat-a", "main", "feat-b"]);
    let c = branch_items(&refs(), None, true, "feat-c");
    assert_eq!(c.len(), 1); // dos remotos con la misma rama: una sola
    assert_eq!(c[0].commit, Some((250, "msg origin/feat-c".into())));
}

#[test]
fn oferta_de_ramas_y_vuelta_a_archivos() {
    // carpeta temporal con «zz.txt»; GitData ya cargado (sin llamar a git)
    let mut g = GitData { refs: Some(refs()), current: Some("main".into()), aliases: Some(vec![]) };
    let none = |_: &str| None;
    let (t, items) = offer("git checkout fe", &dir, &none, &mut g).unwrap();
    assert_eq!((t.token.as_str(), t.typed_dir.as_str()), ("fe", ""));
    assert!(items.iter().all(Item::is_branch));
    let (_, items) = offer("git checkout zz", &dir, &none, &mut g).unwrap();
    assert_eq!(items[0].name, "zz.txt");
    assert!(offer("git switch -c ", &dir, &none, &mut g).is_none());
}

#[test]
fn oferta_sin_repo_da_archivos() {
    let mut g = GitData { refs: Some(vec![]), current: None, aliases: Some(vec![]) };
    let (_, items) = offer("git merge zz", &dir, &|_| None, &mut g).unwrap();
    assert_eq!(items[0].kind, Kind::File);
}

#[test]
fn oferta_rama_con_barra() {
    // refs con «feat/x»: con «feat/» se ofrece «feat/x» y la palabra no se parte en la «/».
    let (t, items) = offer("git checkout feat/", &dir, &|_| None, &mut g).unwrap();
    assert_eq!((t.typed_dir.as_str(), items[0].name.as_str()), ("", "feat/x"));
}

#[test]
fn reemplazo_de_rama_escapado() {
    let t = target("git checkout fi").unwrap().whole();
    let item = Item { name: "fix(ui)".into(), kind: Kind::Branch, commit: None };
    assert_eq!(replacement(&t, &item), "fix\\(ui\\) ");
}
```

  Las pruebas actuales de `candidates_and_replacement` se adaptan a `kind` sin cambiar lo
  que comprueban.

- [ ] **Paso 2:** `cargo test ramas_ oferta_ reemplazo_de_rama` → no compila.

- [ ] **Paso 3: Implementar.**
  - `branch_items`: las locales siempre; las remotas solo con `remotes`, sin el primer tramo
    del nombre (`origin/`), sin repetir una local ni otra remota con el mismo nombre. Filtro:
    contiene `prefix` sin distinguir mayúsculas. Orden por `(no empieza por prefix, no es la
    actual, -time)`. Como mucho 300, igual que los archivos.
  - `offer`:
    1. `target(line)?`.
    2. Si no es la primera palabra, `git_arg(line, shell_alias, &mut |n| git.alias(cwd, n))`:
       - `Shell` → `None`.
       - `Branch{remotes}` → `branch_items(git.refs(cwd), git.current…, remotes, &whole.prefix)`;
         si sale algo, `Some((whole, items))`.
    3. En cualquier otro caso, `Some((t, candidates(cwd, &t)))`.
  - `GitData::refs` carga `git::refs(cwd)` y `git::branch(cwd)` juntos. `GitData::alias`
    carga `git::aliases(cwd)`. Las dos solo la primera vez.

- [ ] **Paso 4:** `cargo test` → todas pasan. `cargo fmt`, `cargo clippy --all-targets`.

- [ ] **Paso 5: Commit:** `feat: ramas de git como sugerencias (modelo)`

### Tarea 3: Ramas en la lista de Tab

**Archivos:**
- Modificar: `src/main.rs` (`complete_key`; extraer `pub fn branch_icon(p: &egui::Painter, c: Pos2, color: Color32)`
  del dibujo de `Icon::Branch` en las cajas del prompt, que pasa a llamarla)
- Modificar: `src/overlay.rs` (`Overlay::Complete` y `complete_ui`)
- Modificar: `src/i18n/overlay.rs` (es/en)

**Interfaces:**
- Consume: `complete::offer`, `GitData`, `Kind`, `Item`, `shell_aliases` (tareas 1–2).
- Produce: `Overlay::Complete { target, items, sel, line, x, git: crate::complete::GitData }`.
  Textos `kind_branch`, `kind_current_branch`, `kind_remote_branch` en `i18n::overlay::T`.

- [ ] **Paso 1: `complete_key`.**
  - Se mira primero si hay `target(línea)`. Si no, no se toca el Tab, como ahora.
  - Después `take_shortcut(Tab)` y `offer(&línea, &s.cwd, &|n| shell_aliases().get(n).cloned(), &mut git)`.
  - Resultado:
    - `None` o vacía → `s.input(b"\t")`;
    - una sola → `replace_word` como ahora;
    - varias → `Overlay::Complete` con `git`.

- [ ] **Paso 2: `complete_ui`.**
  - Al cambiar la línea, se rehace con `offer(…, &mut git)` en vez de con `candidates`. El
    `GitData` ya cargado evita volver a llamar a git.
  - Icono por `kind`: `branch_icon` para las tres de rama, el de terminal para `Command` y
    `file_icon` para el resto.
  - Etiqueta por `kind`:

    | `kind` | Español | Inglés |
    |---|---|---|
    | `Branch` | «Rama» | «Branch» |
    | `CurrentBranch` | «Rama actual» | «Current branch» |
    | `RemoteBranch` | «Rama remota» | «Remote branch» |

  - El ancho de la columna de la etiqueta se mide con su texto y deja de ser 64 fijo: «Rama
    actual» no cabe en 64.

- [ ] **Paso 3: Comprobar con el guion.**
  - Repositorio de prueba en el scratchpad:
    - `git init -q -b main`;
    - commits vacíos con `git -c user.name=Prueba -c user.email=dev@example.com commit -q --allow-empty -m …`;
    - ramas `feat-a`, `feat-b` y `feat-c` (la actual).
  - Guion:
    ```
    wait 2500
    type cd <repo>\n
    wait 800
    type gco f
    key tab
    wait 400
    shot <scratch>/ramas.bmp
    key esc
    quit
    ```
  - Ejecutar con `COLIBRI_SCRIPT`, `COLIBRI_CONFIG`/`COLIBRI_STATE` temporales,
    `HISTFILE=<scratch>/hist`, `COLIBRI_LANG=es` y `cargo run`.
  - Pasar la captura a PNG con `sips -s format png` y mirarla.
  - Esperado: tres filas con icono de rama, «feat-c» con «Rama actual» arriba, y «Rama» en
    las otras.

- [ ] **Paso 4:** `cargo fmt`, `cargo clippy --all-targets`, `cargo test` → limpio.

- [ ] **Paso 5: Commit:** `feat: Tab ofrece ramas de git, también con alias`

### Tarea 4: Panel de detalles

**Archivos:**
- Modificar: `src/complete.rs` (`Detail`, `detail`, `human_size`, `base_dir` extraído de `entries`)
- Modificar: `src/overlay.rs` (`complete_ui`: el panel; `Overlay::Complete` gana
  `detail: Option<(String, Option<crate::complete::Detail>)>`, caché por nombre)
- Modificar: `src/i18n/overlay.rs`

**Interfaces:**
- Consume: `Item`, `Kind`, `Target`, `shell_aliases`, `command_paths` (tareas 1–2).
- Produce:
  - `complete::Detail { Commit { at: u64, subject: String }, Entries(usize), File { bytes: u64, modified: u64 }, Alias(String), Builtin, Path(String) }`
    (`Clone, PartialEq, Debug`)
  - `complete::detail(cwd: &Path, t: &Target, item: &Item, aliases: &HashMap<String, String>, paths: &HashMap<String, String>) -> Option<Detail>`
  - `complete::human_size(bytes: u64) -> String`
  - Textos:
    - `detail_alias: fn(&str) -> String`: «alias de {x}» / «alias for {x}»;
    - `detail_builtin`: «Orden interna del shell» / «Shell builtin»;
    - `detail_entries: fn(usize) -> String`: «1 elemento» / «{n} elementos», «1 item» / «{n} items»;
    - `detail_file: fn(&str, &str) -> String`: «{tamaño} · modificado {hace}» / «{size} · modified {ago}».

- [ ] **Paso 1: Pruebas que fallan**

```rust
#[test]
fn detalles_de_cada_tipo() {
    // carpeta temporal: «sub/» con 3 entradas, «a.bin» de 2048 bytes
    let (al, pa) = (HashMap::from([("gco".into(), "git checkout".into())]),
                    HashMap::from([("git".into(), "/usr/bin/git".into())]));
    let t = target("ls ").unwrap();
    let it = |name: &str, kind| Item { name: name.into(), kind, commit: None };
    assert_eq!(detail(&dir, &t, &it("sub", Kind::Dir), &al, &pa), Some(Detail::Entries(3)));
    assert!(matches!(detail(&dir, &t, &it("a.bin", Kind::File), &al, &pa),
                     Some(Detail::File { bytes: 2048, .. })));
    assert_eq!(detail(&dir, &t, &it("gco", Kind::Command), &al, &pa),
               Some(Detail::Alias("git checkout".into())));
    assert_eq!(detail(&dir, &t, &it("git", Kind::Command), &al, &pa),
               Some(Detail::Path("/usr/bin/git".into())));
    assert_eq!(detail(&dir, &t, &it("cd", Kind::Command), &al, &pa), Some(Detail::Builtin));
    let vacio = HashMap::new();
    assert_eq!(detail(&dir, &t, &it("cd", Kind::Command), &vacio, &vacio), None);
    let rama = Item { name: "x".into(), kind: Kind::Branch, commit: Some((5, "m".into())) };
    assert_eq!(detail(&dir, &t, &rama, &al, &pa),
               Some(Detail::Commit { at: 5, subject: "m".into() }));
    assert_eq!((human_size(512), human_size(2048), human_size(5 << 20)),
               ("512 B".into(), "2 KB".into(), "5 MB".into()));
}
```

- [ ] **Paso 2:** `cargo test detalles_de_cada_tipo` → no compila.

- [ ] **Paso 3: Implementar `detail` y `human_size`.**
  - Una carpeta o un archivo se resuelve con la misma base que `entries`: `typed_dir` sin
    escapes, `~/` o absoluta.
  - `Builtin` solo si hay rutas conocidas (`paths` no vacío) y el nombre no está ni en
    alias ni en rutas.
  - `human_size` redondea a entero: B, KB o MB.

- [ ] **Paso 4: Dibujar el panel en `complete_ui`.**
  - Qué fila describe: la que está bajo el puntero (apuntada en el bucle de filas); si no
    hay, la elegida; si no hay elegida, la primera visible.
  - `detail(…)` solo se recalcula cuando cambia el nombre descrito (campo `detail`).
  - `Area` propia (`Id::unique("complete-detail")`, `Order::Foreground`), 240 pt de ancho,
    8 pt a la derecha de la lista y a su misma altura. Si no cabe dentro de
    `screen.right() - 8`, no se pinta.
  - Mismo marco que la lista: `pal.panel`, borde `pal.border` y radio 8.
  - Contenido:
    - el nombre en monoespaciada 13 y `pal.fg`;
    - el tipo, pequeño y `pal.muted`;
    - el detalle: commit = mensaje (hasta 3 líneas) + `(o.ago)(ahora - at)` en `muted`; el
      resto, su texto de i18n.
  - Se le aplica `fade_in`.

- [ ] **Paso 5: Comprobar con el guion.**
  - Rehacer el guion de la tarea 3: la captura ya incluye el panel («feat-c», «Rama
    actual», «trabajo en c», «hace … »).
  - Añadir `type ls\s` (`\s` = un espacio al final) + `key tab` + `shot` en una carpeta con
    subcarpetas, para ver «N elementos».

- [ ] **Paso 6:** `cargo fmt`, `cargo clippy --all-targets`, `cargo test` → limpio.

- [ ] **Paso 7: Commit:** `feat: panel de detalles junto a las sugerencias de Tab`

### Tarea 5: Qué palabras del comando son rutas (puro)

**Archivos:**
- Modificar: `src/session.rs` (`Tok::Path`, `path_words`, `path_exists`, pruebas)

**Interfaces:**
- Produce:
  - Variante `Tok::Path`.
  - `pub fn path_words(text: &[char], upto: Option<usize>) -> Vec<(usize, usize, String, bool)>`:
    tramo `[inicio, fin)` en caracteres, la palabra sin escapes ni comillas y si basta con
    que sea el principio de algo que existe (la palabra del cursor).
  - `pub fn path_exists(dir: &Path, word: &str, prefix: bool) -> bool`

- [ ] **Paso 1: Pruebas que fallan**

```rust
fn words(s: &str, upto: Option<usize>) -> Vec<(usize, usize, String, bool)> {
    path_words(&chars(s), upto)
}

#[test]
fn palabras_de_ruta() {
    assert_eq!(words("cd src", None), [(3, 6, "src".into(), false)]);
    assert_eq!(words("ls -la src/main.rs", None), [(7, 18, "src/main.rs".into(), false)]);
    assert_eq!(words("cat mi\\ archivo.txt", None), [(4, 19, "mi archivo.txt".into(), false)]);
    assert_eq!(words("cat 'mi archivo'", None), [(4, 16, "mi archivo".into(), false)]);
    assert_eq!(words("devops-scripts/ x", None)[0], (0, 15, "devops-scripts/".into(), false));
    assert_eq!(words("git status", None), [(4, 10, "status".into(), false)]); // «git» es orden
    assert!(words("echo $HOME/x *.rs --file=a", None).is_empty());
    assert_eq!(words("make && ./run.sh", None), [(8, 16, "./run.sh".into(), false)]);
    assert_eq!(words("sort > out.txt", None), [(7, 14, "out.txt".into(), false)]);
    assert_eq!(words("A=1 ls src", None), [(7, 10, "src".into(), false)]);
}

#[test]
fn palabras_de_ruta_hasta_el_cursor() {
    // «cd s» tecleado; «rc && cargo build» es la sugerencia gris.
    assert_eq!(words("cd src && cargo build", Some(4)), [(3, 4, "s".into(), true)]);
    assert_eq!(words("cd src ", Some(7)), [(3, 6, "src".into(), false)]);
    assert!(words("cd ", Some(3)).is_empty());
}

#[test]
fn existe_relativa_a_la_carpeta_dada() {
    // temporal con «src/main.rs»
    assert!(path_exists(&dir, "src", false));
    assert!(path_exists(&dir, "src/main.rs", false));
    assert!(path_exists(&dir, "sr", true));
    assert!(!path_exists(&dir, "sr", false));
    assert!(path_exists(&dir, "src/", true));
    assert!(!path_exists(&dir.join("borrada"), "src", false));
    assert!(path_exists(Path::new("/"), "~", false)); // la carpeta personal
}
```

- [ ] **Paso 2:** `cargo test palabras_de_ruta existe_relativa` → no compila.

- [ ] **Paso 3: Implementar.**
  - Las palabras se separan por espacios sin escapar. Una palabra entre comillas que cierra
    es una sola, y su texto va sin las comillas.
  - `|`, `||`, `&&`, `;` y `&` vuelven a poner «primera palabra». `A=1` se salta y deja la
    primera palabra para la siguiente.
  - Se descartan:
    - las opciones (empiezan por `-`);
    - los redirectores sueltos (`>`, `<`, `>>`, `2>`);
    - las palabras con `$`, `*`, `?`, `` ` ``, `(`, `{` o `=`.
  - La primera palabra solo cuenta si lleva `/` o empieza por `.` o `~`.
  - Con `upto`:
    - las palabras que empiezan en `upto` o después no cuentan;
    - la que contiene `upto`, o acaba justo en él, se recorta a `[inicio, upto)` con
      `prefix = true`.
  - `path_exists`:
    - `~` y `~/…` se resuelven con la carpeta personal, `/…` es absoluta y lo demás es
      `dir.join`;
    - sin prefix, `exists()`;
    - con prefix, si acaba en `/` mira que la carpeta exista, y si no busca en su carpeta
      una entrada cuyo nombre empiece por el último tramo.

- [ ] **Paso 4:** `cargo test` → pasan. `cargo fmt`, `cargo clippy --all-targets`.

- [ ] **Paso 5: Commit:** `feat: reconocer las rutas en el comando`

### Tarea 6: Pintar las rutas

**Archivos:**
- Modificar: `src/session.rs`:
  - `Session::path_tokens`;
  - caché `paths_seen: HashMap<(PathBuf, String, bool), (bool, Instant)>`;
  - `split_rows`, extraído de los tramos de ssh en `chip_rows`, que pasa a usarlo.
- Modificar: `src/view.rs`:
  - `show_grid`;
  - `Look` y `GridPass`, que ganan `paths: &[(i32, usize, usize, Tok)]`;
  - `GridCache`, que gana `paths: Vec<(i32, usize, usize, Tok)>`;
  - `token_at`, `walk_row` y `GridPass::run`.

**Interfaces:**
- Consume: `path_words`, `path_exists`, `Tok::Path` (tarea 5); `prompt_tag`, `tag_dir`,
  `same_prompt`, `chip_kind` (existentes).
- Produce:
  - `pub fn path_tokens(&mut self) -> Vec<(i32, usize, usize, Tok)>`: fila de la pantalla,
    columna inicial y final (exclusiva) y `Tok::Path`.
  - `fn split_rows(at: &[(i32, usize)], from: usize, to: usize) -> Vec<(i32, usize, usize)>`

- [ ] **Paso 1: Prueba que falla**

```rust
#[test]
fn tramos_partidos_por_filas() {
    let at = [(5, 78), (5, 79), (6, 0), (6, 1)];
    assert_eq!(split_rows(&at, 0, 4), [(5, 78, 80), (6, 0, 2)]);
    assert_eq!(split_rows(&at, 1, 2), [(5, 79, 80)]);
}
```

- [ ] **Paso 2:** `cargo test tramos_partidos` → no compila.

- [ ] **Paso 3: `split_rows`.** Implementarla y usarla en los tramos de ssh de `chip_rows`.
  Las pruebas de ssh existentes siguen pasando.

- [ ] **Paso 4: `path_tokens`.**
  - Vacío si `self.remote`.
  - Por cada fila visible:
    - Se salta la fila si:
      - su columna 0 no lleva `prompt_tag`;
      - o es una línea de cajas (`chip_kind` en la columna 0);
      - o la fila siguiente lleva la marca del mismo prompt (`same_prompt`): entonces es la
        línea de arriba de un prompt de dos.
    - Si no, el comando empieza tras las celdas marcadas y sigue por las filas con `WRAPLINE`.
      Se saltan las celdas `WIDE_CHAR_SPACER`; `'\0'` cuenta como espacio.
  - `upto`: solo en el prompt que contiene el cursor y con el shell en primer plano
    (`fg_pid == shell_pid`). Es el índice de la primera celda en o tras el cursor.
  - Las palabras de `path_words` pasan por `path_exists(tag_dir(...), …)` a través de
    `paths_seen`:
    - una entrada vale 2 s;
    - la caché se vacía al pasar de 4096 entradas.
  - Los tramos salen con `split_rows`.

- [ ] **Paso 5: `view.rs`.**
  - `show_grid` llama a `s.path_tokens()` siempre, no solo con cajas, y lo pasa a `Look`
    y `GridPass`.
  - `token_at` mira primero los tramos de `chip_rows` y luego `paths`.
  - En `walk_row`, `Tok::Path` → `pal.ansi[6]`, y no se añade subrayado; el tachado sí.
  - `GridPass::run`: `paths` **no** entra en `GridKey`. Con un cambio de clave se rehace
    todo, como hoy; si no, se rehacen las filas dañadas más las filas cuyos tramos en `paths`
    difieren de los de `cache.paths`. Después se guarda `cache.paths`.

- [ ] **Paso 6: Comprobar con el guion.**
  - Carpeta temporal con `src/` y `notas.txt`.
  - Guion:
    ```
    type cd <tmp>\n
    wait 600
    type ls src notas.txt nada\n
    wait 600
    type cd s
    wait 300
    shot <scratch>/rutas.bmp
    quit
    ```
  - Esperado:
    - en la línea ya ejecutada, `src` y `notas.txt` en cian y sin subrayar, y `nada` sin color;
    - en la que se escribe, la `s` en cian;
    - `ls` y `cd` en el verde del plugin.
  - Repetir con `COLIBRI_GRID=egui`: debe dar la misma captura (`cmp`).

- [ ] **Paso 7: Medir.**
  - Release y `COLIBRI_FPS=1`, con la pantalla llena de prompts con rutas (≈15 `ls src`).
  - Anotar la columna de la rejilla por cuadro, antes y después del cambio
    (`git stash` o el binario anterior):
    - en reposo con un agente falso trabajando en otra pestaña;
    - tecleando.
  - Las cifras van a AGENTS.md (tarea 7).

- [ ] **Paso 8:** `cargo fmt`, `cargo clippy --all-targets`, `cargo test` → limpio.

- [ ] **Paso 9: Commit:** `feat: las rutas del comando, de color como en Warp`

### Tarea 7: Un clic abre los enlaces web

**Archivos:**
- Modificar: `src/config.rs` (`link_click: bool` en `Config`, `true` en `Default`)
- Modificar: `src/overlay.rs`: `PREF_ROWS` pasa a 25 con `"link_click"` detrás de
  `"complete_popup"`, y las mismas tres ramas que `complete_popup` (etiqueta, valor y
  `pref_change`).
- Modificar: `src/i18n/overlay.rs`:
  - `pref_link_click`: «Abrir enlaces con un clic» / «Open links with one click».
- Modificar: `src/view.rs`:
  - `Metrics` gana `pub link_click: bool`;
  - `handle_input`, `report_mouse` y las funciones nuevas de abajo.
- Modificar: `src/session.rs` (campo `pub link_press: Option<view::Withheld>`)
- Modificar: donde se construye `Metrics` (`main.rs`), con `self.cfg.link_click`.

**Interfaces:**
- Produce (en `view.rs`):
  - `fn opens_on_click(target: &Target, in_command: bool) -> bool`: `true` solo con
    `Target::Url` fuera del comando que se escribe.
  - `pub struct Withheld { pub button: u8, pub cell: (usize, usize) }`
    (`Clone, Copy, PartialEq, Debug`): pulsación aplazada.
  - `enum PointerEv { Move((usize, usize)), Release }` y
    `enum WithheldOut { Wait, SendPress, Open }`.
  - `fn withheld_step(w: Withheld, ev: PointerEv) -> (Option<Withheld>, WithheldOut)`
- La tarea 8 usa el enlace que devuelve `handle_input`, ahora también sin ⌘, y el campo de
  `Session` `link_hover: Option<(Span, Instant)>`, que crea esta tarea: el enlace bajo el
  puntero y desde cuándo.

- [ ] **Paso 1: Pruebas que fallan** (`view.rs`, `mod tests`)

```rust
#[test]
fn clic_normal_solo_en_enlaces_web() {
    let url = Target::Url("https://example.com/pr/13".into());
    let file = Target::File(PathBuf::from("/tmp/x"), None, None);
    assert!(opens_on_click(&url, false));
    assert!(!opens_on_click(&url, true)); // dentro del comando que se escribe
    assert!(!opens_on_click(&file, false)); // las rutas siguen siendo de Quick Peek / ⌘
}

#[test]
fn pulsacion_aplazada_sobre_un_enlace() {
    let w = Withheld { button: 0, cell: (24, 1) };
    assert_eq!(withheld_step(w, PointerEv::Move((24, 1))), (Some(w), WithheldOut::Wait));
    assert_eq!(withheld_step(w, PointerEv::Move((25, 1))), (None, WithheldOut::SendPress));
    assert_eq!(withheld_step(w, PointerEv::Release), (None, WithheldOut::Open));
}
```

- [ ] **Paso 2:** `cargo test clic_normal pulsacion_aplazada` → no compila.

- [ ] **Paso 3: Implementar.**
  - En `handle_input`, el enlace se busca con ⌘ como hoy o, con `m.link_click` y sin
    modificadores, si el puntero está encima.
  - Sin ⌘, solo cuenta si `opens_on_click(&link.target, in_command)`, siendo `in_command`
    que el punto cae en las celdas de `s.input_cells()`.
  - Se actualiza `s.link_hover`: se conserva la hora si el `Span` es el mismo, se pone
    `Instant::now()` si cambia y `None` si no hay enlace.
  - Con ⌘: lo de hoy (abre y vuelve).
  - Sin ⌘, el puntero pasa a mano y la salida temprana desaparece: lo de abajo sigue.
  - Con informes de ratón, `report_mouse` recibe `on_link: bool`:
    - Si la pulsación izquierda cae sobre el enlace, en vez de mandarla guarda
      `s.link_press = Some(Withheld{…})`.
    - Con `link_press` puesto, cada celda nueva y la suelta pasan por `withheld_step`:
      - `SendPress`: manda la pulsación guardada en su celda, pone `mouse_down` y sigue como
        un arrastre normal;
      - `Open`: `open_link` y no manda la suelta.
  - Sin informes, `resp.clicked()` sobre el enlace abre en vez de borrar la selección y
    colocar el cursor. El arrastre y el doble clic siguen igual.

- [ ] **Paso 4: Comprobar con el guion.**
  - Con `prog.sh`, el programa falso de la verificación, que imita a Claude Code a pantalla
    completa. Se lanza con `type LOG=<scratch>/mouse.log <scratch>/prog.sh\n`:
    ```bash
    #!/bin/bash
    printf '\e[?1049h\e[?1000h\e[?1003h\e[?1006h\e[H'
    printf 'Enlace OSC8: \e]8;;https://example.com/pr/13\e\\https://example.com/pr/13\e]8;;\e\\\r\n'
    printf 'Texto suelto: https://example.org/otra/ruta\r\n'
    stty raw -echo
    exec cat -v > "$LOG"
    ```
    Con la ventana del guion (1180×760 pt, panel de agentes abierto), la primera URL queda
    hacia `450 54` y la segunda hacia `480 71`.
  - Esperado:
    - clic en la línea OSC 8 → stderr `enlace: https://example.com/pr/13` y en el log del
      programa ningún `<0;…M` de esa pulsación;
    - `press` en el enlace + `moveto` tres celdas más allá + `release` → en el log, la
      pulsación (`<0;…M`) en la celda inicial, el arrastre (`<32;…M`) y la suelta, y nada en
      stderr;
    - el mismo clic con `pref link_click -1` → el programa recibe `<0;…M` y no se abre nada.
  - En la shell (sin programa): `type echo https://example.org/x\n`, clic sobre la URL de la
    salida → `enlace: …`; clic sobre la URL del comando escrito (`type echo https://a.b/c`,
    sin Enter) → nada en stderr.

- [ ] **Paso 5:** `cargo fmt`, `cargo clippy --all-targets`, `cargo test` → limpio.

- [ ] **Paso 6: Commit:** `feat: un clic abre los enlaces web, también en Claude Code`

### Tarea 8: Recuadro del enlace

**Archivos:**
- Modificar: `src/view.rs` (`show_grid`: dibujar el recuadro; `fn middle_truncate(s: &str, max: usize) -> String`)
- Modificar: `src/i18n/term.rs` (`link_open`: «Clic para abrir» / «Click to open»)

**Interfaces:**
- Consume: `s.link_hover` y el enlace devuelto por `handle_input` (tarea 7).

- [ ] **Paso 1: Prueba que falla**

```rust
#[test]
fn direccion_cortada_por_el_medio() {
    assert_eq!(middle_truncate("https://a.b/c", 40), "https://a.b/c");
    let s = middle_truncate(&format!("https://example.com/{}", "x".repeat(100)), 30);
    assert_eq!(s.chars().count(), 30);
    assert!(s.starts_with("https://example") && s.contains('…') && s.ends_with("xxx"));
}
```

- [ ] **Paso 2:** `cargo test direccion_cortada` → no compila.

- [ ] **Paso 3: Implementar.**
  - `middle_truncate` corta por caracteres y deja más del principio que del final.
  - Recuadro: si `s.link_hover` lleva 400 ms o más con el mismo `Span`, un `egui::Area`
    (`Order::Tooltip`) con:
    - la dirección en monoespaciada pequeña, cortada a 80 caracteres;
    - «Clic para abrir» en `muted`;
    - el marco con `pal.panel`, `pal.border` y radio 6.
  - Posición: bajo la fila del puntero, x en el puntero, recortado al rectángulo de la
    terminal; si no cabe debajo, encima.
  - Si todavía no llegan los 400 ms, se pide **una** pasada con
    `request_repaint_after(400 ms − transcurrido)`.
  - Solo sin ⌘, porque con ⌘ ya se sabe qué hace el clic, y solo con `link_click`.

- [ ] **Paso 4: Comprobar con el guion.**
  - Con `prog.sh`: `moveto` sobre la URL, `wait 600` y `shot` → recuadro con la dirección y
    «Clic para abrir».
  - `moveto` fuera, `wait 100` y `shot` → sin recuadro.
  - Con `COLIBRI_FPS=1` y `COLIBRI_LAZY=1`, puntero quieto sobre el enlace tras aparecer el
    recuadro → 0 cuadros por segundo.

- [ ] **Paso 5:** `cargo fmt`, `cargo clippy --all-targets`, `cargo test` → limpio.

- [ ] **Paso 6: Commit:** `feat: recuadro con la dirección al pasar sobre un enlace`

### Tarea 9: Documentación y cierre

**Archivos:**
- Modificar: `AGENTS.md`:
  - en la lista de archivos, la descripción de `complete.rs`;
  - el apartado «Sugerencias con Tab», con ramas, alias, panel y Tab al shell en los
    contextos sin rama; quitar «No hay subcomandos ni opciones como en Warp» y poner que solo
    hay ramas;
  - el apartado «Integración con zsh», con los archivos `aliases` y `paths`;
  - un apartado nuevo «Rutas de color», con las reglas, la resolución contra la carpeta de
    la marca, «solo hasta el cursor» y las cifras medidas en la tarea 6.
  - el apartado «Enlaces»:
    - un clic abre los web, con la pulsación aplazada en programas que leen el ratón;
    - el recuadro;
    - la preferencia `link_click`;
    - por qué no basta con Claude Code: `FORCE_HYPERLINK=1` hace que deje los enlaces a la
      terminal; comprobado con `FORCE_HYPERLINK=0 claude`.
    - Y en «Probar la interfaz», que `pref link_click` existe.
- Modificar: `README.md:18`: «sugerencias de rutas y ramas con Tab, y rutas de color en el
  comando».

- [ ] **Paso 1:** Escribir los cambios.
- [ ] **Paso 2:** Ejecutar:
  - `cargo fmt --check`;
  - `cargo clippy --all-targets` (sin avisos nuevos);
  - `cargo test`.

  Pegar el resumen de cada uno.
- [ ] **Paso 3: Commit:** `docs: ramas con Tab y rutas de color en AGENTS.md`
