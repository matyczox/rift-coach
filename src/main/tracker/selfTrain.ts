import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { gunzipSync, gzipSync } from 'node:zlib';
import path from 'node:path';
import type { Detection } from '../vision/detect';
import { EnemyTracker, type EnemyInput } from './tracker';
import type { Learner } from './learning';
import type { Terrain } from './terrain';

/**
 * Self-training from your own games, no replays needed:
 *  1. during a real game, once a second, store what the app saw (enemy detections,
 *     a coarse fog mask, enemy states) — a few hundred KB per game;
 *  2. after the game, the Riot API timeline gives every player's true position each minute;
 *  3. replay the observations through a fresh tracker and, at each minute mark where an
 *     enemy was in fog, credit the behaviours (goal kinds) that explained the true position.
 */
export interface ObsFrame {
  t: number;                                   // game seconds
  d: Array<[string, number, number]>;          // enemy detections: champion, x, y
  m: string;                                   // 32x32 fog mask, base64 bitset (1 = visible)
  e: Array<[string, string, number, number, number]>; // champion, role, isDead(0/1), moveSpeed, itemGold
  dr: number | null; br: number | null;        // dragon / baron seconds to spawn
}

export interface ObsLog { gameId: number | null; team: 'ORDER' | 'CHAOS'; enemyTeam: 'ORDER' | 'CHAOS'; frames: ObsFrame[] }

const G = 32;

export function packMask(mask: Float32Array, size = 64): string {
  const bits = new Uint8Array((G * G) / 8);
  for (let y = 0; y < G; y++) for (let x = 0; x < G; x++) {
    const v = mask[Math.floor((y * size) / G) * size + Math.floor((x * size) / G)]!;
    if (v >= 0.5) bits[(y * G + x) >> 3]! |= 1 << ((y * G + x) & 7);
  }
  return Buffer.from(bits).toString('base64');
}

export function unpackMask(s: string): (x: number, y: number) => number {
  const bits = Buffer.from(s, 'base64');
  return (x, y) => {
    const gx = Math.min(G - 1, Math.max(0, Math.floor(x * G))), gy = Math.min(G - 1, Math.max(0, Math.floor(y * G)));
    const i = gy * G + gx;
    return (bits[i >> 3]! >> (i & 7)) & 1;
  };
}

export class ObsRecorder {
  log: ObsLog | null = null;
  private lastT = -1;

  start(team: 'ORDER' | 'CHAOS') { this.log = { gameId: null, team, enemyTeam: team === 'ORDER' ? 'CHAOS' : 'ORDER', frames: [] }; this.lastT = -1; }

  add(f: Omit<ObsFrame, 'm'> & { mask: Float32Array | null }) {
    if (!this.log || f.t - this.lastT < 1 || !f.mask) return;
    this.lastT = f.t;
    this.log.frames.push({ t: +f.t.toFixed(1), d: f.d, m: packMask(f.mask), e: f.e, dr: f.dr, br: f.br });
  }

  save(dir: string, gameId: number | null): string | null {
    if (!this.log || this.log.frames.length < 120) return null;
    this.log.gameId = gameId;
    mkdirSync(dir, { recursive: true });
    const file = path.join(dir, `${gameId ?? Date.now()}.json.gz`);
    writeFileSync(file, gzipSync(JSON.stringify(this.log)));
    return file;
  }
}

export function loadObs(file: string): ObsLog { return JSON.parse(gunzipSync(readFileSync(file)).toString('utf8')); }

/** Summoner's Rift game units -> minimap coords (y flipped: minimap y grows downwards). */
export function riotToMinimap(p: { x: number; y: number }) {
  return { x: Math.min(0.999, Math.max(0, (p.x + 120) / 14990)), y: Math.min(0.999, Math.max(0, 1 - (p.y + 120) / 15100)) };
}

/**
 * Replay a game's observations and learn from the timeline truth.
 * `truth(champion, minute)` -> true minimap position at that minute frame, if known.
 * Returns how the model did vs the "stayed where last seen" baseline on fogged enemies.
 */
export function trainFromObservations(log: ObsLog, terrain: Terrain, learner: Learner, truth: (champion: string, minute: number) => { x: number; y: number } | null) {
  const tracker = new EnemyTracker(terrain, learner, 250);
  tracker.learningEnabled = true;
  let n = 0, errModel = 0, errBase = 0, inside = 0;
  let nextMinute = 1;
  for (const f of log.frames) {
    const enemies: EnemyInput[] = f.e.map(([champion, role, dead, ms, gold]) => ({ champion, team: log.enemyTeam, role: role as EnemyInput['role'], isDead: dead === 1, moveSpeed: ms, itemGold: gold }));
    const dets: Detection[] = f.d.map(([champion, x, y]) => ({ champion, team: log.enemyTeam, x, y, score: 0.9, margin: 0.5, votes: 1 }));
    const tracks = tracker.update({ t: f.t, detections: dets, visible: unpackMask(f.m), enemies, dragonIn: f.dr, baronIn: f.br });
    if (f.t >= nextMinute * 60) {
      for (const tr of tracks) {
        if (tr.state !== 'fog' || !tr.estimate || !tr.last) continue;
        const q = truth(tr.champion, nextMinute);
        if (!q) continue;
        const em = Math.hypot(q.x - tr.estimate.x, q.y - tr.estimate.y), eb = Math.hypot(q.x - tr.last.x, q.y - tr.last.y);
        n++; errModel += em; errBase += eb; if (em <= tr.estimate.radius) inside++;
        tracker.trainAt(tr.champion, q, f.t);
      }
      nextMinute++;
    }
  }
  return { samples: n, errModel: n ? errModel / n : 0, errBaseline: n ? errBase / n : 0, inside: n ? inside / n : 0 };
}

export const obsDir = (userData: string) => path.join(userData, 'model', 'observations');
export const hasObs = (file: string) => existsSync(file);
