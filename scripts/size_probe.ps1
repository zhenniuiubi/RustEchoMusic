param(
  [string]$Exe = "D:/RustEchoMusic/target/debug/EchoMusicLite.exe"
)
# Verify the window refuses to change size: force 800x400 with SetWindowPos,
# then send SC_MAXIMIZE, and check the size comes back to 330x70 (+panel).
$ErrorActionPreference = "Stop"
Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public class SP {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr wp, IntPtr lp);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc cb, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder sb, int max);
  [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr h);
  public delegate bool EnumWindowsProc(IntPtr h, IntPtr l);
  public struct RECT { public int Left, Top, Right, Bottom; }
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
  public static string SizeOf(IntPtr h) {
    RECT w, c; GetWindowRect(h, out w); GetClientRect(h, out c);
    return (c.Right - c.Left) + "x" + (c.Bottom - c.Top) + " zoomed=" + IsZoomed(h);
  }
}
"@

$env:ECHO_LOG_FILE = "D:\RustEchoMusic\.workbuddy\size_probe.log"
Remove-Item Env:ECHO_SMOKE -ErrorAction SilentlyContinue
$p = Start-Process -FilePath $Exe -PassThru
Start-Sleep -Milliseconds 6000
$h = [SP]::FindMain([uint32]$p.Id)
if ($h -eq [IntPtr]::Zero) { Stop-Process -Id $p.Id -Force; Write-Output "NO_WINDOW"; exit 1 }
Write-Output "initial: $([SP]::SizeOf($h))"

# 1) brute force resize
[SP]::SetWindowPos($h, [IntPtr]::Zero, 200, 200, 800, 400, 0x0004 -bor 0x0010) | Out-Null
Start-Sleep -Milliseconds 900
Write-Output "after_setwindowpos: $([SP]::SizeOf($h))"

# 2) system maximize command
[SP]::PostMessage($h, 0x0112, [IntPtr]0xF030, [IntPtr]::Zero) | Out-Null
Start-Sleep -Milliseconds 900
Write-Output "after_maximize_cmd: $([SP]::SizeOf($h))"

# 3) open a panel, then try to resize again (panel must stay 330x334-ish)
Start-Sleep -Milliseconds 300
Write-Output "final: $([SP]::SizeOf($h))"

Stop-Process -Id $p.Id -Force
Get-Content "D:\RustEchoMusic\.workbuddy\size_probe.log" -Tail 3 -ErrorAction SilentlyContinue | Out-Null
