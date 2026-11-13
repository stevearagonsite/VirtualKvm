# VirtualKvm — Plan y research inicial

> Fecha: 2026-11-12 · Estado: **Fase 0 (research)**.
> **VirtualKvm es un fork de [Lan Mouse](https://github.com/feschber/lan-mouse)** (Ferdinand Schober y contribuyentes),
> tomado del commit `69958c6` (después de v0.11.0). Mantiene su historia, su autoría y su licencia
> **GPL-3.0-or-later**, y sigue siendo open source. Ver [NOTICE](NOTICE).
> Repo: https://github.com/stevearagonsite/VirtualKvm · Upstream: `feschber/lan-mouse` (remote `upstream`).

## 1. Objetivo

Construir en Rust un "share mouse": un solo teclado y mouse que se mueven entre varias
computadoras de la LAN, como si los monitores de todas formaran un único escritorio. Es el
primer paso hacia un KVM completo. El video viene después y no entra en este alcance.

Partimos de Lan Mouse, que ya resuelve la captura y emulación de entrada en macOS, Windows y
Linux (Wayland/X11), la red cifrada (DTLS) y la autorización por fingerprint. Encima agregamos lo
que nos falta (§4).

| Máquina | Rol inicial | Rol futuro |
|---|---|---|
| Mac | **Servidor** (dueña del teclado y el mouse físicos) | Cliente o servidor |
| Windows (casi siempre apagada) | Cliente | **Servidor** opcional (se elige en la UI) |
| Ubuntu Desktop o Server | — | Cliente (Server: teclado a la consola TTY + TUI) |

Requisitos que no se negocian:

- **Solo LAN.** Nada expuesto a Internet: sin UPnP, sin relays y sin puertos abiertos en el router.
- **Interfaz web en `localhost`** para decidir la distribución de monitores y a qué máquina pertenece cada uno.
- **CLI/TUI** para máquinas sin entorno gráfico (Ubuntu Server).
- **Atribución clara y misma licencia.** Es un fork público de Lan Mouse, no un proyecto "desde cero".

### Fuera de alcance por ahora
Streaming de video o escritorio remoto, archivos, audio, acceso por Internet, móviles.

## 2. Glosario

- **Nodo**: cada máquina que corre el daemon. Todas corren el mismo binario.
- **Servidor**: el nodo que tiene el teclado y el mouse físicos y *captura* la entrada.
- **Cliente**: el nodo que *emula* la entrada que recibe.
- **Layout**: el mapa global de monitores (posición, tamaño, escala y dueño de cada uno).
- **Borde**: el segmento compartido entre dos monitores de nodos distintos. Cuando el cursor lo cruza, el foco pasa al otro nodo.
- **Upstream**: el repo original `feschber/lan-mouse`.

## 3. Estado del arte

| Proyecto | Lenguaje / licencia | Estado (nov-2026) | Qué nos sirve |
|---|---|---|---|
| [Lan Mouse](https://github.com/feschber/lan-mouse) | Rust · GPL-3.0 | Activo. v0.11.0 (jun-2026), último push nov-2026 | **Nuestra base (fork).** Arquitectura capture → red → emulation. UDP + DTLS (webrtc-rs) en el puerto 4242, autorización por *fingerprint* de certificado, modo daemon + CLI y GUI GTK |
| [Mousehop](https://github.com/jondkinney/mousehop) | Rust · GPL-3.0 | Fork de Lan Mouse (abr-2026), joven | Portapapeles de texto (4 KiB, opt-in por dirección, suprime `ConcealedType` en macOS). Misma licencia, así que se puede traer con atribución |
| [Deskflow](https://github.com/deskflow/deskflow) | C++ · GPL-2.0 | Activo, sucesor oficial de Synergy 1 | TLS por defecto. libei + libportal en Wayland. Corrigió el CVE-2026-41477 (IPC del daemon de Windows) |
| Input Leap | C++ | **Archivado el 26-jul-2026.** Vulnerabilidad sin parche: IPC sin autenticar en `input-leapd` → ejecución como SYSTEM ([oss-sec](https://seclists.org/oss-sec/2026/q3/938)) | Lección de seguridad para el servicio de Windows |
| Barrier | C++ | Abandonado. Solo X11 | — |
| [rkvm](https://github.com/htrefil/rkvm) / evkvm | Rust | Solo Linux | Linux con evdev + uinput **sin importar el display server** (X11, Wayland o TTY) |
| Mouse Without Borders (PowerToys) | C# · solo Windows | Activo | Necesita un **servicio** de Windows para llegar a la pantalla de bloqueo y a UAC |

## 4. Decisión: fork de Lan Mouse (GPL-3.0-or-later)

**Decisión (2026-11-12):** VirtualKvm es un fork público de Lan Mouse, con la misma licencia.

- **Atribución:** README con aviso al principio, archivo `NOTICE` y toda la historia de git del original. No se borra ningún copyright ni autoría existente.
- **Licencia:** todo lo que agreguemos también es GPL-3.0-or-later. Las dependencias nuevas tienen que ser compatibles con GPL-3; el `deny.toml` del upstream ya lo controla.
- **Upstream:** traemos sus cambios seguido (`git fetch upstream && git merge upstream/main`). Lo que sea útil en general (por ejemplo, el backend uinput) se ofrece de vuelta como PR a Lan Mouse.
- **Divergencia mínima:** el código nuevo va en crates nuevos. Los crates existentes se tocan lo menos posible para que los merges con el upstream sean simples. El binario y los nombres de crates (`lan-mouse`, `input-capture`, …) se mantienen por ahora; renombrar es una decisión para más adelante.

### Lo que Lan Mouse no tiene y agregamos

| Falta en Lan Mouse | Qué agregamos | Fase |
|---|---|---|
| Sin backend **uinput**: sin entorno gráfico no recibe nada, así que Ubuntu Server queda afuera | Backend `uinput` en `input-emulation` (candidato a PR upstream) | 4 |
| La posición es un borde por cliente (izq/der/arriba/abajo), **sin layout por monitor** | Enumeración de monitores por SO + editor de layout | 3 |
| UI solo en GTK | **UI web en localhost** + TUI | 3, 4 |
| IPs u hostnames a mano | **Discovery mDNS** | 2 |
| Sin WoL | **Wake-on-LAN** desde la UI | 2 |
| Sin filtros de red | **Garantías LAN-only** (§7) | 2 |
| Sin portapapeles | Texto, tomando como referencia Mousehop | 5 |
| En Windows no llega a login ni UAC; sin mouse físico el cursor no se ve | Servicio de Windows con IPC autenticada; investigar el cursor | 5 |
| Sin captura en X11 | Baja prioridad (Ubuntu usa Wayland por defecto) | — |

## 5. Arquitectura

La de Lan Mouse: un binario simétrico por nodo. Cada nodo puede capturar y emular, y el rol es
configuración. Agregamos una API local y discovery al lado del daemon existente.

```
                 ┌──────────────────────── nodo (Mac) ──────────────────────────────┐
 teclado/mouse → │ input-capture ─→ daemon lan-mouse ─→ UDP + DTLS :4242 ═══════════╪══╗
                 │                       ↑  lan-mouse-ipc                           │  ║ LAN
                 │   vkvm-web (axum, 127.0.0.1) ←── navegador · vkvm-tui · CLI      │  ║ (solo)
                 │   vkvm-displays · vkvm-discovery (mDNS) · vkvm-wol               │  ║
                 └──────────────────────────────────────────────────────────────────┘  ║
                 ┌──────────────────────── nodo (Windows) ──────────────────────────┐  ║
                 │ daemon lan-mouse ─→ input-emulation (SendInput) ─→ SO            │◀═╝
                 │ vkvm-displays reporta monitores (EnumDisplayMonitors)            │
                 └──────────────────────────────────────────────────────────────────┘
```

### Workspace

```
--- upstream (tocar lo mínimo) ---
input-capture/     captura: macos, windows, libei, layer-shell, x11
input-emulation/   emulación: macos, windows, libei, wlroots, x11, portal   ← + uinput (nuevo)
input-event/       eventos y scancodes
lan-mouse-proto/   protocolo de red (cambios = bump de versión)
lan-mouse-ipc/     IPC daemon ↔ frontends
lan-mouse-cli/     CLI
lan-mouse-gtk/     GUI GTK (se mantiene; la web es adicional)
src/               daemon: servicio, DTLS, config
--- nuevos (VirtualKvm) ---
vkvm-displays/     enumeración de monitores y notificación de cambios por SO
vkvm-discovery/    mDNS/DNS-SD
vkvm-wol/          Wake-on-LAN
vkvm-web/          API local (axum + WebSocket) sobre lan-mouse-ipc + frontend embebido (rust-embed)
vkvm-tui/          ratatui, cliente de la misma API
web/               código fuente del frontend
```

## 6. Por plataforma

Lan Mouse ya implementa captura y emulación en macOS, Windows y Wayland/X11. La tabla resume
qué hay debajo y qué nos toca agregar (en **negrita**).

| | **macOS** | **Windows** | **Ubuntu Desktop** | **Ubuntu Server** |
|---|---|---|---|---|
| Captura | `CGEventTap` (existe) | Hooks low-level (existe) | libei / layer-shell (existe) | evdev con grab (**nuevo**, si hace falta) |
| Emulación | `CGEventPost` (existe) | `SendInput` (existe) | libei / wlroots / portal / X11 (existe) | **uinput** (`evdev` crate): el teclado llega a la consola virtual. El mouse solo sirve con `gpm` o apps TUI |
| Monitores (**nuevo**) | `CGGetActiveDisplayList` + `CGDisplayBounds` + escala (`NSScreen.backingScaleFactor`). Cambios con `CGDisplayRegisterReconfigurationCallback` | `EnumDisplayMonitors` + `GetMonitorInfo` + `GetDpiForMonitor`, nombres con `QueryDisplayConfig`. Cambios con `WM_DISPLAYCHANGE`. Requiere `PER_MONITOR_AWARE_V2` | X11: RandR. Wayland: `wl_output` / `xdg-output` | Ninguno: se usa una "pantalla virtual" |
| Permisos | **Accesibilidad** + **Input Monitoring** + **Red local** (macOS 15+; los daemons de launchd y lo que corre desde Terminal están exentos) | Firewall (el upstream trae reglas en `firewall/`). UIPI: no inyecta en ventanas elevadas | Portales: el usuario acepta un diálogo. uinput: grupo `input` o regla udev | Servicio systemd (el upstream trae `service/`) |
| Trampas conocidas | *Secure Event Input* (campos de password, 1Password, "Secure Keyboard Entry") **oculta teclas al tap** | Login, UAC y Ctrl+Alt+Del viven en el **escritorio seguro**: hace falta un servicio SYSTEM + proceso en `Winsta0\Winlogon`. Sin mouse físico el cursor puede no verse | Con uinput relativo no se puede "entrar" en un punto exacto: usar un dispositivo absoluto (tipo tablet) | Sin cursor gráfico |

Crates candidatos para lo nuevo (versiones al 12-nov-2026): `evdev` 0.13, `mdns-sd` 0.21, `axum` 0.8, `rust-embed` 8, `ratatui` 0.30, `windows` 0.62, `core-graphics` 0.25 / `objc2-*`, `display-info` 0.5 / `xcap` 0.9 (referencia para monitores).

## 7. Red y seguridad (solo LAN)

### Transporte e identidad (se mantiene lo de Lan Mouse)
- UDP + **DTLS** (webrtc-rs), puerto 4242 configurable. TCP en el mismo puerto para pedidos de conexión.
- Cada nodo tiene un certificado. Su **fingerprint** se autoriza en el receptor (`authorized_fingerprints`). Se identifica por clave, no por IP.
- No reemplazamos el transporte (se había evaluado QUIC) para no divergir del upstream.

### Garantías LAN-only (lo que agregamos)
1. **Nunca** UPnP, NAT-PMP, STUN ni relays. El router no abre nada.
2. Rechazar pares cuya IP no sea privada: `10/8`, `172.16/12`, `192.168/16`, `169.254/16`, `fe80::/10`, `fc00::/7`. Opcional: exigir la misma subred que alguna interfaz local.
3. Escuchar solo en interfaces con IP privada. **Ojo con IPv6:** cada equipo tiene una dirección global y algunos routers dejan pasar tráfico entrante. Por eso por defecto no se escucha en direcciones IPv6 globales.
4. La **UI web y la API escuchan solo en `127.0.0.1`/`::1`**, validan `Host` y `Origin` (contra *DNS rebinding*) y usan un token local (archivo con permisos 0600). Es la lección del CVE de Deskflow y del de Input Leap: **toda IPC local se autentica**.
5. Solo eventos de entrada y metadatos: nada de ejecutar comandos remotos.

### El problema de las IPs (DHCP)
- **Discovery con mDNS/DNS-SD** (`mdns-sd`): servicio `_vkvm._udp.local` con TXT `fp=<fingerprint>`. Si la IP cambia, el par se reencuentra por su fingerprint.
- Respaldo: IP manual en la config (como hoy en Lan Mouse) y recomendar una **reserva DHCP** en el router para la Windows.
- Problemas a detectar y explicar en la UI: *AP/client isolation* del Wi-Fi, redes de invitados, VLAN o mesh que no reenvían multicast. En macOS 15+, sin permiso de **Red local** falla con `EHOSTUNREACH`.

### La Windows apagada: Wake-on-LAN
- La UI muestra cada nodo como *online* u *offline*, con un botón **"Despertar"** que envía el *magic packet* (102 bytes por UDP broadcast al puerto 9). La MAC se guarda cuando el nodo se conecta por primera vez.
- Requisitos del lado Windows (checklist en la UI): WoL activado en BIOS/UEFI, en el adaptador "Wake on Magic Packet" y "Shutdown Wake-On-LAN", **Fast Startup desactivado** (la causa más común de que no despierte desde apagado) y **Ethernet** (WoL por Wi-Fi casi nunca funciona).
- Autostart en Windows: primero con una tarea programada al logon y, más adelante, como servicio.

## 8. Layout y monitores

1. Cada nodo envía `Displays[]`: id estable (EDID/serial o *device path*, **no el orden de enumeración**), nombre, rectángulo en sus coordenadas nativas, escala y si es el principal. Lo reenvía cada vez que cambia algo.
2. El servidor mantiene un **escritorio virtual global** en unidades lógicas. El usuario arrastra los rectángulos en la UI web, igual que en "Configuración de pantalla" de Windows, y el sistema los pega a los bordes.
3. Los bordes compartidos se calculan por adyacencia. Al cruzar uno se convierte la coordenada (escala y offset) al monitor destino.
4. Hoy Lan Mouse solo tiene `position = left|right|top|bottom` por cliente. El layout por monitor necesita extender el protocolo (`lan-mouse-proto`, con bump de versión) o un canal de control aparte. Se decide en la fase 3, priorizando lo que se pueda proponer upstream.
5. El layout se guarda en `config.toml`, con perfiles por conjunto de monitores ("con monitor externo" / "solo laptop").
6. Hotkeys: Lan Mouse ya tiene `release_bind` (volver al servidor). Agregar: saltar a un nodo y bloquear en la pantalla actual.

## 9. Interfaces

- **Web (localhost):** `axum` (REST + WebSocket para estado en vivo) que habla con el daemon por `lan-mouse-ipc`, con el frontend embebido con `rust-embed` (sin servidor aparte). Pantallas: nodos (online, latencia, despertar, fingerprints), **editor de layout**, hotkeys, diagnóstico de red y permisos. Frontend liviano (Svelte, Preact o vanilla TS); se decide en la fase 3.
- **TUI (`ratatui`):** la misma API. Pensada para Ubuntu Server y SSH.
- **CLI:** la de Lan Mouse (`lan-mouse cli`, `lan-mouse daemon`), más subcomandos nuevos (`wake`, `discover`).
- La GUI GTK del upstream queda como está.

## 10. Roadmap por fases

| Fase | Entregable | Criterio de "listo" |
|---|---|---|
| **0. Research + fork** (hoy) | Este plan, fork con atribución, sesión Windows conectada | Hecho cuando la Windows completa el inventario (§11) |
| **1. Línea base** | Compilar y correr Lan Mouse **sin cambios** en Mac y Windows. Mac → Windows por cable. Anotar qué funciona y qué no (teclas, rueda, escalas, cursor invisible, latencia) | Usar la Windows desde la Mac 1 h. Lista de problemas reales |
| **2. Red** | `vkvm-discovery` (mDNS), `vkvm-wol`, filtros LAN-only | Cambiar la IP de la Windows no rompe nada. Despertarla desde la Mac |
| **3. UI web + monitores** | `vkvm-displays`, `vkvm-web`, editor de layout, elegir el servidor desde la UI | Reordenar monitores en la UI cambia los bordes en vivo. Usar la Mac desde la Windows |
| **4. Linux** | Backend uinput (PR upstream), Ubuntu Desktop (libei existente), Ubuntu Server (TTY, systemd), `vkvm-tui` | Escribir en la consola del server desde la Mac |
| **5. Pulido** | Portapapeles de texto, servicio de Windows para login/UAC con IPC autenticada, firma y notarización en macOS, releases | Instaladores en GitHub Releases |
| Futuro | KVM real: video y escritorio remoto | — |

CI: el upstream ya trae GitHub Actions (`rust.yml`, releases). Se mantienen y se agregan los crates nuevos. La lógica nueva (layout, bordes, filtros de IP, WoL) va en código puro con tests; los backends de SO quedan finos.

## 11. Cómo trabajamos: dos sesiones de Claude (Mac + Windows)

- **Sesión Mac** (`virtualkvm-d4`): coordina, hace el lado macOS y mantiene este plan.
- **Sesión Windows** (Claude Code nativo, no WSL): compila, prueba y reporta el lado Windows.
- Se comunican con *cross-session messaging*. Requiere **Remote Control activo en las dos** y la misma cuenta de claude.ai. Los mensajes van como texto; el código se comparte por git (branches y PRs en este repo).
- Primera tarea de la sesión Windows: **inventario** (versión de Windows, monitores con posición, escala y nombre, adaptador Ethernet o Wi-Fi, MAC, soporte de WoL, estado de Fast Startup, toolchain de Rust).

## 12. Decisiones

1. ~~Licencia y base~~ → **decidido:** fork de Lan Mouse, GPL-3.0-or-later, open source (§4).
2. ~~Repo~~ → **decidido:** https://github.com/stevearagonsite/VirtualKvm (público, fork).
3. **Hardware:** lo responde el inventario de la sesión Windows. Falta la Mac (versión de macOS, chip, monitores).
4. **Hotkey de regreso** preferida y mapeo ⌘ ↔ Ctrl.
5. **Frontend:** Svelte, Preact o vanilla TS (fase 3).

## 13. Riesgos principales

| Riesgo | Mitigación |
|---|---|
| Divergir del upstream y que los merges se vuelvan imposibles | Código nuevo en crates nuevos, cambios mínimos a los existentes, merge de `upstream/main` frecuente, PRs upstream |
| Que se entienda como apropiación de código | Fork visible en GitHub, README con aviso, `NOTICE`, historia intacta, misma licencia |
| Permisos de macOS (Accesibilidad, Input Monitoring, Red local) | Diagnóstico de permisos en la UI. Bundle `.app` firmado más adelante |
| Secure Event Input en Mac deja de capturar teclas | Detectarlo (`IsSecureEventInputEnabled`) y avisar en la UI |
| Windows apagada o WoL que no despierta | Checklist en la UI + Ethernet + desactivar Fast Startup |
| Exposición accidental (IPv6, UI web) | §7: filtros de IP, bind a loopback, token local y validación de Origin |
| Escritorio seguro de Windows | Fase 5. Mientras tanto, documentarlo como limitación conocida |

## 14. Fuentes

- Lan Mouse: [repo](https://github.com/feschber/lan-mouse) · [AGENTS.md (arquitectura)](https://docs.rs/crate/lan-mouse/latest/source/AGENTS.md)
- Mousehop: [README](https://docs.rs/crate/mousehop/0.11.3/source/README.md)
- Deskflow: [repo](https://github.com/deskflow/deskflow)
- Input Leap, vulnerabilidad y archivo: [oss-sec](https://seclists.org/oss-sec/2026/q3/938) · [Openwall](https://www.openwall.com/lists/oss-security/2026/09/25/6)
- rkvm: [repo](https://github.com/htrefil/rkvm) · [uinput (kernel)](https://docs.kernel.org/input/uinput.html)
- libei en Rust: [reis](https://docs.rs/reis/0.7.1) · [ashpd InputCapture](https://docs.rs/crate/ashpd/0.13.0/source/src/desktop/input_capture.rs)
- Windows: [procesos interactivos desde un servicio (Session 0)](https://learn.microsoft.com/en-us/archive/blogs/winsdk/launching-an-interactive-process-from-windows-service-in-windows-vista-and-later) · [Mouse Without Borders](https://learn.microsoft.com/windows/powertoys/mouse-without-borders) · [SendInput y UAC](https://kiwix.lema.org/stackoverflow.com_en_all_2023-11/questions/56595640/sendinput-fails-on-uac-prompt)
- macOS Red local: [Eclectic Light (oct-2026)](https://eclecticlight.co/2026/10/22/local-network-privacy-is-getting-better/) · [Apple forums: launchd/daemons](https://developer.apple.com/forums/thread/767391)
- macOS Secure Input: [1Password community](https://www.1password.community/1password-at-work-58/secure-input-blocking-other-apps-event-taps-25015)
- mDNS: [mdns-sd](https://docs.rs/crate/mdns-sd/0.20.3/source/README.md)
- WoL: [Windows OS Hub](https://woshub.com/wake-on-lan-windows/) · [Dell troubleshooting](https://dell.com/support/kbdoc/en-mq/000129137/wake-on-lan-wol-troubleshooting-best-practices)
- IPv6 en routers hogareños: [CircleID](https://circleid.com/posts/study-finds-ipv6-adoption-exposes-more-residential-devices-to-unsolicited-traffic)
- Monitores: [display-config](https://docs.rs/crate/display-config/0.1.1) · [xcap](https://docs.rs/crate/xcap/0.9.4/source/src/windows/impl_monitor.rs)
- Cross-session messaging: [docs de Claude Code](https://code.claude.com/docs/en/cross-session-messaging.md)
