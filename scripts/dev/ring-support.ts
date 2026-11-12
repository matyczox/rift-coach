// Dev: ring support of both team colours at given points.
import { readFileSync } from 'node:fs';
import { decodePng } from '../../src/main/vision/png';
import { ringSupport, DEFAULT_PARAMS } from '../../src/main/vision/detect';

const img = decodePng(readFileSync(process.argv[2]!));
const R = DEFAULT_PARAMS.ringR * img.width;
for (const pt of process.argv.slice(3)) {
  const [x, y] = pt.split(',').map(Number) as [number, number];
  let best = { red: 0, blue: 0 };
  for (let dy = -2; dy <= 2; dy++) for (let dx = -2; dx <= 2; dx++) {
    best.red = Math.max(best.red, ringSupport(img, x + dx, y + dy, R, 1));
    best.blue = Math.max(best.blue, ringSupport(img, x + dx, y + dy, R, 2));
  }
  console.log(pt, 'red', best.red.toFixed(2), 'blue', best.blue.toFixed(2));
}
