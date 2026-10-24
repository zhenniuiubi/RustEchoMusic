param(
  [string]$Exe = "D:/RustEchoMusic/target/debug/echo-music.exe"
)
$ErrorActionPreference = "Stop"
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class M4 {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, int extra);
  public struct RECT { public int Left, Top, Right, Bottom; }
}
"@
$env:ECHO_LOG_FILE = "D:\RustEchoMusic\.workbuddy\mem2_app.log"
$p = Start-Process -FilePath $Exe -PassThru
Start-Sleep -Milliseconds 9000
$p.Refresh()
$h = $p.MainWindowHandle
[M4]::ShowWindow($h, 5) | Out-Null
[M4]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 500
$r = New-Object M4+RECT
[M4]::GetWindowRect($h, [ref]$r) | Out-Null
$ox = $r.Left; $oy = $r.Top
function Report([string]$tag) {
  Start-Sleep -Milliseconds 2000
  $p.Refresh()
  Write-Output ("{0}: WS={1}MB" -f $tag, [math]::Round($p.WorkingSet64 / 1MB, 1))
}
function ClickAt([int]$x, [int]$y) {
  [M4]::SetCursorPos($ox + $x, $oy + $y) | Out-Null
  Start-Sleep -Milliseconds 180
  [M4]::mouse_event(0x0002, 0, 0, 0, 0)
  Start-Sleep -Milliseconds 90
  [M4]::mouse_event(0x0004, 0, 0, 0, 0)
  Start-Sleep -Milliseconds 700
}
Report "home"
ClickAt 500 137; Report "ranking-page"
ClickAt 400 300; Report "rank-detail(100首)"
ClickAt 170 220; Report "favorites(云歌单)"
ClickAt 170 295; Report "history"
ClickAt 170 150; Report "explore"
ClickAt 170 115; Report "home-again"
Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
