// Fog-of-war vision mask from a minimap crop (SPEC 5.3).
//
// The fog overlay is a near-uniform multiplicative darkening of the map texture (~0.36×
// luminance, hue preserved). Compared with the clean reference texture every map pixel
// therefore votes "visible" (frame ≈ reference) or "fogged" (frame ≈ 0.36 × reference); pixels
// whose color is not a scaled copy of the reference (icons, pings, camera box, vision rings)
// abstain. Cells without votes (walls/void, cells fully under an icon) are filled from their
// neighbours, then lightly smoothed.
import type { Rgba } from './png';

export interface FogParams {
  /** Luminance factor the fog overlay applies (calibrated on replay data). */
  fogFactor: number;
  /** Reference pixels darker than this (luminance 0–255) carry no signal: walls/void. */
  minRefLum: number;
  /** Max per-channel color residual (fraction of reference channel sum) for a pixel to vote. */
  maxResidual: number;
  /** Center weight of the 3×3 smoothing kernel (neighbours weigh 1); 0 disables smoothing. */
  smoothCenter: number;
}

export const DEFAULT_FOG_PARAMS: FogParams = { fogFactor: 0.36, minRefLum: 40, maxResidual: 0.12, smoothCenter: 8 };

/**
 * Decision threshold on the returned probability, calibrated on the first half of a 39-min replay
 * (see scripts/dev/fog-eval.ts). `mask[i] >= VISIBLE_THRESHOLD` ⇒ treat the cell as visible.
 */
export const VISIBLE_THRESHOLD = 0.52;

interface Prep { grid: number; cell: Int32Array; refR: Uint8Array; refG: Uint8Array; refB: Uint8Array; refSum: Float32Array; pix: Int32Array; mapCells: Uint8Array }

const prepCache = new WeakMap<Rgba, Map<number, Prep>>();

function prepare(reference: Rgba, grid: number, p: FogParams): Prep {
  let byGrid = prepCache.get(reference);
  if (!byGrid) { byGrid = new Map(); prepCache.set(reference, byGrid); }
  const key = grid * 1000 + p.minRefLum;
  const hit = byGrid.get(key);
  if (hit) return hit;
  const { width: w, height: h, data } = reference;
  const pixList: number[] = [];
  for (let i = 0; i < w * h; i++) {
    const o = i * 4, l = 0.299 * data[o]! + 0.587 * data[o + 1]! + 0.114 * data[o + 2]!;
    if (l >= p.minRefLum) pixList.push(i);
  }
  const n = pixList.length;
  const prep: Prep = {
    grid, pix: Int32Array.from(pixList), cell: new Int32Array(n), refR: new Uint8Array(n), refG: new Uint8Array(n), refB: new Uint8Array(n),
    refSum: new Float32Array(n), mapCells: new Uint8Array(grid * grid),
  };
  const counts = new Int32Array(grid * grid), totals = new Int32Array(grid * grid);
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) totals[Math.floor(y * grid / h) * grid + Math.floor(x * grid / w)]!++;
  for (let k = 0; k < n; k++) {
    const i = pixList[k]!, o = i * 4, x = i % w, y = (i - x) / w;
    const c = Math.floor(y * grid / h) * grid + Math.floor(x * grid / w);
    prep.cell[k] = c; counts[c]!++;
    prep.refR[k] = data[o]!; prep.refG[k] = data[o + 1]!; prep.refB[k] = data[o + 2]!;
    prep.refSum[k] = data[o]! + data[o + 1]! + data[o + 2]!;
  }
  for (let c = 0; c < grid * grid; c++) prep.mapCells[c] = counts[c]! >= totals[c]! * 0.25 ? 1 : 0;
  byGrid.set(key, prep);
  return prep;
}

/**
 * Cells (row-major, `grid`×`grid`, index = gy*grid + gx, y down as in the image) that carry
 * fog signal at all: at least a quarter of the cell is non-wall map texture in the reference.
 */
export function mapCellMask(reference: Rgba, grid = 64, params: FogParams = DEFAULT_FOG_PARAMS): Uint8Array {
  return prepare(reference, grid, params).mapCells;
}

/**
 * Per grid cell probability that the cell is currently VISIBLE to the player's team (1) vs
 * fogged (0). `frame` and `reference` must be the same size (use loadReference(frame.width)).
 * Returns a row-major `grid*grid` array, index = gy*grid + gx (y down, blue base bottom-left).
 */
export function readVisionMask(frame: Rgba, reference: Rgba, grid = 64, params: FogParams = DEFAULT_FOG_PARAMS): Float32Array {
  if (frame.width !== reference.width || frame.height !== reference.height) throw new Error(`readVisionMask: frame ${frame.width}x${frame.height} vs reference ${reference.width}x${reference.height}`);
  const prep = prepare(reference, grid, params);
  const nc = grid * grid, d = frame.data, f = params.fogFactor, tol = params.maxResidual;
  const vis = new Float32Array(nc), votes = new Float32Array(nc);
  const { pix, cell, refR, refG, refB, refSum } = prep;
  for (let k = 0; k < pix.length; k++) {
    const o = pix[k]! * 4, r = d[o]!, g = d[o + 1]!, b = d[o + 2]!;
    const R = refR[k]!, G = refG[k]!, B = refB[k]!, S = refSum[k]!;
    const s = (r + g + b) / S; // brightness ratio
    if (s > 1.2) continue; // brighter than the map: icon/ping/camera box
    const res = Math.abs(r - s * R) + Math.abs(g - s * G) + Math.abs(b - s * B);
    if (res > tol * S * Math.max(s, 0.3) + 6) continue; // not a scaled copy of the map: icon
    let v = (s - f) / (1 - f);
    v = v < 0 ? 0 : v > 1 ? 1 : v;
    const c = cell[k]!;
    vis[c]! += v; votes[c]! += 1;
  }
  const out = new Float32Array(nc), known = new Uint8Array(nc);
  for (let c = 0; c < nc; c++) if (votes[c]! >= 2) { out[c] = vis[c]! / votes[c]!; known[c] = 1; }
  fillUnknown(out, known, grid);
  return params.smoothCenter > 0 ? smooth(out, grid, params.smoothCenter) : out;
}

/** Fill cells without votes with the mean of known 8-neighbours, growing outward. */
function fillUnknown(v: Float32Array, known: Uint8Array, grid: number): void {
  let frontier = true, pass = 0;
  const next = new Uint8Array(known.length);
  while (frontier && pass++ < grid) {
    frontier = false;
    next.set(known);
    for (let y = 0; y < grid; y++) for (let x = 0; x < grid; x++) {
      const c = y * grid + x;
      if (known[c]) continue;
      let s = 0, n = 0;
      for (let dy = -1; dy <= 1; dy++) { const yy = y + dy; if (yy < 0 || yy >= grid) continue;
        for (let dx = -1; dx <= 1; dx++) { const xx = x + dx; if (xx < 0 || xx >= grid) continue;
          const cc = yy * grid + xx; if (known[cc]) { s += v[cc]!; n++; } } }
      if (n) { v[c] = s / n; next[c] = 1; frontier = true; }
    }
    known.set(next);
  }
  for (let c = 0; c < v.length; c++) if (!known[c]) v[c] = 0; // nothing at all known: assume fog
}

function smooth(v: Float32Array, grid: number, center: number): Float32Array {
  const out = new Float32Array(v.length);
  for (let y = 0; y < grid; y++) for (let x = 0; x < grid; x++) {
    let s = v[y * grid + x]! * center, n = center;
    for (let dy = -1; dy <= 1; dy++) { const yy = y + dy; if (yy < 0 || yy >= grid) continue;
      for (let dx = -1; dx <= 1; dx++) { const xx = x + dx; if ((dx | dy) === 0 || xx < 0 || xx >= grid) continue;
        s += v[yy * grid + xx]!; n++; } }
    out[y * grid + x] = s / n;
  }
  return out;
}

/**
 * Ground-truth vision mask from a fogged frame and the fog-free frame at the same game time:
 * per cell median luminance ratio over map pixels; ratio < `fogRatio` ⇒ fogged (0), else 1.
 * With `reference`, only pixels where the clear frame shows the bare map count (icons excluded).
 * Cells with too few map pixels get 255 (unknown). Used for calibration and tests.
 */
export function truthVisionMask(fogged: Rgba, clear: Rgba, grid = 64, fogRatio = 0.75, reference?: Rgba): Uint8Array {
  const { width: w, height: h } = clear;
  const lists: number[][] = Array.from({ length: grid * grid }, () => []);
  const L = (d: Uint8Array, o: number) => 0.299 * d[o]! + 0.587 * d[o + 1]! + 0.114 * d[o + 2]!;
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
    const o = (y * w + x) * 4, lt = L(clear.data, o);
    if (lt < 40) continue;
    if (reference) { // skip pixels where the clear frame shows an icon instead of the map
      const rd = reference.data;
      if (L(rd, o) < 40 || Math.abs(clear.data[o]! - rd[o]!) + Math.abs(clear.data[o + 1]! - rd[o + 1]!) + Math.abs(clear.data[o + 2]! - rd[o + 2]!) > 60) continue;
    }
    lists[Math.floor(y * grid / h) * grid + Math.floor(x * grid / w)]!.push(L(fogged.data, o) / lt);
  }
  const cellPix = (w / grid) * (h / grid);
  const out = new Uint8Array(grid * grid);
  for (let c = 0; c < grid * grid; c++) {
    const l = lists[c]!;
    if (l.length < cellPix * 0.25) { out[c] = 255; continue; }
    l.sort((a, b) => a - b);
    out[c] = l[l.length >> 1]! < fogRatio ? 0 : 1;
  }
  return out;
}
