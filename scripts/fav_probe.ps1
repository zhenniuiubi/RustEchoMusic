param(
  [string]$Exe = "D:/RustEchoMusic/target/debug/EchoMusicLite.exe",
  [string]$Name = "fav_probe",
  [string]$Query = "",
  [int]$Notches = -25
)
# Probe for the Lite favorites panel: opens the panel (via ECHO_SMOKE=favsearch),
# dumps window ex-style (taskbar visibility bits), scrolls the list, then grabs a screenshot.
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class FP {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, int data, int extra);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc cb, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr h, int idx);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, System.Text.StringBuilder sb, int max);
  public delegate bool EnumWindowsProc(IntPtr h, IntPtr l);
  public struct RECT { public int Left, Top, Right, Bottom; }
  public static IntPtr Find(uint pid) {
    IntPtr best = IntPtr.Zero; long area = 0;
    EnumWindows((h, l) => {
      uint wpid; GetWindowThreadProcessId(h, out wpid);
      if (wpid != pid || !IsWindowVisible(h)) return true;
      var sb = new System.Text.StringBuilder(256); GetClassName(h, sb, 256);
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

$env:ECHO_LOG_FILE = "D:\RustEchoMusic\.workbuddy\fav_probe.log"
$env:ECHO_SMOKE = "favsearch"
$env:ECHO_SMOKE_Q = $Query

$p = Start-Process -FilePath $Exe -PassThru
Start-Sleep -Milliseconds 7000
$h = [FP]::Find([uint32]$p.Id)
if ($h -eq [IntPtr]::Zero) { Stop-Process -Id $p.Id -Force; Write-Output "NO_WINDOW"; exit 1 }
[FP]::ShowWindow($h, 5) | Out-Null
[FP]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 600

$ex = [FP]::GetWindowLong($h, -20)
$tool = ($ex -band 0x80) -ne 0
$appwin = ($ex -band 0x40000) -ne 0
Write-Output ("EXSTYLE=0x{0:X} TOOLWINDOW={1} APPWINDOW={2}" -f $ex, $tool, $appwin)

$r = New-Object FP+RECT
[FP]::GetWindowRect($h, [ref]$r) | Out-Null
$ox = $r.Left; $oy = $r.Top
$w = $r.Right - $r.Left; $ht = $r.Bottom - $r.Top
Write-Output "RECT=${w}x${ht} at ($ox,$oy)"

# hover inside the list area first (hitbox warm-up), then wheel
$cx = $ox + [int]($w / 2)
$cy = $oy + $ht - 60
[FP]::SetCursorPos($cx, $cy) | Out-Null
Start-Sleep -Milliseconds 150
[FP]::mouse_event(0x0001, 0, 0, 0, 0) | Out-Null
Start-Sleep -Milliseconds 200
for ($i = 0; $i -lt 8; $i++) {
  [FP]::mouse_event(0x0800, 0, 0, $Notches, 0) | Out-Null
  Start-Sleep -Milliseconds 90
}
Start-Sleep -Milliseconds 700

$bmp = New-Object System.Drawing.Bitmap $w, $ht
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($ox, $oy, 0, 0, (New-Object System.Drawing.Size $w, $ht))
$out = "D:\RustEchoMusic\docs\screenshots\$Name.png"
$bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
Write-Output "saved $out"

Stop-Process -Id $p.Id -Force
