#!/usr/bin/env bash
# Instala la última versión de VirtualKvm en macOS (Apple Silicon):
#   curl -fsSL https://install.stevearagonsite.com/virtualkvm/install.sh | bash
# VirtualKvm está basado en Lan Mouse (https://github.com/feschber/lan-mouse), GPL-3.0-or-later.
set -euo pipefail
BASE="https://install.stevearagonsite.com/virtualkvm/latest"
ZIP="VirtualKvm-macos-arm64.zip"
[[ "$(uname -s)" == "Darwin" ]] || { echo "este instalador es para macOS" >&2; exit 1; }
[[ "$(uname -m)" == "arm64" ]] || { echo "por ahora solo hay versión para Apple Silicon (arm64)" >&2; exit 1; }
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
echo "Descargando VirtualKvm…"
curl -fsSL "$BASE/$ZIP" -o "$TMP/$ZIP"
curl -fsSL "$BASE/SHA256SUMS" -o "$TMP/SHA256SUMS"
expected="$(awk -v f="$ZIP" '$2 == f {print $1}' "$TMP/SHA256SUMS")"
actual="$(shasum -a 256 "$TMP/$ZIP" | awk '{print $1}')"
[[ -n "$expected" && "$expected" == "$actual" ]] || { echo "el checksum SHA-256 no coincide: descarga cancelada" >&2; exit 1; }
unzip -q "$TMP/$ZIP" -d "$TMP"
bash "$TMP/VirtualKvm/install.sh" "$@"
