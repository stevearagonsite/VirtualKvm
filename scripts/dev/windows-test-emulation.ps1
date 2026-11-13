# Prueba `lan-mouse test-emulation` en el escritorio de steve y muestrea el cursor.
#
# Se corre desde SSH (sin escritorio). Crea dos tareas programadas en la sesión interactiva:
#   1. lan-mouse.exe test-emulation (RUST_LOG=info) -> $LogFile
#   2. este mismo script con -Role sampler: GetCursorPos cada 100 ms -> $SamplesFile
# Al final (siempre, por try/finally) mata lan-mouse.exe y borra las tareas.
#
# OJO: test-emulation mueve el mouse en un 8 sin parar hasta que se lo mata.
#
# -Elevated lanza lan-mouse con RunLevel Highest. Sin elevar, si la ventana en primer plano es un
# proceso elevado (p. ej. una terminal como administrador), UIPI descarta el input en silencio:
# SendInput devuelve éxito pero el cursor no se mueve.
param(
  [switch] $Elevated,
  [ValidateSet('runner', 'sampler')] [string] $Role = 'runner',
  [int] $RunSeconds = 6,
  [int] $SampleSeconds = 5,
  [int] $SampleIntervalMs = 100
)

$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)            # ...\VirtualKvm
$Exe = Join-Path $Root 'target\release\lan-mouse.exe'
$OutDir = Join-Path $env:USERPROFILE 'vkvm'
$LogFile = Join-Path $OutDir 'test-emulation.log'
$SamplesFile = Join-Path $OutDir 'cursor-samples.txt'

if ($Role -eq 'sampler') {
  # Corre dentro de la sesión interactiva.
  Add-Type @'
using System; using System.Runtime.InteropServices;
public class VC {
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X; public int Y; }
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr v);
}
'@
  [VC]::SetProcessDpiAwarenessContext([IntPtr]-4) | Out-Null   # per-monitor v2: coordenadas físicas
  $lines = @("# session=$((Get-Process -Id $PID).SessionId) user=$env:USERNAME intervalo=${SampleIntervalMs}ms")
  $sw = [Diagnostics.Stopwatch]::StartNew()
  while ($sw.ElapsedMilliseconds -lt $SampleSeconds * 1000) {
    $p = New-Object VC+POINT
    $ok = [VC]::GetCursorPos([ref]$p)
    $lines += '{0},{1},{2},{3}' -f $sw.ElapsedMilliseconds, $p.X, $p.Y, $ok
    Start-Sleep -Milliseconds $SampleIntervalMs
  }
  $lines | Set-Content -Path $SamplesFile -Encoding utf8
  return
}

# --- runner (sesión SSH) ---
if (-not (Test-Path $Exe)) { throw "No existe $Exe. Compilá antes: cargo build --release --no-default-features" }
Remove-Item $LogFile, $SamplesFile -ErrorAction SilentlyContinue

$principal = New-ScheduledTaskPrincipal -UserId steve -LogonType Interactive
$lmPrincipal = if ($Elevated) {
  New-ScheduledTaskPrincipal -UserId steve -LogonType Interactive -RunLevel Highest
} else { $principal }
$settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Minutes 2) -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
$tasks = @{
  'vkvm-test-emulation' = New-ScheduledTaskAction -Execute 'cmd.exe' -WorkingDirectory $Root `
    -Argument "/c set RUST_LOG=info&& `"$Exe`" test-emulation > `"$LogFile`" 2>&1"
  'vkvm-cursor-sampler' = New-ScheduledTaskAction -Execute 'powershell.exe' `
    -Argument "-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File `"$PSCommandPath`" -Role sampler -SampleSeconds $SampleSeconds -SampleIntervalMs $SampleIntervalMs"
}

function Stop-LanMouse {
  Get-Process lan-mouse -ErrorAction SilentlyContinue |
    Where-Object { $_.Path -eq $Exe } |
    ForEach-Object { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue; "detenido lan-mouse.exe pid=$($_.Id)" }
}

try {
  foreach ($name in $tasks.Keys) {
    $pr = if ($name -eq 'vkvm-test-emulation') { $lmPrincipal } else { $principal }
    Register-ScheduledTask -TaskName $name -Action $tasks[$name] -Principal $pr -Settings $settings -Force | Out-Null
  }
  Start-ScheduledTask -TaskName 'vkvm-cursor-sampler'
  Start-ScheduledTask -TaskName 'vkvm-test-emulation'
  Start-Sleep -Seconds $RunSeconds
  # El muestreador tarda en arrancar (PowerShell + Add-Type): esperarlo hasta 10 s más.
  $deadline = (Get-Date).AddSeconds(10)
  while ((Get-ScheduledTask -TaskName 'vkvm-cursor-sampler').State -eq 'Running' -and (Get-Date) -lt $deadline) {
    Start-Sleep -Milliseconds 250
  }
}
finally {
  Stop-LanMouse
  foreach ($name in $tasks.Keys) {
    Stop-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue
    Unregister-ScheduledTask -TaskName $name -Confirm:$false -ErrorAction SilentlyContinue
  }
  Start-Sleep -Milliseconds 500
  Stop-LanMouse   # por si quedó algo vivo
  if (Get-Process lan-mouse -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $Exe }) {
    Write-Warning 'lan-mouse.exe sigue vivo: matarlo a mano'
  }
}

# Resumen de las muestras
if (Test-Path $SamplesFile) {
  $pts = Get-Content $SamplesFile | Where-Object { $_ -notmatch '^#' } | ForEach-Object {
    $f = $_.Split(','); [pscustomobject]@{ X = [int]$f[1]; Y = [int]$f[2] }
  }
  $x = $pts | Measure-Object X -Minimum -Maximum
  $y = $pts | Measure-Object Y -Minimum -Maximum
  $distinct = ($pts | ForEach-Object { "$($_.X),$($_.Y)" } | Sort-Object -Unique).Count
  "elevated=$Elevated muestras=$($pts.Count) posiciones_distintas=$distinct X=[$($x.Minimum)..$($x.Maximum)] Y=[$($y.Minimum)..$($y.Maximum)]"
} else {
  Write-Warning "No se generó $SamplesFile"
}
