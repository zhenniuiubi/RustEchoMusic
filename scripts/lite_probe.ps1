param(
  [string]$Name = "lite",
  [int]$Wait = 8000,
  [string]$Clicks = "",
  [string]$Smoke = "",
  [string]$Exe = "D:\RustEchoMusic\target\debug\EchoMusicLite.exe"
)
$ErrorActionPreference = "Stop"
Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public class LG {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder sb, int max);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, int d, int e);
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  public struct RECT { public int Left, Top, Right, Bottom; }
  public static IntPtr Find(uint pid) {
    IntPtr best = IntPtr.Zero; long area = 0;
    EnumWindows((h, l) => {
      uint wp; GetWindowThreadProcessId(h, out wp);
      if (wp != pid || !IsWindowVisible(h)) return true;
      var sb = new StringBuilder(256); GetClassName(h, sb, 256);
      if (sb.ToString() == "ConsoleWindowClass") return true;
      RECT r; GetWindowRect(h, out r);
      long a = (long)(r.Right - r.Left) * (r.Bottom - r.Top);
      if (a > area) { area = a; best = h; }
      return true;
    }, IntPtr.Zero);
    return best;
  }
}
"@

$env:ECHO_LOG_FILE = "D:\RustEchoMusic\.workbuddy\shot_app.log"
if ($Smoke -ne "") { $env:ECHO_SMOKE = $Smoke } else { Remove-Item Env:ECHO_SMOKE -ErrorAction SilentlyContinue }
$p = Start-Process -FilePath $Exe -PassThru
Start-Sleep -Milliseconds $Wait
$h = [LG]::Find([uint32]$p.Id)
if ($h -eq [IntPtr]::Zero) { Stop-Process -Id $p.Id -Force; Write-Output "NO_WINDOW"; exit 1 }

function Rect() {
  $r = New-Object LG+RECT
  [LG]::GetWindowRect($h, [ref]$r) | Out-Null
  return "$($r.Left),$($r.Top) $($r.Right - $r.Left)x$($r.Bottom - $r.Top)"
}
Write-Output "initial: $(Rect)"

$r0 = New-Object LG+RECT
[LG]::GetWindowRect($h, [ref]$r0) | Out-Null
$ox = $r0.Left; $oy = $r0.Top

if ($Clicks -ne "") {
  foreach ($c in $Clicks.Split("|")) {
    if ($c.Trim() -eq "") { continue }
    $parts = $c.Split(":")
    $x = $ox + [int]$parts[0]; $y = $oy + [int]$parts[1]
    [LG]::SetCursorPos($x - 8, $y - 8) | Out-Null
    Start-Sleep -Milliseconds 120
    [LG]::mouse_event(0x0001, 0, 0, 0, 0) | Out-Null
    [LG]::SetCursorPos($x, $y) | Out-Null
    Start-Sleep -Milliseconds 120
    [LG]::mouse_event(0x0001, 0, 0, 0, 0) | Out-Null
    Start-Sleep -Milliseconds 150
    [LG]::mouse_event(0x0002, 0, 0, 0, 0) | Out-Null
    Start-Sleep -Milliseconds 100
    [LG]::mouse_event(0x0004, 0, 0, 0, 0) | Out-Null
    Start-Sleep -Milliseconds 700
    Write-Output "after click $c : $(Rect)"
  }
}

Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
Write-Output "done"
