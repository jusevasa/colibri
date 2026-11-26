# Ramas de git con Tab y rutas de color en el comando

## Por qué

Se viene de Warp y se echan en falta dos cosas al escribir en el prompt:

1. Con `gco f` + Tab, Warp ofrece las ramas que empiezan por «f» (la actual marcada como
   «Current branch», las demás como «Branch») con un panel al lado que describe la elegida.
   Colibrí ofrece carpetas y archivos, que ahí no sirven.
2. En Warp las rutas del comando van de otro color que la orden. En Colibrí la línea la
   colorea `zsh-syntax-highlighting` (plugin del `.zshrc` del usuario): orden en verde y
   rutas solo subrayadas.

La idea es hacer lo mismo que Warp y, donde sea barato, algo mejor.

## Objetivos

### 1. Ramas con Tab

- Al pulsar Tab en un argumento de una orden de git que admite ramas, la lista de
  sugerencias (`Overlay::Complete`) ofrece **ramas** en vez de archivos.
- La orden se reconoce aunque venga de un alias:
  - alias del shell (`gco` → `git checkout`, del plugin `git` de oh-my-zsh, o cualquier
    alias del usuario);
  - alias de git (`git co` → `git checkout`, de `git config alias.co`). Los alias de git
    que empiezan por `!` (órdenes del shell) no se siguen.
- Órdenes y posición de la rama:
  - `checkout`, `switch`, `merge`, `rebase`, `diff`, `log`, `show`, `cherry-pick`, `reset`:
    cualquier argumento que no sea una opción.
  - `branch`: el argumento tras `-d`, `-D`, `-m`, `-M` (y sus formas largas `--delete`,
    `--move`); `git branch` sin esas opciones crea una rama: ahí no se ofrece nada.
  - `push`, `pull`, `fetch`: el segundo argumento que no sea opción (el primero es el
    remoto).
  - Donde la orden de git no admite rama (tras `-b`, `-B`, `-c`, `-C`, que piden un nombre
    nuevo; `git branch <nueva>`; el primer argumento de `push`/`pull`/`fetch`, que es el
    remoto) **el Tab pasa al shell** sin abrir la lista de Colibrí: el completado de zsh
    (`_git`) sabe ofrecer remotos y no inventa archivos donde no tocan. Hoy ahí se abre la
    lista de archivos; ese caso cambia a propósito.
  - Tras `--` son rutas: archivos y carpetas como hoy.
  - En ramas no se separa la palabra en carpeta + nombre: `feat/x` es un nombre entero que se
    filtra tal cual (hoy `complete::target` cortaría en la `/`).
  - Solo cuenta la orden en la que está el cursor: en `make && gco f` se mira `gco f`, lo que
    hay tras el último `|`, `||`, `&&` o `;`.
- Qué ramas:
  - las **locales**, todas las órdenes;
  - en `checkout` y `switch`, también las **remotas que no tienen rama local con el mismo
    nombre**, escritas sin el prefijo del remoto (`origin/feat-x` se ofrece como `feat-x`:
    git crea la rama local que la sigue). Se omite `origin/HEAD`.
- Etiqueta a la derecha de cada fila: «Rama actual», «Rama», «Rama remota» (en inglés
  «Current branch», «Branch», «Remote branch»). Icono: el de rama de la caja del prompt.
- Orden y filtro: las que **contienen** lo escrito (sin distinguir mayúsculas); primero las
  que empiezan por ello; dentro de cada grupo, la actual primero y luego por fecha del
  último commit, de la más reciente a la más antigua. El trozo que coincide va resaltado,
  como ahora.
- Elegirla escribe el nombre (con los caracteres especiales escapados, como los archivos)
  seguido de un espacio.
- Si no es un repositorio, o ninguna rama coincide, se comporta como hoy (archivos y
  carpetas; si tampoco hay, el Tab llega al shell).
- El resto del comportamiento de la lista no cambia: se abre con Tab, sin nada elegido, el
  teclado sigue llegando al shell y escribir filtra; Tab/flechas eligen, Enter acepta, Esc
  devuelve lo que había.

### 2. Panel de detalles (todas las sugerencias)

- A la derecha de la lista, un panel con el nombre (en negrita) y su tipo, como Warp, más un
  dato útil según el tipo:
  - rama: mensaje del último commit y hace cuánto (`i18n` ya tiene `ago`);
  - carpeta: cuántos elementos tiene;
  - archivo: tamaño y hace cuánto se modificó;
  - comando: a qué equivale si es un alias («alias de `git checkout`»), «orden interna» si
    es un builtin, o la ruta del ejecutable. Si Colibrí no sabe la ruta tal como la ve el
    shell (su PATH puede ser otro), solo «Comando».
- Describe la fila bajo el puntero; si no hay ninguna, la elegida; si no hay elegida, la
  primera.
- Si no cabe a la derecha de la lista dentro de la ventana, no se pinta.

### 3. Rutas de color en el comando

- En el comando de **cada prompt marcado visible** (el que se escribe y los del historial),
  las palabras que son **rutas que existen** se pintan del cian del tema (`pal.ansi[6]`, el
  que usa Warp para los argumentos) y **sin el subrayado** que les pone
  `zsh-syntax-highlighting`.
- Una ruta relativa se resuelve contra la carpeta del prompt en que se escribió (la que
  lleva su marca, `tag_dir`), no contra la carpeta actual de la pestaña. Valen también las
  absolutas y las que empiezan por `~/`.
- En el prompt que se está escribiendo, la palabra en la que está el cursor también se pinta
  si es el **principio** de una entrada que existe (como el `path_prefix` del plugin), para
  que no pierda color mientras se teclea.
- En el prompt que se está escribiendo **solo se mira hasta el cursor**: lo que hay detrás
  puede ser la sugerencia en gris de `zsh-autosuggestions`, que está en las celdas como si se
  hubiera tecleado (con `cd s` y la sugerencia `rc/foo` saldría en cian algo que nadie
  escribió). Al editar en mitad de la línea, las rutas de detrás del cursor se quedan sin
  color hasta pulsar Enter; en cuanto el prompt pasa al historial (zsh ya quitó la
  sugerencia) se pinta la línea entera.
- La primera palabra (la orden) solo cuenta como ruta si lo parece (lleva `/` o empieza por
  `.` o `~`, como el `devops-scripts/` de autocd); si no, es una orden y se queda como la
  pinte el plugin.
- No son rutas: opciones (`-x`, `--algo`, `--algo=valor`), palabras con variables, comodines
  o sustituciones (`$`, `*`, `?`, `` ` ``, `$(`). Las barras de escape se interpretan
  (`mi\ carpeta`) y las comillas que rodean la palabra entera se quitan (`'mi carpeta'`).
- Lo demás de la línea (el verde de la orden, opciones, cadenas) no se toca: sigue siendo
  lo que pinte el shell.

### 4. Enlaces web con un clic (añadido el mismo día)

Por qué: en Warp un clic normal sobre una URL de Claude Code la abre. Comprobado que no lo
hace Warp (su clic normal solo enseña un recuadro; abrir directamente es ⌘+clic), sino
Claude Code a pantalla completa, que recibe el clic. En Colibrí no lo hace porque la pestaña
arranca con `FORCE_HYPERLINK=1` y entonces Claude Code deja los enlaces a la terminal (el
usuario lo comprobó: con `FORCE_HYPERLINK=0 claude` se abren con un clic). Quitar la
variable no vale (una URL partida en dos líneas se abriría a trozos), así que lo hace
Colibrí, sin depender del programa.

- **Al pasar el puntero** sobre un enlace web (los que hoy abre ⌘+clic con `Target::Url`:
  OSC 8 o dirección en el texto; `http`, `https`, `mailto`), sin ⌘: se resalta como hoy con
  ⌘ (color de acento y subrayado) y el puntero es una mano. Si el puntero se queda quieto
  sobre el mismo enlace 400 ms, aparece debajo (o encima, si no cabe) un recuadro con la
  dirección completa (cortada por el medio si es muy larga) y «Clic para abrir». Al salir del
  enlace se va.
- **Un clic normal** (botón izquierdo, sin modificadores, sin arrastrar) lo abre, también en
  un programa que lee el ratón (Claude Code a pantalla completa).
  - Ahí la pulsación que cae sobre un enlace web no se le manda al programa hasta saber qué
    es: si se suelta sin moverse de celda, se abre el enlace y el programa no recibe nada; si
    se arrastra, se le manda la pulsación aplazada y el arrastre sigue como siempre (se puede
    seleccionar empezando en un enlace).
  - El movimiento del puntero se le sigue mandando al programa como hoy.
- **Se quedan como están**:
  - Las rutas de archivos (y los `file://`): solo con ⌘, para no quitarle los clics a Quick
    Peek.
  - Un enlace dentro del comando que se escribe en el prompt actual: el clic coloca el
    cursor, como hoy; ⌘+clic lo abre.
  - ⌘+clic abre cualquier enlace, como hoy.
- **Preferencia** «Abrir enlaces con un clic» (`link_click`, activada por defecto). Apagada,
  todo es como hoy: sin resaltar ni recuadro sin ⌘.
- Con el guion, el enlace no se abre: se escribe en stderr (`enlace: <url>`), como hoy.

## No objetivos

- Seleccionar una URL con doble clic: el primer clic ya la abre. Para seleccionarla, arrastrar
  (o apagar la preferencia).
- Menú del recuadro (copiar enlace, abrir en…): solo enseña la dirección.
- Sugerencias de subcomandos u opciones (`git ch…` → `checkout`), como las especificaciones
  de Warp. Solo ramas.
- Etiquetas, remotos, stashes o commits como sugerencias.
- Ramas remotas en órdenes que no sean `checkout`/`switch` (`git merge origin/x`).
- Que Colibrí coloree localmente la orden, las opciones o las cadenas.
- Rutas de color sin la integración de zsh (sin marca de prompt no se sabe dónde empieza el
  comando) y en sesiones ssh (no se puede mirar el disco remoto).
- Comandos de varias líneas reales (`\` + Enter): solo la primera línea lógica (la que
  sigue con `WRAPLINE`).
- El recuadro «→» de la sugerencia en gris de `zsh-autosuggestions`.
- Abrir la lista sola mientras se escribe: sigue siendo con Tab.

## Anti-objetivos

- **Repintados o despertares en reposo** (regla 5 de AGENTS.md): el color de las rutas se
  calcula al pintar, sin temporizadores. Si un archivo se crea o se borra, el color cambia
  en el siguiente cuadro que se pinte por otra causa; no se sondea el disco.
- **Mirar el disco por cada palabra en cada cuadro**: lo que existe se guarda en memoria
  (por carpeta + palabra) y se reutiliza; el coste se mide y se dice (regla 6).
- **Bloquear la interfaz al escribir con la lista abierta**: git se consulta una vez, al
  abrir la lista con Tab; al seguir escribiendo se filtra en memoria.
- Conexiones de red (regla 2): nada de `git fetch` ni `ls-remote`; solo lo que ya está en el
  repositorio local (`for-each-ref`).
- Tocar la configuración de `zsh-syntax-highlighting` o cualquier archivo del usuario.
- Que el recuadro del enlace repinte mientras el puntero está quieto: una sola pasada
  programada a los 400 ms, nada más.
- Que un clic sobre un enlace llegue a la vez al programa y a Colibrí (abriría dos veces o
  movería el cursor de Claude Code).
- Cambiar el orden o el comportamiento de las sugerencias actuales de archivos, carpetas y
  comandos.

## Enfoques

**Ramas.** Un solo camino razonable: ampliar `complete.rs` (qué completar y con qué) y
`Overlay::Complete` (icono, etiqueta, panel). Descartados: un desplegable aparte como el de
la caja de la rama (duplicaría la lista y su manejo del teclado) y pedirle las ramas al
sistema de completado de zsh (`_git`), que es justo lo que ya hace el Tab que llega al shell
y no se puede pintar desde Colibrí.

**Rutas.** Se consideraron dos:

- A. Que la integración cambie `ZSH_HIGHLIGHT_STYLES[path]` del plugin a un color. Poco
  código, pero solo funciona con el plugin y no es lo que hace Warp.
- **B. Elegido por el usuario: que Colibrí pinte las rutas**, como Warp, con o sin plugin.

**Enlaces.** Se consideraron: quitar `FORCE_HYPERLINK=1` para que Claude Code abra él los
enlaces (descartado: las URLs partidas en dos líneas se abrirían a trozos, y solo serviría con
Claude Code), copiar a Warp (clic = recuadro, ⌘+clic = abrir; descartado: el usuario quiere
abrir con un clic) y **el elegido**: un clic abre los enlaces web en Colibrí, con el recuadro
al pasar el puntero.

Dentro de B, se pinta **al dibujar** (como los tramos del comando en ssh, `Tok` + `token_at`
en `view.rs`) y no escribiendo colores en las celdas de la rejilla: no hay que resellar en
cada cuadro, y los prompts del historial resuelven sus rutas con la carpeta de su marca.

## Cómo (decisiones mías)

- **Alias del shell**: `_colibri_precmd` (`assets/shell/zshenv.zsh`) escribe
  `~/Library/Caches/colibri/zsh/aliases` (nombre y valor, separados por NUL:
  `print -rN -- ${(kv)aliases}`) en el primer `precmd` y cuando cambia el PATH, junto al
  archivo `commands`. Colibrí lo lee con la misma caché por mtime que `commands()`. Se
  expande la primera palabra hasta 3 veces (alias de alias). Sin integración no hay alias:
  solo `git checkout` escrito tal cual.
- **Alias de git**: `git config --get-regexp '^alias\.'` en la carpeta de la pestaña, solo al
  pulsar Tab y solo si el subcomando no es uno de los conocidos.
- **Ramas**: una sola llamada, `git --no-optional-locks -C <cwd> for-each-ref
  --sort=-committerdate` con `%(refname)`, `%(committerdate:unix)` y `%(subject)` sobre
  `refs/heads` (y `refs/remotes` en checkout/switch). La rama actual sale de `git::branch`.
  La lista de ramas se guarda con la lista abierta y se filtra en memoria al seguir
  escribiendo.
- **Datos de cada sugerencia**: `complete::Item` pasa a llevar su tipo (carpeta, archivo,
  comando, rama, rama actual, rama remota) y, para las ramas, fecha y mensaje del último
  commit. Lo del panel que haya que mirar en disco (elementos de una carpeta, tamaño y fecha
  de un archivo) se calcula solo para la fila que se describe, no para toda la lista.
- **Rutas**: variante nueva `Tok::Path`. Se calculan por cada prompt marcado visible (su
  fila con la marca y las que la siguen con `WRAPLINE`), a partir del final de las celdas
  marcadas, y viajan como los tramos de ssh, de modo que entran en `GridKey` y un cambio en
  ellas rehace la rejilla. `walk_row` les pone `pal.ansi[6]` y no les añade subrayado.
  Caché de existencia en la `Session`, con caducidad corta para el prompt que se escribe.
- **Textos** en `i18n/overlay.rs` (es/en): tipos de rama y textos del panel.
- **Pruebas** (partes puras): reconocer el contexto de git (alias, subcomandos, `-b`, `--`,
  segundo argumento de push), orden y filtro de ramas, y qué palabras de una línea son rutas
  (con la comprobación de existencia inyectada).
- **Comprobación con el guion** (regla 8): un repositorio temporal con varias ramas,
  `type gco f`, `key tab`, `shot`; y `type cd src`, `shot` para el color. Si los pasos que
  ya existen no bastan, se añade uno.
- **AGENTS.md**: actualizar «Sugerencias con Tab» (ya no es cierto que no haya nada como
  Warp), la integración con zsh (archivo de alias) y añadir las rutas de color.

## Lo que queda para quien lo construya

- Medidas y aspecto exactos del panel (ancho, márgenes, tipografía), dentro del estilo de la
  lista actual.
- Duración de la caché de existencia y dónde vive exactamente.
- Si los tramos de ruta se calculan dentro de `chip_rows` o en una función aparte.
- Cómo se reorganiza `complete::Item` y sus pruebas actuales.
