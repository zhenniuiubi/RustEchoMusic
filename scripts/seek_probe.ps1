param(
  [string]$Name = "seek",
  [string]$Exe = "D:\RustEchoMusic\target\debug\EchoMusicLite.exe",
  [int]$Boot = 14000,
  [string]$Bar = "",
  [string]$Log = "D:\RustEchoMusic\.workbuddy\seek_test.log"
)
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing

# NOTE: PS 5.1 reads a BOM-less UTF-8 file as GBK, so this file is ASCII-only.
# All P/Invoke wrappers return int[] instead of taking [ref] structs.
if (-not ("SeekProbe" -as [type])) { Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public class SeekProbe {
  [DllImport("user32.dll")] static extern bool GetClientRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] static extern bool ClientToScreen(IntPtr h, ref POINT p);
  [DllImport("user32.dll")] static extern bool EnumWindows(EP cb, IntPtr l);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder sb, int max);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, int d, int e);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  public static bool IsVisible(IntPtr h) { return IsWindowVisible(h); }
  delegate bool EP(IntPtr h, IntPtr l);
  struct RECT { public int L, T, R, B; }
  struct POINT { public int X, Y; }

  static IntPtr best; static long bestArea;
  static bool Scan(IntPtr h, IntPtr l) {
    uint wp; GetWindowThreadProcessId(h, out wp);
    if (wp != (uint)l || !IsWindowVisible(h)) return true;
    var sb = new StringBuilder(256); GetClassName(h, sb, 256);
    if (sb.ToString() == "ConsoleWindowClass") return true;
    RECT r; GetClientRect(h, out r);
    long a = (long)(r.R - r.L) * (r.B - r.T);
    if (a > bestArea) { bestArea = a; best = h; }
    return true;
  }
  public static IntPtr Find(uint pid) {
    best = IntPtr.Zero; bestArea = 0;
    EnumWindows(Scan, (IntPtr)pid);
    return best;
  }
  public static int[] ClientInfo(IntPtr h) {
    RECT r; GetClientRect(h, out r);
    POINT p; p.X = 0; p.Y = 0; ClientToScreen(h, ref p);
    return new int[] { p.X, p.Y, r.R - r.L, r.B - r.T };
  }
}
"@ }

# --- optional: switch the progress-bar style ---
# Set-Content -Encoding utf8 in PS 5.1 writes a BOM, which makes serde_json fail;
# the app then falls back to defaults and overwrites the file on exit.
# So: back up the real file, patch with a BOM-less writer, restore it afterwards.
$sp = Join-Path $env:APPDATA "EchoMusic\settings.json"
$spBak = ""
if (($Bar -ne "") -and (Test-Path $sp)) {
  $spBak = $sp + ".seekprobe.bak"
  Copy-Item $sp $spBak -Force
  $j = Get-Content $sp -Raw | ConvertFrom-Json
  $j.lite_bar = $Bar
  $json = $j | ConvertTo-Json -Depth 6
  [System.IO.File]::WriteAllText($sp, $json, (New-Object System.Text.UTF8Encoding($false)))
  Write-Output ("settings.lite_bar -> " + $Bar + " (backup " + $spBak + ")")
}

if (Test-Path $Log) { Remove-Item $Log -Force -ErrorAction SilentlyContinue }
$env:ECHO_LOG_FILE = $Log
$env:ECHO_SMOKE = "seek"

$p = Start-Process -FilePath $Exe -PassThru
try {
  Start-Sleep -Milliseconds $Boot
  $h = [SeekProbe]::Find([uint32]$p.Id)
  if ($h -eq [IntPtr]::Zero) { Write-Output "NO_WINDOW"; exit 1 }
  [SeekProbe]::ShowWindow($h, 5) | Out-Null
  [SeekProbe]::SetForegroundWindow($h) | Out-Null
  Start-Sleep -Milliseconds 600

  $ci = [SeekProbe]::ClientInfo($h)
  $ox = $ci[0]; $oy = $ci[1]; $cw = $ci[2]; $ch = $ci[3]
  $scale = [double]$cw / 330.0
  Write-Output ("client: origin=({0},{1}) phys={2}x{3} scale={4:N4}" -f $ox, $oy, $cw, $ch, $scale)

  # The app holds the log open; read it with FileShare.ReadWrite.
  $fs = [System.IO.File]::Open($Log, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::ReadWrite)
  $sr = New-Object System.IO.StreamReader($fs)
  $txt = $sr.ReadToEnd()
  $sr.Close(); $fs.Close()
  $mm = [regex]::Matches($txt, 'smoke-rect\] bar=Some\(\(([\d\.\-]+), ([\d\.\-]+), ([\d\.\-]+), ([\d\.\-]+)\)')
  if ($mm.Count -eq 0) { Write-Output "NO_RECT_LOG"; exit 1 }
  $gr = $mm[$mm.Count - 1].Groups
  $bx = [double]$gr[1].Value; $by = [double]$gr[2].Value
  $bw = [double]$gr[3].Value; $bh = [double]$gr[4].Value
  Write-Output ("bar(logical): x={0} y={1} w={2} h={3}" -f $bx, $by, $bw, $bh)
  $cy = $by + $bh / 2.0

  function DumpRect([string]$tag) {
    $c = [SeekProbe]::ClientInfo($h)
    $vis = [SeekProbe]::IsVisible($h)
    Write-Output ("  rect[{0}] origin=({1},{2}) {3}x{4} visible={5}" -f $tag, $c[0], $c[1], $c[2], $c[3], $vis)
  }
  function MoveTo([double]$lx, [double]$ly) {
    [SeekProbe]::SetCursorPos([int]($ox + $lx * $scale), [int]($oy + $ly * $scale)) | Out-Null
    Start-Sleep -Milliseconds 60
    [SeekProbe]::mouse_event(0x0001, 0, 0, 0, 0) | Out-Null
    Start-Sleep -Milliseconds 60
    [SeekProbe]::mouse_event(0x0001, 0, 0, 0, 0) | Out-Null
  }
  function Down() { [SeekProbe]::mouse_event(0x0002, 0, 0, 0, 0) | Out-Null; Start-Sleep -Milliseconds 80 }
  function Up()   { [SeekProbe]::mouse_event(0x0004, 0, 0, 0, 0) | Out-Null; Start-Sleep -Milliseconds 80 }
  function Snap([string]$tag) {
    $ci2 = [SeekProbe]::ClientInfo($h)
    $w = $ci2[2]; $ht = $ci2[3]
    $bmp = New-Object System.Drawing.Bitmap($w, $ht)
    $gg = [System.Drawing.Graphics]::FromImage($bmp)
    $gg.CopyFromScreen($ci2[0], $ci2[1], 0, 0, (New-Object System.Drawing.Size($w, $ht)))
    $gg.Dispose()
    $out = Join-Path "D:\RustEchoMusic\docs\screenshots" "$Name-$tag.png"
    $bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    Write-Output ("shot {0} ({1} x {2})" -f $out, $w, $ht)
  }

  # ---------- 1) click at 75% ----------
  MoveTo ($bw / 2) 12
  Start-Sleep -Milliseconds 300
  MoveTo ($bx + $bw * 0.75) $cy
  Start-Sleep -Milliseconds 200
  Write-Output "MARK click75-down"
  DumpRect "click75-down"
  Down
  Start-Sleep -Milliseconds 150
  Write-Output "MARK click75-up"
  DumpRect "click75-up"
  Up
  Start-Sleep -Milliseconds 1800

  # ---------- 2) drag 20% -> 60% ----------
  MoveTo ($bx + $bw * 0.20) $cy
  Start-Sleep -Milliseconds 150
  Write-Output "MARK drag-down-20"
  DumpRect "drag-down-20"
  Down
  for ($i = 0; $i -le 12; $i++) {
    MoveTo ($bx + $bw * (0.20 + 0.40 * $i / 12)) $cy
    Start-Sleep -Milliseconds 70
  }
  Write-Output "MARK drag-hold-60"
  DumpRect "drag-hold-60"
  Start-Sleep -Milliseconds 800
  Write-Output "MARK drag-up-60"
  DumpRect "drag-up-60"
  Up
  Start-Sleep -Milliseconds 1800

  # ---------- 3) lyric panel ----------
  MoveTo 303 29
  Start-Sleep -Milliseconds 200
  Down; Up
  Write-Output "MARK lyric-open"
  DumpRect "lyric-open"
  Start-Sleep -Milliseconds 1200
  Snap "lyric-a"
  Start-Sleep -Milliseconds 5000
  Snap "lyric-b"
  Start-Sleep -Milliseconds 5000
  Snap "lyric-c"
  Down; Up
  Start-Sleep -Milliseconds 900

  # ---------- 4) middle-zone hover ----------
  MoveTo 150 25
  Start-Sleep -Milliseconds 700
  Write-Output "MARK hover-mid"
  DumpRect "hover-mid"
  Snap "hover-mid"
  MoveTo 232 25
  Start-Sleep -Milliseconds 700
  Write-Output "MARK hover-mid-right"
  DumpRect "hover-mid-right"
  Snap "hover-mid-right"
  [SeekProbe]::SetCursorPos($ox - 80, $oy - 80) | Out-Null
  [SeekProbe]::mouse_event(0x0001, 0, 0, 0, 0) | Out-Null
  Start-Sleep -Milliseconds 800
  Write-Output "MARK hover-out"
  DumpRect "hover-out"
  Snap "hover-out"
} finally {
  Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
  Remove-Item Env:ECHO_SMOKE -ErrorAction SilentlyContinue
  if ($spBak -ne "") {
    Copy-Item $spBak $sp -Force
    Remove-Item $spBak -Force
    Write-Output "settings restored"
  }
}
Write-Output "done"
