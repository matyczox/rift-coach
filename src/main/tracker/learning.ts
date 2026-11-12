import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import type { GoalKind, Role } from './predictor';

/**
 * What the predictor learns across games: for each (role, game phase) a
 * multiplier per goal kind ("junglers mid-game go to river/dragon more than the
 * default prior says"), updated every time a fogged enemy reappears.
 * Also keeps a running score of model vs the naive baseline ("still where we last saw it").
 */
export interface LearnedModel {
  version: 1;
  multipliers: Record<string, Partial<Record<GoalKind, number>>>;
  score: { n: number; logModel: number; logBaseline: number; hits: number };
  recent: Array<{ at: number; champion: string; gapSec: number; model: number; baseline: number; errorPred: number; errorBase: number }>;
}

/** Learning schedule (validated leave-one-out on harvested replays, see scripts/dev/eval-tracker.ts). */
export const LEARN_RATE = Number(process.env.RC_LEARN_RATE ?? 0.05);
export const LEARN_BAND: [number, number] = [0.6, 1.6];
export const LEARN_GAP: [number, number] = [5, 40];
export const LEARN_DECAY = 0.995;

export const phaseOf = (gameTime: number) => (gameTime < 14 * 60 ? 'early' : gameTime < 25 * 60 ? 'mid' : 'late');

export class Learner {
  model: LearnedModel = { version: 1, multipliers: {}, score: { n: 0, logModel: 0, logBaseline: 0, hits: 0 }, recent: [] };
  private dirty = false;

  constructor(private file: string | null = null) {
    if (file && existsSync(file)) {
      try { const m = JSON.parse(readFileSync(file, 'utf8')); if (m.version === 1) this.model = m; } catch { /* start fresh */ }
    }
  }

  multipliers(role: Role, gameTime: number) { return this.model.multipliers[`${role}|${phaseOf(gameTime)}`] ?? {}; }

  /**
   * A fogged enemy reappeared at q.
   * credit = goal-kind mass of particles near q; prior = goal-kind shares the filter used.
   * Exponentiated-gradient step towards kinds that explained the reappearance.
   */
  observe(e: {
    champion: string; role: Role; gameTime: number; gapSec: number;
    credit: Partial<Record<GoalKind, number>>; prior: Partial<Record<GoalKind, number>>;
    massModel: number; massBaseline: number; errorPred: number; errorBase: number;
  }) {
    const s = this.model.score, eps = 1e-3;
    s.n++;
    s.logModel += Math.log(e.massModel + eps);
    s.logBaseline += Math.log(e.massBaseline + eps);
    if (e.errorPred < e.errorBase) s.hits++;
    this.model.recent.unshift({ at: Date.now(), champion: e.champion, gapSec: Math.round(e.gapSec), model: +e.massModel.toFixed(3), baseline: +e.massBaseline.toFixed(3), errorPred: +e.errorPred.toFixed(3), errorBase: +e.errorBase.toFixed(3) });
    this.model.recent = this.model.recent.slice(0, 200);

    const creditTotal = Object.values(e.credit).reduce((a, b) => a + (b ?? 0), 0);
    const priorTotal = Object.values(e.prior).reduce((a, b) => a + (b ?? 0), 0);
    // Learn only from gaps where the destination is informative (very short = still in place, very long =
    // anything could have happened), in small steps, within a narrow band, slowly pulled back to neutral.
    if (creditTotal > 0.02 && priorTotal > 0 && e.gapSec >= LEARN_GAP[0] && e.gapSec <= LEARN_GAP[1]) {
      const key = `${e.role}|${phaseOf(e.gameTime)}`;
      const m = (this.model.multipliers[key] ??= {});
      const lr = LEARN_RATE;
      for (const k of new Set([...Object.keys(e.credit), ...Object.keys(e.prior)]) as Set<GoalKind>) {
        const c = (e.credit[k] ?? 0) / creditTotal, p = (e.prior[k] ?? 0) / priorTotal;
        const prev = 1 + ((m[k] ?? 1) - 1) * LEARN_DECAY;
        m[k] = Math.min(LEARN_BAND[1], Math.max(LEARN_BAND[0], prev * Math.exp(lr * (c - p))));
      }
    }
    this.dirty = true;
  }

  summary() {
    const s = this.model.score;
    return { samples: s.n, avgLogModel: s.n ? s.logModel / s.n : 0, avgLogBaseline: s.n ? s.logBaseline / s.n : 0, betterThanBaseline: s.n ? s.hits / s.n : 0 };
  }

  save() {
    if (!this.file || !this.dirty) return;
    mkdirSync(path.dirname(this.file), { recursive: true });
    writeFileSync(this.file, JSON.stringify(this.model));
    this.dirty = false;
  }
}
