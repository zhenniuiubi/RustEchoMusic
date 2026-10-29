param(
  [string]$Exe = "D:\RustEchoMusic\target\debug\EchoMusicLite.exe",
  [int]$Wait = 9000,
  [string]$Click = "28:12"
)
$ErrorActionPreference = "Stop"
Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public class CT {
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
$p = Start-Process -FilePath $Exe -PassThru
Start-Sleep -Milliseconds $Wait
$h = [CT]::Find([uint32]$p.Id)
if ($h -eq [IntPtr]::Zero) { Stop-Process -Id $p.Id -Force; Write-Output "NO_WINDOW"; exit 1 }
$r = New-Object CT+RECT
[CT]::GetWindowRect($h, [ref]$r) | Out-Null
$parts = $Click.Split(":")
$x = $r.Left + [int]$parts[0]; $y = $r.Top + [int]$parts[1]
[CT]::SetCursorPos(($x - 4), ($y - 4)) | Out-Null
Start-Sleep -Milliseconds 200
[CT]::mouse_event(0x0001, 0, 0, 0, 0) | Out-Null
[CT]::SetCursorPos($x, $y) | Out-Null
Start-Sleep -Milliseconds 200
[CT]::mouse_event(0x0002, 0, 0, 0, 0) | Out-Null
Start-Sleep -Milliseconds 80
[CT]::mouse_event(0x0004, 0, 0, 0, 0) | Out-Null
Start-Sleep -Milliseconds 1500
$alive = $null -ne (Get-Process -Id $p.Id -ErrorAction SilentlyContinue)
Write-Output "alive_after_click=$alive"
if ($alive) { Stop-Process -Id $p.Id -Force }
