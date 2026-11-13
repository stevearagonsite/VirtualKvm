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
| git | ❌ No instalado |
| rustup / cargo / rustc | ❌ No instalado |
| Visual Studio Build Tools (MSVC) | ❌ No instalado (sin `vswhere`) |
| winget | ✅ v1.29.380 |
| Claude | App de escritorio (Claude Code 2.1.317 integrado). Sin `claude` en el PATH de la consola |
| WSL | Servicio presente |

`cargo build` todavía no se puede probar. Hace falta instalar: Git, Visual Studio Build Tools con
"Desktop development with C++" y rustup (toolchain `stable-x86_64-pc-windows-msvc`).

## Acceso desde la Mac

- OpenSSH Server de Windows 9.5. Solo clave (`PasswordAuthentication no`). Firewall: perfil `Private`, `RemoteAddress=LocalSubnet`.
- Shell por defecto: Windows PowerShell 5.1.
- Lo que corre por SSH **no tiene escritorio**. Para lo que necesita la sesión del usuario (monitores,
  `SendInput`, correr Lan Mouse) se usa una tarea programada con `LogonType Interactive`, como en
  `windows-displays.ps1`.
