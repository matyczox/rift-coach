import { EventEmitter } from 'node:events';
import { readFileSync } from 'node:fs';
import type { ReplayStatus } from '../shared/types';
import { framePath, loadSession, type Session } from './recorder';
import type { AllGameData } from './sources/liveApi';

/** Index of the last element <= t in an ascending array, or -1. */
export function lastAtOrBefore<T>(arr: T[], t: number, key: (v: T) => number): number {
  let lo = 0, hi = arr.length - 1, ans = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (key(arr[mid]!) <= t) { ans = mid; lo = mid + 1; } else hi = mid - 1;
  }
  return ans;
}

/**
 * Plays a recorded session back on a virtual clock.
 * Emits 'frame' ({offsetMs, png}), 'api' (AllGameData, offsetMs) and 'status'.
 * Consumers treat these exactly like live data.
 */
export class Replayer extends EventEmitter {
  session: Session;
  status: ReplayStatus;
  private timer: NodeJS.Timeout | null = null;
  private lastTick = 0;
  private frameIdx = -1;
  private apiIdx = -1;

  constructor(root: string, id: string) {
    super();
    this.session = loadSession(root, id);
    this.status = { sessionId: id, playing: false, speed: 1, positionMs: 0, durationMs: this.session.durationMs };
  }

  play() {
    if (this.status.playing) return;
    if (this.status.positionMs >= this.status.durationMs) this.seek(0);
    this.status.playing = true;
    this.lastTick = Date.now();
    this.timer = setInterval(() => this.tick(), 33);
    this.emit('status', this.status);
  }

  pause() {
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
    this.status.playing = false;
    this.emit('status', this.status);
  }

  setSpeed(speed: number) {
    this.status.speed = Math.min(16, Math.max(0.25, speed));
    this.emit('status', this.status);
  }

  seek(positionMs: number) {
    this.status.positionMs = Math.min(this.status.durationMs, Math.max(0, positionMs));
    this.frameIdx = -1;
    this.apiIdx = -1;
    this.emitUpTo(this.status.positionMs);
    this.emit('status', this.status);
  }

  close() { this.pause(); this.removeAllListeners(); }

  private tick() {
    const now = Date.now();
    this.status.positionMs += (now - this.lastTick) * this.status.speed;
    this.lastTick = now;
    if (this.status.positionMs >= this.status.durationMs) {
      this.status.positionMs = this.status.durationMs;
      this.emitUpTo(this.status.positionMs);
      this.pause();
      return;
    }
    this.emitUpTo(this.status.positionMs);
  }

  /** Emit only the newest frame/api sample at or before t (skip frames at high speed). */
  private emitUpTo(t: number) {
    const s = this.session;
    const fi = lastAtOrBefore(s.frames, t, v => v);
    if (fi >= 0 && fi !== this.frameIdx) {
      this.frameIdx = fi;
      const o = s.frames[fi]!;
      try { this.emit('frame', { offsetMs: o, png: readFileSync(framePath(s, o)) }); } catch { /* missing file */ }
    }
    const ai = lastAtOrBefore(s.api, t, v => v.o);
    if (ai >= 0 && ai !== this.apiIdx) {
      this.apiIdx = ai;
      this.emit('api', s.api[ai]!.d as AllGameData, s.api[ai]!.o);
    }
  }
}
