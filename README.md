# VirtualKvm

> **VirtualKvm está basado en [Lan Mouse](https://github.com/feschber/lan-mouse)**, de Ferdinand Schober
> ([@feschber](https://github.com/feschber)) y los contribuyentes de Lan Mouse. Es un fork: todo el código
> base viene de ese proyecto y su historia de git completa está en este repo. VirtualKvm mantiene la misma
> licencia, **GPL-3.0-or-later**, y es open source. Ver [LICENSE](LICENSE) y [NOTICE](NOTICE).
>
> *English: VirtualKvm is a fork of Lan Mouse, keeping its history, authorship and GPL-3.0-or-later license.*

VirtualKvm comparte **un mouse y un teclado entre varias computadoras de la misma red local**, como si
sus monitores formaran un solo escritorio. Es un "KVM por software", pensado para usarse **solo en la LAN**.

## Qué agrega VirtualKvm sobre Lan Mouse

| | Lan Mouse | VirtualKvm |
|---|---|---|
| Ubicación de las máquinas | Un borde por cliente (izquierda/derecha/arriba/abajo) | **Layout por monitor**: los monitores de la otra máquina se ubican como un grupo y el mouse cruza por **todos** los bordes que se tocan, entrando y volviendo a la misma altura |
| Configuración | GUI GTK o archivo | **UI web en localhost** para arrastrar los monitores, **compartida**: lo que guardás en una máquina se aplica en la otra |
| Uso diario | Ventana / daemon | **Ícono en la barra de menú (macOS) o en la bandeja (Windows)** para prender o apagar, con arranque al iniciar sesión |
| Portapapeles | No | **Texto que viaja con el cursor**. En macOS ignora el contenido oculto de los gestores de contraseñas |
| Agentes de IA | No | **`vkvm mcp`**: los Claude Code de cada máquina se mandan mensajes de texto por la conexión de VirtualKvm, sin claves SSH ([guía](docs/guia-claude-code.md)) |
| Opciones | — | Idioma (es/en), portapapeles, invertir el scroll del mouse remoto (por ejemplo, para LinearMouse) |
| IP que cambia (DHCP) | Hay que actualizarla a mano | El peer se reconoce por su certificado y la IP nueva se suma sola |

Plataformas probadas: **macOS** (Apple Silicon) y **Windows 11**. Linux mantiene el soporte de Lan Mouse
(Wayland/X11); las funciones nuevas todavía no están probadas ahí. Hoja de ruta: [PLAN.md](PLAN.md).

## Instalar

Descargas y versiones: **[install.stevearagonsite.com/virtualkvm](https://install.stevearagonsite.com/virtualkvm/)** (por ahora macOS
Apple Silicon y Windows x64).

```sh
# macOS (Terminal)
curl -fsSL https://install.stevearagonsite.com/virtualkvm/install.sh | bash
```
```powershell
# Windows (PowerShell como Administrador)
irm https://install.stevearagonsite.com/virtualkvm/install.ps1 | iex
```

Los instaladores bajan la última versión, verifican el SHA-256 e instalan el ícono, que arranca al iniciar sesión.

### Desde el código

Requiere Rust estable. En Windows, además, Visual Studio Build Tools con C++.

```sh
cargo build --release --no-default-features
```

| Sistema | Instalar y arrancar al iniciar sesión |
|---|---|
| macOS | `scripts/dev/macos-install-autostart.sh`. La primera vez, en Ajustes → Privacidad y seguridad, agregá `~/.local/share/virtualkvm/VirtualKvm` en **Accesibilidad** y **Monitoreo de entrada** |
| Windows | `scripts\dev\windows-install-autostart.ps1 -User <usuario>`, en PowerShell como Administrador. Abre el puerto 4242 solo para la red local |

Cada máquina tiene que autorizar el **fingerprint del certificado** de la otra en `[authorized_fingerprints]`
de su config: `~/.config/virtualkvm/config.toml` en macOS o `%LOCALAPPDATA%\virtualkvm\config.toml` en Windows.

## Usar

- **Ícono → Prender / Apagar**: activa o detiene VirtualKvm (se recuerda al reiniciar).
- **Ícono → Monitores y opciones…**: arrastrá los monitores de la otra máquina hasta que toquen los tuyos y guardá.
- **Recuperar el mouse y el teclado**: **Ctrl + Shift + Win + Alt** (en macOS, **Ctrl + Shift + Cmd + Option**).
- **Gestos del trackpad en la Windows**: con el cursor en la Windows, deslizar tres dedos hacia arriba o abajo en el trackpad de la Mac manda **Win + Tab** (abre y cierra la vista de tareas), y hacia los costados **Win + Ctrl + ← / →** (cambia de escritorio virtual). Ningún gesto actúa en la Mac mientras tanto. Con el cursor en la Mac, los gestos son los de siempre.
- **Si una Windows no responde**: **Ctrl + Alt + Supr** → Administrador de tareas → finalizar `vkvm.exe`.
  Más en [docs/design/safety.md](docs/design/safety.md).

| Comando | Para qué |
|---|---|
| `vkvm tray` | Ícono de la bandeja (lanza y apaga el daemon) |
| `vkvm daemon` | Solo el servicio, sin ícono |
| `vkvm web` | UI de monitores y opciones en `127.0.0.1:4243` (con token) |
| `vkvm mcp` | Servidor MCP para agentes de IA ([guía](docs/guia-claude-code.md)) |
| `vkvm cli …` | Línea de comandos de Lan Mouse |

## Documentación

- [PLAN.md](PLAN.md): objetivo, research y hoja de ruta.
- [docs/design/layout-gui.md](docs/design/layout-gui.md): layout compartido, protocolo y UI.
- [docs/design/safety.md](docs/design/safety.md): no perder nunca el control de una máquina.
- [docs/guia-claude-code.md](docs/guia-claude-code.md): conectar Claude Code entre máquinas.
- [docs/release.md](docs/release.md): cómo se publican los instalables.
- [docs/lan-mouse/README.md](docs/lan-mouse/README.md): **README original de Lan Mouse** (backends, compilación en
  cada sistema, configuración avanzada).

## Licencia y créditos

- **Base:** [Lan Mouse](https://github.com/feschber/lan-mouse), © Ferdinand Schober y los contribuyentes de Lan Mouse, GPL-3.0-or-later.
- **VirtualKvm:** © 2026 Steve Aragon y los contribuyentes de VirtualKvm, también GPL-3.0-or-later.
- Idea de ignorar el portapapeles oculto de los gestores de contraseñas: [Mousehop](https://github.com/jondkinney/mousehop), otro fork de Lan Mouse.
- Detalle en [NOTICE](NOTICE). Los cambios que sirvan en general se van a ofrecer de vuelta a Lan Mouse.
