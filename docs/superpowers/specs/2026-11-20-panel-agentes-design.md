# Panel de agentes

Diseño del panel de agentes de Colibrí, tomado de la captura de referencia que mandó el
usuario (2026-11-20). Es el tercero de los cambios de diseño pedidos; ya están hechos la
pestaña unida a la terminal, el indicador de carga en el icono y las pestañas rectas.

## Por qué

Colibrí existe para correr varios agentes a la vez y saber cuál trabaja y cuál terminó.
Hoy eso vive en una sección «Agentes» del panel derecho, apilada con Puertos y Procesos,
con una fila alta por agente y sin forma de filtrar ni buscar. El usuario quiere un panel
propio, como el de la captura: de un vistazo, qué agentes hay por proyecto, cuáles
trabajan y cuáles le esperan, y abrir uno nuevo desde ahí.

## Qué se quiere (decidido con el usuario)

- **Panel de arriba abajo** (opción A): ocupa toda la altura de la ventana. Los semáforos
  de macOS (cerrar, minimizar, ampliar) quedan dentro del panel, en su franja superior, y
  a su derecha va el botón para ocultar el panel. La barra de pestañas empieza donde acaba
  el panel.
- **A la izquierda por defecto, configurable a la derecha.** Con el panel a la derecha,
  los semáforos siguen arriba a la izquierda, en la barra de pestañas, como hoy.
- **Ocultable** con ese botón y con el atajo de siempre (`toggle_agents`, ⌘⇧A).
- **Arriba:** «Nueva sesión» (⌘T) y un botón de carpeta.
- **Buscador** de agentes.
- **Filtros** Todos / Esperando / Trabajando, cada uno con su contador.
- **Lista agrupada por proyecto** (la carpeta de la pestaña). Cada fila: icono del agente
  con su insignia de estado (la de las pestañas: puntos girando si trabaja, punto verde
  si terminó sin verse), título de la tarea, una segunda línea con el estado y el tiempo
  a la derecha.
- **Abajo:** una fila de iconos de agentes; cada uno abre una sesión nueva con ese agente.
- **Sin «Archivadas»** (el usuario no le ve uso).

## No objetivos

- Archivar o guardar conversaciones terminadas.
- Mover pestañas entre ventanas o sacarlas a una ventana nueva (es otro proyecto, ya
  apuntado, va después).
- Idiomas (va al final; los textos nuevos se escriben en español como el resto).
- Cambiar Puertos y Procesos más allá de su sitio.

## Anti-objetivos

- Que la ventana deje de poder arrastrarse por arriba, o que los semáforos queden tapados
  o descolocados.
- Repintar de más: el panel no añade repintados propios. Lo que se anima es la insignia
  de «trabajando», que ya tiene su refresco (66 ms solo mientras un agente trabaja).
- Que escribir en la terminal se cuele en el buscador o al revés.

## Enfoques considerados

- **A) Panel de arriba abajo** — elegido por el usuario: es lo que muestra su captura y
  deja más alto para la lista. Cuesta mover la zona de los semáforos y el arrastre de la
  ventana, que hoy dependen de la barra de pestañas.
- B) Panel bajo la barra de pestañas: más sencillo, no toca la barra de título, pero no se
  parece a la referencia.
- Mantener un único panel lateral con todas las secciones y lado configurable: se
  descartó porque mezcla lo de los agentes con Puertos y Procesos, que es justo lo que se
  quiere separar.

## Comportamiento

### Contenido de la lista

- **Qué agentes:** las pestañas con agente detectado (`Session::agent`) de **todas las
  ventanas**. Un clic en una fila lleva a esa pestaña, trayendo su ventana al frente si es
  otra (`App::focus_session`, que ya lo hace). La fila de la pestaña activa de la ventana
  en curso va resaltada.
- **Grupos:** por proyecto (`Session::project()`, el nombre de la carpeta), en el orden
  en que aparecen las pestañas; dentro de cada grupo, en el orden de las pestañas.
- **Título de la fila:** el título que publica el agente (`clean_title()`); si no hay, o
  es el nombre del agente, el nombre del agente («Claude Code»). Un nombre puesto a mano
  a la pestaña tiene prioridad.
- **Segunda línea:** el estado — «Trabajando…», «Terminó» (verde) o «Esperando tu
  mensaje».
- **Tiempo:** cuánto lleva en su estado actual: trabajando desde que empezó, terminado o
  esperando desde que acabó. Formato corto como hoy (`ago`): «27s», «3m», «1h».

### Filtros y buscador

- **Trabajando:** `busy`. **Esperando:** con agente y no `busy` (incluye los que
  terminaron sin verse). **Todos:** ambos. Los contadores cuentan lo que hay en cada uno,
  sin aplicar el buscador.
- **Buscador:** filtra por título, proyecto y nombre del agente, sin distinguir
  mayúsculas ni acentos. Vacío, no filtra.
- **Enfoque:** el buscador solo recibe teclas cuando se ha hecho clic en él; `Esc` lo
  vacía y devuelve el teclado a la terminal. La captura muestra `/` como atajo, pero
  aquí `/` se escribe en la terminal todo el rato: no se usa como atajo global.

### Acciones

- **Nueva sesión (⌘T):** lo mismo que hoy hace ⌘T (pestaña nueva en la carpeta de la
  activa). ⇧⌘T sigue siendo «reabrir lo cerrado».
- **Botón de carpeta:** elegir una carpeta y abrir ahí una pestaña nueva. Se reutiliza el
  selector de carpetas que ya existe (`Overlay::Picker`).
- **Iconos de abajo:** uno por agente conocido que esté instalado (su orden en el PATH),
  en este orden: Claude Code, Codex, Gemini, Aider, OpenCode. Un clic abre una pestaña en
  la carpeta de la activa y arranca ese agente en ella. Los no instalados no salen.
- **Ocultar:** el botón junto a los semáforos y ⌘⇧A. El botón de Agentes (el robot) de la
  derecha de la barra de pestañas se quita: su papel lo hace el nuevo botón. Los de
  Puertos y Procesos siguen.

### Sitio y tamaño

- Preferencia nueva `agents_side`: «izquierda» (por defecto) o «derecha», en
  Preferencias. Panel visible por defecto (`show_agents` pasa a `true` por defecto; quien
  ya tenga el archivo de configuración conserva su valor).
- **Izquierda:** el panel ocupa la altura completa; los semáforos y el botón de ocultar en
  su franja superior (alta como la barra de pestañas); la barra de pestañas va a su
  derecha y ya no reserva hueco para los semáforos (`TRAFFIC_LIGHTS_W`). Puertos y
  Procesos, a la derecha, bajo la barra, como hoy.
- **Derecha:** el panel ocupa la altura completa a la derecha; la barra de pestañas va de
  los semáforos hasta el panel; el botón de ocultar va en la franja superior del panel.
  Puertos y Procesos pasan a la izquierda, bajo la barra.
- **Oculto:** la barra de pestañas vuelve a ir de lado a lado, como hoy, con el botón de
  mostrar el panel junto a los semáforos.
- Ancho ajustable arrastrando su borde (220–420, 260 por defecto); no se guarda entre
  arranques, igual que el panel actual.
- La franja superior del panel se puede arrastrar para mover la ventana, como la barra de
  título (macOS la trata así porque la ventana tiene la barra transparente).

## Restricciones

- Código, comentarios y textos en español.
- Repintado bajo demanda: nada nuevo repinta en reposo.
- La barra de menús: ⌘T ya está ahí como «Nueva pestaña»; no se duplica.
- Una preferencia nueva lleva su fila en `PREF_ROWS`, su valor mostrado y su rama en
  `pref_change` (`overlay.rs`), con `#[serde(default)]`.
- Lo que se pueda verificar con el guion de depuración lleva su paso (`Step`) si no
  existe ya (por ejemplo, escribir en el buscador o elegir un filtro).

## Decisiones del diseñador tras la revisión (corregibles)

1. **Estado compartido por todas las ventanas.** Visibilidad, lado, filtro y texto del
   buscador viven en `App` (como hoy `show_agents`); cada ventana dibuja su panel con
   ellos. ⌘⇧A y el botón de ocultar cambian el de todas. La lista lee las sesiones de la
   ventana en curso (`self.w`) y de las aparcadas (`parked`).
2. **Tiempo en el estado actual.** Campo nuevo en `Session`, «en este estado desde»: se
   pone al abrir la pestaña, al detectar el agente y en cada cambio de `busy`. Siempre hay
   dato (como mucho, desde que se abrió la pestaña o se detectó el agente).
3. **«Terminó» frente a «Esperando tu mensaje».** «Terminó» (verde) es `unseen`: acabó una
   tarea y no se ha mirado esa pestaña. Al mirarla pasa a «Esperando tu mensaje». Ambos
   cuentan en el filtro «Esperando».
4. **Aider y OpenCode.** Se lanzan con `aider` y `opencode`, sin argumentos; la detección
   ya los conoce (`Agent::from_word`). Su icono es el genérico que ya dibuja `agent_icon`
   para ellos.
5. **Agentes `Other`** (amp, goose, copilot…): salen en la lista y en los contadores con
   su icono genérico; no tienen icono abajo para lanzarlos.
6. **Botón de carpeta.** Modo nuevo del selector de carpetas: empieza en la carpeta de la
   pestaña activa, solo lista carpetas y al elegir una abre una pestaña nueva en ella (no
   hace `cd`).
7. **Qué agentes están instalados.** Se averigua una vez al arrancar, en un hilo, con el
   PATH del shell de inicio de sesión (`$SHELL -lc`), porque el de una app abierta desde
   Finder no incluye `~/.local/bin` ni lo de npm. Si se instala uno después, aparece al
   reiniciar Colibrí. Hasta que el hilo responda, la fila de abajo no muestra iconos.
8. **Grupos por carpeta completa.** Se agrupa por la ruta, no por el nombre: dos carpetas
   `app/` distintas son dos grupos. El encabezado muestra el nombre; si dos grupos se
   llaman igual, muestran la ruta abreviada (`~/…`).
9. **Arranque del agente en la pestaña nueva.** La orden se escribe en cuanto el shell
   está en su prompt (`Session::at_prompt()`), no antes.

## Lo que queda al constructor

- Cómo arrancar el agente en la pestaña nueva sin escribir antes de que el shell esté
  listo (escribirlo antes descuadraba el prompt; ver «Restaurar pestañas» en
  `CLAUDE.md`).
- Cómo saber si un agente está instalado sin bloquear la interfaz.
- Medidas, tipografías y colores, tomando la captura como guía y los del tema actual.
- Cómo se reparte el código (un módulo nuevo para el panel parece razonable, dado el
  tamaño de `main.rs`).
