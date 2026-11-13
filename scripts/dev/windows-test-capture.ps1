# Prueba `lan-mouse test-capture` en el escritorio de steve durante N segundos.
#
# Se corre desde SSH (sin escritorio). Crea una tarea programada en la sesión interactiva que lanza
# lan-mouse.exe test-capture (RUST_LOG=info) con salida a $LogFile. Al final (siempre, por
# try/finally) mata lan-mouse.exe y borra la tarea.
#
# Necesita a alguien en la Windows: mover el mouse físico hacia un borde de la pantalla mientras corre.
# -Elevated lanza lan-mouse con RunLevel Highest (ver windows-test-emulation.ps1: UIPI).
param(
  [int] $Seconds = 60,
  [switch] $Elevated
)

$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)            # ...\VirtualKvm
$Exe = Join-Path $Root 'target\release\lan-mouse.exe'
$LogFile = Join-Path $env:USERPROFILE 'vkvm\test-capture.log'
$TaskName = 'vkvm-test-capture'

if (-not (Test-Path $Exe)) { throw "No existe $Exe. Compilá antes: cargo build --release --no-default-features" }
Remove-Item $LogFile -ErrorAction SilentlyContinue

$principal = if ($Elevated) {
  New-ScheduledTaskPrincipal -UserId steve -LogonType Interactive -RunLevel Highest
} else {
  New-ScheduledTaskPrincipal -UserId steve -LogonType Interactive
}
$settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Seconds ($Seconds + 120)) -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
$action = New-ScheduledTaskAction -Execute 'cmd.exe' -WorkingDirectory $Root `
  -Argument "/c set RUST_LOG=info&& `"$Exe`" test-capture > `"$LogFile`" 2>&1"

function Stop-LanMouse {
  Get-Process lan-mouse -ErrorAction SilentlyContinue |
    Where-Object { $_.Path -eq $Exe } |
    ForEach-Object { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue; "detenido lan-mouse.exe pid=$($_.Id)" }
}

try {
  Register-ScheduledTask -TaskName $TaskName -Action $action -Principal $principal -Settings $settings -Force | Out-Null
  Start-ScheduledTask -TaskName $TaskName
  "test-capture corriendo $Seconds s (elevated=$Elevated): mover el mouse físico hacia un borde"
  Start-Sleep -Seconds $Seconds
}
finally {
  Stop-LanMouse
  Stop-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
  Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
  Start-Sleep -Milliseconds 500
  Stop-LanMouse
  if (Get-Process lan-mouse -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $Exe }) {
    Write-Warning 'lan-mouse.exe sigue vivo: matarlo a mano'
  }
}

if (Test-Path $LogFile) { Get-Content $LogFile -Tail 40 } else { Write-Warning "No se generó $LogFile" }
