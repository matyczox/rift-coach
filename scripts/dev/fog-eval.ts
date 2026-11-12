// Calibrate + evaluate readVisionMask against fog-free truth frames of a two-pass harvest.
// Threshold is tuned on the first half of the game and reported on the second half.
// usage: fog-eval <sessionDir> [every=5] [debugOutDir] [debugMs]
import { readFileSync, readdirSync, writeFileSync, mkdirSync } from 'node:fs';
import path from 'node:path';
import { decodePng, type Rgba } from '../../src/main/vision/png';
import { encodePng } from '../../src/main/vision/pngEncode';
import { loadReference } from '../../src/main/vision/background';
import { readVisionMask, truthVisionMask, mapCellMask } from '../../src/main/vision/fog';

const [dir, everyArg, dbgDir, dbgMsArg] = process.argv.slice(2);
if (!dir) { console.error('usage: fog-eval <sessionDir> [every] [debugOutDir] [debugMs]'); process.exit(1); }
const every = Number(everyArg ?? 5), G = 64;
const ms = (sub: string) => readdirSync(path.join(dir, sub)).filter(f => f.endsWith('.png')).map(f => parseInt(f)).sort((a, b) => a - b);
// Skip the first minute (loading) and the post-nexus end screen (whole minimap greyed out).
const endMs = (JSON.parse(readFileSync(path.join(dir, 'meta.json'), 'utf8')) as { durationMs?: number }).durationMs ?? Infinity;
const frames = ms('frames').filter(t => t > 60_000 && t < endMs - 5000), truths = ms('truth');
const nearest = (t: number) => { let lo = 0, hi = truths.length - 1; while (hi - lo > 1) { const m = (lo + hi) >> 1; if (truths[m]! < t) lo = m; else hi = m; } return Math.abs(truths[lo]! - t) < Math.abs(truths[hi]! - t) ? truths[lo]! : truths[hi]!; };
const load = (sub: string, t: number) => decodePng(readFileSync(path.join(dir, sub, `${t}.png`)));

const ref = loadReference(341);
const mapCells = mapCellMask(ref, G);
type Sample = { t: number; pred: Float32Array; truth: Uint8Array };
const samples: Sample[] = [];
let timeMs = 0, nTimed = 0;
for (let i = 0; i < frames.length; i += every) {
  const t = frames[i]!, tt = nearest(t);
  if (Math.abs(tt - t) > 250) continue;
  const f = load('frames', t), tr = load('truth', tt);
  const t0 = performance.now();
  const pred = readVisionMask(f, ref, G);
  timeMs += performance.now() - t0; nTimed++;
  samples.push({ t, pred, truth: truthVisionMask(f, tr, G, 0.75, ref) });
}
const half = samples[samples.length >> 1]!.t;
const acc = (set: Sample[], thr: number) => {
  let ok = 0, n = 0, fogN = 0, fogOk = 0;
  for (const s of set) for (let c = 0; c < G * G; c++) {
    const tv = s.truth[c]!; if (tv === 255 || !mapCells[c]) continue;
    const pv = s.pred[c]! >= thr ? 1 : 0; n++; if (pv === tv) ok++;
    if (tv === 0) { fogN++; if (pv === 0) fogOk++; }
  }
  return { acc: ok / n, fogRecall: fogOk / fogN, n };
};
const train = samples.filter(s => s.t < half), test = samples.filter(s => s.t >= half);
let best = 0.5, bestAcc = 0;
for (let thr = 0.1; thr <= 0.9; thr += 0.02) { const a = acc(train, thr).acc; if (a > bestAcc) { bestAcc = a; best = thr; } }
const te = acc(test, best), te05 = acc(test, 0.5);
console.log(`samples ${samples.length} (train ${train.length}, test ${test.length}), split at ${(half / 60000).toFixed(1)} min`);
console.log(`train-best threshold ${best.toFixed(2)} train acc ${(bestAcc * 100).toFixed(2)}%`);
console.log(`held-out acc ${(te.acc * 100).toFixed(2)}% (fog recall ${(te.fogRecall * 100).toFixed(1)}%, ${te.n} cells); @0.5: ${(te05.acc * 100).toFixed(2)}%`);
console.log(`readVisionMask ${(timeMs / nTimed).toFixed(2)} ms/frame (incl. first-call prep)`);
// per-sample worst
const per = test.map(s => ({ t: s.t, a: acc([s], best).acc })).sort((a, b) => a.a - b.a);
console.log('worst test frames:', per.slice(0, 5).map(p => `${p.t}:${(p.a * 100).toFixed(1)}%`).join(' '));

if (dbgDir) {
  mkdirSync(dbgDir, { recursive: true });
  const t = dbgMsArg ? frames.reduce((a, b) => Math.abs(b - Number(dbgMsArg)) < Math.abs(a - Number(dbgMsArg)) ? b : a) : per[Math.floor(per.length / 2)]!.t;
  const f = load('frames', t), tr = load('truth', nearest(t));
  const pred = readVisionMask(f, ref, G), truth = truthVisionMask(f, tr, G, 0.75, ref);
  writeFileSync(path.join(dbgDir, 'fog-example.png'), encodePng(sideBySide([overlay(f, c => pred[c]! >= best ? null : [255, 0, 0, 0.45]),
    overlay(f, c => truth[c] === 255 ? [128, 128, 128, 0.3] : truth[c] ? null : [255, 0, 0, 0.45]),
    overlay(tr, c => truth[c] === 255 || !mapCells[c] ? null : (pred[c]! >= best ? 1 : 0) !== truth[c] ? [255, 255, 0, 0.8] : null)])));
  console.log(`debug frame ${t} -> ${path.join(dbgDir, 'fog-example.png')} (pred | truth | errors on truth frame)`);
}

function overlay(img: Rgba, color: (cell: number) => [number, number, number, number] | null): Rgba {
  const out = new Uint8Array(img.data);
  for (let y = 0; y < img.height; y++) for (let x = 0; x < img.width; x++) {
    const c = color(Math.floor(y * G / img.height) * G + Math.floor(x * G / img.width)); if (!c) continue;
    const o = (y * img.width + x) * 4;
    for (let k = 0; k < 3; k++) out[o + k] = Math.round(out[o + k]! * (1 - c[3]) + c[k]! * c[3]);
  }
  return { width: img.width, height: img.height, data: out };
}
function sideBySide(imgs: Rgba[]): Rgba {
  const h = imgs[0]!.height, gap = 6, w = imgs.reduce((s, i) => s + i.width, 0) + gap * (imgs.length - 1);
  const out = new Uint8Array(w * h * 4).fill(255);
  let x0 = 0;
  for (const im of imgs) { for (let y = 0; y < h; y++) out.set(im.data.subarray(y * im.width * 4, (y + 1) * im.width * 4), (y * w + x0) * 4); x0 += im.width + gap; }
  return { width: w, height: h, data: out };
}
