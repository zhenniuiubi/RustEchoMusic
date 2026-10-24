# 无鼠标验证窗口行为：直接给窗口 PostMessage（会话锁屏时鼠标自动化不可用）
# 用法：wintest.ps1 -Exe <path> -Post close|space|maximize
param(
  [string]$Exe = "D:\RustEchoMusic\target\debug\echo-music.exe",
  [string]$Post = "close",
  [int]$Wait = 12000
)

Add-Type -TypeDefinition @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public class W {
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern int GetClassName(IntPtr h, StringBuilder sb, int max);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string cls, IntPtr title);
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  public struct RECT { public int Left, Top, Right, Bottom; }
  public static IntPtr Find(uint pid) {
    IntPtr best = IntPtr.Zero; long area = 0;
    EnumWindows((h, l) => {
      uint p; GetWindowThreadProcessId(h, out p);
      if (p != pid) return true;
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

function Save-Shot($hwnd, $path) {
  $r = New-Object W+RECT
  [W]::GetWindowRect($hwnd, [ref]$r) | Out-Null
  $w = $r.Right - $r.Left; $hgt = $r.Bottom - $r.Top
  Add-Type -AssemblyName System.Drawing
  $bmp = New-Object System.Drawing.Bitmap($w, $hgt)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  try {
    $g.CopyFromScreen($r.Left, $r.Top, 0, 0, (New-Object System.Drawing.Size($w, $hgt)))
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    Write-Output ("shot -> {0} ({1}x{2})" -f $path, $w, $hgt)
  } catch {
    Write-Output ("shot failed: {0}" -f $_.Exception.Message)
  } finally {
    $g.Dispose(); $bmp.Dispose()
  }
}

$log = "D:\RustEchoMusic\.workbuddy\wintest.log"
if (Test-Path $log) { Remove-Item $log -Force }
$env:ECHO_LOG_FILE = $log
$p = Start-Process -FilePath $Exe -PassThru
Start-Sleep -Milliseconds $Wait
$h = [W]::Find([uint32]$p.Id)
if ($h -eq [IntPtr]::Zero) { Write-Output "no window"; exit 1 }

Write-Output ("hwnd={0} visible={1} zoomed={2}" -f $h, [W]::IsWindowVisible($h), [W]::IsZoomed($h))

if ($Post -eq "close") {
  [W]::PostMessage($h, 0x0010, [IntPtr]0, [IntPtr]0) | Out-Null   # WM_CLOSE
  Start-Sleep -Milliseconds 1200
  $p.Refresh()
  Write-Output ("after close: alive={0} visible={1}" -f (-not $p.HasExited), [W]::IsWindowVisible($h))
}
elseif ($Post -eq "maximize") {
  $r = New-Object W+RECT
  [W]::GetWindowRect($h, [ref]$r) | Out-Null
  Write-Output ("before: {0},{1} {2}x{3}" -f $r.Left, $r.Top, ($r.Right-$r.Left), ($r.Bottom-$r.Top))
  [W]::PostMessage($h, 0x0112, [IntPtr]0xF030, [IntPtr]0) | Out-Null  # SC_MAXIMIZE
  Start-Sleep -Milliseconds 900
  [W]::GetWindowRect($h, [ref]$r) | Out-Null
  Write-Output ("after maximize: zoomed={0} rect={1},{2} {3}x{4}" -f [W]::IsZoomed($h), $r.Left, $r.Top, ($r.Right-$r.Left), ($r.Bottom-$r.Top))
  Save-Shot $h "D:\RustEchoMusic\docs\screenshots\win_max.png"
  [W]::PostMessage($h, 0x0112, [IntPtr]0xF120, [IntPtr]0) | Out-Null  # SC_RESTORE
  Start-Sleep -Milliseconds 900
  Write-Output ("after restore: zoomed={0}" -f [W]::IsZoomed($h))
}
elseif ($Post -eq "f9") {
  [W]::PostMessage($h, 0x0100, [IntPtr]0x78, [IntPtr]0x00430001) | Out-Null
  Start-Sleep -Milliseconds 120
  [W]::PostMessage($h, 0x0101, [IntPtr]0x78, [IntPtr]0xC0430001) | Out-Null
  Start-Sleep -Milliseconds 900
  [W]::PostMessage($h, 0x0100, [IntPtr]0x78, [IntPtr]0x00430001) | Out-Null
  Start-Sleep -Milliseconds 120
  [W]::PostMessage($h, 0x0101, [IntPtr]0x78, [IntPtr]0xC0430001) | Out-Null
  Start-Sleep -Milliseconds 1200
  Write-Output ("after f9 x2: zoomed={0}" -f [W]::IsZoomed($h))
}
elseif ($Post -eq "tray") {
  # 1) 先关掉主窗口（隐藏），2) 用托盘菜单「显示主窗口」把它叫回来，3) 用托盘「退出」结束进程
  [W]::PostMessage($h, 0x0010, [IntPtr]0, [IntPtr]0) | Out-Null
  Start-Sleep -Milliseconds 1000
  Write-Output ("after close: visible={0}" -f [W]::IsWindowVisible($h))
  $tray = [W]::FindWindow("EchoMusicTrayWnd", [IntPtr]::Zero)
  Write-Output ("tray hwnd={0}" -f $tray)
  [W]::PostMessage($tray, 0x0111, [IntPtr]1, [IntPtr]0) | Out-Null   # WM_COMMAND ID_SHOW
  Start-Sleep -Milliseconds 1200
  Write-Output ("after tray show: visible={0}" -f [W]::IsWindowVisible($h))
  [W]::PostMessage($tray, 0x0111, [IntPtr]5, [IntPtr]0) | Out-Null   # WM_COMMAND ID_QUIT
  Start-Sleep -Milliseconds 1500
  $p.Refresh()
  Write-Output ("after tray quit: alive={0}" -f (-not $p.HasExited))
}
elseif ($Post -eq "space") {
  [W]::PostMessage($h, 0x0100, [IntPtr]0x20, [IntPtr]0x00390001) | Out-Null   # WM_KEYDOWN VK_SPACE
  Start-Sleep -Milliseconds 120
  [W]::PostMessage($h, 0x0101, [IntPtr]0x20, [IntPtr]0xC0390001) | Out-Null   # WM_KEYUP
  Start-Sleep -Milliseconds 1200
}

Start-Sleep -Milliseconds 300
Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
Write-Output "--- log ---"
if (Test-Path $log) { Get-Content $log | Select-String -Pattern "toggle_maximize|隐藏|空格|托盘|PANIC" }
