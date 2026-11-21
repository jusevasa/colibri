# Pestaña unida a la terminal y arco de carga sobre el icono del agente

Diseño corto (cambio pequeño). Primero de los cuatro cambios de diseño pedidos el
2026-11-20; el orden acordado es: esto → panel de agentes a la izquierda → idiomas.

## Por qué

Hoy cada pestaña es una pastilla flotante: no toca la terminal y la activa se ve
desconectada de lo que muestra. Y mientras un agente trabaja, la luna que se llena
sustituye a su icono, así que no se ve qué agente es justo cuando más importa.

## Qué se quiere

1. **Pestaña activa unida a la terminal.** Esquinas redondeadas solo arriba, baja hasta
   el borde inferior de la barra, con el mismo fondo que la terminal (`pal.bg` con la
   misma opacidad que el `CentralPanel`), sin línea entre ambas. Contorno fino
   (`pal.border`) arriba y a los lados; el borde inferior de la barra se dibuja con esa
   misma línea salvo bajo la pestaña activa. Las inactivas siguen como hoy.
2. **Arco de carga alrededor del icono.** Con agente, el icono se queda siempre; mientras
   `busy`, gira a su alrededor un arco del color del agente sobre un anillo tenue. En la
   barra de pestañas y en la lista de agentes del panel. Todos los agentes.
3. **Giro fluido** (elegido por el usuario, opción B): ~15 cuadros/s solo mientras algún
   agente de esa ventana trabaja y la ventana se ve.

## Enfoques considerados

- A) Giro a saltos con el refresco de 250 ms que ya existe: coste cero, se ve a tirones.
- **B) Giro fluido a ~15 cuadros/s mientras trabaja un agente** — elegido: se nota vivo y
  el coste (~2 ms por cuadro) solo existe mientras un agente trabaja.

## Decisiones del diseñador (corregibles)

- Pestaña activa con color propio: el tinte no puede ir en el relleno (rompería la unión
  con la terminal), así que su raya de color pasa al **borde superior** de la pestaña. Las
  inactivas con color siguen igual (relleno tintado + raya abajo).
- Sin agente (un `npm run dev`, un `sleep`) se mantiene la luna a saltos de hoy: un
  servidor en primer plano cuenta como `busy` indefinidamente y no debe tener la app a
  15 cuadros/s.
- Sin «pies» curvos en la base de la pestaña (estilo Chrome); se puede añadir después.

## Restricciones

- Repintado bajo demanda: el refresco rápido se pide con `request_repaint_after` desde
  `ui()` (que no corre con la ventana minimizada) y solo si hay un agente `busy` en esa
  ventana; en reposo, 0 cuadros/s como hoy.
- Arrastre y deslizamiento de pestañas siguen funcionando (la forma unida se mueve con la
  pestaña; el hueco en la línea inferior la sigue).
- Los cuatro temas incluidos deben verse bien (claro incluido).

## Cómo se comprueba

- Capturas del guion (`shot`) con una y varias pestañas, activa con y sin color, y tema
  claro: ningún píxel distinto entre la base de la pestaña activa y la terminal.
- `claude` falso (`#!/bin/sh` + `sleep`) para tener agente `busy`: captura del arco y
  `COLIBRI_FPS` ≈ 15/s mientras trabaja y 0/s en reposo; con un `sleep` sin agente, 4/s.

## Cambio tras verlo (2026-11-20)

El usuario pidió, con una captura de referencia, que el indicador no sea un arco alrededor
del icono sino una **insignia en su esquina superior derecha**: un disco pequeño con puntos
que se iluminan girando mientras trabaja, y un punto verde cuando terminó sin mirarse. Así
queda en la pestaña y en el panel (`badged_icon`). El ritmo de repintado no cambia.
