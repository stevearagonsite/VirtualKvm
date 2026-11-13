$ErrorActionPreference = 'SilentlyContinue'
function H($t) { "`n=== $t ===" }

H 'OS'
$os = Get-CimInstance Win32_OperatingSystem
$cv = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
"{0} | {1} | build {2}.{3}" -f $os.Caption, $cv.DisplayVersion, $cv.CurrentBuild, $cv.UBR
$cs = Get-CimInstance Win32_ComputerSystem
"Equipo: {0} {1} | RAM {2:N0} GB" -f $cs.Manufacturer, $cs.Model, ($cs.TotalPhysicalMemory/1GB)
"CPU: " + (Get-CimInstance Win32_Processor | select -First 1 -Expand Name)
"Placa: " + ((Get-CimInstance Win32_BaseBoard | % { "$($_.Manufacturer) $($_.Product)" }))

H 'Sesiones de usuario'
query user 2>&1

H 'GPU'
Get-CimInstance Win32_VideoController | % { "{0} | {1}x{2} @ {3}Hz | driver {4}" -f $_.Name, $_.CurrentHorizontalResolution, $_.CurrentVerticalResolution, $_.CurrentRefreshRate, $_.DriverVersion }

H 'Monitores (EDID)'
Get-CimInstance -Namespace root\wmi WmiMonitorID | % {
  $n = ($_.UserFriendlyName | ? { $_ -ne 0 } | % { [char]$_ }) -join ''
  $m = ($_.ManufacturerName | ? { $_ -ne 0 } | % { [char]$_ }) -join ''
  "{0} | {1} | {2}" -f $m, $n, $_.InstanceName
}
Get-CimInstance -Namespace root\wmi WmiMonitorBasicDisplayParams | % { "{0} | {1}x{2} cm" -f $_.InstanceName, $_.MaxHorizontalImageSize, $_.MaxVerticalImageSize }

H 'Red'
Get-NetAdapter | sort Status | % { "{0} | {1} | {2} | {3} | MAC {4} | {5}" -f $_.Name, $_.InterfaceDescription, $_.MediaType, $_.Status, $_.MacAddress, $_.LinkSpeed }
Get-NetIPAddress -AddressFamily IPv4 | ? { $_.IPAddress -notlike '127.*' } | % { "{0} | {1}/{2} | {3}" -f $_.InterfaceAlias, $_.IPAddress, $_.PrefixLength, $_.PrefixOrigin }
Get-NetIPAddress -AddressFamily IPv6 | ? { $_.PrefixOrigin -ne 'WellKnown' } | % { "{0} | {1} | {2}" -f $_.InterfaceAlias, $_.IPAddress, $_.SuffixOrigin }
Get-NetConnectionProfile | % { "Perfil: {0} | {1} | {2}" -f $_.InterfaceAlias, $_.Name, $_.NetworkCategory }

H 'Wake-on-LAN'
"wake_armed:"; powercfg /devicequery wake_armed
"wake_programmable (dispositivos que pueden despertar):"; powercfg /devicequery wake_programmable
Get-NetAdapter -Physical | % {
  $a = $_.Name
  Get-NetAdapterAdvancedProperty -Name $a | ? { $_.DisplayName -match 'Wake|WOL|Shutdown|Energy|Green|EEE|Power' } | % { "{0} | {1} = {2}" -f $a, $_.DisplayName, $_.DisplayValue }
  Get-NetAdapterPowerManagement -Name $a | % { "{0} | WakeOnMagicPacket={1} WakeOnPattern={2} AllowComputerToTurnOffDevice={3}" -f $a, $_.WakeOnMagicPacket, $_.WakeOnPattern, $_.AllowComputerToTurnOffDevice }
}

H 'Energia'
$hb = (Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Power').HiberbootEnabled
"Fast Startup (HiberbootEnabled) = $hb"
powercfg /a

H 'Toolchain'
foreach ($c in 'git','rustup','cargo','rustc','claude','winget','node') {
  $p = Get-Command $c | select -First 1 -Expand Source
  if ($p) { $v = (& $c --version 2>&1 | select -First 1); "$c | $p | $v" } else { "$c | NO" }
}
foreach ($d in "$env:USERPROFILE\.cargo\bin\cargo.exe", "$env:USERPROFILE\.local\bin\claude.exe") { if (Test-Path $d) { "existe: $d" } }
$vsw = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
if (Test-Path $vsw) { & $vsw -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -format value -property displayName; "VC tools path:"; & $vsw -products * -latest -property installationPath } else { "vswhere: NO (sin Visual Studio / Build Tools)" }

H 'Firewall SSH'
Get-NetFirewallRule -Name OpenSSH-Server-In-TCP | % { "Enabled={0} Profile={1} Action={2}" -f $_.Enabled, $_.Profile, $_.Action }
Get-NetFirewallRule -Name OpenSSH-Server-In-TCP | Get-NetFirewallAddressFilter | % { "RemoteAddress=" + ($_.RemoteAddress -join ',') }
