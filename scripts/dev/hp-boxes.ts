// Dev: montage of every detected bar's level box (10 per row, 3x zoom) for hand labelling.
// Prints "<k> <file> <color> <x> <y> <readLevel>" in montage order.
// usage: node hp-boxes.cjs <out.png>
import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { decodePng, type Rgba } from '../../src/main/vision/png';
import { encodePng } from '../../src/main/vision/pngEncode';
import { readHealthBars } from '../../src/main/vision/healthbars';
import { DATASET, loadIndex } from './hp-truth';

const out = process.argv[2]!;
const Z = 3, BW = 36, BH = 34, COLS = 10;
const crops: Array<{ img: Rgba; x: number; y: number }> = [];
let k = 0;
for (const info of loadIndex()) {
  const img = decodePng(readFileSync(join(DATASET, info.file)));
  for (const color of ['red', 'blue'] as const) {
    for (const b of readHealthBars(img, { enemyColor: color })) {
      console.log(k++, info.file, color, b.x, b.y, b.level);
      crops.push({ img, x: b.x - 32, y: b.y - 8 });
    }
  }
}
const rows = Math.ceil(crops.length / COLS);
const m: Rgba = { width: COLS * BW * Z, height: rows * BH * Z, data: new Uint8Array(COLS * BW * Z * rows * BH * Z * 4) };
crops.forEach((c, i) => {
  const ox = (i % COLS) * BW * Z, oy = Math.floor(i / COLS) * BH * Z;
  for (let y = 0; y < (BH - 2) * Z; y++) for (let x = 0; x < (BW - 2) * Z; x++) {
    const sx = c.x + Math.floor(x / Z), sy = c.y + Math.floor(y / Z);
    if (sx < 0 || sy < 0 || sx >= c.img.width || sy >= c.img.height) continue;
    const si = (sy * c.img.width + sx) * 4, di = ((oy + y) * m.width + ox + x) * 4;
    for (let q = 0; q < 4; q++) m.data[di + q] = c.img.data[si + q]!;
  }
});
writeFileSync(out, encodePng(m));
