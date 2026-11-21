#!/bin/sh
# Compila en release y arma target/Colibri.app para abrirla desde Finder o el Dock.
set -e
cd "$(dirname "$0")/.."
cargo build --release
# La versión sale de Cargo.toml (única fuente: «0.2.0-beta.1»); el número de compilación
# es el de commits, que solo crece. «Acerca de Colibrí» enseña «versión (compilación)».
VERSION=$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -1)
BUILD=$(git rev-list --count HEAD 2>/dev/null || echo 0)
APP=target/Colibri.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp target/release/colibri "$APP/Contents/MacOS/colibri"
mkdir -p "$APP/Contents/Resources"
cp assets/Colibri.icns "$APP/Contents/Resources/Colibri.icns"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>Colibrí</string>
  <key>CFBundleDisplayName</key><string>Colibrí</string>
  <key>CFBundleIdentifier</key><string>dev.colibri.terminal</string>
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
# Firma con una identidad estable para que macOS recuerde los permisos (carpetas,
# notificaciones) entre compilaciones. Con firma ad-hoc cada compilación cuenta como una
# app nueva y vuelve a preguntar. Otra identidad: COLIBRI_SIGN_ID="<hash o nombre>".
SIGN_ID="${COLIBRI_SIGN_ID:-EAB96628E356EEBE58D3594A2CAEEDED0DDE2F10}"
if ! codesign --force --sign "$SIGN_ID" "$APP" 2>/dev/null; then
  echo "Aviso: no se pudo firmar con $SIGN_ID; uso firma ad-hoc (los permisos no persistirán)."
  codesign --force --sign - "$APP"
fi
echo "Lista: $APP  (ábrela con: open $APP)"
