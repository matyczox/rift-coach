// Dev: build per-camp appearance templates (up / respawning-grey / respawning-yellow / empty) from the
// fog-free truth frames of a harvest session, as per-pixel medians over frames pre-labelled by a crude
// color count. Prints a base64 blob to paste into src/main/vision/camps.ts (TEMPLATE_DATA) and writes a
// debug sheet of all templates.
// usage: camps-build <sessionDir> <outSheet.png> [every=4] [--write]   (--write replaces TEMPLATE_DATA in camps.ts)
import { readFileSync, readdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { decodePng, type Rgba } from '../../src/main/vision/png';
import { encodePng } from '../../src/main/vision/pngEncode';
import { CAMPS, PATCH_R, STATES } from '../../src/main/vision/camps';

const args = process.argv.slice(2).filter(a => a !== '--write'), write = process.argv.includes('--write');
const [dir, outSheet, everyArg] = args as [string, string, string?];
const every = Number(everyArg ?? 4);
const N = 2 * PATCH_R + 1;
const files = readdirSync(path.join(dir, 'truth')).map(f => parseInt(f)).sort((a, b) => a - b);
const endMs = files[files.length - 1]! - 8000;

function crude(img: Rgba, cx: number, cy: number): { warm: number; grey: number; yellow: number } {
  let warm = 0, grey = 0, yellow = 0;
  for (let y = cy - 4; y <= cy + 4; y++) for (let x = cx - 4; x <= cx + 4; x++) {
    const o = (y * img.width + x) * 4, r = img.data[o]!, g = img.data[o + 1]!, b = img.data[o + 2]!;
    if ((r > 180 && g > 110 && g < 200 && b < 90 && r - g > 40) || (r > 130 && r < 215 && g > 85 && g < 145 && b > 35 && b < 95 && r - g > 45)) warm++;
    else if (Math.abs(r - g) < 12 && Math.abs(g - b) < 12 && r > 110 && r < 210) grey++;
    else if (r > 200 && g > 190 && b < 120) yellow++;
  }
  return { warm, grey, yellow };
}

// samples[camp][state] = list of patches (Uint8 RGB N*N*3)
const samples = CAMPS.map(() => STATES.map(() => [] as Uint8Array[]));
for (let i = 0; i < files.length; i += every) {
  const t = files[i]!;
  if (t < 20000 || t > endMs) continue;
  const img = decodePng(readFileSync(path.join(dir, 'truth', `${t}.png`)));
  if (img.width !== 341) throw new Error('expected 341px crops');
  CAMPS.forEach((c, k) => {
    const cx = Math.round(c.x * 341), cy = Math.round(c.y * 341);
    const { warm, grey, yellow } = crude(img, cx, cy);
    let s = -1;
    if (warm >= 7 && grey < 3 && yellow < 2) s = 0;
    else if (grey >= 10 && warm < 2 && yellow < 2) s = 1;
    else if (yellow >= 10 && warm < 2) s = 2;
    else if (warm === 0 && grey === 0 && yellow === 0) s = 3;
    if (s < 0) return;
    const p = new Uint8Array(N * N * 3);
    for (let y = 0; y < N; y++) for (let x = 0; x < N; x++) {
      const o = ((cy - PATCH_R + y) * 341 + cx - PATCH_R + x) * 4;
      for (let ch = 0; ch < 3; ch++) p[(y * N + x) * 3 + ch] = img.data[o + ch]!;
    }
    samples[k]![s]!.push(p);
  });
}

const MAXV = 3;
const median = (list: Uint8Array[]) => { const m = new Uint8Array(N * N * 3); for (let i = 0; i < m.length; i++) { const v = list.map(p => p[i]!).sort((a, b) => a - b); m[i] = v[v.length >> 1]!; } return m; };
const core: number[] = [];
for (let y = 2; y < N - 2; y++) for (let x = 2; x < N - 2; x++) for (let ch = 0; ch < 3; ch++) core.push((y * N + x) * 3 + ch);
const dist = (a: Uint8Array, b: Uint8Array) => { let d = 0; for (const i of core) d += Math.abs(a[i]! - b[i]!); return d / core.length; };

/** k-medians (k<=MAXV) over time-ordered samples: the icon rendering changes between game phases. */
function variants(list: Uint8Array[]): Uint8Array[] {
  if (!list.length) return [];
  let cents = [0, 0.5, 0.999].map(q => list[Math.floor(q * list.length)]!);
  for (let it = 0; it < 10; it++) {
    const groups: Uint8Array[][] = cents.map(() => []);
    for (const p of list) { let b = 0; cents.forEach((c, j) => { if (dist(p, c) < dist(p, cents[b]!)) b = j; }); groups[b]!.push(p); }
    cents = groups.filter(g => g.length >= Math.max(8, list.length * 0.06)).map(median);
    if (!cents.length) cents = [median(list)];
  }
  const out: Uint8Array[] = []; // merge near-duplicates
  for (const c of cents) if (!out.some(o => dist(o, c) < 5)) out.push(c);
  return out;
}

const blob: number[] = [];
const CW = N * 6 + 4;
const sheet: Rgba = { width: STATES.length * MAXV * CW, height: CAMPS.length * CW, data: new Uint8Array(STATES.length * MAXV * CW * CAMPS.length * CW * 4).fill(255) };
CAMPS.forEach((c, k) => {
  // The bare floor has one look; clustering it would only capture champions standing on the camp.
  const vs = samples[k]!.map((l, s) => (STATES[s] === 'empty' ? (l.length ? [median(l)] : []) : variants(l)));
  console.error(`${c.id.padEnd(12)} ${STATES.map((s, i) => `${s} ${samples[k]![i]!.length}/${vs[i]!.length}v`).join(' ')}`);
  vs.forEach((list, s) => {
    blob.push(list.length);
    list.forEach((med, v) => {
      blob.push(...med);
      for (let y = 0; y < N * 6; y++) for (let x = 0; x < N * 6; x++) {
        const o = ((k * CW + 2 + y) * sheet.width + (s * MAXV + v) * CW + 2 + x) * 4, j = ((Math.floor(y / 6) * N) + Math.floor(x / 6)) * 3;
        sheet.data[o] = med[j]!; sheet.data[o + 1] = med[j + 1]!; sheet.data[o + 2] = med[j + 2]!; sheet.data[o + 3] = 255;
      }
    });
  });
});
writeFileSync(outSheet, encodePng(sheet));
const b64 = Buffer.from(blob).toString('base64');
if (write) {
  const file = path.join(process.cwd(), 'src', 'main', 'vision', 'camps.ts'); // run from the repo root
  const lines = (b64.match(/.{1,140}/g) ?? []).map(l => `  '${l}'`).join(' +\n');
  const src = readFileSync(file, 'utf8').replace(/\/\/ TEMPLATE_DATA_BEGIN[\s\S]*\/\/ TEMPLATE_DATA_END/, () =>
    `// TEMPLATE_DATA_BEGIN (generated by scripts/dev/camps-build.ts from replay truth frames; do not edit)\nconst TEMPLATE_DATA =\n${lines};\n// TEMPLATE_DATA_END`);
  writeFileSync(file, src);
  console.error(`wrote ${b64.length} chars of template data`);
} else process.stdout.write(b64);
