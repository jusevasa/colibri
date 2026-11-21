#!/bin/sh
# Escribe en la salida el cask de Homebrew para una versión publicada:
#   scripts/cask.sh <versión> <sha256 del zip>
# Lo usa .github/workflows/release.yml para actualizar Casks/colibri.rb en
# jusevasa/homebrew-colibri. Sin `auto_updates` ni `uninstall quit:` a propósito: las
# actualizaciones llegan con `brew upgrade` (Colibrí no abre conexiones de red), el botón
# «Actualizar» aparece solo al cambiar el ejecutable, y cerrar la app al actualizar mataría
# los agentes de sus pestañas.
set -e
VERSION="${1:?falta la versión}"
SHA256="${2:?falta el sha256}"
cat <<CASK
cask "colibri" do
  version "$VERSION"
  sha256 "$SHA256"

  url "https://github.com/jusevasa/colibri/releases/download/v#{version}/Colibri-#{version}.zip"
  name "Colibrí"
  desc "Terminal para correr varios agentes de código en pestañas"
  homepage "https://github.com/jusevasa/colibri"

  depends_on macos: :monterey

  app "Colibri.app"

  zap trash: [
    "~/.config/colibri",
    "~/Library/Caches/colibri",
    "~/Library/Logs/Colibri",
    "~/Library/Preferences/io.github.jusevasa.colibri.plist",
    "~/Library/Saved Application State/io.github.jusevasa.colibri.savedState",
  ]
end
CASK
