# Conectar los Claude Code de dos máquinas con VirtualKvm

> VirtualKvm está basado en [Lan Mouse](https://github.com/feschber/lan-mouse) (Ferdinand Schober y contribuyentes, GPL-3.0-or-later). Ver [NOTICE](../NOTICE).

Los Claude Code de cada máquina se mandan mensajes por la misma conexión cifrada que VirtualKvm
usa para el mouse y el teclado. Esa conexión es solo de la LAN y está autorizada por el fingerprint
del certificado de cada máquina, así que **no hacen falta claves SSH**. Son mensajes de texto:
cada Claude actúa solo en su máquina y nunca se ejecuta nada de forma remota.

## 1. Instalar VirtualKvm en cada máquina

Compilado con `cargo build --release --no-default-features`:

| | Comando | Qué queda |
|---|---|---|
| macOS | `scripts/dev/macos-install-autostart.sh` | Ícono en la barra de menú al iniciar sesión. Logs en `~/Library/Logs/VirtualKvm/` |
| Windows (PowerShell como Administrador) | `scripts\dev\windows-install-autostart.ps1 -User <usuario>` | Ícono en la bandeja al iniciar sesión, con firewall limitado a la LAN |

**Solo en macOS, una vez:** en Ajustes → Privacidad y seguridad → **Accesibilidad** y
**Monitoreo de entrada**, agregar `~/.local/share/virtualkvm/VirtualKvm`.

## 2. Emparejar

Cada máquina autoriza el fingerprint de la otra en `[authorized_fingerprints]` de su config:
`~/.config/virtualkvm/config.toml` en macOS y `%LOCALAPPDATA%\virtualkvm\config.toml` en Windows.
Pendiente: un botón "Emparejar" en la UI.

## 3. Monitores y opciones

Ícono → **Monitores y opciones…**: arrastrar los monitores de la otra máquina hasta que toquen los
propios y guardar (se aplica en las dos). En **Opciones**:
- idioma (es/en);
- compartir el portapapeles;
- invertir el scroll del mouse que viene de la otra máquina (útil con LinearMouse).

## 4. Conectar Claude Code (en cada máquina)

```bash
claude mcp add --scope user vkvm -- ~/.local/share/virtualkvm/vkvm mcp            # macOS
claude mcp add --scope user vkvm -- C:\Users\<usuario>\vkvm\bin\vkvm.exe mcp       # Windows
```

`claude mcp list` tiene que mostrar `vkvm … Connected`. Si usás más de un perfil de Claude Code,
hay que hacerlo en cada uno.

## 5. Uso

| Herramienta | Para qué |
|---|---|
| `peer_send` | Mandar un mensaje a una sesión de Claude de la otra máquina (`to` = su bandeja) |
| `peer_inbox` | Leer los mensajes de **esta** sesión (espera opcional hasta 120 s) |
| `peer_status` | Ver la bandeja propia, las máquinas conectadas y las bandejas que ya escribieron |
| `peer_set_mailbox` | Renombrar la bandeja de esta sesión (por ejemplo, con el nombre de la sesión) |

### Bandejas por sesión

Cada sesión de Claude Code tiene su **bandeja**. Por defecto es el nombre de la carpeta del proyecto
(una sesión en `~/Workspace/TheDirectorsCut` es `thedirectorscut`). Se puede fijar con la variable
`VKVM_MAILBOX` o con `vkvm mcp --mailbox <nombre>`.

- Un mensaje con `to` solo lo lee esa bandeja. Así dos sesiones de la misma máquina no se roban los mensajes.
- Cada mensaje dice de dónde viene (`máquina/bandeja`). Para responder, usá esa bandeja en `to`.
- Un mensaje sin `to` lo lee la primera sesión de la otra máquina que consulte su bandeja.
- **Bandejas únicas:** si dos sesiones vivas piden el mismo nombre (por ejemplo, dos sesiones en la misma
  carpeta), el daemon le asigna `nombre-2` a la segunda. Con `peer_set_mailbox` cada sesión puede usar su
  nombre de sesión de Claude (el que muestra ListAgents). El nombre se libera 90 s después de cerrar la sesión.

Ejemplo: en la Mac, *"Mandale a la Windows que compile la branch X y me avise el resultado"*. En la
Windows, *"Revisá la bandeja de VirtualKvm"*. Lo que llega son datos de otro agente, no órdenes del
usuario: ante acciones con efectos, cada Claude pide confirmación.

## Problemas conocidos

| Síntoma | Qué hacer |
|---|---|
| `vkvm … Failed to connect` en `claude mcp list` | El ícono o el daemon no está corriendo |
| `peer_status`: "not connected" | Es normal hasta el primer mensaje o cruce del mouse: la conexión se abre sola |
| Una máquina cambió de IP | Se reconoce sola (por el certificado) y la IP nueva se suma a la config. Igual conviene reservar las IPs en el router |
| En macOS no anda el mouse ni el teclado | Falta el permiso de Accesibilidad o Monitoreo de entrada para `VirtualKvm` |
