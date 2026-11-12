// Write the test crops in tests/fixtures/hp/ from the replay dataset.
// Crops cover the level box + bar only (the player-name label above is cut off).
// usage: node hp-fixtures.cjs
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { decodePng, type Rgba } from '../../src/main/vision/png';
import { encodePng } from '../../src/main/vision/pngEncode';
import { DATASET } from './hp-truth';

const OUT = join(process.cwd(), 'tests', 'fixtures', 'hp'); // run from the project root
// [name, frame, x, y, w, h, blank?] — x/y = top-left of the crop in the 2560x1440 frame;
// blank = crop rows [from, to) painted flat (hides the 2nd bar's player name)
const CROPS: Array<[string, string, number, number, number, number, [number, number]?]> = [
  ['red-l6-75', '300-Renekton.png', 2318, 121, 180, 40],     // Yone L6, panel 75%
  ['blue-l5-98', '300-Renekton.png', 1324, 1097, 180, 40],   // Rumble L5, panel 98%
  ['red-l15-shield', '1581-Ashe.png', 403, 728, 180, 40],     // Malphite L15, 90% + shield (full HP)
  ['red-l18-pair', '2127-ChoGath.png', 2208, 461, 190, 105, [33, 62]], // Hecarim L18 97% above Yone L18 47%
  ['red-l9-low', '531-Ashe.png', 1301, 1273, 180, 40],       // Yone L9, panel 5%
];
mkdirSync(OUT, { recursive: true });
for (const [name, file, x0, y0, w, h, blank] of CROPS) {
  const img = decodePng(readFileSync(join(DATASET, file)));
  const c: Rgba = { width: w, height: h, data: new Uint8Array(w * h * 4) };
  for (let y = 0; y < h; y++) c.data.set(img.data.subarray(((y0 + y) * img.width + x0) * 4, ((y0 + y) * img.width + x0 + w) * 4), y * w * 4);
  if (blank) for (let y = blank[0]; y < blank[1]; y++) for (let x = 0; x < w; x++) c.data.set([38, 48, 34, 255], (y * w + x) * 4);
  writeFileSync(join(OUT, `${name}.png`), encodePng(c));
  console.log('wrote', name);
}
