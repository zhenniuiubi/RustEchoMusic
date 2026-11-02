param(
  [string]$Exe = "D:/RustEchoMusic/target/debug/EchoMusicLite.exe",
  [string]$Name = "tray_probe"
)
# Verify: tray icon thread starts, close button hides the bar (process stays),
# tray menu command ID_SHOW brings it back, ID_QUIT exits.
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public class TP {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, int data, int extra);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc cb, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder sb, int max);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowW(string cls, string title);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr wp, IntPtr lp);
  public delegate bool EnumWindowsProc(IntPtr h, IntPtr l);
  public struct RECT { public int Left, Top, Right, Bottom; }
  public static IntPtr FindByClass(uint pid, string cls) {
    IntPtr found = IntPtr.Zero;
    EnumWindows((h, l) => {
      uint wpid; GetWindowThreadProcessId(h, out wpid);
      if (wpid != pid) return true;
      var sb = new StringBuilder(256); GetClassName(h, sb, 256);
      if (sb.ToString() == cls) { found = h; return false; }
      return true;
    }, IntPtr.Zero);
    return found;
  }
  public static IntPtr FindMain(uint pid) {
    IntPtr best = IntPtr.Zero; long area = 0;
    EnumWindows((h, l) => {
      uint wpid; GetWindowThreadProcessId(h, out wpid);
      if (wpid != pid) return true;
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

$env:ECHO_LOG_FILE = "D:\RustEchoMusic\.workbuddy\tray_probe.log"
Remove-Item Env:ECHO_SMOKE -ErrorAction SilentlyContinue
$p = Start-Process -FilePath $Exe -PassThru
Start-Sleep -Milliseconds 6000
$h = [TP]::FindMain([uint32]$p.Id)
if ($h -eq [IntPtr]::Zero) { Stop-Process -Id $p.Id -Force; Write-Output "NO_WINDOW"; exit 1 }
Write-Output "main=$h visible=$([TP]::IsWindowVisible($h))"

$tray = [TP]::FindByClass([uint32]$p.Id, "EchoMusicTrayWnd")
if ($tray -eq [IntPtr]::Zero) {
  # 退一步：从日志里拿 hwnd（win.rs 打的 "tray ready wnd=0x..."）
  $m = Select-String -Path "D:\RustEchoMusic\.workbuddy\tray_probe.log" -Pattern 'tray ready wnd=0x([0-9a-fA-F]+)' | Select-Object -Last 1
  if ($m -ne $null) {
    $tray = [IntPtr][Convert]::ToInt64($m.Matches[0].Groups[1].Value, 16)
  }
}
Write-Output "tray_wnd=$tray"

[TP]::ShowWindow($h, 5) | Out-Null
[TP]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 500

$r = New-Object TP+RECT
[TP]::GetWindowRect($h, [ref]$r) | Out-Null
$ox = $r.Left; $oy = $r.Top
$w = $r.Right - $r.Left; $ht = $r.Bottom - $r.Top

# close button: left rail, top (client ~20,19 / window +8,+4)
$cx = $ox + 20 + 8; $cy = $oy + 19 + 4
[TP]::SetCursorPos($cx - 10, $cy - 8) | Out-Null
Start-Sleep -Milliseconds 120
[TP]::mouse_event(0x0001, 0, 0, 0, 0) | Out-Null
Start-Sleep -Milliseconds 120
[TP]::SetCursorPos($cx, $cy) | Out-Null
Start-Sleep -Milliseconds 120
[TP]::mouse_event(0x0001, 0, 0, 0, 0) | Out-Null
Start-Sleep -Milliseconds 100
[TP]::mouse_event(0x0002, 0, 0, 0, 0) | Out-Null
Start-Sleep -Milliseconds 80
[TP]::mouse_event(0x0004, 0, 0, 0, 0) | Out-Null
Start-Sleep -Milliseconds 700

$alive = (Get-Process -Id $p.Id -ErrorAction SilentlyContinue) -ne $null
Write-Output "after_close: alive=$alive visible=$([TP]::IsWindowVisible($h))"

# tray menu command 1 = ID_SHOW
if ($tray -ne [IntPtr]::Zero) {
  [TP]::PostMessage($tray, 0x0111, [IntPtr]1, [IntPtr]::Zero) | Out-Null
  Start-Sleep -Milliseconds 900
  Write-Output "after_show: visible=$([TP]::IsWindowVisible($h))"
  # menu command 5 = ID_QUIT
  [TP]::PostMessage($tray, 0x0111, [IntPtr]5, [IntPtr]::Zero) | Out-Null
  Start-Sleep -Milliseconds 1200
  $alive2 = (Get-Process -Id $p.Id -ErrorAction SilentlyContinue) -ne $null
  Write-Output "after_quit: alive=$alive2"
} else {
  Write-Output "TRAY_WINDOW_NOT_FOUND"
}
if ((Get-Process -Id $p.Id -ErrorAction SilentlyContinue) -ne $null) { Stop-Process -Id $p.Id -Force }
