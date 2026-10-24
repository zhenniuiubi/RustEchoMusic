param(
  [string]$Name = "click",
  [string]$Smoke = "",
  [int]$Wait = 8000,
  # clicks: "x:y|x:y" in window-relative coords
  [string]$Clicks = "",
  # drag: "x1:y1:x2:y2"
  [string]$Drag = "",
  [int]$After = 1200,
  # keys: 点击后发送的按键（SendKeys 语法），用于验证输入框焦点
  [string]$Keys = "",
  # text: 点击后逐字符 PostMessage(WM_CHAR) 输入（可输入中文，绕开键盘布局/IME）
  [string]$Text = "",
  # clicks2: 输入文字之后的点击（"x:y|x:y"），用于「输入→点结果」
  [string]$Clicks2 = "",
  # text2: 在 clicks2 之后再输入一次（验证失焦后打字无效）
  [string]$Text2 = "",
  # enter: 输入文字后按一次回车（提交搜索）
  [switch]$Enter,
  # wheel: 滚轮 "x:y:notches"（负值向下）
  [string]$Wheel = "",
  # noshot: 不落截图（只看日志/存活状态时用）
  [switch]$NoShot,
  [string]$Exe = "D:/RustEchoMusic/target/debug/echo-music.exe"
)
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class M {
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, int data, int extra);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr SendMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc cb, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, System.Text.StringBuilder sb, int max);
  public delegate bool EnumWindowsProc(IntPtr h, IntPtr l);
  public struct RECT { public int Left, Top, Right, Bottom; }

  // 找出进程里最大的可见窗口，跳过控制台窗口
  // （debug 构建带控制台，MainWindowHandle 可能指到控制台，截图就拍到黑窗口）
  public static IntPtr FindAppWindow(uint pid) {
    IntPtr best = IntPtr.Zero;
    long bestArea = 0;
    EnumWindows((h, l) => {
      uint wpid;
      GetWindowThreadProcessId(h, out wpid);
      if (wpid != pid || !IsWindowVisible(h)) return true;
      var sb = new System.Text.StringBuilder(256);
      GetClassName(h, sb, 256);
      if (sb.ToString() == "ConsoleWindowClass") return true;
      RECT r;
      GetWindowRect(h, out r);
      long area = (long)(r.Right - r.Left) * (r.Bottom - r.Top);
      if (area > bestArea) { bestArea = area; best = h; }
      return true;
    }, IntPtr.Zero);
    return best;
  }
}
"@

if ($Smoke -ne "") { $env:ECHO_SMOKE = $Smoke } else { Remove-Item Env:ECHO_SMOKE -ErrorAction SilentlyContinue }
$logFile = "D:\RustEchoMusic\.workbuddy\click_app.log"
if (Test-Path $logFile) { Remove-Item $logFile -Force }
$env:ECHO_LOG_FILE = $logFile
$p = Start-Process -FilePath $Exe -PassThru
Start-Sleep -Milliseconds $Wait
$p.Refresh()
$h = [M]::FindAppWindow([uint32]$p.Id)
if ($h -eq [IntPtr]::Zero) {
  Start-Sleep -Seconds 2
  $h = [M]::FindAppWindow([uint32]$p.Id)
}
[M]::ShowWindow($h, 5) | Out-Null
[M]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 600

$r = New-Object M+RECT
[M]::GetWindowRect($h, [ref]$r) | Out-Null
$ox = $r.Left
$oy = $r.Top

function ClickAt([int]$x, [int]$y) {
  [M]::SetCursorPos($ox + $x, $oy + $y) | Out-Null
  Start-Sleep -Milliseconds 180
  [M]::mouse_event(0x0002, 0, 0, 0, 0)
  Start-Sleep -Milliseconds 90
  [M]::mouse_event(0x0004, 0, 0, 0, 0)
  Start-Sleep -Milliseconds 700
}

if (-not [string]::IsNullOrEmpty($Clicks)) {
  foreach ($c in $Clicks.Split("|")) {
    if ($c.Trim() -eq "") { continue }
    $parts = $c.Split(":")
    Write-Output ("click {0},{1}" -f $parts[0], $parts[1])
    ClickAt ([int]$parts[0]) ([int]$parts[1])
  }
}

if (-not [string]::IsNullOrEmpty($Text)) {
  Start-Sleep -Milliseconds 300
  foreach ($ch in $Text.ToCharArray()) {
    [M]::SendMessage($h, 0x0102, [IntPtr][int]$ch, [IntPtr]0) | Out-Null
    Start-Sleep -Milliseconds 70
  }
  Start-Sleep -Milliseconds 800
}

if (-not [string]::IsNullOrEmpty($Keys)) {
  Start-Sleep -Milliseconds 300
  [System.Windows.Forms.SendKeys]::SendWait($Keys)
  Start-Sleep -Milliseconds 600
}

if ($Enter) {
  Start-Sleep -Milliseconds 250
  [M]::SendMessage($h, 0x0100, [IntPtr]13, [IntPtr]0x001C0001) | Out-Null
  Start-Sleep -Milliseconds 60
  [M]::SendMessage($h, 0x0101, [IntPtr]13, [IntPtr]0xC01C0001) | Out-Null
  Start-Sleep -Milliseconds 900
}

if (-not [string]::IsNullOrEmpty($Wheel)) {
  $wparts = $Wheel.Split(":")
  $wx = [int]$wparts[0]; $wy = [int]$wparts[1]; $wn = [int]$wparts[2]
  [M]::SetCursorPos($ox + $wx, $oy + $wy) | Out-Null
  Start-Sleep -Milliseconds 300
  for ($i = 0; $i -lt [Math]::Abs($wn); $i++) {
    [M]::mouse_event(0x0800, 0, 0, [Math]::Sign($wn) * 120, 0)
    Start-Sleep -Milliseconds 120
  }
  Start-Sleep -Milliseconds 900
}

if (-not [string]::IsNullOrEmpty($Clicks2)) {
  foreach ($c in $Clicks2.Split("|")) {
    if ($c.Trim() -eq "") { continue }
    $parts = $c.Split(":")
    Write-Output ("click2 {0},{1}" -f $parts[0], $parts[1])
    ClickAt ([int]$parts[0]) ([int]$parts[1])
  }
}

if (-not [string]::IsNullOrEmpty($Text2)) {
  Start-Sleep -Milliseconds 300
  foreach ($ch in $Text2.ToCharArray()) {
    [M]::SendMessage($h, 0x0102, [IntPtr][int]$ch, [IntPtr]0) | Out-Null
    Start-Sleep -Milliseconds 70
  }
  Start-Sleep -Milliseconds 800
}

if (-not [string]::IsNullOrEmpty($Drag)) {
  $d = $Drag.Split(":")
  $x1 = [int]$d[0]; $y1 = [int]$d[1]; $x2 = [int]$d[2]; $y2 = [int]$d[3]
  [M]::SetCursorPos($ox + $x1, $oy + $y1) | Out-Null
  Start-Sleep -Milliseconds 200
  [M]::mouse_event(0x0002, 0, 0, 0, 0)
  Start-Sleep -Milliseconds 150
  $steps = 12
  for ($i = 1; $i -le $steps; $i++) {
    $cx = $x1 + [int](($x2 - $x1) * $i / $steps)
    $cy = $y1 + [int](($y2 - $y1) * $i / $steps)
    [M]::SetCursorPos($ox + $cx, $oy + $cy) | Out-Null
    Start-Sleep -Milliseconds 60
  }
  [M]::mouse_event(0x0004, 0, 0, 0, 0)
  Start-Sleep -Milliseconds 400
}

Start-Sleep -Milliseconds $After

$p.Refresh()
Write-Output ("alive={0} exitcode={1}" -f (-not $p.HasExited), $p.ExitCode)

if ($NoShot) {
  Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
  Write-Output "noshot"
  exit 0
}

$w = $r.Right - $r.Left
$ht = $r.Bottom - $r.Top
$bmp = New-Object System.Drawing.Bitmap($w, $ht)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$hdc = $g.GetHdc()
[M]::PrintWindow($h, $hdc, 2) | Out-Null
$g.ReleaseHdc($hdc)
$g.Dispose()

$dir = "D:\RustEchoMusic\docs\screenshots"
if (!(Test-Path $dir)) { New-Item -ItemType Directory -Path $dir | Out-Null }
$out = Join-Path $dir "$Name.png"
$bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()

Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
Write-Output $out
