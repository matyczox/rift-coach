import { sampleField, type Terrain } from './terrain';

/**
 * Where is an enemy we can't see? A particle filter per enemy:
 *  - particles start at the last sighting (or the fountain after respawn),
 *  - each particle follows one behaviour: walk to a goal (camp, a point on a lane,
 *    objective, river, our jungle), stay around where it vanished (bush, farming),
 *    or recall (8 s channel, then it is in the fountain),
 *  - movement follows walkable terrain at the champion's move speed; at a goal the
 *    particle wanders nearby instead of collapsing onto one point,
 *  - goals are drawn from a prior that depends on role, game phase, objective
 *    timers, the last movement direction, the player's profile and learning,
 *  - particles inside areas we currently see are removed (we would see them).
 * Output: centre (densest area) + radius holding ~70% of the probability.
 */

/** Summoner's Rift side length in game units (map is square on the minimap). */
export const MAP_UNITS = 14870;

export type GoalKind = 'camp' | 'lane-top' | 'lane-mid' | 'lane-bot' | 'dragon' | 'baron' | 'base' | 'invade' | 'river' | 'stay';

export interface Goal { id: string; kind: GoalKind; x: number; y: number; neutral?: boolean }

/** Goals in minimap coords for an enemy on the given team (CHAOS = red, top-right base). */
export function goalsFor(team: 'ORDER' | 'CHAOS'): Goal[] {
  // Side-relative goals are defined for a RED (CHAOS) enemy and mirrored (1-x, 1-y) for BLUE.
  // Lanes/river/objectives are neutral: several points along each lane, so a laner's
  // hypotheses spread along the lane instead of piling onto one point.
  const red: Goal[] = [
    { id: 'own-blue-buff', kind: 'camp', x: 0.75, y: 0.53 },
    { id: 'own-gromp', kind: 'camp', x: 0.82, y: 0.49 },
    { id: 'own-wolves', kind: 'camp', x: 0.72, y: 0.43 },
    { id: 'own-raptors', kind: 'camp', x: 0.53, y: 0.36 },
    { id: 'own-red-buff', kind: 'camp', x: 0.48, y: 0.26 },
    { id: 'own-krugs', kind: 'camp', x: 0.44, y: 0.17 },
    { id: 'enemy-blue-buff', kind: 'invade', x: 0.25, y: 0.47 },
    { id: 'enemy-raptors', kind: 'invade', x: 0.47, y: 0.62 },
    { id: 'enemy-red-buff', kind: 'invade', x: 0.52, y: 0.74 },
    { id: 'base', kind: 'base', x: 0.95, y: 0.05 },
  ];
  const neutralPoints: Goal[] = [
    { id: 'top-1', kind: 'lane-top', x: 0.08, y: 0.45 }, { id: 'top-2', kind: 'lane-top', x: 0.08, y: 0.25 },
    { id: 'top-3', kind: 'lane-top', x: 0.12, y: 0.10 }, { id: 'top-4', kind: 'lane-top', x: 0.28, y: 0.07 },
    { id: 'top-5', kind: 'lane-top', x: 0.45, y: 0.07 },
    { id: 'mid-1', kind: 'lane-mid', x: 0.38, y: 0.62 }, { id: 'mid-2', kind: 'lane-mid', x: 0.50, y: 0.50 },
    { id: 'mid-3', kind: 'lane-mid', x: 0.62, y: 0.38 },
    { id: 'bot-1', kind: 'lane-bot', x: 0.55, y: 0.93 }, { id: 'bot-2', kind: 'lane-bot', x: 0.72, y: 0.93 },
    { id: 'bot-3', kind: 'lane-bot', x: 0.88, y: 0.90 }, { id: 'bot-4', kind: 'lane-bot', x: 0.92, y: 0.75 },
    { id: 'bot-5', kind: 'lane-bot', x: 0.92, y: 0.55 },
    { id: 'dragon', kind: 'dragon', x: 0.67, y: 0.70, neutral: true },
    { id: 'baron', kind: 'baron', x: 0.33, y: 0.30, neutral: true },
    { id: 'river-top', kind: 'river', x: 0.30, y: 0.38 },
    { id: 'river-bot', kind: 'river', x: 0.70, y: 0.62 },
  ];
  const neutral: Goal[] = neutralPoints.map(g => ({ ...g, neutral: true }));
  const side = team === 'CHAOS' ? red : red.map(g => ({ ...g, x: 1 - g.x, y: 1 - g.y }));
  return [...side, ...neutral];
}

export function fountainOf(team: 'ORDER' | 'CHAOS') { return team === 'CHAOS' ? { x: 0.95, y: 0.05 } : { x: 0.05, y: 0.95 }; }

export type Role = 'TOP' | 'JUNGLE' | 'MIDDLE' | 'BOTTOM' | 'UTILITY' | 'UNKNOWN';

export interface PriorContext {
  role: Role;
  gameTime: number;
  /** Seconds until dragon / baron (re)spawns; ≤ 0 = alive now; null unknown. */
  dragonIn: number | null;
  baronIn: number | null;
  /** Enemy (this champion) is low/has gold to spend → likely to recall. */
  likelyRecall: boolean;
  /** Learned / profile multipliers per goal kind (default 1). */
  learned?: Partial<Record<GoalKind, number>>;
  /** Jungle profile: share of games starting top-side (0..1) for early game. */
  startTopSide?: number;
  /** Just left the fountain (respawn / proven recall): nobody stays or recalls again. */
  leavingBase?: boolean;
}

/** Tunable behaviour constants (fitted on harvested replays). */
export const TUNING = {
  stayWeight: 2.0,       // prior weight of "stays around where it vanished"
  recallWeight: 1.2,     // laners, after the first back timing
  recallChannel: 8,      // seconds before a recalling particle is in the fountain
  wanderFrac: 0.35,      // speed fraction while lingering at a goal
  wanderRadius: 0.04,
  leaveGoalRate: 0.06,   // per second, chance to pick a new goal after arriving
  leaveStayRate: 0.05,
  speedMin: 0.55,        // walking speed jitter (fraction of max)
};

/** Prior weight of each goal kind for an enemy, before learning. */
export function goalPrior(kind: GoalKind, c: PriorContext): number {
  const early = c.gameTime < 14 * 60;
  if (c.leavingBase && (kind === 'stay' || kind === 'base')) return 0.01;
  if (kind === 'stay') return TUNING.stayWeight * (c.learned?.stay ?? 1);
  const table: Record<Role, Partial<Record<GoalKind, number>>> = {
    JUNGLE: { camp: 5, 'lane-top': 1.2, 'lane-mid': 1.2, 'lane-bot': 1.2, dragon: 1.5, baron: 1, base: 0.5, invade: 0.8, river: 1.5 },
    TOP: { camp: 0.3, 'lane-top': 8, 'lane-mid': 0.4, 'lane-bot': 0.1, dragon: 0.2, baron: 0.6, base: TUNING.recallWeight, invade: 0.2, river: 0.6 },
    MIDDLE: { camp: 0.5, 'lane-top': 0.8, 'lane-mid': 8, 'lane-bot': 0.8, dragon: 0.8, baron: 0.6, base: TUNING.recallWeight, invade: 0.3, river: 1.2 },
    BOTTOM: { camp: 0.2, 'lane-top': 0.1, 'lane-mid': 0.4, 'lane-bot': 8, dragon: 1, baron: 0.2, base: TUNING.recallWeight, invade: 0.1, river: 0.4 },
    UTILITY: { camp: 0.3, 'lane-top': 0.2, 'lane-mid': 0.6, 'lane-bot': 6, dragon: 1.5, baron: 0.4, base: TUNING.recallWeight, invade: 0.6, river: 2 },
    UNKNOWN: {},
  };
  let w = table[c.role][kind] ?? 1;
  if (kind === 'base' && c.gameTime < 3.5 * 60) w *= 0.2; // nobody backs in the first minutes
  if (!early && (kind === 'lane-top' || kind === 'lane-mid' || kind === 'lane-bot')) w = Math.max(w * 0.6, 1.5); // roles blur mid/late game
  if (c.dragonIn !== null && c.dragonIn < 50 && kind === 'dragon') w *= 4;
  if (c.baronIn !== null && c.baronIn < 50 && kind === 'baron') w *= 4;
  if (c.likelyRecall && kind === 'base') w *= 5;
  w *= c.learned?.[kind] ?? 1;
  return Math.max(0.01, w);
}

interface Particle { x: number; y: number; goal: number; w: number; arrived: boolean }

export interface Estimate { x: number; y: number; radius: number; confidence: number; spread: number }

/** Deterministic small PRNG so tests are reproducible. */
export function rng(seed: number) {
  let s = seed >>> 0 || 1;
  return () => { s ^= s << 13; s ^= s >>> 17; s ^= s << 5; return ((s >>> 0) % 1_000_000) / 1_000_000; };
}

const clamp01 = (v: number) => Math.min(0.999, Math.max(0.001, v));

export class ParticleFilter {
  particles: Particle[] = [];
  private goals: Goal[];
  private fields: Float32Array[];
  private goalW: number[] = [];
  private elapsed = 0;
  private fountain: { x: number; y: number };

  constructor(
    private terrain: Terrain,
    baseGoals: Goal[],
    private n = 400,
    private rand: () => number = Math.random,
  ) {
    // Last goal slot is the dynamic "stay" goal (set on reset).
    this.goals = [...baseGoals, { id: 'stay', kind: 'stay', x: 0.5, y: 0.5 }];
    this.fields = this.goals.map(g => terrain.fieldTo(g));
    this.fountain = baseGoals.find(g => g.kind === 'base') ?? { x: 0.95, y: 0.05 };
  }

  /** Start from a point (last sighting / fountain) with a small spread. */
  reset(at: { x: number; y: number }, ctx: PriorContext, heading?: { vx: number; vy: number }) {
    const si = this.goals.length - 1;
    this.goals[si] = { id: 'stay', kind: 'stay', x: at.x, y: at.y };
    this.fields[si] = this.terrain.fieldTo(this.goals[si]!);
    this.elapsed = 0;
    this.setPrior(ctx, heading ? { at, heading } : undefined);
    this.particles = Array.from({ length: this.n }, () => ({
      x: clamp01(at.x + (this.rand() - 0.5) * 0.01), y: clamp01(at.y + (this.rand() - 0.5) * 0.01),
      goal: this.sampleGoal(), w: 1 / this.n, arrived: false,
    }));
  }

  /** Update goal weights (e.g. objective timer changed). Heading favours goals ahead of the last motion. */
  setPrior(ctx: PriorContext, motion?: { at: { x: number; y: number }; heading: { vx: number; vy: number } }) {
    const count = new Map<GoalKind, number>();
    for (const g of this.goals) count.set(g.kind, (count.get(g.kind) ?? 0) + 1);
    this.goalW = this.goals.map(g => {
      // A kind's weight is shared by its points (5 lane points ≠ 5× the lane).
      let w = goalPrior(g.kind, ctx) / count.get(g.kind)!;
      if (motion && g.kind !== 'stay' && g.kind !== 'base') {
        const sp = Math.hypot(motion.heading.vx, motion.heading.vy);
        if (sp > 1e-4) {
          const dx = g.x - motion.at.x, dy = g.y - motion.at.y, d = Math.hypot(dx, dy) || 1;
          const cos = (dx * motion.heading.vx + dy * motion.heading.vy) / (d * sp);
          w *= 0.35 + 1.3 * Math.max(0, cos); // goals behind the last movement are unlikely
        }
      }
      return w;
    });
  }

  private sampleGoal(exclude = -1): number {
    let total = 0;
    for (let i = 0; i < this.goalW.length; i++) if (i !== exclude) total += this.goalW[i]!;
    let r = this.rand() * total;
    for (let i = 0; i < this.goalW.length; i++) { if (i === exclude) continue; r -= this.goalW[i]!; if (r <= 0) return i; }
    return this.goalW.length - 2;
  }

  /** Advance by dt seconds at move speed (game units/s). */
  step(dt: number, moveSpeed: number) {
    // Large catch-up steps are split so terrain is followed.
    while (dt > 1.0) { this.stepOnce(1.0, moveSpeed); dt -= 1.0; }
    if (dt > 0) this.stepOnce(dt, moveSpeed);
  }

  private stepOnce(dt: number, moveSpeed: number) {
    this.elapsed += dt;
    const stepLen = (moveSpeed / MAP_UNITS) * dt;
    const t = this.terrain, cell = 1 / t.size;
    for (const p of this.particles) {
      const goal = this.goals[p.goal]!;
      // Recall: after the channel the champion is in its fountain.
      if (goal.kind === 'base' && this.elapsed >= TUNING.recallChannel + (p.w * 1e6 % 3)) {
        p.x = clamp01(this.fountain.x + (this.rand() - 0.5) * 0.02); p.y = clamp01(this.fountain.y + (this.rand() - 0.5) * 0.02);
        p.arrived = true;
        continue;
      }
      if (p.arrived || goal.kind === 'stay') {
        // Linger: wander around the goal, sometimes move on.
        const leave = goal.kind === 'stay' ? TUNING.leaveStayRate : TUNING.leaveGoalRate;
        if (this.rand() < dt * leave) { p.goal = this.sampleGoal(this.goals.length - 1); p.arrived = false; continue; }
        const a = this.rand() * Math.PI * 2, l = stepLen * TUNING.wanderFrac * this.rand();
        const nx = clamp01(p.x + Math.cos(a) * l), ny = clamp01(p.y + Math.sin(a) * l);
        if (t.walkable(nx, ny) && Math.hypot(nx - goal.x, ny - goal.y) < TUNING.wanderRadius + 0.01) { p.x = nx; p.y = ny; }
        continue;
      }
      let remaining = stepLen * (TUNING.speedMin + (1 - TUNING.speedMin) * this.rand());
      const field = this.fields[p.goal]!;
      for (let guard = 0; remaining > 1e-6 && guard < 12; guard++) {
        const here = sampleField(t, field, p.x, p.y);
        if (here < TUNING.wanderRadius * 0.5) { p.arrived = true; break; }
        let bx = 0, by = 0, best = here;
        for (const [dx, dy] of [[1, 0], [-1, 0], [0, 1], [0, -1], [1, 1], [1, -1], [-1, 1], [-1, -1]] as const) {
          const v = sampleField(t, field, p.x + dx * cell, p.y + dy * cell);
          if (v < best) { best = v; bx = dx; by = dy; }
        }
        if (bx === 0 && by === 0) break;
        const len = Math.hypot(bx, by) * cell, move = Math.min(remaining, len);
        p.x = clamp01(p.x + (bx / Math.hypot(bx, by)) * move);
        p.y = clamp01(p.y + (by / Math.hypot(bx, by)) * move);
        remaining -= move;
      }
    }
  }

  /**
   * Negative evidence: `visible(x,y)` = probability that this point is in our
   * vision right now (from the minimap fog mask). We didn't see the champion,
   * so particles there are (mostly) wrong.
   */
  observeAbsent(visible: (x: number, y: number) => number) {
    let total = 0;
    for (const p of this.particles) {
      const v = visible(clamp01(p.x), clamp01(p.y));
      p.w *= 1 - 0.95 * (Number.isFinite(v) ? Math.min(1, Math.max(0, v)) : 0);
      total += p.w;
    }
    if (!(total > 1e-9)) return false; // every hypothesis contradicted
    for (const p of this.particles) p.w /= total;
    if (this.effectiveN() < this.n * 0.5) this.resample();
    return true;
  }

  effectiveN() { let s = 0; for (const p of this.particles) s += p.w * p.w; return 1 / Math.max(s, 1e-12); }

  private resample() {
    const out: Particle[] = [];
    const step = 1 / this.n;
    let c = this.particles[0]!.w, i = 0;
    const r = this.rand() * step;
    for (let k = 0; k < this.n; k++) {
      const u = r + k * step;
      while (u > c && i < this.particles.length - 1) { i++; c += this.particles[i]!.w; }
      const p = this.particles[i]!;
      out.push({ x: clamp01(p.x + (this.rand() - 0.5) * 0.004), y: clamp01(p.y + (this.rand() - 0.5) * 0.004), goal: p.goal, w: step, arrived: p.arrived });
    }
    this.particles = out;
  }

  /** Densest area (mean-shift from the weighted mean) and the radius containing `mass`. */
  estimate(mass = 0.7): Estimate {
    let mx = 0, my = 0;
    for (const p of this.particles) { mx += p.x * p.w; my += p.y * p.w; }
    for (let it = 0; it < 4; it++) {
      let sx = 0, sy = 0, sw = 0;
      for (const p of this.particles) {
        const d2 = (p.x - mx) ** 2 + (p.y - my) ** 2, k = Math.exp(-d2 / (2 * 0.06 ** 2)) * p.w;
        sx += p.x * k; sy += p.y * k; sw += k;
      }
      if (sw > 1e-9) { mx = sx / sw; my = sy / sw; }
    }
    const ds = this.particles.map(p => ({ d: Math.hypot(p.x - mx, p.y - my), w: p.w })).sort((a, b) => a.d - b.d);
    let acc = 0, radius = 0;
    for (const e of ds) { acc += e.w; radius = e.d; if (acc >= mass) break; }
    let spread = 0;
    for (const p of this.particles) spread += p.w * Math.hypot(p.x - mx, p.y - my);
    return { x: mx, y: my, radius: Math.max(0.02, radius), confidence: Math.max(0, Math.min(1, 1 - radius / 0.35)), spread };
  }

  /** Probability mass within r of q (for learning / evaluation). */
  massNear(q: { x: number; y: number }, r = 0.05) {
    let m = 0;
    for (const p of this.particles) if (Math.hypot(p.x - q.x, p.y - q.y) <= r) m += p.w;
    return m;
  }

  /** Goal kinds of particles near q, weighted (credit assignment for learning). */
  goalKindsNear(q: { x: number; y: number }, r = 0.06): Partial<Record<GoalKind, number>> {
    const out: Partial<Record<GoalKind, number>> = {};
    for (const p of this.particles) if (Math.hypot(p.x - q.x, p.y - q.y) <= r) {
      const k = this.goals[p.goal]!.kind;
      out[k] = (out[k] ?? 0) + p.w;
    }
    return out;
  }
}
