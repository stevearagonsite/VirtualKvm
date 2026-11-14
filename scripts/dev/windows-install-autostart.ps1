# VirtualKvm (dev): instala el icono de bandeja (lan-mouse tray) como tarea programada que
# arranca al iniciar sesion. La bandeja lanza y apaga el daemon.
#
# - Copia el binario compilado a C:\Users\<user>\vkvm\bin (asi recompilar no pisa el que corre).
# - Arranca elevado (RunLevel Highest) para que el input llegue tambien a ventanas elevadas (UIPI).
# - Corre en la sesion interactiva del usuario (LogonType Interactive), que es la unica con escritorio.
# - Se reinicia solo si el proceso se cae.
# - Ajusta las reglas de firewall (solo LAN, solo este ejecutable).
#
# Uso (PowerShell como Administrador o por SSH con una cuenta admin):
#   .\scripts\dev\windows-install-autostart.ps1 [-User steve]

param(
    [string]$User = $env:USERNAME,
    [string]$Repo = (Resolve-Path "$PSScriptRoot\..\..").Path
)
$ErrorActionPreference = 'Stop'

$base = "C:\Users\$User\vkvm"
$bin  = "$base\bin"
$exe  = "$bin\vkvm.exe"
$log  = "$base\daemon.log"
$trayLog = "$base\tray.log"
$task = 'VirtualKvm'
$src  = "$Repo\target\release\vkvm.exe"

if (-not (Test-Path $src)) { throw "no existe ${src}: compila antes con cargo build --release --no-default-features" }

# detener lo que este corriendo antes de reemplazar el binario
Stop-ScheduledTask -TaskName $task -ErrorAction SilentlyContinue
Get-Process vkvm, lan-mouse -ErrorAction SilentlyContinue | Stop-Process -Force
# antes del rebranding: tarea, reglas y binario se llamaban lan-mouse
Unregister-ScheduledTask -TaskName 'vkvm-lanmouse-daemon' -Confirm:$false -ErrorAction SilentlyContinue
Get-NetFirewallRule -DisplayName 'VirtualKvm lan-mouse *' -ErrorAction SilentlyContinue | Remove-NetFirewallRule
Remove-Item "$bin\lan-mouse.exe" -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 500

New-Item -ItemType Directory -Force $bin | Out-Null
Copy-Item $src $exe -Force

# firewall: solo LAN y solo este ejecutable
foreach ($p in 'UDP', 'TCP') {
    $n = "VirtualKvm $p 4242"
    Get-NetFirewallRule -DisplayName $n -ErrorAction SilentlyContinue | Remove-NetFirewallRule
    New-NetFirewallRule -DisplayName $n -Direction Inbound -Action Allow -Protocol $p -LocalPort 4242 `
        -Program $exe -Profile Private -RemoteAddress LocalSubnet | Out-Null
}

# tarea programada: al iniciar sesion, elevada, interactiva, con reinicio automatico
$cmd = "`$env:RUST_LOG='info'; & '$exe' tray --log '$log' *> '$trayLog'"
$action = New-ScheduledTaskAction -Execute 'powershell.exe' `
    -Argument "-NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -Command `"$cmd`""
$trigger = New-ScheduledTaskTrigger -AtLogOn -User $User
$trigger.Delay = 'PT10S'   # darle unos segundos a la red y al escritorio
$principal = New-ScheduledTaskPrincipal -UserId $User -LogonType Interactive -RunLevel Highest
$settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit ([TimeSpan]::Zero) -AllowStartIfOnBatteries `
    -DontStopIfGoingOnBatteries -StartWhenAvailable -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1) `
    -MultipleInstances IgnoreNew
Register-ScheduledTask -TaskName $task -Action $action -Trigger $trigger -Principal $principal `
    -Settings $settings -Force | Out-Null

Start-ScheduledTask -TaskName $task
Start-Sleep -Seconds 3
$t = Get-ScheduledTask -TaskName $task
"tarea: $($t.State) | disparador: al iniciar sesion de $User"
Get-Process vkvm -ErrorAction SilentlyContinue | Select-Object Id, SessionId, Path | Format-Table | Out-String
