import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { decodePng, resize, type Rgba } from '../src/main/vision/png';
import { readHealthBars, matchBarsToChampions } from '../src/main/vision/healthbars';

// Crops from a 2560x1440 replay (scripts/dev/hp-fixtures.ts). Expected HP values
// are the replay side-panel truth, not the detector's own output.
const FIX = path.join(__dirname, 'fixtures', 'hp');
const load = (f: string) => decodePng(readFileSync(path.join(FIX, f)));
const H = 1440; // crops come from a 1440p window

describe('readHealthBars', () => {
  it('reads a red bar: HP and level', () => {
    const bars = readHealthBars(load('red-l6-75.png'), { enemyColor: 'red', screenHeight: H });
    expect(bars).toHaveLength(1);
    expect(bars[0]!.pct).toBeCloseTo(0.75, 1);
    expect(Math.abs(bars[0]!.pct - 0.75)).toBeLessThan(0.02);
    expect(bars[0]!.level).toBe(6);
    expect(bars[0]!.shieldPct).toBe(0);
    expect(bars[0]!.confidence).toBeGreaterThan(0.8);
    // geometry: fill area left edge / width in crop coords
    expect(bars[0]!.x).toBe(40);
    expect(bars[0]!.width).toBe(125);
  });

  it('reads a blue bar and ignores it when asked for red', () => {
    const img = load('blue-l5-98.png');
    const blue = readHealthBars(img, { enemyColor: 'blue', screenHeight: H });
    expect(blue).toHaveLength(1);
    expect(Math.abs(blue[0]!.pct - 0.98)).toBeLessThan(0.02);
    expect(blue[0]!.level).toBe(5);
    expect(readHealthBars(img, { enemyColor: 'red', screenHeight: H })).toHaveLength(0);
  });

  it('reads a near-empty bar', () => {
    const bars = readHealthBars(load('red-l9-low.png'), { enemyColor: 'red', screenHeight: H });
    expect(bars).toHaveLength(1);
    expect(Math.abs(bars[0]!.pct - 0.055)).toBeLessThan(0.02);
    expect(bars[0]!.level).toBe(9);
  });

  it('flags a shield that fills the bar (HP is a lower bound)', () => {
    // Malphite at 100% HP with a shield: bar shows ~91% red + ~9% grey.
    const bars = readHealthBars(load('red-l15-shield.png'), { enemyColor: 'red', screenHeight: H });
    expect(bars).toHaveLength(1);
    const b = bars[0]!;
    expect(b.level).toBe(15);
    expect(b.shieldPct).toBeGreaterThan(0.05);
    expect(b.shieldCompressed).toBe(true);
    expect(b.pct + b.shieldPct).toBeGreaterThan(0.97);
  });

  it('reads two stacked bars of the same team', () => {
    const bars = readHealthBars(load('red-l18-pair.png'), { enemyColor: 'red', screenHeight: H });
    expect(bars).toHaveLength(2);
    expect(bars.map((b) => b.level)).toEqual([18, 18]);
    expect(Math.abs(bars[0]!.pct - 0.96)).toBeLessThan(0.03); // Hecarim, panel 96%
    expect(Math.abs(bars[1]!.pct - 0.47)).toBeLessThan(0.02); // Yone, panel 47%
    expect(bars[0]!.y).toBeLessThan(bars[1]!.y);
  });

  it('scales to a 1080p window', () => {
    for (const [f, pct, level] of [['red-l6-75.png', 0.75, 6], ['red-l15-shield.png', 0.91, 15], ['blue-l5-98.png', 0.98, 5]] as const) {
      const img = load(f);
      const small = resize(img, Math.round(img.width * 0.75), Math.round(img.height * 0.75));
      const bars = readHealthBars(small, { enemyColor: f.startsWith('red') ? 'red' : 'blue', screenHeight: 1080 });
      expect(bars, f).toHaveLength(1);
      expect(Math.abs(bars[0]!.pct - pct), f).toBeLessThan(0.025);
      expect(bars[0]!.level, f).toBe(level);
    }
  });

  it('finds nothing in a frame without bars and is fast on a full 1440p frame', () => {
    // synthetic 1440p frame: noisy reds/blues/greys but no bar structure
    const W = 2560, img: Rgba = { width: W, height: H, data: new Uint8Array(W * H * 4) };
    let seed = 7;
    const rnd = () => ((seed = (seed * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff);
    for (let i = 0; i < W * H; i++) {
      const k = rnd();
      const c = k < 0.2 ? [180, 50, 40] : k < 0.4 ? [50, 160, 240] : k < 0.6 ? [90, 90, 90] : [20, 20, 20];
      img.data.set([c[0]!, c[1]!, c[2]!, 255], i * 4);
    }
    readHealthBars(img, { enemyColor: 'red' }); // warm-up
    const t0 = performance.now();
    const bars = [...readHealthBars(img, { enemyColor: 'red' }), ...readHealthBars(img, { enemyColor: 'blue' })];
    const ms = performance.now() - t0;
    expect(bars).toHaveLength(0);
    expect(ms).toBeLessThan(200); // worst case (every pixel a candidate); real frames take ~1.5 ms per colour
  });
});

describe('matchBarsToChampions', () => {
  const bar = (x: number, y: number, level: number | null) => ({ x, y, width: 125, level, pct: 0.5 });

  it('assigns by unique level', () => {
    const res = matchBarsToChampions([bar(100, 100, 6), bar(900, 500, 9)], [
      { champion: 'Yone', level: 6 }, { champion: 'Hecarim', level: 9 }, { champion: 'Xayah', level: 7 },
    ]);
    expect(res.map((r) => r.champion)).toEqual(['Yone', 'Hecarim']);
  });

  it('leaves equal-level bars unassigned without positions', () => {
    const res = matchBarsToChampions([bar(100, 100, 18)], [{ champion: 'Yone', level: 18 }, { champion: 'Hecarim', level: 18 }]);
    expect(res[0]!.champion).toBeNull();
  });

  it('breaks level ties by screen proximity (bar ~1.6 widths above the ground point)', () => {
    const res = matchBarsToChampions([bar(1000, 400, 18), bar(300, 800, 18)], [
      { champion: 'Yone', level: 18, screen: { x: 1060, y: 400 + 200 } },
      { champion: 'Hecarim', level: 18, screen: { x: 370, y: 800 + 200 } },
    ]);
    expect(res.map((r) => r.champion)).toEqual(['Yone', 'Hecarim']);
  });

  it('stays unassigned when two candidates are about equally close', () => {
    const res = matchBarsToChampions([bar(1000, 400, null)], [
      { champion: 'Yone', level: 18, screen: { x: 1000, y: 600 } },
      { champion: 'Hecarim', level: 17, screen: { x: 1120, y: 600 } },
    ]);
    expect(res[0]!.champion).toBeNull();
  });

  it('uses proximity for an unread level and never reuses a champion', () => {
    const res = matchBarsToChampions([bar(1000, 400, 11), bar(1010, 420, null)], [
      { champion: 'Yone', level: 11, screen: { x: 1060, y: 600 } },
      { champion: 'Xayah', level: 10, screen: { x: 1700, y: 900 } },
    ]);
    expect(res[0]!.champion).toBe('Yone');
    expect(res[1]!.champion).toBeNull(); // Xayah is too far away, Yone already taken
  });
});
