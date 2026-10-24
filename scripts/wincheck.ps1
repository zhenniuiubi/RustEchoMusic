# Window behaviour check: maximize/restore button, titlebar drag, window title.
# Usage: wincheck.ps1 [-Exe <path>]
# NOTE: keep this file ASCII-only (PowerShell 5.1 reads .ps1 as ANSI, UTF-8 comments break the parser).
param(
  [string]$Exe = "D:\RustEchoMusic\target\debug\echo-music.exe",
  [int]$Wait = 13000
)
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public class WC {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, int d, int e);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder sb, int max);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder sb, int max);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  public struct RECT { public int Left, Top, Right, Bottom; }
  public struct POINT { public int X, Y; }

  // client origin in screen coords + client size
  public static int[] ClientBox(IntPtr h) {
    POINT p = new POINT();
    ClientToScreen(h, ref p);
    RECT r; GetClientRect(h, out r);
    return new int[] { p.X, p.Y, r.Right - r.Left, r.Bottom - r.Top };
  }

  public static IntPtr Find(uint pid) {
    IntPtr best = IntPtr.Zero; long area = 0;
    EnumWindows((h, l) => {
      uint p; GetWindowThreadProcessId(h, out p);
      if (p != pid || !IsWindowVisible(h)) return true;
      var sb = new StringBuilder(256); GetClassName(h, sb, 256);
      if (sb.ToString() == "ConsoleWindowClass") return true;
      RECT r; GetWindowRect(h, out r);
      long a = (long)(r.Right - r.Left) * (r.Bottom - r.Top);
      if (a > area) { area = a; best = h; }
      return true;
    }, IntPtr.Zero);
    return best;
  }
  public static void ClickAt(int x, int y) {
    SetCursorPos(x, y);
    System.Threading.Thread.Sleep(140);
    mouse_event(0x0002, 0, 0, 0, 0);
    System.Threading.Thread.Sleep(90);
    mouse_event(0x0004, 0, 0, 0, 0);
  }
  public static void DragTo(int x1, int y1, int x2, int y2) {
    SetCursorPos(x1, y1);
    System.Threading.Thread.Sleep(180);
    mouse_event(0x0002, 0, 0, 0, 0);
    System.Threading.Thread.Sleep(180);
    int steps = 14;
    for (int i = 1; i <= steps; i++) {
      SetCursorPos(x1 + (x2 - x1) * i / steps, y1 + (y2 - y1) * i / steps);
      System.Threading.Thread.Sleep(45);
    }
    System.Threading.Thread.Sleep(180);
    mouse_event(0x0004, 0, 0, 0, 0);
  }
  public static string Title(IntPtr h) {
    var sb = new StringBuilder(512); GetWindowTextW(h, sb, 512); return sb.ToString();
  }
}
"@

$log = "D:\RustEchoMusic\.workbuddy\wincheck.log"
if (Test-Path $log) { Remove-Item $log -Force }
# kill any leftover instance first, otherwise clicks may land on an older window
Get-Process echo-music, EchoMusic -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 800
$env:ECHO_LOG_FILE = $log
$p = Start-Process -FilePath $Exe -PassThru
Start-Sleep -Milliseconds $Wait
$h = [WC]::Find([uint32]$p.Id)
if ($h -eq [IntPtr]::Zero) { Write-Output "no window"; exit 1 }
[WC]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 500

function Get-RectStr {
  param($hw)
  $rr = New-Object WC+RECT
  [WC]::GetWindowRect($hw, [ref]$rr) | Out-Null
  return ("{0},{1} {2}x{3}" -f $rr.Left, $rr.Top, ($rr.Right - $rr.Left), ($rr.Bottom - $rr.Top))
}

function Save-Shot {
  param($hw, $path)
  $rr = New-Object WC+RECT
  [WC]::GetWindowRect($hw, [ref]$rr) | Out-Null
  $w = $rr.Right - $rr.Left
  $hh = $rr.Bottom - $rr.Top
  if ($w -le 0 -or $hh -le 0) { return }
  $bmp = New-Object System.Drawing.Bitmap($w, $hh)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  try {
    $g.CopyFromScreen($rr.Left, $rr.Top, 0, 0, (New-Object System.Drawing.Size($w, $hh)))
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
  }
  catch {
    Write-Output ("shot fail: " + $_.Exception.Message)
  }
  finally {
    $g.Dispose()
    $bmp.Dispose()
  }
}

Write-Output ("start: {0} zoomed={1} title='{2}'" -f (Get-RectStr $h), [WC]::IsZoomed($h), [WC]::Title($h))

# maximize button sits at (clientWidth - 63, 30) inside the client area
function Get-MaxPoint {
  param($hw)
  $b = [WC]::ClientBox($hw)
  return @(($b[0] + $b[2] - 63), ($b[1] + 30))
}

$pt = Get-MaxPoint $h
Write-Output ("max btn point: {0},{1} client={2}" -f $pt[0], $pt[1], (([WC]::ClientBox($h)) -join ','))
[WC]::ClickAt($pt[0], $pt[1])
Start-Sleep -Milliseconds 1400
Write-Output ("after max-click: {0} zoomed={1} visible={2}" -f (Get-RectStr $h), [WC]::IsZoomed($h), [WC]::IsWindowVisible($h))

$pt = Get-MaxPoint $h
Write-Output ("restore btn point: {0},{1} client={2}" -f $pt[0], $pt[1], (([WC]::ClientBox($h)) -join ','))
[WC]::ClickAt($pt[0], $pt[1])
Start-Sleep -Milliseconds 1400
Write-Output ("after restore-click: {0} zoomed={1} visible={2}" -f (Get-RectStr $h), [WC]::IsZoomed($h), [WC]::IsWindowVisible($h))

# drag the empty part of the titlebar: the window should follow the mouse
$b = [WC]::ClientBox($h)
$before = (Get-RectStr $h)
[WC]::DragTo(($b[0] + 800), ($b[1] + 30), ($b[0] + 700), ($b[1] + 180))
Start-Sleep -Milliseconds 900
Write-Output ("after drag: {0} (before {1})" -f (Get-RectStr $h), $before)

# double click on the titlebar empty area = maximize / restore
$b = [WC]::ClientBox($h)
[WC]::ClickAt(($b[0] + 700), ($b[1] + 30))
Start-Sleep -Milliseconds 90
[WC]::ClickAt(($b[0] + 700), ($b[1] + 30))
Start-Sleep -Milliseconds 1200
Write-Output ("after dblclick: {0} zoomed={1}" -f (Get-RectStr $h), [WC]::IsZoomed($h))
$b = [WC]::ClientBox($h)
[WC]::ClickAt(($b[0] + 700), ($b[1] + 30))
Start-Sleep -Milliseconds 90
[WC]::ClickAt(($b[0] + 700), ($b[1] + 30))
Start-Sleep -Milliseconds 1200
Write-Output ("after dblclick2: {0} zoomed={1}" -f (Get-RectStr $h), [WC]::IsZoomed($h))

# play a song, then read the window title
$b = [WC]::ClientBox($h)
[WC]::ClickAt(($b[0] + 556), ($b[1] + 588))
Start-Sleep -Milliseconds 7000
Write-Output ("title after play: '{0}'" -f [WC]::Title($h))

Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
Write-Output "--- log ---"
if (Test-Path $log) { Get-Content $log | Select-String -Pattern "toggle_maximize|start_drag|PANIC" }

