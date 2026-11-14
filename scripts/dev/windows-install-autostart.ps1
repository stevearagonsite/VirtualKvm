# VirtualKvm (dev): instala el binario recien compilado con el instalador de usuario.
# Mismas opciones que packaging\windows\install.ps1 (-User, -Uninstall).
param([string]$User = $env:USERNAME, [switch]$Uninstall)
$repo = (Resolve-Path "$PSScriptRoot\..\..").Path
& "$repo\packaging\windows\install.ps1" -User $User -Source "$repo\target\release\vkvm.exe" -Uninstall:$Uninstall
