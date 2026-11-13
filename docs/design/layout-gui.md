# Diseño: layout por monitor + interfaz gráfica

> Estado: en implementación (branch `feat/layout-gui`) · Fase 3 de `PLAN.md`.

## Problema

Setup de Steve:

```
┌──────────────┐┌──────────────┐
│   Windows    ││ Mac LF27T35  │   ← secundario de la Mac (1920×1080)
│  1920×1080   ││  1920×1080   │
└──────────────┘└──┬───────────┘
              ┌────┴──────┐
              │ Mac laptop │      ← principal (1512×982)
              └───────────┘
```

Lan Mouse hoy:
1. Calcula el borde sobre el rectángulo que envuelve **todas** las pantallas: el "borde izquierdo" es el de la laptop (x=0), no el del secundario (x=927).
2. `Enter(Position)` no dice **a qué altura** se cruzó. El cursor aparece en la Windows donde había quedado.
3. Al volver pasa lo mismo: el cursor reaparece en la Mac donde se fue, no a la altura por la que salió de la Windows.
4. No hay UI para ubicar los monitores.

## Modelo: un "link" por cliente

Cada cliente puede tener un **link**: un monitor local, un monitor remoto, el lado donde está el remoto
y un **offset**, que es dónde arranca el remoto a lo largo de ese borde, en píxeles lógicos locales.

```toml
[[clients]]
position = "left"
ips = ["192.168.0.210"]
activate_on_startup = true

[clients.link]
local_display = "LF27T35"     # nombre o id del monitor local
remote_display = 0            # índice del monitor remoto (lo informa el peer)
offset = 0                    # top del remoto - top del local (en px lógicos locales)
```

Sin `link`, el comportamiento es el de Lan Mouse (borde del escritorio completo). Es retrocompatible.

### Geometría
- Tramo activo del borde: la intersección entre el borde del monitor local y el remoto ubicado con `offset`.
  Fuera de ese tramo, el cursor se frena como en un borde normal.
- Al cruzar en (x_l, y_l), el punto de entrada remoto es
  `y_r = remote.y + (y_l - (local.y + offset)) * remote.h / remote.h_logico`. La x es el borde opuesto
  del remoto (1 px adentro). Para `top`/`bottom` es lo mismo con los ejes intercambiados.
- Al volver, el peer manda su punto de salida y la Mac hace la cuenta inversa para ubicar el cursor.

## Cambios por capa

| Capa | Cambio |
|---|---|
| `lan-mouse-proto` | Eventos nuevos (21 bytes máx.): `EnterAt{pos, x, y}`, `LeaveAt{x, y}`, `DisplayInfo{idx, count, x, y, w, h, scale%}`. Los peers viejos los ignoran (forward-compat ya existente) |
| `input-capture` | `CaptureEvent::BeginAt{x, y}` con el punto de cruce. En macOS: barrera limitada a un monitor y un tramo (`create_with_segment`). En Windows (después): lo mismo |
| `input-emulation` | `Emulation::warp(x, y)` con no-op por defecto. Windows: `SetCursorPos`. macOS: `CGWarpMouseCursorPosition` |
| `vkvm-displays` (crate nuevo) | Enumeración de monitores: macOS (CoreGraphics) y Windows (`EnumDisplayMonitors` + `GetDpiForMonitor`) |
| daemon (`src/`) | El receptor manda sus `DisplayInfo` al recibir `Hello`. El emisor los guarda por cliente. Mapeo de entrada y salida con el link. Config `[clients.link]` |
| `lan-mouse-ipc` | `FrontendRequest::{GetDisplays, UpdateLink}`, `FrontendEvent::Displays{local, remote}` |
| UI web (`lan-mouse web`) | `axum` en `127.0.0.1:4243` con token. Una página (HTML + JS, sin build) dibuja los monitores locales en su posición real; los remotos se arrastran, se pegan al borde más cercano y guardan el link |

## Orden de implementación

1. `vkvm-displays` + IPC `GetDisplays` (local).
2. Proto `DisplayInfo` + intercambio al conectar → displays remotos en la Mac.
3. Link en la config + barrera por monitor en macOS + `EnterAt` + `warp` en Windows → **entrada alineada**.
4. UI web: editor de layout que guarda el link.
5. `LeaveAt` → **vuelta alineada**.
6. Lo mismo en la dirección Windows → Mac (captura en Windows).

## Seguridad
- La UI escucha solo en loopback, exige un token en cada request (archivo 0600 en el directorio de config)
  y valida `Host`/`Origin` contra DNS rebinding.
- Nota aparte: la IPC de Windows del upstream (`127.0.0.1:5252`) no tiene autenticación. Queda para la fase 5.
