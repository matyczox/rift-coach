import { describe, it, expect } from 'vitest';
import { readFileSync, readdirSync } from 'node:fs';
import path from 'node:path';
import { decodePng } from '../src/main/vision/png';
import { loadReference, REFERENCE_SIZE } from '../src/main/vision/background';
import { readVisionMask, truthVisionMask, mapCellMask, VISIBLE_THRESHOLD } from '../src/main/vision/fog';
import { walkableGrid, loadWalkable, distanceField, WALKABLE_GRID } from '../src/main/vision/terrain';

const ASSETS = path.join(__dirname, '..', 'assets');
const FIX = path.join(__dirname, 'fixtures', 'fog');
const pairs = readdirSync(FIX).filter(f => f.endsWith('-fog.png')).map(f => f.replace('-fog.png', ''));
const load = (f: string) => decodePng(readFileSync(path.join(FIX, f)));

describe('reference texture', () => {
  it('loads at native size and resizes', () => {
    const ref = loadReference(REFERENCE_SIZE, ASSETS);
    expect(ref.width).toBe(341);
    const small = loadReference(256, ASSETS);
    expect(small.width).toBe(256);
    expect(small.height).toBe(256);
  });
});

describe('readVisionMask', () => {
  const ref = loadReference(341, ASSETS);
  const map = mapCellMask(ref, 64);

  it.each(pairs)('matches the fog-free truth frame on map cells (%s)', (t) => {
    const fog = load(`${t}-fog.png`), clear = load(`${t}-clear.png`);
    const pred = readVisionMask(fog, ref, 64);
    const truth = truthVisionMask(fog, clear, 64, 0.75, ref);
    let ok = 0, n = 0, fogged = 0;
    for (let c = 0; c < 64 * 64; c++) {
      if (truth[c] === 255 || !map[c]) continue;
      n++; if ((pred[c]! >= VISIBLE_THRESHOLD ? 1 : 0) === truth[c]) ok++;
      if (truth[c] === 0) fogged++;
    }
    expect(n).toBeGreaterThan(800);
    expect(fogged).toBeGreaterThan(50); // the fixture really contains fog
    expect(ok / n).toBeGreaterThanOrEqual(0.9);
    for (const v of pred) { expect(v).toBeGreaterThanOrEqual(0); expect(v).toBeLessThanOrEqual(1); }
  });

  it('reads a fog-free frame as fully visible', () => {
    const clear = load(`${pairs[0]}-clear.png`);
    const pred = readVisionMask(clear, ref, 64);
    let vis = 0, n = 0;
    for (let c = 0; c < 64 * 64; c++) if (map[c]) { n++; if (pred[c]! >= VISIBLE_THRESHOLD) vis++; }
    expect(vis / n).toBeGreaterThan(0.97);
  });

  it('is fast (< 5 ms per 341 px frame after warm-up)', () => {
    const fog = load(`${pairs[0]}-fog.png`);
    readVisionMask(fog, ref, 64);
    const N = 20, t0 = performance.now();
    for (let i = 0; i < N; i++) readVisionMask(fog, ref, 64);
    expect((performance.now() - t0) / N).toBeLessThan(5);
  });
});

describe('terrain', () => {
  const G = WALKABLE_GRID;
  const walk = loadWalkable(G, ASSETS);
  const cell = (x: number, y: number) => Math.floor(y * G) * G + Math.floor(x * G);

  it('asset matches walkableGrid(reference)', () => {
    const fresh = walkableGrid(loadReference(341, ASSETS), G);
    let diff = 0; for (let i = 0; i < fresh.length; i++) if (fresh[i] !== walk[i]) diff++;
    expect(diff).toBe(0);
  });

  it('blue fountain area reaches red fountain area', () => {
    const df = distanceField(walk, { x: 0.06, y: 0.94 }, G);
    const d = df[cell(0.94, 0.06)]!;
    expect(Number.isFinite(d)).toBe(true);
    expect(d).toBeGreaterThanOrEqual(Math.hypot(0.88, 0.88) - 2 / G);
    expect(d).toBeLessThan(2);
    // all three lanes are reachable (side-lane midpoints)
    for (const [x, y] of [[0.08, 0.5], [0.5, 0.08], [0.92, 0.5], [0.5, 0.92], [0.1, 0.1], [0.9, 0.9]] as const) expect(Number.isFinite(df[cell(x, y)]!), `${x},${y}`).toBe(true);
  });

  it('known walls and the void are unwalkable', () => {
    expect(walk[cell(0.137, 0.61)]).toBe(0); // jungle wall between top lane and blue jungle
    expect(walk[cell(0.5, 0.995)]).toBe(0); // void below the map frame
    expect(walk[cell(0.5, 0.5)]).toBe(1); // mid lane / river crossing
    const df = distanceField(walk, { x: 0.06, y: 0.94 }, G);
    expect(df[cell(0.137, 0.61)]).toBe(Infinity);
  });

  it('distanceField is a consistent geodesic', () => {
    const from = { x: 0.5, y: 0.5 };
    const df = distanceField(walk, from, G);
    const s = cell(from.x, from.y);
    expect(df[s]).toBe(0);
    let checked = 0;
    for (let c = 0; c < G * G; c++) {
      const d = df[c]!; if (!Number.isFinite(d) || c === s) continue;
      const x = c % G, y = (c - x) / G;
      // never shorter than the straight line between cell centers
      expect(d).toBeGreaterThanOrEqual(Math.hypot(x - (s % G), y - Math.floor(s / G)) / G - 1e-4);
      // every reached cell has a neighbour closer to the source (monotone descent back to the start)
      let best = Infinity;
      for (let dy = -1; dy <= 1; dy++) for (let dx = -1; dx <= 1; dx++) {
        if ((dx | dy) === 0) continue; const xx = x + dx, yy = y + dy; if (xx < 0 || yy < 0 || xx >= G || yy >= G) continue;
        best = Math.min(best, df[yy * G + xx]!);
      }
      expect(best).toBeLessThan(d);
      checked++;
    }
    expect(checked).toBeGreaterThan(5000);
  });
});
