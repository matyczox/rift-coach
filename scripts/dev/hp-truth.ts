// Ground truth from the replay HUD (shared by hp-eval / hp-digits).
// Side panels: ORDER on the left, CHAOS on the right; 5 slots each in index.json order.
// Each slot has a green HP bar under the portrait whose fill = true HP%.
// Bottom-left: the selected champion's HP bar (wider → finer truth).
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import type { Rgba } from '../../src/main/vision/png';

// First harvested replay in %APPDATA%/Rift Coach/hp-dataset, or RC_HP_DATASET.
const HP_ROOT = join(process.env.APPDATA ?? '', 'Rift Coach', 'hp-dataset');
export const DATASET = process.env.RC_HP_DATASET ?? join(HP_ROOT, (() => { try { return readdirSync(HP_ROOT).sort()[0] ?? ''; } catch { return ''; } })());

export interface FrameInfo { file: string; t: number; selected: string; team: 'ORDER' | 'CHAOS'; levels: Record<string, number>; dead: string[] }
export function loadIndex(): FrameInfo[] { return JSON.parse(readFileSync(join(DATASET, 'index.json'), 'utf8')); }

/** Champions in panel order: first 5 ORDER, last 5 CHAOS. */
export function teams(info: FrameInfo): { ORDER: string[]; CHAOS: string[] } {
  const names = Object.keys(info.levels);
  return { ORDER: names.slice(0, 5), CHAOS: names.slice(5, 10) };
}

const isGreen = (r: number, g: number, b: number) => g >= 60 && g - r >= 40 && g - b >= 30;
const isEmpty = (r: number, g: number, b: number) => Math.max(r, g, b) <= 40;
const isWhite = (r: number, g: number, b: number) => Math.min(r, g, b) >= 120 && Math.max(r, g, b) - Math.min(r, g, b) <= 40;

function measure(img: Rgba, x0: number, x1: number, rows: number[]): number | null {
  let best: number | null = null;
  for (const y of rows) {
    let last = -1, emptyEnd = -1, ok = true, odd = 0, shield = false;
    for (let x = x0; x <= x1; x++) {
      const o = (y * img.width + x) * 4;
      const r = img.data[o]!, g = img.data[o + 1]!, b = img.data[o + 2]!;
      if (isGreen(r, g, b)) { if (emptyEnd >= 0 || shield) { ok = false; break; } last = x; }
      else if (isWhite(r, g, b)) { if (emptyEnd >= 0) { ok = false; break; } shield = true; }
      else if (isEmpty(r, g, b)) emptyEnd = x;
      else if (++odd > 3) { ok = false; break; }
    }
    if (!ok) continue;
    const p = (last - x0 + 1) / (x1 - x0 + 1);
    best = best == null ? p : Math.max(best, p);
  }
  return best;
}

/** Panel HP truth (1440p layout, scaled by height). */
export function panelTruth(img: Rgba, info: FrameInfo): Map<string, number> {
  const s = img.height / 1440, sx = (v: number) => Math.round(v * s);
  const out = new Map<string, number>();
  const t = teams(info);
  for (let i = 0; i < 5; i++) {
    const y = sx(275 + 137 * i);
    const rows = [y + sx(3), y + sx(5)];
    const L = measure(img, sx(42), sx(96), rows);
    const R = measure(img, img.width - sx(2560 - 2463), img.width - sx(2560 - 2517), rows);
    const a = t.ORDER[i]!, b = t.CHAOS[i]!;
    if (L != null && !info.dead.includes(a)) out.set(a, L);
    if (R != null && !info.dead.includes(b)) out.set(b, R);
  }
  return out;
}

/** Selected champion's HP from the bottom-left stats bar (row above the "cur/max" text). */
export function selectedTruth(img: Rgba): number | null {
  const s = img.height / 1440, sx = (v: number) => Math.round(v * s);
  return measure(img, sx(101), sx(234), [img.height - sx(1440 - 1138), img.height - sx(1440 - 1140)]);
}

/**
 * HP oracle for identity: the team champion whose panel HP is closest to the
 * bar's pct, if that is clearly better than the runner-up. Independent of the
 * level digits, so it can grade level-based assignment.
 */
export function hpOracle(pct: number, team: string[], truth: Map<string, number>, tol = 0.05, sep = 0.08): string | null {
  const c = team.filter((n) => truth.has(n)).map((n) => ({ n, d: Math.abs(truth.get(n)! - pct) })).sort((a, b) => a.d - b.d);
  if (!c.length || c[0]!.d > tol) return null;
  if (c[1] && c[1].d < sep) return null;
  return c[0]!.n;
}
