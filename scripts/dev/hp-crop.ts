// Dev helper: crop + nearest-neighbour zoom a region of a frame, or dump pixel values.
// usage: node hp-crop.cjs <png> <x> <y> <w> <h> <scale> <out.png>
//        node hp-crop.cjs <png> <x> <y> <w> <h> dump
import { readFileSync, writeFileSync } from 'node:fs';
import { decodePng, type Rgba } from '../../src/main/vision/png';
import { encodePng } from '../../src/main/vision/pngEncode';

const [file, xs, ys, ws, hs, mode, out] = process.argv.slice(2);
const img = decodePng(readFileSync(file!));
const x0 = +xs!, y0 = +ys!, w = +ws!, h = +hs!;
if (mode === 'dump') {
  for (let y = y0; y < y0 + h; y++) {
    const row: string[] = [];
    for (let x = x0; x < x0 + w; x++) {
      const o = (y * img.width + x) * 4;
      row.push(`${img.data[o]},${img.data[o + 1]},${img.data[o + 2]}`);
    }
    console.log(y, row.join(' '));
  }
} else {
  const s = +mode!;
  const o: Rgba = { width: w * s, height: h * s, data: new Uint8Array(w * s * h * s * 4) };
  for (let y = 0; y < h * s; y++) for (let x = 0; x < w * s; x++) {
    const sx = x0 + Math.floor(x / s), sy = y0 + Math.floor(y / s);
    const si = (sy * img.width + sx) * 4, di = (y * w * s + x) * 4;
    for (let c = 0; c < 4; c++) o.data[di + c] = img.data[si + c]!;
  }
  writeFileSync(out!, encodePng(o));
}
