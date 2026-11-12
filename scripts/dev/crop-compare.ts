// Dev: crop the same minimap region from two harvest passes (e.g. blue view vs full view) at the
// nearest frames to a game time, upscale and save side by side with the detections drawn.
// usage: crop-compare <sessionDir> <ms> <x> <y> <size> <out.png>   (x, y, size as minimap fractions)
import { readdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { decodePng, type Rgba } from '../../src/main/vision/png';
import { encodePng } from '../../src/main/vision/pngEncode';

const [dir, msArg, xArg, yArg, sArg, out] = process.argv.slice(2) as [string, string, string, string, string, string];
const ms = Number(msArg), cx = Number(xArg), cy = Number(yArg), size = Number(sArg);
const SUBS = ['frames', 'all'] as const;
const SCALE = 6;
const tiles: Rgba[] = [];
for (const sub of SUBS) {
  const files = readdirSync(path.join(dir, sub)).map(f => parseInt(f, 10)).filter(Number.isFinite);
  const best = files.reduce((b, f) => Math.abs(f - ms) < Math.abs(b - ms) ? f : b, files[0]!);
  const img = decodePng(readFileSync(path.join(dir, sub, `${best}.png`)));
  const W = img.width, half = Math.round(size * W / 2), x0 = Math.round(cx * W) - half, y0 = Math.round(cy * W) - half, n = half * 2;
  const t: Rgba = { width: n * SCALE, height: n * SCALE, data: new Uint8Array(n * SCALE * n * SCALE * 4) };
  for (let y = 0; y < n * SCALE; y++) for (let x = 0; x < n * SCALE; x++) {
    const sx = Math.min(W - 1, Math.max(0, x0 + Math.floor(x / SCALE))), sy = Math.min(img.height - 1, Math.max(0, y0 + Math.floor(y / SCALE)));
    for (let k = 0; k < 4; k++) t.data[(y * t.width + x) * 4 + k] = img.data[(sy * W + sx) * 4 + k]!;
  }
  // detections as small white crosses
  const det = JSON.parse(readFileSync(path.join(dir, `detections-${sub}.json`), 'utf8'))[best] as Array<{ c: string; x: number; y: number }> | undefined;
  for (const d of det ?? []) {
    const px = Math.round((d.x * W - x0) * SCALE), py = Math.round((d.y * W - y0) * SCALE);
    for (let k = -8; k <= 8; k++) for (const [xx, yy] of [[px + k, py], [px, py + k]] as const)
      if (xx >= 0 && yy >= 0 && xx < t.width && yy < t.height) t.data.set([255, 255, 255, 255], (yy * t.width + xx) * 4);
    console.log(sub, best, d.c, ((d.x * W - x0) / n).toFixed(2), ((d.y * W - y0) / n).toFixed(2));
  }
  tiles.push(t);
}
const gap = 8, Wt = tiles.reduce((a, t) => a + t.width, 0) + gap, Ht = Math.max(...tiles.map(t => t.height));
const o: Rgba = { width: Wt, height: Ht, data: new Uint8Array(Wt * Ht * 4) };
let ox = 0;
for (const t of tiles) { for (let y = 0; y < t.height; y++) o.data.set(t.data.subarray(y * t.width * 4, (y + 1) * t.width * 4), (y * Wt + ox) * 4); ox += t.width + gap; }
writeFileSync(out, encodePng(o));
