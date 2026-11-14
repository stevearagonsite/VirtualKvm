# Publicar los instalables

> VirtualKvm está basado en [Lan Mouse](https://github.com/feschber/lan-mouse) (Ferdinand Schober y contribuyentes, GPL-3.0-or-later). Ver [NOTICE](../NOTICE).

Los instalables se publican en **https://install.stevearagonsite.com/virtualkvm/**. Por ahora solo
**macOS (Apple Silicon)** y **Windows (x64)**, que son las plataformas que compilamos y probamos; las demás
se suman cuando las configuremos.

## Cada actualización

1. Subir la versión en `VERSION` (por ejemplo `0.1.1` → `0.1.2`).
2. Commit y push a GitHub. La página linkea el código fuente exacto de cada compilación, como pide la GPL.
3. `scripts/release/publish.sh`. Hace todo lo siguiente:
   - compila macOS en la Mac y Windows en la PC Windows (por `ssh windows-kvm`);
   - arma los zips (binario, instalador, README, LICENSE y NOTICE) y los `SHA256SUMS`;
   - sube la versión fija (`/virtualkvm/<versión>/`, caché inmutable) y `latest` (5 min de caché);
   - publica las páginas y los instaladores de una línea, e invalida CloudFront.

Con `--dry` solo compila y empaqueta en `target/dist/<versión>/`.

## Qué hay publicado

| Ruta | Contenido |
|---|---|
| `/` | Índice de instalables |
| `/virtualkvm/` | Página de descargas (generada desde `packaging/web/virtualkvm/index.html.tmpl`) |
| `/virtualkvm/install.sh` | Instalador de una línea para macOS (`curl … \| bash`) |
| `/virtualkvm/install.ps1` | Instalador de una línea para Windows (`irm … \| iex`) |
| `/virtualkvm/latest/` | `VirtualKvm-macos-arm64.zip`, `VirtualKvm-windows-x64.zip`, `SHA256SUMS`, `latest.json` |
| `/virtualkvm/<versión>/` | Los zips de esa versión con su `SHA256SUMS` |

## Infraestructura (cuenta AWS DeliciasCastillo, us-east-1)

Creada a mano por CLI el 2026-11-14, con el mismo esquema que `diagrams.stevearagonsite.com`:

| Recurso | Valor |
|---|---|
| Bucket S3 (privado, acceso público bloqueado) | `stevearagonsite-install` |
| CloudFront | `E3K2B712AJR9ZE` (`d5zfn0fofafy1.cloudfront.net`), PriceClass_100, HTTPS obligatorio |
| Acceso al bucket | OAC `E36OYBM63JHELH` + bucket policy solo para esa distribución |
| Certificado ACM | `install.stevearagonsite.com` (validación DNS) |
| CloudFront Function | `install-index-rewrite`: `/carpeta/` → `/carpeta/index.html` |
| DNS (Cloudflare, perfil `personal`) | `install` CNAME → CloudFront, sin proxy, más el CNAME de validación de ACM |

El subdominio está pensado para **todos** los instalables, no solo VirtualKvm: cada proyecto va en su propia carpeta.
