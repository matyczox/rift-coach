import { Worker } from 'node:worker_threads';
import { EventEmitter } from 'node:events';
import path from 'node:path';
import type { Rgba } from './png';
import type { Detection, Team } from './detect';
import type { CampReading } from './camps';
import type { WorkerIn, WorkerOut } from './worker';

export interface VisionResult { at: number; width: number; detections: Detection[]; mask: Float32Array | null; camps: CampReading[]; ms: number }

/**
 * Main-process side of the vision worker. Drops frames while the worker is busy
 * (always processes the newest frame). Emits 'result' (VisionResult).
 */
export class VisionService extends EventEmitter {
  private worker: Worker | null = null;
  private busy = false;
  private nextId = 1;
  private pending: { at: number; png: Uint8Array; hints: Array<{ x: number; y: number; team: Team }> } | null = null;
  avgMs = 0;
  /** Max detections per second (CPU budget while the game runs). 5/s is plenty for minimap movement. */
  maxFps = 5;
  private lastStart = 0;
  private timer: NodeJS.Timeout | null = null;
  rosterKey = '';

  start(workerFile = path.join(__dirname, 'vision-worker.js')) {
    this.worker = new Worker(workerFile);
    this.worker.on('message', (m: WorkerOut) => {
      this.busy = false;
      if (m.type === 'result') {
        this.avgMs = this.avgMs ? this.avgMs * 0.9 + m.ms * 0.1 : m.ms;
        this.emit('result', { at: m.at, width: m.width, detections: m.detections, mask: m.mask, camps: m.camps, ms: m.ms } satisfies VisionResult);
      } else this.emit('error', new Error(m.error));
      this.pump();
    });
    this.worker.on('error', e => { this.busy = false; this.emit('error', e); });
  }

  setRoster(champions: Array<{ champion: string; team: Team; icon: Rgba }>, ownTeam: Team, relativeColors = true) {
    this.rosterKey = champions.map(c => c.champion).join('|') + ownTeam;
    this.worker?.postMessage({ type: 'roster', champions, ownTeam, relativeColors } satisfies WorkerIn);
  }

  push(at: number, png: Uint8Array, hints: Array<{ x: number; y: number; team: Team }> = []) {
    this.pending = { at, png, hints };
    this.pump();
  }

  private pump() {
    if (this.busy || !this.pending || !this.worker) return;
    const wait = this.lastStart + 1000 / this.maxFps - Date.now();
    if (wait > 0) { this.timer ??= setTimeout(() => { this.timer = null; this.pump(); }, wait); return; }
    this.lastStart = Date.now();
    const f = this.pending;
    this.pending = null;
    this.busy = true;
    this.worker.postMessage({ type: 'frame', id: this.nextId++, ...f } satisfies WorkerIn);
  }

  stop() { void this.worker?.terminate(); this.worker = null; }
}
