param(
  [string]$Name = "shot",
  [string]$Smoke = "",
  [int]$Wait = 8000,
  [int]$HoverX = -1,
  [int]$HoverY = -1,
  [string]$Exe = "D:/RustEchoMusic/target/debug/echo-music.exe"
)
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class W {
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
}
"@

$exe = $Exe
if ($Smoke -ne "") { $env:ECHO_SMOKE = $Smoke } else { Remove-Item Env:ECHO_SMOKE -ErrorAction SilentlyContinue }
$p = Start-Process -FilePath $exe -PassThru
Start-Sleep -Milliseconds $Wait
$p.Refresh()
$h = $p.MainWindowHandle
if ($h -eq [IntPtr]::Zero) { Start-Sleep -Seconds 2; $p.Refresh(); $h = $p.MainWindowHandle }

[W]::ShowWindow($h, 5) | Out-Null
[W]::SetWindowPos($h, [IntPtr]::Zero, 0, 0, 0, 0, 0x0001 -bor 0x0040) | Out-Null
[W]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 800

if ($HoverX -ge 0) {
  # 先移到别处再移入，确保产生 mouse move
  [System.Windows.Forms.Cursor]::Position = New-Object System.Drawing.Point(($HoverX + 12), ($HoverY + 12))
  Start-Sleep -Milliseconds 250
  [System.Windows.Forms.Cursor]::Position = New-Object System.Drawing.Point($HoverX, $HoverY)
  Start-Sleep -Milliseconds 700
}

$r = New-Object W+RECT
[W]::GetWindowRect($h, [ref]$r) | Out-Null
$w = $r.Right - $r.Left
$ht = $r.Bottom - $r.Top
$bmp = New-Object System.Drawing.Bitmap($w, $ht)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$hdc = $g.GetHdc()
[W]::PrintWindow($h, $hdc, 2) | Out-Null
$g.ReleaseHdc($hdc)
$g.Dispose()

$dir = "D:\RustEchoMusic\docs\screenshots"
if (!(Test-Path $dir)) { New-Item -ItemType Directory -Path $dir | Out-Null }
$out = Join-Path $dir "$Name.png"
$bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()

Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
Write-Output $out
