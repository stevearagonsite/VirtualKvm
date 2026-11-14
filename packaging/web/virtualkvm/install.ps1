# Instala la ultima version de VirtualKvm en Windows 10/11 (x64). PowerShell como Administrador:
#   irm https://install.stevearagonsite.com/virtualkvm/install.ps1 | iex
# VirtualKvm esta basado en Lan Mouse (https://github.com/feschber/lan-mouse), GPL-3.0-or-later.
$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$base = 'https://install.stevearagonsite.com/virtualkvm/latest'
$zip = 'VirtualKvm-windows-x64.zip'
$tmp = Join-Path $env:TEMP ("vkvm-" + [guid]::NewGuid())
New-Item -ItemType Directory $tmp | Out-Null
try {
    "Descargando VirtualKvm..."
    Invoke-WebRequest "$base/$zip" -OutFile "$tmp\$zip" -UseBasicParsing
    $sums = [Text.Encoding]::UTF8.GetString((Invoke-WebRequest "$base/SHA256SUMS" -UseBasicParsing).RawContentStream.ToArray())
    $expected = ($sums -split "`n" | Where-Object { $_ -match [regex]::Escape($zip) } | Select-Object -First 1) -split '\s+' | Select-Object -First 1
    $actual = (Get-FileHash "$tmp\$zip" -Algorithm SHA256).Hash.ToLower()
    if (-not $expected -or $expected -ne $actual) { throw "el checksum SHA-256 no coincide: descarga cancelada" }
    Expand-Archive "$tmp\$zip" -DestinationPath $tmp
    powershell -NoProfile -ExecutionPolicy Bypass -File "$tmp\VirtualKvm\install.ps1"
} finally {
    Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
}
