# Arrastrar una pestaña fuera de la ventana

## Para qué

Separar una pestaña en su propia ventana, o pasarla a otra ventana de Colibrí, arrastrándola,
como en un navegador. La pestaña se mueve **con su proceso vivo**: un agente que está
trabajando sigue trabajando, con su pantalla e historial.

## Objetivos

- Soltar la pestaña fuera de cualquier ventana de Colibrí → ventana nueva con esa pestaña,
  con su esquina superior izquierda donde se soltó.
- Soltarla sobre otra ventana de Colibrí → pasa a esa ventana: en el hueco de la barra bajo el
  puntero si cae sobre la barra, al final si cae en otro sitio. Queda activa y esa ventana
  pasa al frente.
- El proceso no se reinicia ni se pierde nada de la pestaña (pantalla, historial, nombre,
  color, estado de agente, «terminado sin ver»).

## No objetivos

- Nada que siga al puntero fuera de la ventana (ni ventana en vivo ni miniatura nativa).
  Se podrá añadir después sin rehacer esto.
- La preferencia de no salir al cerrar la última ventana (sigue pendiente aparte).

## Antiobjetivos

- Que el proceso de la pestaña termine o se duplique al moverla.
- Que un arrastre normal para reordenar dispare el despegue por error.
- Que la ventana de origen o la de destino se queden sin repintar la salida de la pestaña.

## Enfoques considerados

1. **Soltar y aparecer** (elegido): la pestaña se marca como «despegada» al salir de la
   barra; al soltar se pregunta a macOS la posición global del puntero y la ventana debajo.
   Poco código y no pelea con winit.
2. Ventana que sigue al puntero (como Chrome): descartado; winit crea ventanas tras el evento
   y el arrastre sigue en la ventana original, así que habría que moverla consultando la
   posición global cada cuadro, con parpadeos y casos de fusión/deshacer.
3. Arrastre nativo (`NSDraggingSession`): descartado; obliga a tomar la `NSView` de winit y
   su ciclo de eventos, frágil ante actualizaciones.

## Diseño

- **Despegue** (`tab_bar`): mientras se arrastra (`drag_tab`), si el puntero se aleja de la
  barra en vertical más de un margen (propuesta: media altura de la barra) o egui lo da por
  perdido (salió de la ventana), la pestaña pasa a «despegada»: se pinta atenuada en su hueco
  y deja de reordenar. Si vuelve a la barra antes de soltar, vuelve al arrastre normal.
- **Al soltar despegada**: `macos.rs` da la posición global del puntero (`NSEvent.mouseLocation`,
  pasada a puntos con origen arriba a la izquierda, como `Win::frame`) y el número de la ventana
  debajo (`NSWindow.windowNumberAtPoint`, que respeta qué tapa a qué). Se busca qué `WindowId`
  de Colibrí tiene ese número.
  - Es la misma ventana → nada (la pestaña vuelve a su hueco).
  - Es otra ventana de Colibrí → se saca la `Session` de `self.w.sessions` y se inserta en la
    `Win` de destino (en `parked`), en el hueco calculado con su marco y el ancho de sus
    pestañas, o al final. Se hace activa y se enfoca esa ventana (como `focus_session`,
    volviendo a la ventana en curso al acabar).
  - No hay ventana de Colibrí → `Fill::Move(Session)` nuevo en `opening` y en `want_windows`
    un marco con origen en el puntero y el tamaño de la ventana de origen; `window_opened` la
    mete como única pestaña.
- **Repintado**: al cambiar de ventana se llama a `Session::set_context` con el contexto de
  la ventana de destino (su `Pace` pedirá cuadros ahí). La caché de la rejilla se rehace sola
  (`GridKey` incluye la generación de la caché de glifos, que es de cada contexto); si no
  bastara, se vacía al mover.
- **Última pestaña de una ventana**: sobre otra ventana → se mueve y la de origen se cierra
  sin apuntarse en ⌘⇧T (no se cerró nada). Fuera de toda ventana → nada (no tiene sentido una
  ventana nueva idéntica).
- **Guion** (`debug.rs` + `run_script`): paso `detach <n> new|<m>` que mueve la pestaña n de
  la ventana del frente a una ventana nueva o a la m-ésima, por el mismo camino que el soltar
  (el puntero simulado no pasa por macOS, así que no puede probar la posición global).
- **Sesión guardada**: no cambia nada; `session.toml` ya guarda las pestañas por ventana.

## Lo que queda al constructor

- Cómo obtener el número de `NSWindow` de cada `WindowId` (desde el `raw_window_handle` de
  winit o guardándolo en `window_opened`).
- El margen exacto del despegue y el aspecto de la pestaña despegada.
- Cuántas pruebas unitarias: al menos el cálculo del hueco de destino y el paso de
  coordenadas de macOS a puntos de egui, que son puros.
