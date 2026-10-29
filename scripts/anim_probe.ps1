param(
  [string]$Exe = "D:\RustEchoMusic\target\debug\EchoMusicLite.exe",
  [int]$Wait = 9000,
  [string]$Click = "20:51",
  [int]$Samples = 16,
  [int]$Step = 25
)
$ErrorActionPreference = "Stop"
Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public class AP {
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
Remove-Item Env:ECHO_SMOKE -ErrorAction SilentlyContinue
$p = Start-Process -FilePath $Exe -PassThru
Start-Sleep -Milliseconds $Wait
$h = [AP]::Find([uint32]$p.Id)
if ($h -eq [IntPtr]::Zero) { Stop-Process -Id $p.Id -Force; Write-Output "NO_WINDOW"; exit 1 }

$script:rr = New-Object AP+RECT
function H() {
  [AP]::GetWindowRect($h, [ref]$script:rr) | Out-Null
  return [int]($script:rr.Bottom - $script:rr.Top)
}
$r0 = New-Object AP+RECT
[AP]::GetWindowRect($h, [ref]$r0) | Out-Null

$parts = $Click.Split(":")
$x = $r0.Left + [int]$parts[0]; $y = $r0.Top + [int]$parts[1]
[AP]::SetCursorPos(($x - 10), ($y - 10)) | Out-Null
Start-Sleep -Milliseconds 150
[AP]::mouse_event(0x0001, 0, 0, 0, 0) | Out-Null
[AP]::SetCursorPos($x, $y) | Out-Null
Start-Sleep -Milliseconds 150
[AP]::mouse_event(0x0001, 0, 0, 0, 0) | Out-Null
Start-Sleep -Milliseconds 60

[AP]::mouse_event(0x0002, 0, 0, 0, 0) | Out-Null
Start-Sleep -Milliseconds 50
$sw = [System.Diagnostics.Stopwatch]::StartNew()
[AP]::mouse_event(0x0004, 0, 0, 0, 0) | Out-Null
$out = @()
for ($i = 0; $i -lt $Samples; $i++) {
  $tmp = New-Object AP+RECT
  [AP]::GetWindowRect($h, [ref]$tmp) | Out-Null
  $hgt = [int]($tmp.Bottom - $tmp.Top)
  $out += ("{0}ms={1}" -f [int]$sw.ElapsedMilliseconds, $hgt)
  Start-Sleep -Milliseconds $Step
}
Write-Output ($out -join "  ")
Start-Sleep -Milliseconds 600
$tf = New-Object AP+RECT
[AP]::GetWindowRect($h, [ref]$tf) | Out-Null
Write-Output ("final=" + [int]($tf.Bottom - $tf.Top))
Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
