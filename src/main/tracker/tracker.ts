import type { Detection } from '../vision/detect';
import { Learner } from './learning';
import { fountainOf, goalsFor, MAP_UNITS, ParticleFilter, rng, type Estimate, type GoalKind, type PriorContext, type Role } from './predictor';
import type { Terrain } from './terrain';

/**
 * Enemy state machine + fog predictions.
 *   visible: portrait detected in ≥2 of the last 3 frames
 *   fog:     not detected for ≥3 frames (or ≥0.35 s) — particle filter running
 *   dead:    from the Live API; on respawn the filter restarts at their fountain
 * A detection always wins: the moment a fogged enemy is seen again its ghost
 * disappears (no duplicate icon), and the reappearance trains the model.
 */

export interface EnemyInput {
  champion: string;
  team: 'ORDER' | 'CHAOS';
  role: Role;
  isDead: boolean;
  /** Move speed in game units/s (estimated). */
  moveSpeed: number;
  likelyRecall?: boolean;
  startTopSide?: number;
  /** Shop value of current items (Live API — visible even when the champion is in fog). */
  itemGold?: number;
  /** Has Flash as a summoner spell. */
  hasFlash?: boolean;
}

export interface FrameInput {
  /** Game time in seconds. */
  t: number;
  detections: Detection[];
  /** P(point currently in our vision), from the fog mask; undefined = unknown (no negative evidence). */
  visible?: (x: number, y: number) => number;
  enemies: EnemyInput[];
  dragonIn: number | null;
  baronIn: number | null;
}

export interface EnemyTrack {
  champion: string;
  team: 'ORDER' | 'CHAOS';
  role: Role;
  state: 'unknown' | 'visible' | 'fog' | 'dead';
  /** Last confirmed position (normalized). */
  last: { x: number; y: number; t: number } | null;
  vel: { vx: number; vy: number };
  fogSince: number | null;
  estimate: Estimate | null;
  /** Game time a recall was proven by an item purchase while unseen. */
  recalledAt: number | null;
  /** Game time of a probable Flash (jump longer than walking, within Flash range). */
  flashSuspectAt: number | null;
}

interface Internal extends EnemyTrack {
  hits: number[];
  pf: ParticleFilter | null;
  baselineAt: { x: number; y: number } | null;
  priorShares: Partial<Record<GoalKind, number>>;
  lastPriorAt: number;
  pending: Detection | null;
  lastItemGold: number | undefined;
  frozen: { massNear: (q: { x: number; y: number }) => number; kinds: (q: { x: number; y: number }) => Partial<Record<GoalKind, number>>; est: Estimate } | null;
}

export class EnemyTracker {
  private tracks = new Map<string, Internal>();
  private lastT: number | null = null;
  private random = rng(12345);
  /** Learn from reappearances only in real Summoner's Rift games (not practice tool / ARAM / Arena). */
  learningEnabled = true;

  constructor(private terrain: Terrain, public learner: Learner = new Learner(), private particles = 400) {}

  reset() { this.tracks.clear(); this.lastT = null; }

  get all(): EnemyTrack[] { return [...this.tracks.values()]; }

  private ctxFor(e: EnemyInput, f: FrameInput): PriorContext {
    return {
      role: e.role, gameTime: f.t, dragonIn: f.dragonIn, baronIn: f.baronIn, likelyRecall: Boolean(e.likelyRecall),
      learned: this.learner.multipliers(e.role, f.t), startTopSide: e.startTopSide,
    };
  }

  private newFilter(e: EnemyInput) {
    return new ParticleFilter(this.terrain, goalsFor(e.team), this.particles, this.random);
  }

  update(f: FrameInput): EnemyTrack[] {
    const dt = this.lastT === null ? 0 : Math.max(0, Math.min(5, f.t - this.lastT));
    this.lastT = f.t;
    for (const e of f.enemies) {
      let tr = this.tracks.get(e.champion);
      if (!tr) {
        tr = { champion: e.champion, team: e.team, role: e.role, state: 'unknown', last: null, vel: { vx: 0, vy: 0 }, fogSince: null, estimate: null, hits: [], pf: null, baselineAt: null, priorShares: {}, lastPriorAt: -99, pending: null, frozen: null, recalledAt: null, flashSuspectAt: null, lastItemGold: e.itemGold };
        // Game start: everyone walks out of their fountain.
        if (f.t < 100) { tr.state = 'fog'; tr.fogSince = 0; tr.last = { ...fountainOf(e.team), t: 0 }; tr.baselineAt = fountainOf(e.team); tr.pf = this.newFilter(e); tr.pf.reset(fountainOf(e.team), this.ctxFor(e, f)); }
        this.tracks.set(e.champion, tr);
      }
      tr.role = e.role;
      const prevItemGold = tr.lastItemGold;
      if (e.itemGold !== undefined) tr.lastItemGold = e.itemGold;

      // Dead: nothing to show; restart at the fountain on respawn.
      if (e.isDead) { tr.state = 'dead'; tr.pf = null; tr.estimate = null; tr.hits = []; continue; }
      if (tr.state === 'dead') {
        const fp = fountainOf(e.team);
        tr.state = 'fog'; tr.fogSince = f.t; tr.last = { ...fp, t: f.t }; tr.baselineAt = fp;
        tr.pf = this.newFilter(e); tr.pf.reset(fp, { ...this.ctxFor(e, f), leavingBase: true });
      }

      // Bought something while we couldn't see them -> they were in their fountain just now.
      // The Live API shows every player's items, even in fog: a proven recall, not a guess.
      if (prevItemGold !== undefined && e.itemGold !== undefined && e.itemGold - prevItemGold >= 250 && tr.state !== 'visible') {
        const fp = fountainOf(e.team);
        tr.state = 'fog'; tr.fogSince = f.t; tr.last = { ...fp, t: f.t }; tr.baselineAt = fp; tr.recalledAt = f.t;
        tr.frozen = null; // not a "reappearance" to learn from
        tr.pf = this.newFilter(e); tr.pf.reset(fp, { ...this.ctxFor(e, f), leavingBase: true });
      }

      const det = f.detections.find(d => d.champion === e.champion && d.team === e.team);
      tr.hits.push(det ? 1 : 0);
      if (tr.hits.length > 3) tr.hits.shift();

      if (det) {
        // Reject teleport-like jumps unless confirmed by the next frame (identity swaps, flicker).
        const jump = tr.last && tr.state === 'visible' ? Math.hypot(det.x - tr.last.x, det.y - tr.last.y) : 0;
        const maxMove = ((e.moveSpeed * 1.6) / MAP_UNITS) * Math.max(0.25, f.t - (tr.last?.t ?? f.t)) + 0.03;
        if (jump > maxMove && (!tr.pending || Math.hypot(tr.pending.x - det.x, tr.pending.y - det.y) > 0.03)) {
          tr.pending = det; // wait for confirmation (Flash/teleport will be confirmed next frame)
        } else {
          tr.pending = null;
          // Probable Flash: visible in consecutive frames, moved ~200–600 units more than walking allows.
          if (e.hasFlash && tr.last && tr.state === 'visible' && f.t - tr.last.t <= 0.6) {
            const walk = (e.moveSpeed / MAP_UNITS) * (f.t - tr.last.t);
            const excess = jump - walk;
            if (excess >= 0.014 && excess <= 0.045 && (tr.flashSuspectAt === null || f.t - tr.flashSuspectAt > 60)) tr.flashSuspectAt = f.t;
          }
          if (tr.state === 'fog' && tr.fogSince !== null) this.learnReappearance(tr, e, det, f.t);
          if (tr.last && tr.state === 'visible' && f.t > tr.last.t) {
            const k = 0.5, dtv = f.t - tr.last.t;
            tr.vel = { vx: tr.vel.vx * (1 - k) + ((det.x - tr.last.x) / dtv) * k, vy: tr.vel.vy * (1 - k) + ((det.y - tr.last.y) / dtv) * k };
          }
          tr.last = { x: det.x, y: det.y, t: f.t };
          if (tr.state !== 'visible' && (tr.hits.reduce((a, b) => a + b, 0) >= 2 || tr.state === 'fog' || tr.state === 'unknown')) {
            tr.state = 'visible';
          }
          tr.pf = null; tr.estimate = null; tr.fogSince = null; tr.frozen = null;
          continue;
        }
      }

      // Not seen this frame.
      if (tr.state === 'visible' && tr.last && tr.hits.every(h => h === 0) && f.t - tr.last.t >= 0.35) {
        tr.state = 'fog';
        tr.fogSince = tr.last.t;
        tr.baselineAt = { x: tr.last.x, y: tr.last.y };
        tr.pf = this.newFilter(e);
        tr.pf.reset(tr.last, this.ctxFor(e, f), tr.vel);
        tr.lastPriorAt = f.t;
        // Catch up the time between the last sighting and now.
        tr.pf.step(f.t - tr.last.t, e.moveSpeed);
      }

      if (tr.state === 'fog' && tr.pf) {
        if (f.t - tr.lastPriorAt > 5) {
          const fromBase = tr.baselineAt !== null && Math.hypot(tr.baselineAt.x - fountainOf(e.team).x, tr.baselineAt.y - fountainOf(e.team).y) < 0.05 && f.t - (tr.fogSince ?? f.t) < 25;
          tr.pf.setPrior({ ...this.ctxFor(e, f), leavingBase: fromBase }); tr.lastPriorAt = f.t;
        }
        if (dt > 0) tr.pf.step(dt, e.moveSpeed);
        if (f.visible) {
          const ok = tr.pf.observeAbsent(f.visible);
          if (!ok && tr.last) {
            // Everything contradicted: restart broadly from the last sighting and let it spread.
            tr.pf.reset(tr.last, this.ctxFor(e, f));
            tr.pf.step(Math.min(60, f.t - tr.last.t), e.moveSpeed);
          }
        }
        // Calibrated on a harvested replay: the particle spread alone is overconfident (truth inside
        // only ~30%); a speed x time floor brings it to ~70% for the first 15 s in fog.
        const est = tr.pf.estimate();
        const age = f.t - (tr.fogSince ?? f.t);
        tr.estimate = { ...est, radius: Math.min(0.45, Math.max(est.radius, (e.moveSpeed / MAP_UNITS) * age)) };
        // Freeze what the model believed, for scoring when the enemy reappears.
        const pf = tr.pf;
        tr.frozen = { massNear: q => pf.massNear(q), kinds: q => pf.goalKindsNear(q), est: tr.estimate };
      }
    }
    return this.all;
  }

  /** Offline training: the true position of a fogged champion at time t is known (Riot timeline). */
  trainAt(champion: string, q: { x: number; y: number }, t: number) {
    const tr = this.tracks.get(champion);
    if (!tr || tr.state !== 'fog') return;
    const e: EnemyInput = { champion, team: tr.team, role: tr.role, isDead: false, moveSpeed: 345 };
    this.learnReappearance(tr, e, { champion, team: tr.team, x: q.x, y: q.y, score: 1, margin: 1, votes: 1 }, t);
  }

  private learnReappearance(tr: Internal, e: EnemyInput, det: Detection, t: number) {
    if (!this.learningEnabled) return;
    const gap = t - (tr.fogSince ?? t);
    if (!tr.frozen || !tr.baselineAt || gap < 3 || gap > 90) return;
    const q = { x: det.x, y: det.y };
    // Baseline: Gaussian around the last sighting with sigma growing at half the move speed.
    const sigma = Math.max(0.02, ((e.moveSpeed * 0.5) / MAP_UNITS) * gap);
    const r = 0.05, d = Math.hypot(q.x - tr.baselineAt.x, q.y - tr.baselineAt.y);
    const massBaseline = Math.min(1, (r * r) / (2 * sigma * sigma) * Math.exp(-(d * d) / (2 * sigma * sigma)));
    const prior: Partial<Record<GoalKind, number>> = {};
    for (const g of goalsFor(e.team)) prior[g.kind] = (prior[g.kind] ?? 0) + 1; // uniform-ish reference
    this.learner.observe({
      champion: e.champion, role: e.role, gameTime: t, gapSec: gap,
      credit: tr.frozen.kinds(q), prior: tr.priorShares && Object.keys(tr.priorShares).length ? tr.priorShares : prior,
      massModel: tr.frozen.massNear(q), massBaseline,
      errorPred: Math.hypot(q.x - tr.frozen.est.x, q.y - tr.frozen.est.y), errorBase: d,
    });
  }
}
