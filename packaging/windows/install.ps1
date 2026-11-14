# VirtualKvm: instalador para Windows 10/11 (x64). Correr en PowerShell como Administrador.
# VirtualKvm esta basado en Lan Mouse (https://github.com/feschber/lan-mouse), GPL-3.0-or-later.
#
#   .\install.ps1                 instala o actualiza y arranca al iniciar sesion
#   .\install.ps1 -Uninstall      lo desinstala (deja la configuracion)
#
# - Copia vkvm.exe a %USERPROFILE%\vkvm\bin (asi actualizar no pisa el que corre).
# - Tarea programada al iniciar sesion: elevada (para controlar ventanas de administrador),
#   en la sesion interactiva (la unica con escritorio) y con reinicio si se cae.
# - Firewall: puerto 4242 solo para la red local y solo para este ejecutable.

param(
    [string]$User = $env:USERNAME,
    [string]$Source = "$PSScriptRoot\vkvm.exe",
    [switch]$Uninstall
)
$ErrorActionPreference = 'Stop'

$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw "Abri PowerShell como Administrador (Win + X > Terminal (Administrador)) y volve a correr este script."
}

$base = "C:\Users\$User\vkvm"
$bin  = "$base\bin"
$exe  = "$bin\vkvm.exe"
$log  = "$base\daemon.log"
$trayLog = "$base\tray.log"
$task = 'VirtualKvm'

function Stop-Vkvm {
    Stop-ScheduledTask -TaskName $task -ErrorAction SilentlyContinue
    Get-Process vkvm, lan-mouse -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Milliseconds 500
}

if ($Uninstall) {
    Stop-Vkvm
    Unregister-ScheduledTask -TaskName $task -Confirm:$false -ErrorAction SilentlyContinue
    Get-NetFirewallRule -DisplayName 'VirtualKvm *' -ErrorAction SilentlyContinue | Remove-NetFirewallRule
    Remove-Item $exe -ErrorAction SilentlyContinue
    "VirtualKvm desinstalado. La configuracion queda en $env:LOCALAPPDATA\virtualkvm."
    return
}

if (-not (Test-Path $Source)) { throw "no encuentro el binario: ${Source}" }

Stop-Vkvm
# versiones anteriores (antes del rebranding): tarea, reglas y binario se llamaban lan-mouse
Unregister-ScheduledTask -TaskName 'vkvm-lanmouse-daemon' -Confirm:$false -ErrorAction SilentlyContinue
Get-NetFirewallRule -DisplayName 'VirtualKvm lan-mouse *' -ErrorAction SilentlyContinue | Remove-NetFirewallRule
Remove-Item "$bin\lan-mouse.exe" -ErrorAction SilentlyContinue

New-Item -ItemType Directory -Force $bin | Out-Null
Copy-Item $Source $exe -Force
Unblock-File $exe -ErrorAction SilentlyContinue

foreach ($p in 'UDP', 'TCP') {
    $n = "VirtualKvm $p 4242"
    Get-NetFirewallRule -DisplayName $n -ErrorAction SilentlyContinue | Remove-NetFirewallRule
    New-NetFirewallRule -DisplayName $n -Direction Inbound -Action Allow -Protocol $p -LocalPort 4242 `
        -Program $exe -Profile Private -RemoteAddress LocalSubnet | Out-Null
}

$cmd = "`$env:RUST_LOG='info'; & '$exe' tray --log '$log' *> '$trayLog'"
$action = New-ScheduledTaskAction -Execute 'powershell.exe' `
    -Argument "-NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -Command `"$cmd`""
$trigger = New-ScheduledTaskTrigger -AtLogOn -User $User
$trigger.Delay = 'PT10S'
$taskPrincipal = New-ScheduledTaskPrincipal -UserId $User -LogonType Interactive -RunLevel Highest
$settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit ([TimeSpan]::Zero) -AllowStartIfOnBatteries `
    -DontStopIfGoingOnBatteries -StartWhenAvailable -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1) `
    -MultipleInstances IgnoreNew
Register-ScheduledTask -TaskName $task -Action $action -Trigger $trigger -Principal $taskPrincipal `
    -Settings $settings -Force | Out-Null
Start-ScheduledTask -TaskName $task
Start-Sleep -Seconds 3

$version = (& $exe --version 2>$null | Select-Object -First 1)
"VirtualKvm instalado ($version): el icono arranca solo al iniciar sesion de $User."
"La red tiene que estar como 'Privada' para que el firewall deje pasar a las otras PCs de la LAN."
"Logs: $base"
