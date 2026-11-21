#!/bin/sh
# Compila en release y arma target/Colibri.app para abrirla desde Finder o el Dock.
#
# Para publicar (lo llama scripts/release.sh), COLIBRI_RELEASE=1: binario universal (Apple
# silicon + Intel), firma obligatoria con un certificado «Developer ID Application» y sello de
# tiempo de Apple, como pide la notarización. COLIBRI_ARCHS cambia las arquitecturas, por
# ejemplo «aarch64» si tu Rust solo trae esa (el de Homebrew no trae la otra; con rustup:
# `rustup target add x86_64-apple-darwin`).
set -e
cd "$(dirname "$0")/.."
if [ -n "$COLIBRI_RELEASE" ]; then
  BINS=""
  for ARCH in ${COLIBRI_ARCHS:-aarch64 x86_64}; do
    cargo build --release --target "$ARCH-apple-darwin"
    BINS="$BINS target/$ARCH-apple-darwin/release/colibri"
  done
  mkdir -p target/universal
  # shellcheck disable=SC2086 # BINS es una lista de rutas sin espacios
  lipo -create $BINS -output target/universal/colibri
  EXE=target/universal/colibri
else
  cargo build --release
  EXE=target/release/colibri
fi
# La versión sale de Cargo.toml (única fuente: «0.2.0-beta.1»); el número de compilación
# es el de commits, que solo crece. «Acerca de Colibrí» enseña «versión (compilación)».
VERSION=$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -1)
BUILD=$(git rev-list --count HEAD 2>/dev/null || echo 0)
APP=target/Colibri.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp "$EXE" "$APP/Contents/MacOS/colibri"
mkdir -p "$APP/Contents/Resources"
cp assets/Colibri.icns "$APP/Contents/Resources/Colibri.icns"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>Colibrí</string>
  <key>CFBundleDisplayName</key><string>Colibrí</string>
  <key>CFBundleIdentifier</key><string>io.github.jusevasa.colibri</string>
  <key>CFBundleExecutable</key><string>colibri</string>
  <key>CFBundleIconFile</key><string>Colibri</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$BUILD</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <!-- Idiomas de la interfaz (src/i18n): sin declararlos, macOS no ofrece Colibrí en
       Ajustes → General → Idioma y región → Aplicaciones. -->
  <key>CFBundleDevelopmentRegion</key><string>es</string>
  <key>CFBundleLocalizations</key><array><string>es</string><string>en</string></array>
  <!-- Lo que macOS enseña cuando un programa de la terminal pide estos permisos (ante macOS
       cuentan como de Colibrí). Van con los derechos de assets/Colibri.entitlements; en
       inglés, en en.lproj/InfoPlist.strings. -->
  <key>NSAppleEventsUsageDescription</key><string>Un programa que ejecutas en Colibrí quiere controlar otra aplicación.</string>
  <key>NSMicrophoneUsageDescription</key><string>Un programa que ejecutas en Colibrí quiere usar el micrófono.</string>
  <key>NSCameraUsageDescription</key><string>Un programa que ejecutas en Colibrí quiere usar la cámara.</string>
  <!-- Carpetas: «Abrir con → Colibrí», soltarlas en el icono del Dock y
       open -a Colibri <carpeta>. Alternate: no se queda con las carpetas por defecto. -->
  <key>CFBundleDocumentTypes</key><array><dict>
    <key>CFBundleTypeName</key><string>Carpeta</string>
    <key>CFBundleTypeRole</key><string>Viewer</string>
    <key>LSHandlerRank</key><string>Alternate</string>
    <key>LSItemContentTypes</key><array><string>public.folder</string></array>
  </dict></array>
  <!-- Clic derecho sobre una carpeta en Finder → Servicios. -->
  <key>NSServices</key><array><dict>
    <key>NSMenuItem</key><dict><key>default</key><string>Nueva pestaña de Colibrí aquí</string></dict>
    <key>NSMessage</key><string>newTab</string>
    <key>NSPortName</key><string>Colibrí</string>
    <key>NSSendFileTypes</key><array><string>public.folder</string></array>
  </dict></array>
</dict></plist>
PLIST
# Una carpeta por idioma (macOS decide con ellas en qué idioma va la app) y el nombre del
# servicio de Finder traducido.
for L in es en; do mkdir -p "$APP/Contents/Resources/$L.lproj"; done
printf '"Nueva pestaña de Colibrí aquí" = "Nueva pestaña de Colibrí aquí";\n' \
  > "$APP/Contents/Resources/es.lproj/ServicesMenu.strings"
printf '"Nueva pestaña de Colibrí aquí" = "New Colibrí Tab Here";\n' \
  > "$APP/Contents/Resources/en.lproj/ServicesMenu.strings"
cat > "$APP/Contents/Resources/en.lproj/InfoPlist.strings" <<'STRINGS'
"NSAppleEventsUsageDescription" = "A program you run in Colibrí wants to control another app.";
"NSMicrophoneUsageDescription" = "A program you run in Colibrí wants to use the microphone.";
"NSCameraUsageDescription" = "A program you run in Colibrí wants to use the camera.";
STRINGS
# Firma. Con una identidad estable macOS recuerda los permisos (carpetas, notificaciones)
# entre compilaciones; con firma ad-hoc cada compilación cuenta como una app nueva y vuelve a
# preguntar. Sin COLIBRI_SIGN_ID se usa el primer «Developer ID Application» del llavero (el
# de las versiones publicadas: así una compilación local y la de Homebrew son la misma app
# para macOS) o, si no hay, el primer «Apple Development».
identity() {
  security find-identity -v -p codesigning 2>/dev/null |
    awk -v kind="\"$1:" 'index($0, kind) { print $2; exit }'
}
SIGN_ID="${COLIBRI_SIGN_ID:-$(identity "Developer ID Application")}"
# Hardened runtime siempre, para que lo que se prueba en local sea lo que se publica.
ENTITLEMENTS=assets/Colibri.entitlements
if [ -n "$COLIBRI_RELEASE" ]; then
  if [ -z "$SIGN_ID" ]; then
    echo "Error: para publicar hace falta un certificado «Developer ID Application» en el llavero." >&2
    exit 1
  fi
  codesign --force --options runtime --timestamp --entitlements "$ENTITLEMENTS" --sign "$SIGN_ID" "$APP"
else
  SIGN_ID="${SIGN_ID:-$(identity "Apple Development")}"
  if [ -z "$SIGN_ID" ] || ! codesign --force --options runtime --timestamp=none \
    --entitlements "$ENTITLEMENTS" --sign "$SIGN_ID" "$APP" 2>/dev/null; then
    echo "Aviso: sin certificado de firma; uso firma ad-hoc (los permisos no persistirán)."
    codesign --force --sign - "$APP"
  fi
fi
echo "Lista: $APP  (ábrela con: open $APP)"
