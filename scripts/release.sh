#!/bin/bash
# Arma la versión para publicar: Colibri.app universal, firmada con Developer ID, notarizada
# por Apple y con el ticket grapado, comprimida en target/Colibri-<versión>.zip.
#
# Credenciales de notarización: en CI, APPLE_ID, APPLE_APP_PASSWORD (contraseña de app) y
# APPLE_TEAM_ID; en un Mac, el perfil del llavero que guarda
# `xcrun notarytool store-credentials colibri --apple-id <correo> --team-id <equipo>`
# (COLIBRI_NOTARY_PROFILE para usar otro nombre). COLIBRI_ARCHS como en bundle.sh.
set -euo pipefail
cd "$(dirname "$0")/.."

COLIBRI_RELEASE=1 scripts/bundle.sh
VERSION=$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -1)
APP=target/Colibri.app
ZIP="target/Colibri-$VERSION.zip"

if [ -n "${APPLE_ID:-}" ]; then
  AUTH=(--apple-id "$APPLE_ID" --password "$APPLE_APP_PASSWORD" --team-id "$APPLE_TEAM_ID")
else
  AUTH=(--keychain-profile "${COLIBRI_NOTARY_PROFILE:-colibri}")
fi

# ditto conserva la firma y los atributos del bundle (zip -r los rompe).
rm -f "$ZIP"
ditto -c -k --keepParent "$APP" "$ZIP"
echo "Enviando a notarizar (suele tardar unos minutos)…"
OUT=$(xcrun notarytool submit "$ZIP" "${AUTH[@]}" --wait --timeout 30m --output-format json)
STATUS=$(plutil -extract status raw -o - - <<<"$OUT")
ID=$(plutil -extract id raw -o - - <<<"$OUT")
if [ "$STATUS" != "Accepted" ]; then
  echo "Apple no la aceptó ($STATUS). Registro de la notarización $ID:" >&2
  xcrun notarytool log "$ID" "${AUTH[@]}" >&2 || true
  exit 1
fi

# El ticket va dentro de la app para que Gatekeeper la acepte también sin conexión; el zip se
# rehace para que lo lleve.
xcrun stapler staple "$APP"
spctl --assess --type exec "$APP"
rm -f "$ZIP"
ditto -c -k --keepParent "$APP" "$ZIP"
SHA256=$(shasum -a 256 "$ZIP" | cut -d' ' -f1)
echo "Lista: $ZIP"
echo "sha256: $SHA256"
if [ -n "${GITHUB_OUTPUT:-}" ]; then
  printf 'version=%s\nzip=%s\nsha256=%s\n' "$VERSION" "$ZIP" "$SHA256" >> "$GITHUB_OUTPUT"
fi
