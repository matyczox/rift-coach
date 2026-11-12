// Build level-digit templates from the replay dataset.
// Labels: hand-verified levels per detected bar (hp-labels.ts, read by eye from
// the hp-boxes.ts montage). The replay never shows a '3' (no level 3/13 bar on
// screen), so '3' is synthesised from the measured '8' (left strokes removed).
// Prints DIGIT_TEMPLATES to paste into src/main/vision/healthbars.ts and a
// leave-one-out style check (each sample vs all templates).
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { decodePng, resize } from '../../src/main/vision/png';
import { readHealthBars, levelGlyphs, GLYPH_W, GLYPH_H } from '../../src/main/vision/healthbars';
import { DATASET, loadIndex } from './hp-truth';
import { LEVEL_LABELS } from './hp-labels';

const N = GLYPH_W * GLYPH_H;
const samples: Array<{ d: number; g: number[]; file: string }> = [];
let unlabeled = 0, mismatch = 0;
for (const info of loadIndex()) {
  const labels = LEVEL_LABELS.filter((l) => l.file === info.file);
  if (!labels.length) continue;
  const img0 = decodePng(readFileSync(join(DATASET, info.file)));
  // native 1440p plus bilinear 1200p/1080p versions so templates cover the blur of smaller UIs
  for (const scale of [1, 0.833, 0.75]) {
  const img = scale === 1 ? img0 : resize(img0, Math.round(img0.width * scale), Math.round(img0.height * scale));
  for (const color of ['red', 'blue'] as const) {
    for (const bar of readHealthBars(img, { enemyColor: color })) {
      const lab = labels.find((l) => l.color === color && Math.abs(l.x * scale - bar.x) <= 4);
      if (!lab) { unlabeled++; continue; }
      const digits = String(lab.level).split('').map(Number);
      const glyphs = levelGlyphs(img, bar);
      if (glyphs.length !== digits.length) { mismatch++; continue; }
      digits.forEach((d, i) => samples.push({ d, g: glyphs[i]!, file: info.file }));
    }
  }
  }
}
const mean = new Map<number, number[]>();
for (let d = 0; d <= 9; d++) {
  const ss = samples.filter((s) => s.d === d);
  if (!ss.length) continue;
  const m = new Array<number>(N).fill(0);
  for (const s of ss) s.g.forEach((v, k) => { m[k]! += v / ss.length; });
  mean.set(d, m);
}
// synthetic '3': the '8' with its left-side strokes (rows 1-3 and 5-6, cols 0-1) blanked
if (!mean.has(3) && mean.has(8)) {
  const t = [...mean.get(8)!];
  for (const y of [1, 2, 3, 5, 6]) for (const x of [0, 1]) t[y * GLYPH_W + x] = 0;
  for (const x of [0, 1]) t[4 * GLYPH_W + x] = 0.1;
  mean.set(3, t);
}
console.log(`samples ${samples.length}, unlabeled bars ${unlabeled}, glyph-count mismatch ${mismatch}`);
console.log('const DIGIT_TEMPLATES: Record<string, string> = {');
for (const [d, m] of [...mean].sort((a, b) => a[0] - b[0])) {
  const n = samples.filter((s) => s.d === d).length;
  console.log(`  '${d}': '${m.map((v) => Math.round(v * 9)).join('')}', // n=${n}${n ? '' : ' (synthetic)'}`);
}
console.log('};');
// self-check: nearest template per sample, and min distance between templates
let bad = 0, worst = 0;
for (const s of samples) {
  let best = -1, bd = Infinity;
  for (const [d, m] of mean) { let e = 0; for (let k = 0; k < N; k++) e += (s.g[k]! - m[k]!) ** 2; if (e < bd) { bd = e; best = d; } }
  worst = Math.max(worst, bd / N);
  if (best !== s.d) { bad++; console.error('misclassified', s.file, s.d, '->', best); }
}
let minSep = Infinity, pair = '';
for (const [a, ma] of mean) for (const [b, mb] of mean) if (a < b) {
  let e = 0; for (let k = 0; k < N; k++) e += (ma[k]! - mb[k]!) ** 2;
  if (e / N < minSep) { minSep = e / N; pair = `${a}/${b}`; }
}
console.error(`self-check: ${bad} misclassified, worst own-class dist ${worst.toFixed(4)}, closest templates ${pair} ${minSep.toFixed(4)}`);
