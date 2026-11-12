import { spawn, type ChildProcess } from 'node:child_process';
import { EventEmitter } from 'node:events';
import { GAME_WINDOW_TITLE } from './capture';

/** Game window client area in physical screen pixels. */
export interface ClientRect { x: number; y: number; width: number; height: number; foreground: boolean }

// A long-lived PowerShell loop that prints the game client rect once a second.
// Read-only Win32 calls (FindWindow, GetClientRect, ClientToScreen); never sends input.
const SCRIPT = `
$ErrorActionPreference = 'SilentlyContinue'
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class W {
  [StructLayout(LayoutKind.Sequential)] public struct R { public int L, T, Rt, B; }
  [StructLayout(LayoutKind.Sequential)] public struct P { public int X, Y; }
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string c, string t);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out R r);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref P p);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
}
"@
[W]::SetProcessDPIAware() | Out-Null
while ($true) {
  $h = [W]::FindWindow($null, '${GAME_WINDOW_TITLE}')
  if ($h -ne [IntPtr]::Zero -and -not [W]::IsIconic($h)) {
    $r = New-Object W+R; $p = New-Object W+P
    [W]::GetClientRect($h, [ref]$r) | Out-Null; [W]::ClientToScreen($h, [ref]$p) | Out-Null
    $fg = [int]([W]::GetForegroundWindow() -eq $h)
    [Console]::Out.WriteLine("$($p.X) $($p.Y) $($r.Rt) $($r.B) $fg")
  } else { [Console]::Out.WriteLine("none") }
  [Console]::Out.Flush()
  Start-Sleep -Milliseconds 1000
}`;

/** Emits 'rect' (ClientRect | null) whenever the game window moves/resizes/appears. */
export class WindowTracker extends EventEmitter {
  rect: ClientRect | null = null;
  private proc: ChildProcess | null = null;

  start() {
    this.proc = spawn('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', '-'], { windowsHide: true, stdio: ['pipe', 'pipe', 'ignore'] });
    this.proc.stdin!.end(SCRIPT);
    let buf = '';
    this.proc.stdout!.on('data', (c: Buffer) => {
      buf += c.toString();
      let i;
      while ((i = buf.indexOf('\n')) >= 0) { this.onLine(buf.slice(0, i).trim()); buf = buf.slice(i + 1); }
    });
    this.proc.on('exit', () => { this.proc = null; });
  }

  stop() { this.proc?.kill(); this.proc = null; }

  private onLine(line: string) {
    let next: ClientRect | null = null;
    const m = /^(-?\d+) (-?\d+) (\d+) (\d+) ([01])$/.exec(line);
    if (m) next = { x: +m[1]!, y: +m[2]!, width: +m[3]!, height: +m[4]!, foreground: m[5] === '1' };
    if (next && next.width < 100) next = null;
    const same = JSON.stringify(next) === JSON.stringify(this.rect);
    this.rect = next;
    if (!same) this.emit('rect', next);
  }
}
