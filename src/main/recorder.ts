import { createWriteStream, mkdirSync, writeFileSync, readFileSync, readdirSync, rmSync, statSync, existsSync, type WriteStream } from 'node:fs';
import { createGzip, gunzipSync, constants as zc, type Gzip } from 'node:zlib';
import path from 'node:path';
import type { RecordingStatus, RecordingSummary } from '../shared/types';
import type { CropFrame, SceneFrame } from './capture';
import type { AllGameData } from './sources/liveApi';
import { toGameInfo } from './sources/liveApi';

/**
 * Session layout (one folder per recording):
 *   meta.json            RecordingSummary + calibration
 *   frames/<ms>.png      minimap crops, <ms> = offset from start
 *   scenes/<ms>.jpg      full-window JPEGs
 *   api.jsonl.gz         {o, d} Live API samples (1 Hz), o = offset ms
 *   lcu.jsonl            {o, phase} gameflow changes
 */
export class Recorder {
  status: RecordingStatus = { active: false, sessionId: null, frames: 0, apiSamples: 0, bytes: 0, startedAt: null };
  private dir: string | null = null;
  private api: { gz: Gzip; out: WriteStream } | null = null;
  private lastFrameAt = 0;
  private lastApiAt = 0;
  private meta: RecordingSummary | null = null;
  private extra: Record<string, unknown> = {};

  constructor(public root: string, private getRecordFps: () => number) { mkdirSync(root, { recursive: true }); }

  start(extra: Record<string, unknown> = {}) {
    if (this.status.active) return;
    const startedAt = Date.now();
    const id = new Date(startedAt).toISOString().replace(/[:.]/g, '-').slice(0, 19);
    this.dir = path.join(this.root, id);
    mkdirSync(path.join(this.dir, 'frames'), { recursive: true });
    mkdirSync(path.join(this.dir, 'scenes'), { recursive: true });
    const gz = createGzip({ flush: zc.Z_SYNC_FLUSH });
    const out = createWriteStream(path.join(this.dir, 'api.jsonl.gz'));
    gz.pipe(out);
    this.api = { gz, out };
    this.extra = extra;
    this.meta = { id, startedAt, durationMs: 0, frames: 0, bytes: 0, gameMode: null, champion: null, allies: [], enemies: [] };
    this.status = { active: true, sessionId: id, frames: 0, apiSamples: 0, bytes: 0, startedAt };
    this.lastFrameAt = 0;
    this.lastApiAt = 0;
    this.writeMeta();
  }

  stop() {
    if (!this.status.active) return;
    this.api?.gz.end();
    this.api = null;
    this.writeMeta();
    this.status = { ...this.status, active: false };
    this.dir = null;
  }

  frame(f: CropFrame) {
    if (!this.status.active || !this.dir) return;
    if (f.at - this.lastFrameAt < 1000 / this.getRecordFps() - 5) return;
    this.lastFrameAt = f.at;
    const o = f.at - this.status.startedAt!;
    writeFileSync(path.join(this.dir, 'frames', `${o}.png`), f.png);
    this.status.frames++;
    this.status.bytes += f.png.length;
    if (this.status.frames % 50 === 0) this.writeMeta();
  }

  scene(s: SceneFrame) {
    if (!this.status.active || !this.dir) return;
    const o = s.at - this.status.startedAt!;
    writeFileSync(path.join(this.dir, 'scenes', `${o}.${s.kind === 'png' ? 'png' : 'jpg'}`), s.data);
    this.status.bytes += s.data.length;
  }

  apiSample(d: AllGameData, at: number) {
    if (!this.status.active || !this.api || !this.meta) return;
    if (at - this.lastApiAt < 950) return;
    this.lastApiAt = at;
    const line = JSON.stringify({ o: at - this.status.startedAt!, d }) + '\n';
    this.api.gz.write(line);
    this.status.apiSamples++;
    this.status.bytes += line.length / 8; // rough compressed size
    const g = toGameInfo(d);
    Object.assign(this.meta, { gameMode: g.gameMode, champion: g.activeChampion, allies: g.allies, enemies: g.enemies });
  }

  lcuPhase(phase: string) {
    if (!this.status.active || !this.dir) return;
    writeFileSync(path.join(this.dir, 'lcu.jsonl'), JSON.stringify({ o: Date.now() - this.status.startedAt!, phase }) + '\n', { flag: 'a' });
  }

  private writeMeta() {
    if (!this.dir || !this.meta) return;
    Object.assign(this.meta, { durationMs: Date.now() - this.meta.startedAt, frames: this.status.frames, bytes: Math.round(this.status.bytes) });
    writeFileSync(path.join(this.dir, 'meta.json'), JSON.stringify({ ...this.meta, ...this.extra }, null, 2));
  }

  list(): RecordingSummary[] {
    const out: RecordingSummary[] = [];
    for (const id of readdirSync(this.root)) {
      const f = path.join(this.root, id, 'meta.json');
      if (!existsSync(f)) continue;
      try { out.push(JSON.parse(readFileSync(f, 'utf8'))); } catch { /* skip broken */ }
    }
    return out.sort((a, b) => b.startedAt - a.startedAt);
  }

  delete(id: string) {
    if (id === this.status.sessionId && this.status.active) throw new Error('recording in progress');
    const dir = path.join(this.root, path.basename(id));
    if (existsSync(path.join(dir, 'meta.json'))) rmSync(dir, { recursive: true, force: true });
  }
}

/** Loaded recording for replay and offline analysis. */
export interface Session {
  id: string;
  dir: string;
  meta: RecordingSummary & Record<string, unknown>;
  /** Frame offsets in ms, ascending. */
  frames: number[];
  scenes: number[];
  api: Array<{ o: number; d: AllGameData }>;
  durationMs: number;
}

export function loadSession(root: string, id: string): Session {
  const dir = path.join(root, path.basename(id));
  const meta = JSON.parse(readFileSync(path.join(dir, 'meta.json'), 'utf8'));
  const offsets = (sub: string) => existsSync(path.join(dir, sub))
    ? readdirSync(path.join(dir, sub)).map(n => parseInt(n, 10)).filter(Number.isFinite).sort((a, b) => a - b) : [];
  const frames = offsets('frames'), scenes = offsets('scenes');
  let api: Session['api'] = [];
  const apiFile = path.join(dir, 'api.jsonl.gz');
  if (existsSync(apiFile) && statSync(apiFile).size > 0) {
    let text = '';
    // Sync-flush tolerates streams that were never closed (app crash mid-recording).
    try { text = gunzipSync(readFileSync(apiFile), { finishFlush: zc.Z_SYNC_FLUSH }).toString('utf8'); } catch { /* corrupt */ }
    api = text.split('\n').filter(Boolean).flatMap(l => { try { return [JSON.parse(l)]; } catch { return []; } });
  }
  const durationMs = Math.max(frames.at(-1) ?? 0, api.at(-1)?.o ?? 0, scenes.at(-1) ?? 0);
  return { id, dir, meta, frames, scenes, api, durationMs };
}

export const framePath = (s: Session, o: number) => path.join(s.dir, 'frames', `${o}.png`);

/**
 * Disk housekeeping:
 *  - keep the newest `keepFull` sessions,
 *  - delete every older session entirely (frames, scenes, API log),
 *  - delete any session older than `maxAgeDays`.
 * (Learned model weights and cached detections live elsewhere and are kept.)
 * Returns bytes freed. Never touches a session that is being written.
 */
export function pruneRecordings(root: string, opts: { keepFull?: number; thinTo?: number; maxAgeDays?: number; active?: string | null } = {}): number {
  const keepFull = opts.keepFull ?? 5, maxAgeMs = (opts.maxAgeDays ?? 14) * 86400000;
  if (!existsSync(root)) return 0;
  const sessions = readdirSync(root)
    .map(id => ({ id, dir: path.join(root, id), meta: (() => { try { return JSON.parse(readFileSync(path.join(root, id, 'meta.json'), 'utf8')); } catch { return null; } })() }))
    .filter(s => s.meta && s.id !== opts.active)
    .sort((a, b) => b.meta.startedAt - a.meta.startedAt);
  let freed = 0;
  sessions.forEach((s, idx) => {
    if (idx < keepFull && Date.now() - s.meta.startedAt <= maxAgeMs) return;
    freed += dirSize(s.dir);
    rmSync(s.dir, { recursive: true, force: true });
  });
  return freed;
}

function dirSize(dir: string): number {
  let total = 0;
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    total += e.isDirectory() ? dirSize(p) : statSync(p).size;
  }
  return total;
}
