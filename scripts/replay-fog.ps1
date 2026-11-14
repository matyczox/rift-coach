# Replay HUD automation for the harvester (replays only, never a live game).
#   replay-fog.ps1 shot  <out.png>              capture the replay window's client area
#   replay-fog.ps1 click <fx> <fy>              click at a fraction of the client area (0..1), then restore focus
# The replay window title is "League of Legends (TM) Client".
param([string]$cmd, [string]$a1, [string]$a2)
$ErrorActionPreference = 'Stop'
Add-Type -ReferencedAssemblies System.Drawing @"
using System;
using System.Drawing;
using System.Runtime.InteropServices;
public static class RcWin {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindow(string c, string t);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, UIntPtr e);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  public static IntPtr Game() { return FindWindow(null, "League of Legends (TM) Client"); }
  public static Rectangle Client(IntPtr h) { RECT r; GetClientRect(h, out r); POINT p = new POINT(); ClientToScreen(h, ref p); return new Rectangle(p.X, p.Y, r.R - r.L, r.B - r.T); }
}
"@
[RcWin]::SetProcessDPIAware() | Out-Null
$h = [RcWin]::Game()
if ($h -eq [IntPtr]::Zero) { Write-Output 'ERR no-replay-window'; exit 2 }
$rc = [RcWin]::Client($h)
if ($cmd -eq 'shot') {
  $bmp = New-Object System.Drawing.Bitmap $rc.Width, $rc.Height
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($rc.X, $rc.Y, 0, 0, $bmp.Size)
  $bmp.Save($a1, [System.Drawing.Imaging.ImageFormat]::Png)
  Write-Output "OK $($rc.Width)x$($rc.Height)"
} elseif ($cmd -eq 'click') {
  $prev = [RcWin]::GetForegroundWindow()
  $pos = New-Object RcWin+POINT; [RcWin]::GetCursorPos([ref]$pos) | Out-Null
  $x = [int]($rc.X + [double]$a1 * $rc.Width); $y = [int]($rc.Y + [double]$a2 * $rc.Height)
  [RcWin]::SetForegroundWindow($h) | Out-Null
  Start-Sleep -Milliseconds 150
  [RcWin]::SetCursorPos($x, $y) | Out-Null
  Start-Sleep -Milliseconds 60
  [RcWin]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)   # left down
  Start-Sleep -Milliseconds 40
  [RcWin]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)   # left up
  Start-Sleep -Milliseconds 250
  [RcWin]::SetCursorPos($pos.X, $pos.Y) | Out-Null
  if ($prev -ne [IntPtr]::Zero -and $prev -ne $h) { [RcWin]::SetForegroundWindow($prev) | Out-Null }
  Write-Output "OK click $x,$y"
} elseif ($cmd -eq 'fog') {
  # Replay HUD "Mgła wojny" dropdown (bottom-left) and its options, as fractions of the client area.
  $opt = @{ blue = 0.881; red = 0.904; all = 0.924 }[$a1]
  if (-not $opt) { Write-Output 'usage: fog blue|red|all'; exit 1 }
  $prev = [RcWin]::GetForegroundWindow()
  $pos = New-Object RcWin+POINT; [RcWin]::GetCursorPos([ref]$pos) | Out-Null
  $click = { param($fx, $fy)
    [RcWin]::SetCursorPos([int]($rc.X + $fx * $rc.Width), [int]($rc.Y + $fy * $rc.Height)) | Out-Null
    Start-Sleep -Milliseconds 40
    [RcWin]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 30
    [RcWin]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
  }
  # The selected value is drawn in the team colour: light blue (Niebieski), red (Czerwony), white (Wszystkie).
  $readLabel = {
    $rx = [int]($rc.X + 0.083 * $rc.Width); $ry = [int]($rc.Y + 0.939 * $rc.Height)
    $rw = [int](0.06 * $rc.Width); $rh = [int](0.017 * $rc.Height)
    $bmp = New-Object System.Drawing.Bitmap $rw, $rh
    $graphics = [System.Drawing.Graphics]::FromImage($bmp)
    $graphics.CopyFromScreen($rx, $ry, 0, 0, $bmp.Size)
    $graphics.Dispose()
    $red = 0; $blue = 0; $white = 0
    for ($y = 0; $y -lt $rh; $y++) { for ($x = 0; $x -lt $rw; $x++) {
      $c = $bmp.GetPixel($x, $y)
      # Red text is darker than the old total-brightness threshold (330).
      # Count coloured glyph pixels directly so a successful red switch is recognised.
      if ($c.R -gt 90 -and $c.R -gt $c.B + 40 -and $c.R -gt $c.G + 25) { $red++ }
      elseif ($c.B -gt 100 -and $c.B -gt $c.R + 40) { $blue++ }
      elseif ($c.R -gt 75 -and $c.G -gt 75 -and $c.B -gt 75 -and [Math]::Abs([int]$c.R - [int]$c.B) -lt 30) { $white++ }
    } }
    $bmp.Dispose()
    if ($red -ge 5 -and $red -gt $blue) { 'red' }
    elseif ($blue -ge 5) { 'blue' }
    elseif ($white -ge 5) { 'all' }
    else { 'none' }
  }
  $got = 'none'
  [RcWin]::SetForegroundWindow($h) | Out-Null
  Start-Sleep -Milliseconds 200
  for ($try = 1; $try -le 3 -and $got -ne $a1; $try++) {
    & $click 0.115 0.947          # open the dropdown
    Start-Sleep -Milliseconds 350
    & $click 0.102 $opt           # pick the team
    Start-Sleep -Milliseconds 400
    $got = & $readLabel
  }
  [RcWin]::SetCursorPos($pos.X, $pos.Y) | Out-Null
  if ($prev -ne [IntPtr]::Zero -and $prev -ne $h) { [RcWin]::SetForegroundWindow($prev) | Out-Null }
  if ($got -eq $a1) { Write-Output "OK fog $a1" } else { Write-Output "FAIL fog want=$a1 got=$got"; exit 3 }
} else { Write-Output 'usage: shot <out.png> | click <fx> <fy> | fog blue|red|all'; exit 1 }
