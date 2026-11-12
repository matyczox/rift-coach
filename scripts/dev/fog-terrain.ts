// Build assets/walkable-128.png from assets/minimap-reference.png and write debug renders.
// usage: fog-terrain [debugOutDir]
import { writeFileSync, mkdirSync } from 'node:fs';
import path from 'node:path';
import type { Rgba } from '../../src/main/vision/png';
import { encodePng } from '../../src/main/vision/pngEncode';
import { loadReference } from '../../src/main/vision/background';
import { walkableGrid, walkableToImage, distanceField, WALKABLE_FILE, WALKABLE_GRID } from '../../src/main/vision/terrain';

const dbg = process.argv[2];
const ref = loadReference(341);
const G = WALKABLE_GRID;
const walk = walkableGrid(ref, G);
mkdirSync('assets', { recursive: true });
writeFileSync(path.join('assets', WALKABLE_FILE), encodePng(walkableToImage(walk, G)));
let n = 0; for (const v of walk) n += v;
console.log(`walkable ${n}/${G * G} cells (${(100 * n / G / G).toFixed(1)}%) -> assets/${WALKABLE_FILE}`);
const blue = { x: 0.06, y: 0.94 }, red = { x: 0.94, y: 0.06 };
const df = distanceField(walk, blue, G);
const rc = Math.floor(red.y * G) * G + Math.floor(red.x * G);
console.log(`geodesic blue->red fountain ${df[rc]?.toFixed(3)} map units (straight line ${Math.hypot(0.88, 0.88).toFixed(3)})`);

if (dbg) {
  mkdirSync(dbg, { recursive: true });
  const S = 2, W = ref.width * S;
  const up = (paint: (x: number, y: number, c: number, o: Uint8Array, i: number) => void): Rgba => {
    const out = new Uint8Array(W * W * 4);
    for (let y = 0; y < W; y++) for (let x = 0; x < W; x++) {
      const sx = Math.floor(x / S), sy = Math.floor(y / S), si = (sy * ref.width + sx) * 4, i = (y * W + x) * 4;
      out[i] = ref.data[si]!; out[i + 1] = ref.data[si + 1]!; out[i + 2] = ref.data[si + 2]!; out[i + 3] = 255;
      paint(x, y, Math.floor(sy * G / ref.height) * G + Math.floor(sx * G / ref.width), out, i);
    }
    return { width: W, height: W, data: out };
  };
  const mix = (o: Uint8Array, i: number, c: [number, number, number], a: number) => { for (let k = 0; k < 3; k++) o[i + k] = Math.round(o[i + k]! * (1 - a) + c[k]! * a); };
  writeFileSync(path.join(dbg, 'walkable-overlay.png'), encodePng(up((x, y, c, o, i) => {
    const cx = c % G, cy = (c - cx) / G;
    const edge = [[1, 0], [-1, 0], [0, 1], [0, -1]].some(([dx, dy]) => { const xx = cx + dx!, yy = cy + dy!; return xx >= 0 && yy >= 0 && xx < G && yy < G && walk[yy * G + xx] !== walk[c]; });
    if (walk[c]) mix(o, i, [0, 255, 0], 0.35); else mix(o, i, [255, 0, 255], 0.25);
    if (edge && walk[c]) mix(o, i, [255, 255, 255], 0.6);
  })));
  let mx = 0; for (const d of df) if (Number.isFinite(d) && d > mx) mx = d;
  writeFileSync(path.join(dbg, 'distance-field.png'), encodePng(up((_x, _y, c, o, i) => {
    const d = df[c]!; if (!Number.isFinite(d)) { mix(o, i, [0, 0, 0], 0.7); return; }
    const t = d / mx; const band = Math.floor(d / 0.1) % 2 === 0;
    mix(o, i, [Math.round(255 * t), 64, Math.round(255 * (1 - t))], band ? 0.6 : 0.45);
  })));
  console.log(`debug -> ${path.join(dbg, 'walkable-overlay.png')}, ${path.join(dbg, 'distance-field.png')}`);
}
