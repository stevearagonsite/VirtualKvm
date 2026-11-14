# Diseño: layout por monitor + interfaz gráfica

> VirtualKvm está basado en [Lan Mouse](https://github.com/feschber/lan-mouse) (Ferdinand Schober y contribuyentes, GPL-3.0-or-later). Ver [NOTICE](../../NOTICE).

> Estado: implementado (branch `feat/layout-gui`, PR #4) · Fase 3 de `PLAN.md`. Binario: `vkvm`.

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

## Modelo (v2): un escritorio virtual compartido

> v1 era "un borde + offset por cliente". No alcanzaba: en el setup de Steve el monitor
> de la Windows toca **dos** bordes (el izquierdo del LF27T35 y el de arriba de la laptop).

Cada cliente tiene un **layout**: dónde están los monitores del peer en nuestro escritorio,
como una traslación. Un punto `(x, y)` del peer aparece en `(x + dx, y + dy)` del nuestro.

```toml
[[clients]]
ips = ["192.168.0.210"]
activate_on_startup = true

[clients.layout]   # Mac: la Windows arriba a la izquierda
dx = -993
dy = -1080
```

- **Bordes activos:** todos los tramos donde un monitor del peer toca uno nuestro (±2 px).
  La captura recibe un `Segment` por tramo, cada uno con su lado.
- **Entrada:** el punto del cruce, trasladado al peer (`EnterAt`), 1 px adentro del borde por el que entra.
- **Vuelta:** el peer manda dónde salió (`LeaveAt`). Se traslada y se ubica en el monitor local más cercano.
- **`position`** (la del upstream) se deriva sola: el lado del contacto más largo.
- **Compartido:** al guardar en una máquina se envía `LayoutSync { dx, dy }` y la otra guarda
  el espejo (`-dx, -dy`). Las dos UIs muestran lo mismo.
- **Monitores del peer:** se intercambian (`DisplayInfo`) al conectar. El daemon sondea cada 5 s a los
  clientes activos cuyos monitores no conoce, así el layout funciona antes del primer cruce.
- **IP cambiada (DHCP):** si un peer autorizado llega desde una IP desconocida y hay un solo cliente,
  se lo asocia y se suma la IP a la config (hasta que exista discovery por mDNS).

Limitaciones actuales:
- En Windows la captura todavía no soporta tramos: la vuelta Windows → Mac usa el borde completo de
  `position`. En el setup de Steve funciona por el borde derecho de la Windows, no por el de abajo.
- Los monitores del peer se ven como "Monitor N" (el nombre no viaja en `DisplayInfo`).

## Cambios por capa

| Capa | Cambio |
|---|---|
| `lan-mouse-proto` | Eventos nuevos (21 bytes máx.): `EnterAt{pos, x, y}`, `LeaveAt{x, y}`, `DisplayInfo{idx, count, x, y, w, h, scale%}`, `LayoutSync{present, dx, dy}`. Los peers viejos los ignoran (forward-compat ya existente) |
| `input-capture` | `CaptureEvent::BeginAt{x, y}` con el punto de cruce. En macOS: barrera limitada a un monitor y un tramo (`create_with_segment`). En Windows (después): lo mismo |
| `input-emulation` | `Emulation::warp(x, y)` con no-op por defecto. Windows: `SetCursorPos`. macOS: `CGWarpMouseCursorPosition` |
| `vkvm-displays` (crate nuevo) | Enumeración de monitores: macOS (CoreGraphics) y Windows (`EnumDisplayMonitors` + `GetDpiForMonitor`) |
| daemon (`src/`) | El receptor manda sus `DisplayInfo` al recibir `Hello`. El emisor los guarda por cliente. Mapeo de entrada y salida con el link. Config `[clients.link]` |
| `lan-mouse-ipc` | `FrontendRequest::{GetDisplays, UpdateLayout}`, `FrontendEvent::Displays`, monitores del peer en `ClientState` |
| UI web (`vkvm web`) | `axum` en `127.0.0.1:4243` con token. Una página (HTML + JS, sin build) dibuja los monitores locales en su posición real; los remotos se arrastran, se pegan al borde más cercano y guardan el link |

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
