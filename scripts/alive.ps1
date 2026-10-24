param(
  [string]$Exe = "D:/RustEchoMusic/target/debug/echo-music.exe",
  [int]$Wait = 9000,
  # clicks in window coords: "x:y|x:y"
  [string]$Clicks = "170:290|1069:276"
)
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class M2 {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, int extra);
  public struct RECT { public int Left, Top, Right, Bottom; }
}
"@
$logFile = "D:\RustEchoMusic\.workbuddy\alive_app.log"
if (Test-Path $logFile) { Remove-Item $logFile -Force }
$env:ECHO_LOG_FILE = $logFile
$p = Start-Process -FilePath $Exe -PassThru
Start-Sleep -Milliseconds $Wait
$p.Refresh()
if ($p.HasExited) { Write-Output "EXITED_EARLY code=$($p.ExitCode)"; exit 0 }
$h = $p.MainWindowHandle
[M2]::ShowWindow($h, 5) | Out-Null
[M2]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 600
$r = New-Object M2+RECT
[M2]::GetWindowRect($h, [ref]$r) | Out-Null
$ox = $r.Left; $oy = $r.Top

foreach ($c in $Clicks.Split("|")) {
  if ($c.Trim() -eq "") { continue }
  $parts = $c.Split(":")
  Write-Output ("click {0},{1}" -f $parts[0], $parts[1])
  [M2]::SetCursorPos($ox + [int]$parts[0], $oy + [int]$parts[1]) | Out-Null
  Start-Sleep -Milliseconds 200
  [M2]::mouse_event(0x0002, 0, 0, 0, 0)
  Start-Sleep -Milliseconds 90
  [M2]::mouse_event(0x0004, 0, 0, 0, 0)
  Start-Sleep -Milliseconds 900
  $p.Refresh()
  if ($p.HasExited) { Write-Output "CRASHED_AFTER_CLICK code=$($p.ExitCode)"; exit 0 }
}
Start-Sleep -Milliseconds 1500
$p.Refresh()
if ($p.HasExited) { Write-Output "CRASHED_LATER code=$($p.ExitCode)" } else { Write-Output "ALIVE" }
Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
