# Contribuir a Colibrí

Gracias por querer mejorar Colibrí. Esta guía explica cómo preparar el entorno, probar los cambios y enviar un PR. Las reglas del repositorio y la arquitectura están en [AGENTS.md](AGENTS.md): léelas antes de empezar, valen igual para personas y agentes.

## Antes de empezar

- Para algo más que un arreglo pequeño, abre primero una issue y cuéntalo. Así no trabajas en algo que no encaja.
- Busca si ya hay una issue o un PR sobre lo mismo.

## Preparar el entorno

Hace falta macOS 12 o posterior, [Rust](https://rustup.rs) estable y las herramientas de línea de órdenes de Xcode (`xcode-select --install`).

```sh
cargo run                      # desarrollo
cargo test                     # pruebas (las de metal.rs usan la GPU)
cargo fmt && cargo clippy --all-targets
scripts/bundle.sh              # release + target/Colibri.app firmada
```

Si tocas `vendor/alacritty_terminal`, corre también sus pruebas desde esa carpeta: `cargo test --lib`.

Si usas Claude Code, `.claude/settings.json` trae un gancho que rearma `target/Colibri.app` al acabar cada turno cuando cambió el código. Se desactiva desde `/hooks`.

## Probar la interfaz

No hay pruebas de interfaz; en su lugar hay un guion de depuración que maneja la app y guarda capturas de su propio render (`COLIBRI_SCRIPT`). Está explicado en AGENTS.md, en «Probar la interfaz sin tocarla a mano». Úsalo siempre con `COLIBRI_CONFIG` apuntando a un archivo temporal para no pisar tu configuración. Si tu cambio añade algo a la interfaz, añade también su paso al guion.

## Commits

- Estilo Conventional Commits, en español: `feat: …`, `fix: …`, `docs: …`, `refactor: …`.
- Un cambio con sentido por commit; el mensaje dice qué cambia y, si no es evidente, por qué.

## Pull requests

1. Crea una rama desde `main` (en tu fork si no tienes acceso de escritura).
2. Mantén el PR pequeño y centrado en una cosa.
3. Rellena la plantilla: qué cambia, por qué y cómo lo probaste (los comandos que corriste y qué salió; si toca la interfaz, el guion y la captura).
4. El CI (formato, clippy y pruebas) tiene que pasar.
5. CodeRabbit revisa el PR automáticamente usando AGENTS.md como criterio. Responde a sus comentarios o resuélvelos; si crees que se equivoca, dilo en el hilo.
6. Para fusionar hace falta la aprobación de una persona que mantenga el proyecto.

## Contribuciones hechas con agentes

Son bienvenidas: Colibrí existe para trabajar con agentes y se construye con ellos. Con estas condiciones:

1. **Respondes tú.** Leíste el cambio entero, lo entiendes y lo probaste. Un PR que no sabes explicar se cerrará.
2. **Se declara.** Marca la casilla de la plantilla y deja el trailer en los commits: `Co-Authored-By: <agente> <correo>` (el que ya añade Claude Code) o `Assisted-by: <agente>:<modelo>`.
3. **Tu agente sigue AGENTS.md.** Claude Code, Codex, Cursor y Copilot lo leen solos al trabajar en el repo; Gemini CLI busca `GEMINI.md` por defecto: configúralo para que lea `AGENTS.md`. Si el resultado contradice alguna regla, corrígelo antes de abrir el PR.
4. **Un agente no firma ni aprueba.** Nada de `Signed-off-by` ni aprobaciones puestas por un agente.
5. **Revisa el origen.** Comprueba que no trae código copiado de proyectos con licencias incompatibles.

## Informar de un fallo

Abre una issue con la versión (menú Colibrí → Acerca de Colibrí), la versión de macOS, los pasos para reproducirlo, qué esperabas y qué pasó. Si es un problema de seguridad, no abras una issue pública: usa «Report a vulnerability» en la pestaña Security del repositorio.

## Licencia de las contribuciones

Colibrí se publica bajo la [licencia Apache 2.0](LICENSE). Al enviar una contribución aceptas que se publique bajo esa misma licencia, como dice su sección 5.
