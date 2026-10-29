param(
  [string]$Name = "shot",
  [int]$Wait = 10000,
  # clicks: "x:y|x:y" window-relative coords
  [string]$Clicks = "",
  [int]$After = 1500,
  [string]$Exe = "D:/RustEchoMusic/target/debug/echo-music.exe",
  [int]$Pre = 0,
  # drag: "x1:y1:x2:y2"
  [string]$Drag = "",
  # value for ECHO_SMOKE self-test hook (e.g. theme, settings, backdrop-mica)
  [string]$Smoke = "",
  # hover only (no click): "x:y" window-relative coords
  [string]$Hover = ""
)
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class G {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc cb, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, System.Text.StringBuilder sb, int max);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, int data, int extra);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr h);
  public delegate bool EnumWindowsProc(IntPtr h, IntPtr l);
  public struct RECT { public int Left, Top, Right, Bottom; }
  public static IntPtr FindAppWindow(uint pid) {
    IntPtr best = IntPtr.Zero;
    long bestArea = 0;
    EnumWindows((h, l) => {
      uint wpid;
      GetWindowThreadProcessId(h, out wpid);
      if (wpid != pid || !IsWindowVisible(h)) return true;
      var sb = new System.Text.StringBuilder(256);
      GetClassName(h, sb, 256);
      if (sb.ToString() == "ConsoleWindowClass") return true;
      RECT r;
      GetWindowRect(h, out r);
      long area = (long)(r.Right - r.Left) * (r.Bottom - r.Top);
      if (area > bestArea) { bestArea = area; best = h; }
      return true;
    }, IntPtr.Zero);
    return best;
  }
}
"@

$logFile = "D:\RustEchoMusic\.workbuddy\shot_app.log"
if (Test-Path $logFile) { Remove-Item $logFile -Force -ErrorAction SilentlyContinue }
$env:ECHO_LOG_FILE = $logFile
if ($Smoke -ne "") { $env:ECHO_SMOKE = $Smoke } else { Remove-Item Env:ECHO_SMOKE -ErrorAction SilentlyContinue }

$p = Start-Process -FilePath $Exe -PassThru
Start-Sleep -Milliseconds $Wait
$h = [G]::FindAppWindow([uint32]$p.Id)
if ($h -eq [IntPtr]::Zero) { Start-Sleep -Seconds 3; $h = [G]::FindAppWindow([uint32]$p.Id) }
if ($h -eq [IntPtr]::Zero) { Stop-Process -Id $p.Id -Force; Write-Output "NO_WINDOW"; exit 1 }
[G]::ShowWindow($h, 5) | Out-Null
[G]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 800

$r = New-Object G+RECT
[G]::GetWindowRect($h, [ref]$r) | Out-Null
$ox = $r.Left; $oy = $r.Top
$w = $r.Right - $r.Left; $ht = $r.Bottom - $r.Top

if ($Pre -gt 0) { Start-Sleep -Milliseconds $Pre }

# gpui hit-testing needs WM_MOUSEMOVE to refresh hitboxes; SetCursorPos alone sends none.
# Keep SetCursorPos (same coordinate space as GetWindowRect) and only emit a 0-delta MOVE
# so a real WM_MOUSEMOVE is delivered without shifting the cursor.
function MoveTo([int]$sx, [int]$sy) {
  [G]::SetCursorPos($sx, $sy) | Out-Null
  Start-Sleep -Milliseconds 80
  [G]::mouse_event(0x0001, 0, 0, 0, 0) | Out-Null
  Start-Sleep -Milliseconds 80
  [G]::mouse_event(0x0001, 0, 0, 0, 0) | Out-Null
}

if (-not [string]::IsNullOrEmpty($Clicks)) {
  foreach ($c in $Clicks.Split("|")) {
    if ($c.Trim() -eq "") { continue }
    $parts = $c.Split(":")
    $x = $ox + [int]$parts[0]; $y = $oy + [int]$parts[1]
    MoveTo ($x - 12) ($y - 12)
    Start-Sleep -Milliseconds 120
    MoveTo $x $y
    Start-Sleep -Milliseconds 160
    [G]::mouse_event(0x0002, 0, 0, 0, 0) | Out-Null
    Start-Sleep -Milliseconds 90
    [G]::mouse_event(0x0004, 0, 0, 0, 0) | Out-Null
    Start-Sleep -Milliseconds 500
  }
}

if (-not [string]::IsNullOrEmpty($Hover)) {
  $hp = $Hover.Split(":")
  MoveTo ($ox + [int]$hp[0]) ($oy + [int]$hp[1])
  Start-Sleep -Milliseconds 300
}

$rectBefore = "pos=($ox,$oy) size=${w}x${ht} zoomed=$([G]::IsZoomed($h))"

if (-not [string]::IsNullOrEmpty($Drag)) {
  $d = $Drag.Split(":")
  $x1 = $ox + [int]$d[0]; $y1 = $oy + [int]$d[1]
  $x2 = $ox + [int]$d[2]; $y2 = $oy + [int]$d[3]
  MoveTo $x1 $y1
  Start-Sleep -Milliseconds 200
  [G]::mouse_event(0x0002, 0, 0, 0, 0) | Out-Null
  Start-Sleep -Milliseconds 120
  $steps = 14
  for ($i = 1; $i -le $steps; $i++) {
    $cx = $x1 + [int](($x2 - $x1) * $i / $steps)
    $cy = $y1 + [int](($y2 - $y1) * $i / $steps)
    MoveTo $cx $cy
    Start-Sleep -Milliseconds 45
  }
  [G]::mouse_event(0x0004, 0, 0, 0, 0) | Out-Null
  Start-Sleep -Milliseconds 400
}

$rb = New-Object G+RECT
[G]::GetWindowRect($h, [ref]$rb) | Out-Null
$rectAfter = "pos=($($rb.Left),$($rb.Top)) size=$(($rb.Right - $rb.Left))x$(($rb.Bottom - $rb.Top)) zoomed=$([G]::IsZoomed($h))"
Write-Output "BEFORE $rectBefore"
Write-Output "AFTER  $rectAfter"
$ox = $rb.Left; $oy = $rb.Top
$w = $rb.Right - $rb.Left; $ht = $rb.Bottom - $rb.Top

Start-Sleep -Milliseconds $After

$size = New-Object System.Drawing.Size($w, $ht)
$bmp = New-Object System.Drawing.Bitmap($w, $ht)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($ox, $oy, 0, 0, $size)
$g.Dispose()
$dir = "D:\RustEchoMusic\docs\screenshots"
if (!(Test-Path $dir)) { New-Item -ItemType Directory -Path $dir | Out-Null }
$out = Join-Path $dir "$Name.png"
$bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
Write-Output ("saved {0} size={1}x{2}" -f $out, $w, $ht)
