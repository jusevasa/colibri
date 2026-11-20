#!/bin/sh
# Arma target/Colibri.app con bundle.sh y la comprime en target/Colibri-<versión>-<arq>.zip
# para pasársela a otras personas. No va notarizada: la primera vez macOS pide confirmar
# en Ajustes del Sistema → Privacidad y seguridad → «Abrir igualmente».
set -e
cd "$(dirname "$0")/.."
scripts/bundle.sh
VERSION=$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -1)
ARCH=$(lipo -archs target/Colibri.app/Contents/MacOS/colibri | tr ' ' '+')
ZIP="target/Colibri-$VERSION-$ARCH.zip"
rm -f "$ZIP"
# ditto conserva la firma y los atributos del bundle (zip -r los rompe).
ditto -c -k --keepParent target/Colibri.app "$ZIP"
echo "Listo: $ZIP ($(du -h "$ZIP" | cut -f1))"
