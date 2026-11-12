// Clean reference minimap texture (no icons, no fog, no camera box).
// Built offline as a per-pixel robust median over many fog-free ("truth") frames:
// icons, pings and the camera rectangle move, the map does not.
import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { decodePng, resize, type Rgba } from './png';

/** Native size the reference asset is stored at (minimap crop of the harvest session). */
export const REFERENCE_SIZE = 341;
export const REFERENCE_FILE = 'minimap-reference.png';

/** Pixels we never trust for the background: the white camera rectangle and saturated team/ping colors. */
function isTransient(r: number, g: number, b: number): boolean {
  const mx = Math.max(r, g, b), mn = Math.min(r, g, b);
  if (mn > 190) return true; // camera box / white text
  if (r > 150 && r - Math.max(g, b) > 70) return true; // red team icons, pings
  return false;
}

/**
 * Per-pixel median of `frames` (all the same size), ignoring obviously transient pixels
 * when a pixel has enough clean samples. Ranking is by luminance so the chosen color is a real
 * observed color (no channel mixing).
 */
export function buildReference(frames: Rgba[]): Rgba {
  const f0 = frames[0];
  if (!f0) throw new Error('buildReference: no frames');
  const { width: w, height: h } = f0;
  const n = frames.length, out = new Uint8Array(w * h * 4);
  const lum = new Float32Array(n), idx = new Int32Array(n);
  for (let p = 0; p < w * h; p++) {
    const o = p * 4;
    let k = 0;
    for (let i = 0; i < n; i++) {
      const d = frames[i]!.data;
      if (!isTransient(d[o]!, d[o + 1]!, d[o + 2]!)) { idx[k] = i; lum[k] = 0.299 * d[o]! + 0.587 * d[o + 1]! + 0.114 * d[o + 2]!; k++; }
    }
    if (k < n * 0.2) { k = 0; for (let i = 0; i < n; i++) { const d = frames[i]!.data; idx[k] = i; lum[k] = 0.299 * d[o]! + 0.587 * d[o + 1]! + 0.114 * d[o + 2]!; k++; } }
    const order = Array.from({ length: k }, (_, j) => j).sort((a, b) => lum[a]! - lum[b]!);
    // Average the middle ~20% of samples by luminance rank: median-like but less noisy.
    const lo = Math.floor(k * 0.4), hi = Math.max(lo + 1, Math.ceil(k * 0.6));
    let r = 0, g = 0, b = 0;
    for (let j = lo; j < hi; j++) { const d = frames[idx[order[j]!]!]!.data; r += d[o]!; g += d[o + 1]!; b += d[o + 2]!; }
    const c = hi - lo;
    out[o] = Math.round(r / c); out[o + 1] = Math.round(g / c); out[o + 2] = Math.round(b / c); out[o + 3] = 255;
  }
  const img = { width: w, height: h, data: out };
  inpaintWhite(img);
  return img;
}

/**
 * The camera rectangle can sit in the same spot most of the game (thin white lines that survive
 * the median). Replace near-white pixels by the mean of non-white neighbours, growing inward.
 */
function inpaintWhite(img: Rgba): void {
  const { width: w, height: h, data } = img;
  const bad = new Uint8Array(w * h);
  for (let i = 0; i < w * h; i++) if (Math.min(data[i * 4]!, data[i * 4 + 1]!, data[i * 4 + 2]!) > 175) bad[i] = 1;
  // Dilate by 1 px: anti-aliased edges of the lines.
  const grown = bad.slice();
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) if (bad[y * w + x])
    for (let dy = -1; dy <= 1; dy++) for (let dx = -1; dx <= 1; dx++) { const xx = x + dx, yy = y + dy; if (xx >= 0 && yy >= 0 && xx < w && yy < h) grown[yy * w + xx] = 1; }
  bad.set(grown);
  for (let pass = 0; pass < 20; pass++) {
    let any = false;
    const fixed: Array<[number, number, number, number]> = [];
    for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
      const i = y * w + x; if (!bad[i]) continue;
      let r = 0, g = 0, b = 0, n = 0;
      for (let dy = -1; dy <= 1; dy++) for (let dx = -1; dx <= 1; dx++) {
        const xx = x + dx, yy = y + dy; if (xx < 0 || yy < 0 || xx >= w || yy >= h) continue;
        const j = yy * w + xx; if (bad[j]) continue;
        r += data[j * 4]!; g += data[j * 4 + 1]!; b += data[j * 4 + 2]!; n++;
      }
      if (n) fixed.push([i, r / n, g / n, b / n]);
    }
    for (const [i, r, g, b] of fixed) { data[i * 4] = Math.round(r); data[i * 4 + 1] = Math.round(g); data[i * 4 + 2] = Math.round(b); bad[i] = 0; any = true; }
    if (!any) break;
  }
}

/** Candidate locations of the repo-level `assets/` folder (dev bundle, packaged app, tests). */
export function assetsDirCandidates(): string[] {
  const c: string[] = [];
  if (typeof __dirname === 'string') c.push(path.join(__dirname, '..', 'assets'), path.join(__dirname, '..', '..', '..', 'assets'));
  const rp = (process as unknown as { resourcesPath?: string }).resourcesPath;
  if (rp) c.push(path.join(rp, 'assets'));
  c.push(path.join(process.cwd(), 'assets'));
  return c;
}

export function findAsset(file: string, assetsDir?: string): string {
  const dirs = assetsDir ? [assetsDir] : assetsDirCandidates();
  for (const d of dirs) { const p = path.join(d, file); if (existsSync(p)) return p; }
  throw new Error(`asset ${file} not found in ${dirs.join(', ')}`);
}

const cache = new Map<string, Rgba>();

/** Reference minimap texture resized to `size`×`size` (cached). */
export function loadReference(size: number = REFERENCE_SIZE, assetsDir?: string): Rgba {
  const key = `${assetsDir ?? ''}|${size}`;
  const hit = cache.get(key);
  if (hit) return hit;
  const base = decodePng(readFileSync(findAsset(REFERENCE_FILE, assetsDir)));
  const img = base.width === size && base.height === size ? base : resize(base, size, size);
  cache.set(key, img);
  return img;
}
