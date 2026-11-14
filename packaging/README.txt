VirtualKvm {{VERSION}}
=====================

Comparte un mouse y un teclado entre las computadoras de tu red local.

VirtualKvm está basado en Lan Mouse (https://github.com/feschber/lan-mouse),
de Ferdinand Schober y sus contribuyentes. Licencia: GPL-3.0-or-later (ver LICENSE y NOTICE).
Código fuente: https://github.com/stevearagonsite/VirtualKvm

Instalar
--------
macOS (Apple Silicon):   ./install.sh
  La primera vez, en Ajustes > Privacidad y seguridad, agregá ~/.local/share/virtualkvm/VirtualKvm
  en "Accesibilidad" y en "Monitoreo de entrada" (el instalador te abre las ventanas).

Windows 10/11 (x64):     PowerShell como Administrador:
  powershell -ExecutionPolicy Bypass -File .\install.ps1

Desinstalar: ./install.sh --uninstall  ·  .\install.ps1 -Uninstall

Después
-------
- Ícono > "Monitores y opciones…": ubicá los monitores de la otra máquina.
- Cada máquina tiene que autorizar el fingerprint de la otra en [authorized_fingerprints]
  (~/.config/virtualkvm/config.toml en macOS, %LOCALAPPDATA%\virtualkvm\config.toml en Windows).
- Recuperar el control: Ctrl + Shift + Win + Alt (macOS: Ctrl + Shift + Cmd + Option).
- Si una Windows no responde: Ctrl + Alt + Supr > Administrador de tareas > finalizar vkvm.exe.

Documentación: https://github.com/stevearagonsite/VirtualKvm
