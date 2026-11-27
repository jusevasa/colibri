# Aire en el prompt, raya entre bloques y pista «→» (parte 1 de 5)

Primera de cinco partes para que el prompt y las sugerencias de Colibrí tengan el acabado de
Warp. Warp es solo guía de comportamiento y medidas (AGPL: nada de su código). Orden acordado:
1) esta, 2) lista de sugerencias, 3) prompts anteriores como línea gris, 4) comando
inexistente subrayado, 5) `git branch` + Tab con ramas y opciones descritas.

## Por qué

En Warp la fila de cajas (carpeta, rama, cambios) respira: hay ~15 px entre ella y la línea
donde se escribe, una raya fina separa un bloque del anterior y, tras la sugerencia gris, una
cajita «→» dice con qué tecla se acepta. En Colibrí las cajas van pegadas al comando y no hay
separación entre bloques. La persona lo quiere «bien refinado», como Warp.

## Objetivos

- Unos 12 px de aire entre la fila de cajas y la línea del comando.
- Una raya de 1 px (texto del tema al 10 %) a todo el ancho entre un bloque y el anterior.
- Tras la sugerencia gris de zsh-autosuggestions, a 10 px, una cajita con «→».

## Fuera de esta parte

- Cambiar el alto de las filas de la rejilla, cuántas caben o dónde cae cada clic.
- Los prompts anteriores como línea gris (parte 3): esta parte toca todas las filas de
  cajas por igual.
- El «▾» de Warp para cambiar la tecla: aquí la tecla la decide zsh-autosuggestions (→ o
  Fin), no Colibrí. Se puede añadir más adelante si se quiere. *Propuesta mía.*
- El margen izquierdo de 16 px de Warp: el de Colibrí es la preferencia `padding`.

## Anti-objetivos

- Que un prompt sin línea en blanco encima (otro tema, `clear`, primera fila de la pantalla)
  se vea peor que hoy: en ese caso todo queda como ahora.
- Repintados de más: todo se pinta en la pasada que ya hay.

## Enfoques

- **A. Subir la fila de cajas hacia la línea en blanco de encima** (elegido). powerlevel10k
  deja una línea en blanco entre comandos; las cajas se dibujan más arriba, invadiendo esa
  línea, y el hueco queda libre sobre el comando. La rejilla no cambia.
- B. Separar de verdad las filas con un desplazamiento vertical por fila: más fiel a Warp,
  pero cambia cuántas filas caben, los clics y el desplazamiento por el historial. Descartado
  por el riesgo.
- C. Pedir al shell una línea más: no da huecos de menos de una fila y depende del tema.
  Descartado.

## Diseño

**Cajas** (`prompt_chips` en `main.rs`, `chip_rows` en `session.rs`):
- `ChipRow` dice además si la fila de encima está en blanco y en pantalla (`room_above`).
- Con hueco: la caja mide lo mismo que hoy y su borde inferior queda 12 px por encima de la
  línea del comando, así que sube dentro de la línea en blanco. Sin hueco: como hoy (sube 3 px).
- Estilo de las cajas, ya cercano al de Warp: se ajustan radio (4 px), borde (texto al 15 %)
  y fondo (texto al 5 %) a esas proporciones sobre el fondo del tema.
- Prompts remotos (cajas en la misma fila que el comando): sin cambios.

**Raya entre bloques** (`view.rs`, junto a las cajas):
- Por cada fila de cajas con hueco encima y con algo por encima (no la primera fila de la
  pantalla): una línea de 1 px a todo el ancho, color texto al 10 %, en la parte de arriba de
  la línea en blanco.

**Pista «→»** (`view.rs`):
- Qué celdas son la sugerencia gris: en el comando que se escribe (`input_cells`), las que van
  detrás del cursor en la misma línea lógica con el color de zsh-autosuggestions (índice 8,
  «gris brillante», o atenuadas). Es una heurística: con otro estilo configurado no sale la
  pista y no pasa nada más. *Propuesta mía; la alternativa exacta sería que la integración
  avisara del texto de `$POSTDISPLAY`, y no hay canal fácil para eso.*
- Dibujo: a 10 px del final de la sugerencia, alto del cursor (mín. 12 px), borde de 1 px y
  radio del 25 % del alto, «→» en la letra de interfaz 2 pt menor, color texto al 40 %, sin
  relleno. No responde al ratón.
- Solo con la ventana enfocada y la pestaña en el prompt, como el cursor.

**Verificación**: guion con `cd ~/Desktop/dev`, un segundo prompt y una sugerencia gris
(historial falso con `COLIBRI_HISTFILE`), `shot`, comparado con la captura de antes. Sin
paso nuevo en el guion: no hay interacción nueva.

## Para quien lo construya

Medidas finas (los 12 px, el sitio exacto de la raya) a ojo con la captura, partiendo de las
de Warp; mantener la caché de filas de la rejilla intacta (lo nuevo se dibuja con egui encima,
como las cajas).
