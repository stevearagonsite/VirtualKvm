#!/usr/bin/env bash
# VirtualKvm (dev): instala el binario recién compilado con el instalador de usuario.
# Mismas opciones que packaging/macos/install.sh (--update-tray, --uninstall).
set -euo pipefail
REPO="$(cd "$(dirname "$0")/../.." && pwd)"
VKVM_BIN="$REPO/target/release/vkvm" exec "$REPO/packaging/macos/install.sh" "$@"
