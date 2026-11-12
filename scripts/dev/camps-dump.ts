// Dev: dump per-frame per-camp state distances for all three views of a harvest session to JSON.
// usage: camps-dump <sessionDir> <out.json> [every=1]
import { readFileSync, readdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { decodePng } from '../../src/main/vision/png';
import { CAMPS, campDistances } from '../../src/main/vision/camps';

const [dir, out, everyArg] = process.argv.slice(2) as [string, string, string?];
const every = Number(everyArg ?? 1);
const res: Record<string, Array<[number, number[][]]>> = {};
for (const sub of ['frames', 'red', 'truth']) {
  const files = readdirSync(path.join(dir, sub)).map(f => parseInt(f)).sort((a, b) => a - b);
  const rows: Array<[number, number[][]]> = [];
  for (let i = 0; i < files.length; i += every) {
    const img = decodePng(readFileSync(path.join(dir, sub, `${files[i]}.png`)));
    rows.push([files[i]!, CAMPS.map((_, k) => campDistances(img, k).map(v => Math.round(v * 10) / 10))]);
  }
  res[sub] = rows;
  console.error(sub, rows.length);
}
writeFileSync(out, JSON.stringify(res));
