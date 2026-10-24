param(
  [string]$Exe = "D:/RustEchoMusic/target/debug/echo-music.exe",
  [string]$Name = "wheel",
  [string]$Clicks = "170:220",
  [int]$Notches = -6
)
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class M5 {
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, int data, int extra);
  public struct RECT { public int Left, Top, Right, Bottom; }
}
"@
$env:ECHO_LOG_FILE = "D:\RustEchoMusic\.workbuddy\wheel_app.log"
$p = Start-Process -FilePath $Exe -PassThru
Start-Sleep -Milliseconds 9000
$p.Refresh()
$h = $p.MainWindowHandle
[M5]::ShowWindow($h, 5) | Out-Null
[M5]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 500
$r = New-Object M5+RECT
[M5]::GetWindowRect($h, [ref]$r) | Out-Null
$ox = $r.Left; $oy = $r.Top
foreach ($c in $Clicks.Split("|")) {
  if ($c.Trim() -eq "") { continue }
  $parts = $c.Split(":")
  [M5]::SetCursorPos($ox + [int]$parts[0], $oy + [int]$parts[1]) | Out-Null
  Start-Sleep -Milliseconds 200
  [M5]::mouse_event(0x0002, 0, 0, 0, 0)
  Start-Sleep -Milliseconds 90
  [M5]::mouse_event(0x0004, 0, 0, 0, 0)
  Start-Sleep -Milliseconds 1200
}
# 鼠标移到列表中间，滚轮滚动
[M5]::SetCursorPos($ox + 500, $oy + 350) | Out-Null
Start-Sleep -Milliseconds 300
for ($i = 0; $i -lt 6; $i++) {
  [M5]::mouse_event(0x0800, 0, 0, $Notches * 120, 0)
  Start-Sleep -Milliseconds 120
}
Start-Sleep -Milliseconds 1500
$w = $r.Right - $r.Left; $ht = $r.Bottom - $r.Top
$bmp = New-Object System.Drawing.Bitmap($w, $ht)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$hdc = $g.GetHdc()
[M5]::PrintWindow($h, $hdc, 2) | Out-Null
$g.ReleaseHdc($hdc)
$g.Dispose()
$out = "D:\RustEchoMusic\docs\screenshots\$Name.png"
$bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
Write-Output $out
