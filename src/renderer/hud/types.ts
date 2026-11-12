// HUD overlay plumbing shared by src/main/hud.ts, src/main/hud-preload.ts and
// the hud renderers. Pure types/constants only (no DOM imports).
import type { HudState, ObjectiveCall, ObjectiveTimer } from '../../shared/intel';

/** The single IPC channel the Hud uses (main -> hud windows). */
export const HUD_CHANNEL = 'hud:state';

export type HudWindowKind = 'minimap' | 'topbar' | 'death' | 'loading' | 'power' | 'build' | 'notify';

export interface HudPayload {
  state: HudState;
  /** Font/size scale relative to a 1440p game client (gameHeight / 1440). */
  scale: number;
}

export interface HudBridge {
  onState(cb: (p: HudPayload) => void): () => void;
}

declare global {
  interface Window { hud?: HudBridge }
}

/**
 * `expiresAt` fields are Date.now() epoch milliseconds (what main sends).
 * Small values (< 1e11) are still accepted as game-time seconds as a fallback.
 */
export function isLive(expiresAt: number, gameTime: number): boolean {
  if (expiresAt > 1e11) return expiresAt > Date.now();
  return expiresAt > gameTime;
}

/** Seconds remaining until `expiresAt` (same dual interpretation as isLive). */
export function secondsUntil(expiresAt: number, gameTime: number): number {
  if (expiresAt > 1e11) return (expiresAt - Date.now()) / 1000;
  return expiresAt - gameTime;
}

export function fmtClock(sec: number): string {
  const s = Math.max(0, Math.ceil(sec));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}

// ---- objective call helpers (top bar + minimap) ----

export const VERDICT: Record<ObjectiveCall['verdict'], { word: string; color: string }> = {
  go: { word: 'IDŹ', color: '#3ccf8e' },
  contest: { word: 'WALCZ', color: '#e8c25a' },
  skip: { word: 'ODPUŚĆ', color: '#ff5a4f' },
};

/** Map an ObjectiveCall.objective name ("Smok", "Baron", ...) to a timer kind. */
export function objectiveKind(name: string): ObjectiveTimer['kind'] | null {
  const n = name.toLowerCase();
  if (n.startsWith('smok') || n.includes('dragon')) return 'dragon';
  if (n.startsWith('baron')) return 'baron';
  if (n.startsWith('herold') || n.includes('herald')) return 'herald';
  if (n.startsWith('larw') || n.includes('grub')) return 'grubs';
  if (n.startsWith('atakhan')) return 'atakhan';
  return null;
}

/** Objective call is echoed under the top bar when alive or spawning within 30 s. */
export function objectiveUrgent(o: ObjectiveCall): boolean {
  return !o.inSec || o.inSec <= 30;
}
