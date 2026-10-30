param(
  [int]$Wait = 40000,
  [string]$Clicks = "311:29",
  [int]$Count = 10,
  [int]$Step = 700,
  [string]$Exe = "D:/RustEchoMusic/target/debug/EchoMusicLite.exe",
  [string]$Smoke = "play"
)
# Continuous screenshots of the same running instance (used to catch a lyric
# word-by-word fill in progress, which only exists for a fraction of a second).
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class LB {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, System.Text.StringBuilder sb, int max);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, int data, int extra);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  public struct RECT { public int Left, Top, Right, Bottom; }
  public static IntPtr FindAppWindow(uint pid) {
    IntPtr best = IntPtr.Zero; long bestArea = 0;
    EnumWindows((h, l) => {
      uint wpid; GetWindowThreadProcessId(h, out wpid);
      if (wpid != pid || !IsWindowVisible(h)) return true;
      var sb = new System.Text.StringBuilder(256); GetClassName(h, sb, 256);
      if (sb.ToString() == "ConsoleWindowClass") return true;
      RECT r; GetWindowRect(h, out r);
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
$h = [LB]::FindAppWindow([uint32]$p.Id)
if ($h -eq [IntPtr]::Zero) { Stop-Process -Id $p.Id -Force; Write-Output "NO_WINDOW"; exit 1 }
[LB]::ShowWindow($h, 5) | Out-Null
[LB]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 700

$r = New-Object LB+RECT
[LB]::GetWindowRect($h, [ref]$r) | Out-Null
$ox = $r.Left; $oy = $r.Top
$w = $r.Right - $r.Left; $ht = $r.Bottom - $r.Top

function Shot($name) {
  $rr = New-Object LB+RECT
  [LB]::GetWindowRect($h, [ref]$rr) | Out-Null
  $ww = $rr.Right - $rr.Left; $hh = $rr.Bottom - $rr.Top
  $bmp = New-Object System.Drawing.Bitmap($ww, $hh)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($rr.Left, $rr.Top, 0, 0, (New-Object System.Drawing.Size($ww, $hh)))
  $g.Dispose()
  $out = "D:\RustEchoMusic\docs\screenshots\$name.png"
  $bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
  $bmp.Dispose()
}

# focus the window first, then click the lyric button in window coords
foreach ($c in $Clicks.Split("|")) {
  if ($c -eq "" -or $c -eq "none") { continue }
  $xy = $c.Split(":")
  $cx = $ox + [int]$xy[0]; $cy = $oy + [int]$xy[1]
  [LB]::SetCursorPos(($cx - 3), ($cy - 3)) | Out-Null
  Start-Sleep -Milliseconds 150
  [LB]::mouse_event(0x0001, 0, 0, 0, 0) | Out-Null
  [LB]::SetCursorPos($cx, $cy) | Out-Null
  Start-Sleep -Milliseconds 120
  [LB]::mouse_event(0x0002, 0, 0, 0, 0) | Out-Null
  Start-Sleep -Milliseconds 80
  [LB]::mouse_event(0x0004, 0, 0, 0, 0) | Out-Null
  Start-Sleep -Milliseconds 500
}
# park the cursor away from the bar so hover doesn't swap the title
[LB]::SetCursorPos(($ox + 4), ($oy + $ht + 40)) | Out-Null

for ($i = 0; $i -lt $Count; $i++) {
  Shot ("burst{0:d2}" -f $i)
  Start-Sleep -Milliseconds $Step
}
Stop-Process -Id $p.Id -Force
Write-Output "BURST_DONE"
