#!/bin/sh
# Regenera target/Colibri.app solo si el código cambió después de la última vez que se armó.
# Lo llama el hook «Stop» de Claude Code (.claude/settings.json) al terminar cada turno, para
# que el Colibrí abierto ofrezca «Actualizar» sin tener que acordarse de correr bundle.sh.
# También sirve a mano: scripts/rebundle.sh
cd "$(dirname "$0")/.." || exit 0
BIN=target/Colibri.app/Contents/MacOS/colibri
if [ -f "$BIN" ] && [ -z "$(find src assets Cargo.toml Cargo.lock scripts/bundle.sh -type f -newer "$BIN" 2>/dev/null | head -1)" ]; then
  exit 0
fi
# La salida es JSON para Claude Code: `systemMessage` es lo que ve la persona.
if LOG=$(scripts/bundle.sh 2>&1); then
  echo '{"systemMessage": "Colibri.app regenerada: pulsa «Actualizar» en Colibrí para tomarla."}'
else
  # Un fallo de compilación no debe bloquear el turno: solo se avisa.
  printf '%s' "$LOG" | tail -15 | jq -Rs '{systemMessage: ("No se pudo regenerar Colibri.app (scripts/bundle.sh falló):\n" + .)}'
fi
exit 0
