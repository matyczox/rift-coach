// Dev: contact sheet of minimap crops over time (nearest-neighbour upscaled) for eyeballing camp icons.
// usage: camps-sheet <sessionDir> <sub[,sub2]> <x0,y0,w,h> <scale> <cols> <out.png> <ms...|from:to:step>
import { readFileSync, readdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { decodePng, type Rgba } from '../../src/main/vision/png';
import { encodePng } from '../../src/main/vision/pngEncode';

const [dir, subsArg, regionArg, scaleArg, colsArg, out, ...times] = process.argv.slice(2) as string[];
const subs = subsArg!.split(',');
const [x0, y0, w, h] = regionArg!.split(',').map(Number) as [number, number, number, number];
const S = Number(scaleArg), cols = Number(colsArg);
const list = (sub: string) => readdirSync(path.join(dir!, sub)).filter(f => f.endsWith('.png')).map(f => parseInt(f)).sort((a, b) => a - b);
const lists = new Map(subs.map(s => [s, list(s)]));
const nearest = (arr: number[], t: number) => arr.reduce((b, f) => (Math.abs(f - t) < Math.abs(b - t) ? f : b));
const ts: number[] = [];
for (const a of times) {
  if (a.includes(':')) { const [f, to, st] = a.split(':').map(Number) as [number, number, number]; for (let t = f; t <= to; t += st) ts.push(t); }
  else ts.push(Number(a));
}
const cellW = w * S + 4, cellH = h * S + 4;
const tiles = ts.length * subs.length;
const rows = Math.ceil(tiles / cols);
const img: Rgba = { width: cellW * cols, height: cellH * rows, data: new Uint8Array(cellW * cols * cellH * rows * 4) };
img.data.fill(255);
let k = 0;
for (const t of ts) for (const sub of subs) {
  const f = nearest(lists.get(sub)!, t);
  const src = decodePng(readFileSync(path.join(dir!, sub, `${f}.png`)));
  const cx = (k % cols) * cellW + 2, cy = Math.floor(k / cols) * cellH + 2;
  for (let y = 0; y < h * S; y++) for (let x = 0; x < w * S; x++) {
    const sx = x0 + Math.floor(x / S), sy = y0 + Math.floor(y / S);
    const o = ((cy + y) * img.width + cx + x) * 4;
    if (sx < 0 || sy < 0 || sx >= src.width || sy >= src.height) continue;
    const i = (sy * src.width + sx) * 4;
    img.data[o] = src.data[i]!; img.data[o + 1] = src.data[i + 1]!; img.data[o + 2] = src.data[i + 2]!; img.data[o + 3] = 255;
  }
  console.log(`${k}: ${sub} ${f}`);
  k++;
}
writeFileSync(out!, encodePng(img));
