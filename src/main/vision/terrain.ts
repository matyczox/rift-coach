// Walkable terrain mask + geodesic distance field on the minimap grid (SPEC 5.5: particles move
// only through passable terrain). The LoL minimap draws walls and the void outside the map as
// (near-)black; lanes, jungle paths, river and bases are bright texture.
import { readFileSync } from 'node:fs';
import type { Rgba } from './png';
import { decodePng } from './png';
import { findAsset } from './background';

export const WALKABLE_FILE = 'walkable-128.png';
export const WALKABLE_GRID = 128;

export interface TerrainParams {
  /** Reference luminance at/above which a pixel is floor. */
  floorLum: number;
  /** Fraction of floor pixels needed for a cell to be walkable. */
  cellFrac: number;
  /** Enclosed non-walkable blobs up to this many cells are icon artefacts and get filled. */
  maxHole: number;
}

export const DEFAULT_TERRAIN_PARAMS: TerrainParams = { floorLum: 40, cellFrac: 0.4, maxHole: 6 };

const lum = (d: Uint8Array, o: number) => 0.299 * d[o]! + 0.587 * d[o + 1]! + 0.114 * d[o + 2]!;

/**
 * Walkable mask (row-major `grid*grid`, index = gy*grid + gx, y down; 1 = champions can stand
 * there) derived from the clean reference minimap texture.
 */
export function walkableGrid(reference: Rgba, grid = WALKABLE_GRID, params: TerrainParams = DEFAULT_TERRAIN_PARAMS): Uint8Array {
  const { width: w, height: h, data } = reference;
  const floor = new Float32Array(grid * grid), total = new Float32Array(grid * grid);
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
    const c = Math.floor(y * grid / h) * grid + Math.floor(x * grid / w);
    total[c]! += 1;
    if (lum(data, (y * w + x) * 4) >= params.floorLum) floor[c]! += 1;
  }
  let m: Uint8Array = new Uint8Array(grid * grid);
  for (let c = 0; c < m.length; c++) m[c] = total[c]! > 0 && floor[c]! / total[c]! >= params.cellFrac ? 1 : 0;
  // Fill small enclosed holes (dark parts of icons baked into the reference).
  const holes = components(m, grid, 0, false);
  for (const comp of holes) if (comp.length <= params.maxHole && !comp.touchesEdge) for (const c of comp) m[c] = 1;
  // Remove 1-cell spurs/specks: walkable cells with ≤1 walkable 4-neighbour.
  m = pruneSpurs(m, grid);
  // Keep the largest 8-connected walkable component (drops specks in the void and decorations).
  const comps = components(m, grid, 1, true).sort((a, b) => b.length - a.length);
  const out = new Uint8Array(grid * grid);
  const main = comps[0];
  if (main) for (const c of main) out[c] = 1;
  return out;
}

type Comp = number[] & { touchesEdge?: boolean };

function components(m: Uint8Array, grid: number, value: number, diag: boolean): Comp[] {
  const seen = new Uint8Array(m.length), out: Comp[] = [];
  const stack: number[] = [];
  for (let s = 0; s < m.length; s++) {
    if (seen[s] || m[s] !== value) continue;
    const comp: Comp = []; comp.touchesEdge = false;
    seen[s] = 1; stack.push(s);
    while (stack.length) {
      const c = stack.pop()!; comp.push(c);
      const x = c % grid, y = (c - x) / grid;
      if (x === 0 || y === 0 || x === grid - 1 || y === grid - 1) comp.touchesEdge = true;
      for (let dy = -1; dy <= 1; dy++) for (let dx = -1; dx <= 1; dx++) {
        if ((dx | dy) === 0 || (!diag && dx !== 0 && dy !== 0)) continue;
        const xx = x + dx, yy = y + dy; if (xx < 0 || yy < 0 || xx >= grid || yy >= grid) continue;
        const n = yy * grid + xx; if (seen[n] || m[n] !== value) continue;
        seen[n] = 1; stack.push(n);
      }
    }
    out.push(comp);
  }
  return out;
}

function pruneSpurs(m: Uint8Array, grid: number): Uint8Array {
  const out = m.slice();
  for (let y = 0; y < grid; y++) for (let x = 0; x < grid; x++) {
    const c = y * grid + x; if (!m[c]) continue;
    let n = 0;
    if (x > 0 && m[c - 1]) n++; if (x < grid - 1 && m[c + 1]) n++;
    if (y > 0 && m[c - grid]) n++; if (y < grid - 1 && m[c + grid]) n++;
    if (n <= 1) out[c] = 0;
  }
  return out;
}

/** Walkable grid → grayscale RGBA (255 = walkable) for the asset / debug renders. */
export function walkableToImage(walkable: Uint8Array, grid: number): Rgba {
  const data = new Uint8Array(grid * grid * 4);
  for (let c = 0; c < grid * grid; c++) { const v = walkable[c] ? 255 : 0; data[c * 4] = data[c * 4 + 1] = data[c * 4 + 2] = v; data[c * 4 + 3] = 255; }
  return { width: grid, height: grid, data };
}

/** Nearest-neighbour resample of a square mask to another grid size. */
export function resampleGrid(mask: Uint8Array, from: number, to: number): Uint8Array {
  if (from === to) return mask.slice();
  const out = new Uint8Array(to * to);
  for (let y = 0; y < to; y++) for (let x = 0; x < to; x++)
    out[y * to + x] = mask[Math.min(from - 1, Math.floor((y + 0.5) * from / to)) * from + Math.min(from - 1, Math.floor((x + 0.5) * from / to))]!;
  return out;
}

const walkCache = new Map<string, Uint8Array>();

/** Load assets/walkable-128.png as a `grid*grid` mask (resampled when grid ≠ 128; cached). */
export function loadWalkable(grid = WALKABLE_GRID, assetsDir?: string): Uint8Array {
  const key = `${assetsDir ?? ''}|${grid}`;
  const hit = walkCache.get(key);
  if (hit) return hit;
  const img = decodePng(readFileSync(findAsset(WALKABLE_FILE, assetsDir)));
  const base = new Uint8Array(img.width * img.height);
  for (let i = 0; i < base.length; i++) base[i] = img.data[i * 4]! >= 128 ? 1 : 0;
  const m = resampleGrid(base, img.width, grid);
  walkCache.set(key, m);
  return m;
}

/** Nearest walkable cell to (gx, gy) by ring search, or -1 if the mask is empty. */
export function nearestWalkable(walkable: Uint8Array, grid: number, gx: number, gy: number): number {
  const x0 = Math.max(0, Math.min(grid - 1, gx)), y0 = Math.max(0, Math.min(grid - 1, gy));
  if (walkable[y0 * grid + x0]) return y0 * grid + x0;
  let best = -1, bestD = Infinity;
  for (let r = 1; r < grid && best < 0; r++) {
    for (let y = y0 - r; y <= y0 + r; y++) for (let x = x0 - r; x <= x0 + r; x++) {
      if (Math.max(Math.abs(x - x0), Math.abs(y - y0)) !== r || x < 0 || y < 0 || x >= grid || y >= grid) continue;
      const c = y * grid + x; if (!walkable[c]) continue;
      const d = (x - x0) ** 2 + (y - y0) ** 2; if (d < bestD) { bestD = d; best = c; }
    }
  }
  return best;
}

/**
 * Geodesic distance (normalized map units: 1 = full minimap width) from `from` (normalized
 * 0..1 coords, x right, y down) to every cell, moving 8-connected through walkable cells
 * (diagonal step = √2 cells; no corner-cutting between two walls). Unreachable / wall cells are
 * Infinity. A start inside a wall snaps to the nearest walkable cell.
 */
export function distanceField(walkable: Uint8Array, from: { x: number; y: number }, grid = WALKABLE_GRID): Float32Array {
  const n = grid * grid, dist = new Float32Array(n).fill(Infinity);
  const start = nearestWalkable(walkable, grid, Math.floor(from.x * grid), Math.floor(from.y * grid));
  if (start < 0) return dist;
  // Dijkstra with a binary heap on (dist, cell).
  const heapD: number[] = [], heapC: number[] = [];
  const push = (d: number, c: number) => {
    let i = heapD.length; heapD.push(d); heapC.push(c);
    while (i > 0) { const p = (i - 1) >> 1; if (heapD[p]! <= d) break; heapD[i] = heapD[p]!; heapC[i] = heapC[p]!; i = p; }
    heapD[i] = d; heapC[i] = c;
  };
  const pop = (): number => {
    const top = heapC[0]!, lastD = heapD.pop()!, lastC = heapC.pop()!;
    if (heapD.length) {
      let i = 0; const len = heapD.length;
      for (;;) { const l = 2 * i + 1, r = l + 1; let m = i, md = lastD;
        if (l < len && heapD[l]! < md) { m = l; md = heapD[l]!; }
        if (r < len && heapD[r]! < md) { m = r; }
        if (m === i) break; heapD[i] = heapD[m]!; heapC[i] = heapC[m]!; i = m; }
      heapD[i] = lastD; heapC[i] = lastC;
    }
    return top;
  };
  const step = 1 / grid, dstep = Math.SQRT2 / grid;
  dist[start] = 0; push(0, start);
  const done = new Uint8Array(n);
  while (heapD.length) {
    const d0 = heapD[0]!, c = pop();
    if (done[c]) continue; // stale heap entry (dist is float32, so compare via `done`, not d0 > dist)
    done[c] = 1;
    const x = c % grid, y = (c - x) / grid;
    for (let dy = -1; dy <= 1; dy++) for (let dx = -1; dx <= 1; dx++) {
      if ((dx | dy) === 0) continue;
      const xx = x + dx, yy = y + dy; if (xx < 0 || yy < 0 || xx >= grid || yy >= grid) continue;
      const nc = yy * grid + xx; if (!walkable[nc] || done[nc]) continue;
      if (dx !== 0 && dy !== 0 && !walkable[y * grid + xx] && !walkable[yy * grid + x]) continue;
      const nd = d0 + (dx !== 0 && dy !== 0 ? dstep : step);
      if (nd < dist[nc]!) { dist[nc] = nd; push(nd, nc); }
    }
  }
  return dist;
}
