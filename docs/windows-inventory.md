# Inventario: PC Windows (Phatom-Desktop)

> Relevado el 2026-11-12 por SSH desde la Mac, con `scripts/dev/windows-inventory.ps1` (sesión SSH)
> y `scripts/dev/windows-displays.ps1` (tarea programada en la sesión interactiva del usuario).
> Cierra #1. Por ser un repo público, se omiten la MAC completa y las direcciones IPv6 globales.

## Sistema

| | |
|---|---|
| SO | Windows 11 Pro 26H2, build 26300.9711 |
| Equipo | ASUS PRIME A520M-K · AMD Ryzen 5 5600X · 32 GB RAM |
| GPU | NVIDIA GeForce RTX 3060 (+ adaptador virtual "Meta Virtual Monitor", inactivo) |
| Sesión | `steve`, consola, sesión 1 |

## Monitores

| Dispositivo | Modelo | Posición | Resolución | Escala | Principal |
|---|---|---|---|---|---|
| `\\.\DISPLAY1` | Samsung LF27T35 (27", 60×34 cm) | (0, 0) | 1920×1080 @ 60 Hz | 100 % (96 dpi) | Sí |

Un solo monitor activo. El "Meta Virtual Monitor" es un driver de pantalla virtual (Meta Quest). Si se
activa, aparecería como otro monitor: hay que tenerlo en cuenta en `vkvm-displays` (§8 del plan).

## Red

| | |
|---|---|
| Adaptador | Realtek PCIe GbE (Ethernet), OUI `F0-2F-74` (ASUSTek) |
| Enlace | **100 Mbps**, aunque la placa es gigabit (revisar el cable o el puerto del router). Sobra para un KVM |
| IPv4 | 192.168.0.210/24 por **DHCP**: conviene reservarla en el router |
| IPv6 | **Tiene direcciones globales** (prefijo del ISP). Confirma el riesgo de §7: no escuchar en IPv6 global |
| Perfil | `Private` |
| Otros | Bluetooth PAN (desconectado), OpenVPN Wintun / DCO y TAP-Windows (no presentes) |

## Wake-on-LAN

| Requisito | Estado |
|---|---|
| Adaptador: Wake on Magic Packet | ✅ Enabled |
| Adaptador: Shutdown Wake-On-Lan | ✅ Enabled |
| Adaptador: Wake on pattern match | Enabled (puede despertar por tráfico; evaluar desactivar) |
| Adaptador: "WOL & Shutdown Link Speed" | 10 Mbps First |
| Placa de red armada para despertar (`wake_armed`) | ✅ Sí |
| **Fast Startup** (`HiberbootEnabled`) | ⚠️ **Activado (1)**: es la causa más común de que no despierte desde apagado |
| Estados de energía | S3, Hibernate, Hybrid Sleep, Fast Startup |
| BIOS/UEFI (ASUS: "Power On By PCI-E") | Sin verificar |

**Próximo paso:** probar WoL con Fast Startup activado. Si no despierta desde apagado, desactivarlo
(`powercfg /h off` o desde el Panel de control) y revisar "Power On By PCI-E" en la BIOS.

## Toolchain

| Herramienta | Estado |
|---|---|
| git | ✅ 2.56.0.windows.4 |
| Visual Studio Build Tools (MSVC) | ✅ Build Tools 2026 18.11.3, workload VCTools con recomendados. MSVC 14.51.36231, `cl.exe` 19.51.36260 (x64) |
| rustup | ✅ 1.29.3 (`%USERPROFILE%\.cargo\bin`; no está en el PATH de las sesiones SSH ya abiertas) |
| rustc / cargo | ✅ 1.100.0. Toolchain por defecto: `stable-x86_64-pc-windows-msvc` |
| winget | ✅ v1.29.380 |
| Claude | App de escritorio (Claude Code 2.1.317 integrado). Sin `claude` en el PATH de la consola |
| WSL | Servicio presente |

### Primera compilación (2026-11-13)

`cargo build --release --no-default-features` sobre `main` (`0ea4e56`), sin la GUI GTK:

- Compila sin errores en **2 min 55 s** desde cero. `target\release\lan-mouse.exe` pesa **4,4 MB**.
- Un único warning: la función `start_service` no se usa (`src\main.rs:129`). Probablemente solo se usa
  con la feature `gtk`.
- `--version` muestra `lan-mouse 0.11.0`. En `--help`, `--capture-backend` y `--emulation-backend` aceptan
  `windows` y `dummy`: los backends de Windows quedan activos sin pasar features.

### Prueba de emulación (2026-11-13)

`scripts/dev/windows-test-emulation.ps1` corre `test-emulation` en la sesión interactiva y muestrea
`GetCursorPos` cada 100 ms durante 5 s:

- **Sin elevar:** el cursor **no se mueve** (1 posición en 46 muestras). La ventana en primer plano era un
  Windows Terminal **elevado**, y UIPI descarta en silencio el input de un proceso con menos integridad.
  `SendInput` devuelve 1 igual, así que lan-mouse no se entera.
- **Elevado** (`-Elevated`, `RunLevel Highest`): funciona, con 46 posiciones distintas, X 844..1503 e Y 0..300.
  El rango X (~660 px) supera el ±200 del 8 y el cursor topa con el borde superior. Probablemente es la
  aceleración del puntero ("Mejorar la precisión del puntero"), que se aplica al movimiento relativo de
  `SendInput`.
- Para la fase 1: lan-mouse sin elevar no controla ventanas elevadas. Hay que decidir si se corre elevado
  o si se espera al servicio de la fase 5.

## Acceso desde la Mac

- OpenSSH Server de Windows 9.5. Solo clave (`PasswordAuthentication no`). Firewall: perfil `Private`, `RemoteAddress=LocalSubnet`.
- Shell por defecto: Windows PowerShell 5.1.
- Lo que corre por SSH **no tiene escritorio**. Para lo que necesita la sesión del usuario (monitores,
  `SendInput`, correr Lan Mouse) se usa una tarea programada con `LogonType Interactive`, como en
  `windows-displays.ps1`.
