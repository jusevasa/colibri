# Idiomas: español e inglés

## Por qué

Colibrí se va a compartir con gente que tiene el Mac en inglés. La interfaz debe salir en
el idioma del sistema: español por defecto, inglés para quien tenga el Mac en inglés.

## Objetivos

- Todos los textos que ve el usuario existen en español e inglés: barra de menús, paleta
  de comandos, Preferencias, renombrar, historial, selectores (ramas, carpetas), panel de
  agentes, Puertos y Procesos, barra de pestañas, botón «Actualizar», notificaciones,
  estados («Esperando tu mensaje», «Trabajando…»), tiempos relativos («hace 3 min»), avisos
  y confirmaciones (detener procesos), nombre del servicio de Finder.
- El idioma se decide al arrancar, a partir del idioma del sistema.
- Un texto que falte en un idioma no compila.

## No objetivos

- Traducir el código, los comentarios, los mensajes de commit, `CLAUDE.md` ni lo que se
  escribe en stderr para depurar (`enlace: …`, `menú: no hay…`, `COLIBRI_FPS`): siguen en
  español.
- Opción de idioma en Preferencias: el selector por app de macOS (Ajustes → General →
  Idioma y región → Aplicaciones) ya lo cubre.
- Cambiar de idioma con la app abierta: se aplica al reiniciar (lo hace macOS igual).
- Más idiomas por ahora (el diseño lo deja fácil).

## Anti-objetivos

- Que aparezca una clave, un texto vacío o una mezcla de idiomas en una misma pantalla.
- Que el cambio altere en algo el comportamiento (atajos, guion de pruebas, sesiones
  guardadas, `config.toml`).

## Cómo elige el idioma

- Al arrancar, `i18n::init` lee el primer idioma de `NSLocale.preferredLanguages`. Con un
  bundle que declara sus idiomas, macOS ya pone ahí el elegido para la app (incluido el
  selector por app). `en*` → inglés; `es*` → español; **cualquier otro → español**
  (decisión mía siguiendo «español por defecto»; cambiar a inglés es una línea).
- `COLIBRI_LANG=es|en` lo fuerza (para probar en inglés sin tocar el sistema).
- `scripts/bundle.sh`: `CFBundleDevelopmentRegion` = `es`, `CFBundleLocalizations` =
  `es`, `en`, y `en.lproj/ServicesMenu.strings` (+ `es.lproj`) con el nombre del servicio
  «New Colibrí Tab Here». Sin estas carpetas macOS no ofrece la app en el selector por app.

## Enfoques considerados

- **A (elegido): un archivo con todos los textos.** `src/i18n.rs` con `struct Texts` (un
  campo `&'static str` por texto; los que llevan datos son `fn(...) -> String`) y dos
  constantes, `ES` y `EN`. `i18n::t()` devuelve la del idioma activo (`OnceLock`).
  Compila solo si cada idioma tiene todos los campos; un idioma nuevo es otro bloque en el
  mismo archivo.
- B: `tr!("es", "en")` en cada sitio. Más rápido de migrar, pero las traducciones quedan
  repartidas y un tercer idioma obliga a tocar cada uso.
- C: `fluent` / `rust-i18n` con archivos externos. Dependencias nuevas y claves que solo
  fallan al ejecutar; demasiado para dos idiomas.

## Detalles

- Uso: `t().new_tab`, `(t().minutes_ago)(3)`. Nombres de campo en inglés corto (son
  identificadores); los textos de `ES` son exactamente los de hoy, así la versión en
  español no cambia ni un píxel.
- Los textos se agrupan en el struct por zona (menú, paleta, preferencias, agentes…) con
  un comentario por grupo.
- Plurales y órdenes de palabras distintos se resuelven dentro de la función de cada
  idioma, no con lógica en el sitio de uso.
- **Menú «Ventana»**: `menu.rs` lo reconoce hoy por el título (`== "Ventana"`); pasa a ser
  un campo de `menu::Menu` (`windows: bool`).
- **Guion**: `menu <título>` busca el título en el idioma activo; los guiones existentes
  siguen valiendo porque el guion corre en español salvo con `COLIBRI_LANG=en`.
- Las pruebas que comparan textos de la interfaz fijan el idioma (`ES`) en vez del global.
- Los textos que vienen de fuera no se traducen: títulos que pone el programa de la
  terminal, nombres de carpetas, ramas, salida de `git`/`lsof`/`ps`.

## Comprobación

- Captura del guion con `COLIBRI_LANG=es` idéntica byte a byte a una de antes del cambio.
- Capturas con `COLIBRI_LANG=en` de: barra y panel de agentes, paleta, Preferencias,
  Puertos/Procesos, historial y un selector; revisar que no quede nada en español.
- Un `grep` de cadenas con letras en `src/` fuera de `i18n.rs`, comentarios y stderr, para
  ver que no se escapó ninguna.
- Bundle: `open -a target/Colibri.app` con el sistema en español y forzando inglés por app
  (`defaults write com.<id> AppleLanguages '(en)'` en una copia de prueba).

## Para quien lo construya

- Nombre exacto de cada campo, orden de los grupos y redacción en inglés (concisa, estilo
  macOS: «New Tab», «Close Tab», «Reopen Closed Tab», Title Case en menús, frase normal
  en el resto).
