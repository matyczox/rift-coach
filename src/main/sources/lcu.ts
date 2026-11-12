import { EventEmitter } from 'node:events';
import { readFile } from 'node:fs/promises';
import { execFile } from 'node:child_process';
import { getJson, postJson } from './http';
import type { SourceState } from '../../shared/types';

interface LcuAuth { port: number; password: string }

const LOCKFILES = [
  'C:\Riot Games\League of Legends\lockfile',
  'D:\Riot Games\League of Legends\lockfile',
];

async function fromLockfile(): Promise<LcuAuth | null> {
  for (const p of LOCKFILES) {
    try {
      const [, , port, password] = (await readFile(p, 'utf8')).split(':');
      if (port && password) return { port: Number(port), password };
    } catch { /* try next */ }
  }
  return null;
}

/** Fallback: read --app-port / --remoting-auth-token from the client process. */
function fromProcess(): Promise<LcuAuth | null> {
  const cmd = "Get-CimInstance Win32_Process -Filter \"Name='LeagueClientUx.exe'\" | Select-Object -ExpandProperty CommandLine";
  return new Promise(resolve => {
    execFile('powershell.exe', ['-NoProfile', '-Command', cmd], { windowsHide: true, timeout: 5000 }, (err, out) => {
      if (err) return resolve(null);
      const port = /--app-port=(\d+)/.exec(out)?.[1], password = /--remoting-auth-token=([\w-]+)/.exec(out)?.[1];
      resolve(port && password ? { port: Number(port), password } : null);
    });
  });
}

/**
 * League client (LCU) connection. Read-only: we only GET.
 * Events: 'state', 'phase' (gameflow phase string), 'champSelect' (session JSON).
 */
export class Lcu extends EventEmitter {
  state: SourceState = 'waiting';
  phase: string | null = null;
  private auth: LcuAuth | null = null;
  private timer: NodeJS.Timeout | null = null;
  private busy = false;

  start() {
    if (this.timer) return;
    this.timer = setInterval(() => void this.tick(), 2000);
    void this.tick();
  }

  stop() {
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
  }

  get<T>(path: string): Promise<T> {
    if (!this.auth) return Promise.reject(new Error('LCU not connected'));
    const token = Buffer.from(`riot:${this.auth.password}`).toString('base64');
    return getJson<T>(`https://127.0.0.1:${this.auth.port}${path}`, { Authorization: `Basic ${token}` });
  }

  post<T>(path: string, body: unknown): Promise<T | null> {
    if (!this.auth) return Promise.reject(new Error('LCU not connected'));
    const token = Buffer.from(`riot:${this.auth.password}`).toString('base64');
    return postJson<T>(`https://127.0.0.1:${this.auth.port}${path}`, body, { Authorization: `Basic ${token}` });
  }

  /** Write request (PUT/POST/PATCH/DELETE) to the client API. */
  request<T>(method: 'PUT' | 'POST' | 'PATCH' | 'DELETE', path: string, body?: unknown): Promise<T | null> {
    if (!this.auth) return Promise.reject(new Error('LCU not connected'));
    const token = Buffer.from(`riot:${this.auth.password}`).toString('base64');
    return postJson<T>(`https://127.0.0.1:${this.auth.port}${path}`, body ?? {}, { Authorization: `Basic ${token}` }, 5000, method);
  }

  private async tick() {
    if (this.busy) return;
    this.busy = true;
    try {
      if (!this.auth) this.auth = (await fromLockfile()) ?? (await fromProcess());
      if (!this.auth) { this.setState('waiting'); return; }
      const phase = await this.get<string>('/lol-gameflow/v1/gameflow-phase');
      this.setState('connected');
      if (phase !== this.phase) { this.phase = phase; this.emit('phase', phase); }
      if (phase === 'ChampSelect') this.emit('champSelect', await this.get('/lol-champ-select/v1/session'));
    } catch {
      this.auth = null; // client restarted -> new port/password
      this.phase = null;
      this.setState('waiting');
    } finally {
      this.busy = false;
    }
  }

  private setState(s: SourceState) {
    if (s === this.state) return;
    this.state = s;
    this.emit('state', s);
  }
}
