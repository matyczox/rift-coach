// Dev: probe ring colours / radius of minimap portraits on a real frame.
import { readFileSync } from 'node:fs';
import { decodePng } from '../../src/main/vision/png';

const img = decodePng(readFileSync(process.argv[2]!));
const cx = Number(process.argv[3]), cy = Number(process.argv[4]);
const px = (x: number, y: number) => { const i = (y * img.width + x) * 4; return [img.data[i]!, img.data[i + 1]!, img.data[i + 2]!]; };
console.log('size', img.width, img.height);
for (let r = 8; r <= 17; r++) {
  const samples: string[] = [];
  let red = 0, blue = 0;
  for (let a = 0; a < 360; a += 30) {
    const t = (a * Math.PI) / 180;
    const [R, G, B] = px(Math.round(cx + Math.cos(t) * r), Math.round(cy + Math.sin(t) * r));
    if (R! > 150 && G! < 110 && B! < 110) red++;
    if (B! > 150 && R! < 120) blue++;
    if (a % 90 === 0) samples.push(`${R},${G},${B}`);
  }
  console.log('r', r, 'red', red, 'blue', blue, samples.join(' | '));
}
