#!/usr/bin/env bash
# VirtualKvm: instalador para macOS (Apple Silicon).
# VirtualKvm está basado en Lan Mouse (https://github.com/feschber/lan-mouse), GPL-3.0-or-later.
#
#   ./install.sh                 instala o actualiza y arranca al iniciar sesión
#   ./install.sh --update-tray   también reemplaza el ícono (obliga a volver a dar el permiso)
#   ./install.sh --uninstall     lo desinstala (deja la configuración)
#
# VKVM_BIN=<ruta> usa ese binario en lugar del que está al lado de este script.
set -euo pipefail

LABEL="com.virtualkvm.tray"
LEGACY_LABEL="com.virtualkvm.lan-mouse"
DEST="$HOME/.local/share/virtualkvm"
BIN="$DEST/vkvm"           # daemon: se actualiza en cada instalación
TRAY="$DEST/VirtualKvm"    # ícono: copia fija, es la que tiene el permiso de macOS
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
LOGDIR="$HOME/Library/Logs/VirtualKvm"
DOMAIN="gui/$(id -u)"
HERE="$(cd "$(dirname "$0")" && pwd)"
SRC="${VKVM_BIN:-$HERE/vkvm}"

if [[ "${1:-}" == "--uninstall" ]]; then
  launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || true
  rm -f "$PLIST" "$BIN" "$TRAY"
  echo "VirtualKvm desinstalado. La configuración queda en ~/.config/virtualkvm."
  echo "Podés quitar 'VirtualKvm' de Ajustes > Privacidad y seguridad > Accesibilidad y Monitoreo de entrada."
  exit 0
fi

[[ "$(uname -s)" == "Darwin" ]] || { echo "este instalador es para macOS" >&2; exit 1; }
[[ -x "$SRC" ]] || { echo "no encuentro el binario: $SRC" >&2; exit 1; }

launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || true
launchctl bootout "$DOMAIN/$LEGACY_LABEL" 2>/dev/null || true
rm -f "$HOME/Library/LaunchAgents/$LEGACY_LABEL.plist" "$DEST/lan-mouse"
mkdir -p "$DEST" "$LOGDIR" "$(dirname "$PLIST")"

cp "$SRC" "$BIN"
xattr -d com.apple.quarantine "$BIN" 2>/dev/null || true
NEW_TRAY=0
if [[ ! -x "$TRAY" || "${1:-}" == "--update-tray" ]]; then
  cp "$SRC" "$TRAY"
  xattr -d com.apple.quarantine "$TRAY" 2>/dev/null || true
  NEW_TRAY=1
fi

cat > "$PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$LABEL</string>
  <key>ProgramArguments</key>
  <array>
    <string>$TRAY</string>
    <string>tray</string>
    <string>--daemon-exe</string>
    <string>$BIN</string>
    <string>--log</string>
    <string>$LOGDIR/daemon.log</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict><key>RUST_LOG</key><string>info</string></dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <key>ThrottleInterval</key><integer>10</integer>
  <key>ProcessType</key><string>Interactive</string>
  <key>StandardOutPath</key><string>$LOGDIR/tray.log</string>
  <key>StandardErrorPath</key><string>$LOGDIR/tray.log</string>
</dict>
</plist>
EOF
plutil -lint "$PLIST" >/dev/null
launchctl bootstrap "$DOMAIN" "$PLIST"
sleep 3

echo "VirtualKvm $("$BIN" --version 2>/dev/null | head -1 | awk '{print $2}') instalado: el ícono arranca solo al iniciar sesión."
echo "Logs: $LOGDIR"
if [[ "$NEW_TRAY" == 1 ]] || tail -20 "$LOGDIR/daemon.log" 2>/dev/null | grep -q "permission is required"; then
  echo
  echo "Falta un paso (una sola vez): en Ajustes > Privacidad y seguridad, agregá"
  echo "  $TRAY"
  echo "en 'Accesibilidad' y en 'Monitoreo de entrada'. Te abro las dos ventanas."
  open -R "$TRAY" || true
  open "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility" || true
  echo "Después reiniciá el ícono: launchctl kickstart -k $DOMAIN/$LABEL"
fi
