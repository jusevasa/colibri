# Atajo para buscar agentes (⌘⇧F)

## Por qué

El buscador del panel de agentes solo recibe el teclado tras un clic, y Enter no hace nada
útil (suelta el foco). Se quiere llegar a él con un atajo que se vea en la propia caja, como
el `⌘T` de «Nueva sesión», para que se descubra solo, y que sirva para saltar a un agente sin
tocar el ratón: `⌘⇧F` + «colib» + Enter.

## Objetivos

- Acción nueva `SearchAgents`, por defecto `cmd+shift+f`, configurable en `config.toml`
  (`search_agents`), en la paleta y en el menú Visualización (tras «Panel de agentes»).
- Al ejecutarla: si el panel está oculto se abre, y el buscador toma el teclado.
- La caja enseña la tecla a la derecha (`self.keys.label`, gris, fuente pequeña) mientras
  está vacía y sin foco. Con la tecla vacía en la config no se pinta nada.
- Con el buscador activo, una fila de la lista está elegida (contorno de acento): la primera
  visible al abrir y cada vez que cambia lo escrito. ↑/↓ recorren las visibles (en el orden
  en que se ven, por grupos) dando la vuelta, como la paleta, y desplazan la lista para que
  se vea la elegida. Enter va a esa pestaña (`focus_session`) y el teclado vuelve a la
  terminal. Esc sigue igual: vacía y devuelve el teclado.

## No objetivos

- Cambiar qué filtra la búsqueda o cómo se agrupa.
- Elegir con el ratón sin clic (el hover no mueve la fila elegida).
- Paso nuevo en el guion: `key cmd+shift+f`, `key down`, `key enter` y `menu Buscar agentes`
  ya lo cubren.

## Anti-objetivos

- Que ↑/↓/Enter lleguen a la terminal mientras el buscador tiene el teclado (Enter
  ejecutaría lo escrito en el prompt).
- Que ↑/↓ muevan el cursor dentro de la caja (se consumen antes del `TextEdit`).
- Repintados continuos: todo ocurre por teclas.

## Enfoques

1. Solo filtrar (como ahora) con el atajo: poco trabajo, pero para ir al agente sigue
   haciendo falta el ratón.
2. **Elegido**: filtrar y saltar (↑/↓ + Enter), como un selector rápido.

La tecla: ⌘⇧F en vez de ⌘F, que está libre pero es lo que se espera para buscar en la salida
de la terminal y conviene guardarla.

## Cómo (decisiones mías)

- La fila elegida se guarda en `Win` (`agents_pick: Option<u64>`, id de sesión): la búsqueda
  se comparte, pero el teclado solo lo tiene una ventana y las demás la borrarían al repintar.
  Si deja de estar visible se vuelve a la primera.
- La parte pura (`agents::step_pick(orden, elegida, paso)`) va con pruebas en `agents.rs`.
- La fila elegida solo se marca mientras el buscador tiene el teclado.
- Añadido al probar: ⌘⇧F selecciona lo que ya hubiera escrito (lo nuevo lo sustituye) y Enter,
  si salta a un agente, vacía la búsqueda para que la lista vuelva a enseñarlos todos.
