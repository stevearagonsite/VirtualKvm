$out = "$env:USERPROFILE\vkvm\displays.txt"
Add-Type @'
using System; using System.Runtime.InteropServices;
public class VD {
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr v);
  [DllImport("shcore.dll")] public static extern int GetDpiForMonitor(IntPtr h, int t, out uint x, out uint y);
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X; public int Y; }
  [DllImport("user32.dll")] public static extern IntPtr MonitorFromPoint(POINT p, uint f);
}
'@
[VD]::SetProcessDpiAwarenessContext([IntPtr]-4) | Out-Null
Add-Type -AssemblyName System.Windows.Forms
$lines = @("session=$((Get-Process -Id $PID).SessionId) user=$env:USERNAME")
foreach ($s in [System.Windows.Forms.Screen]::AllScreens) {
  $b = $s.Bounds
  $p = New-Object VD+POINT; $p.X = $b.X + [int]($b.Width/2); $p.Y = $b.Y + [int]($b.Height/2)
  $h = [VD]::MonitorFromPoint($p, 2); $dx = 0; $dy = 0
  [VD]::GetDpiForMonitor($h, 0, [ref]$dx, [ref]$dy) | Out-Null
  $lines += "{0} | primary={1} | x={2} y={3} w={4} h={5} | dpi={6} escala={7}%" -f $s.DeviceName, $s.Primary, $b.X, $b.Y, $b.Width, $b.Height, $dx, [int]($dx*100/96)
}
$lines | Set-Content -Path $out -Encoding utf8
