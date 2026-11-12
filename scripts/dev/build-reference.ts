// Regenerate assets/minimap-reference.png from a two-pass harvest session's fog-free truth/ frames.
// usage: build-reference <sessionDir> [samples=300] [out=assets/minimap-reference.png]
import { readFileSync, readdirSync, writeFileSync, mkdirSync } from 'node:fs';
import path from 'node:path';
import { decodePng, type Rgba } from '../../src/main/vision/png';
import { encodePng } from '../../src/main/vision/pngEncode';
import { buildReference, REFERENCE_FILE } from '../../src/main/vision/background';

const [dir, samplesArg, outArg] = process.argv.slice(2);
if (!dir) { console.error('usage: build-reference <sessionDir> [samples] [out]'); process.exit(1); }
const samples = Number(samplesArg ?? 300);
const out = outArg ?? path.join('assets', REFERENCE_FILE);
const tdir = path.join(dir, 'truth');
const names = readdirSync(tdir).filter(f => f.endsWith('.png')).sort((a, b) => parseInt(a) - parseInt(b));
// Skip the first 90 s (loading/fountain) and spread samples over the whole game.
const usable = names.filter(n => parseInt(n) > 90_000);
const step = Math.max(1, Math.floor(usable.length / samples));
const frames: Rgba[] = [];
for (let i = 0; i < usable.length && frames.length < samples; i += step) frames.push(decodePng(readFileSync(path.join(tdir, usable[i]!))));
console.log(`median over ${frames.length} frames (${frames[0]?.width}px)`);
const t0 = Date.now();
const ref = buildReference(frames);
mkdirSync(path.dirname(out), { recursive: true });
writeFileSync(out, encodePng(ref));
console.log(`wrote ${out} in ${Date.now() - t0} ms`);
