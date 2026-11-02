param(
  [string]$Out = "D:/RustEchoMusic/docs/screenshots/tray_area.png",
  [string]$Exe = "D:/RustEchoMusic/target/debug/EchoMusicLite.exe"
)
# Grab the notification area (bottom-right of the primary screen) so the tray icon
# can be inspected. Starts the app first, waits for the icon, then screenshots.
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class TS {
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, int d, int e);
}
"@

$env:ECHO_LOG_FILE = "D:\RustEchoMusic\.workbuddy\tray_area.log"
Remove-Item Env:ECHO_SMOKE -ErrorAction SilentlyContinue
$p = Start-Process -FilePath $Exe -PassThru
Start-Sleep -Milliseconds 6000

$b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
# hover over the tray icons so dead/blank entries get cleaned up by Explorer
[TS]::SetCursorPos($b.Width - 120, $b.Height - 30) | Out-Null
Start-Sleep -Milliseconds 400
[TS]::SetCursorPos($b.Width - 200, $b.Height - 30) | Out-Null
Start-Sleep -Milliseconds 400
[TS]::SetCursorPos($b.Width - 300, $b.Height - 30) | Out-Null
Start-Sleep -Milliseconds 600

$w = 420
$h = 40
$x = $b.Width - $w
$y = $b.Height - $h
$bmp = New-Object System.Drawing.Bitmap $w, $h
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($x, $y, 0, 0, (New-Object System.Drawing.Size $w, $h))
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
Write-Output "saved $Out screen=${($b.Width)}x${($b.Height)}"

Stop-Process -Id $p.Id -Force
