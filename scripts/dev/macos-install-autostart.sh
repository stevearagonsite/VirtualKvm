#!/usr/bin/env bash
# VirtualKvm (dev): instala el icono de la barra de menú (lan-mouse tray) como LaunchAgent que
# arranca al iniciar sesión en la Mac. La bandeja lanza y apaga el daemon.
#
# - Copia el binario compilado a ~/.local/share/virtualkvm (ruta fija: el permiso de
#   Accesibilidad / Monitoreo de entrada se otorga a ESA ruta).
# - LaunchAgent con RunAtLoad; se reinicia solo si se cae ("Salir" lo cierra de verdad).
#   Logs: ~/Library/Logs/VirtualKvm/{tray,daemon}.log
#
# Uso:  scripts/dev/macos-install-autostart.sh            # instala / actualiza y reinicia
#       scripts/dev/macos-install-autostart.sh --uninstall
#
# Ojo: el binario va firmado ad-hoc, así que cada vez que cambia, macOS puede volver a pedir
# los permisos de Accesibilidad y Monitoreo de entrada.
set -euo pipefail

LABEL="com.virtualkvm.lan-mouse"
DEST="$HOME/.local/share/virtualkvm"
BIN="$DEST/lan-mouse"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
LOGDIR="$HOME/Library/Logs/VirtualKvm"
REPO="$(cd "$(dirname "$0")/../.." && pwd)"
DOMAIN="gui/$(id -u)"

if [[ "${1:-}" == "--uninstall" ]]; then
  launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || true
  rm -f "$PLIST"
  echo "desinstalado (el binario queda en $BIN)"
  exit 0
fi

SRC="$REPO/target/release/lan-mouse"
[[ -x "$SRC" ]] || { echo "no existe $SRC: compilá antes con cargo build --release --no-default-features" >&2; exit 1; }

launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || true
mkdir -p "$DEST" "$LOGDIR" "$(dirname "$PLIST")"
# solo copiar si cambió, para no invalidar el permiso de macOS sin necesidad
if ! cmp -s "$SRC" "$BIN"; then
  cp "$SRC" "$BIN"
  echo "binario actualizado: puede que macOS vuelva a pedir Accesibilidad / Monitoreo de entrada"
fi

cat > "$PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$LABEL</string>
  <key>ProgramArguments</key>
  <array>
    <string>$BIN</string>
    <string>tray</string>
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
launchctl print "$DOMAIN/$LABEL" | grep -E "^\s+(state|pid) =" | head -2
if tail -20 "$LOGDIR/daemon.log" | grep -q "accessibility permission is required"; then
  echo
  echo "FALTA PERMISO: Ajustes > Privacidad y seguridad > Accesibilidad y Monitoreo de entrada"
  echo "agregar: $BIN   y después volver a correr este script"
fi
