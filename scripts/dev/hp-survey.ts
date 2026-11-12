// Dev survey: run the detector over the dataset and write montages of detected bars.
// usage: node hp-survey.cjs <outDir> [color=red|blue] [maxFrames]
import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { decodePng, type Rgba } from '../../src/main/vision/png';
import { encodePng } from '../../src/main/vision/pngEncode';
import { readHealthBars, type HealthBar } from '../../src/main/vision/healthbars';
import { DATASET, loadIndex, panelTruth } from './hp-truth';

const [outDir, color = 'red', maxFrames = '9999'] = process.argv.slice(2);
const index = loadIndex().slice(0, +maxFrames);
const TW = 190, TH = 60, COLS = 5, PER = 40;
const tiles: Array<{ img: Rgba; bar: HealthBar; label: string }> = [];
for (const info of index) {
  const img = decodePng(readFileSync(join(DATASET, info.file)));
  const bars = readHealthBars(img, { enemyColor: color as 'red' | 'blue', minConfidence: +(process.env.MINC ?? 0.6) });
  const truth = panelTruth(img, info);
  console.log(info.file, bars.map((b) => `(${b.x},${b.y}) ${(b.pct * 100).toFixed(0)}% sh${(b.shieldPct * 100).toFixed(0)} L${b.level} c${b.confidence.toFixed(2)}`).join(' | '),
    '  truth', [...truth].map(([k, v]) => `${k}:${(v * 100).toFixed(0)}`).join(' '));
  for (const b of bars) tiles.push({ img, bar: b, label: info.file });
}
for (let p = 0; p * PER < tiles.length; p++) {
  const chunk = tiles.slice(p * PER, (p + 1) * PER);
  const rows = Math.ceil(chunk.length / COLS);
  const m: Rgba = { width: TW * COLS, height: TH * rows, data: new Uint8Array(TW * COLS * TH * rows * 4) };
  chunk.forEach((t, k) => {
    const ox = (k % COLS) * TW, oy = Math.floor(k / COLS) * TH;
    for (let y = 0; y < TH - 2; y++) for (let x = 0; x < TW - 2; x++) {
      const sx = t.bar.x - 40 + x, sy = t.bar.y - 32 + y;
      if (sx < 0 || sy < 0 || sx >= t.img.width || sy >= t.img.height) continue;
      const si = (sy * t.img.width + sx) * 4, di = ((oy + y) * m.width + ox + x) * 4;
      for (let c = 0; c < 4; c++) m.data[di + c] = t.img.data[si + c]!;
    }
  });
  writeFileSync(join(outDir!, `montage-${color}-${p}.png`), encodePng(m));
}
console.log('tiles', tiles.length);
