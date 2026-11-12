// Dev: run the detector on one harvested frame and write an annotated, 2x-upscaled PNG.
// usage: detect-frame <sessionDir> <sub: frames|red|truth> <gameMs> <out.png> [key=value params...]
import { readFileSync, readdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { decodePng, resize, type Rgba } from '../../src/main/vision/png';
import { encodePng } from '../../src/main/vision/pngEncode';
import { buildTemplate, detect, DEFAULT_PARAMS, type DetectParams, type Template } from '../../src/main/vision/detect';
import { loadChampionIcon } from '../../src/main/vision/icons';
import { loadStaticData } from '../../src/main/intel/ddragon';

async function main() {
  const [dir, sub, ms, out, ...kv] = process.argv.slice(2) as [string, string, string, string, ...string[]];
  const p: DetectParams = { ...DEFAULT_PARAMS };
  for (const s of kv) { const [k, v] = s.split('='); (p as unknown as Record<string, number>)[k!] = Number(v); }
  const root = path.join(process.env.APPDATA!, 'Rift Coach');
  const sd = await loadStaticData(path.join(root, 'ddragon'));
  const meta = JSON.parse(readFileSync(path.join(dir, 'meta.json'), 'utf8'));
  const files = readdirSync(path.join(dir, sub)).map(f => parseInt(f, 10));
  const target = Number(ms);
  const nearest = files.reduce((b, f) => (Math.abs(f - target) < Math.abs(b - target) ? f : b));
  const img = decodePng(readFileSync(path.join(dir, sub, `${nearest}.png`)));
  const templates: Template[] = [];
  for (const [team, list] of [['ORDER', meta.allies], ['CHAOS', meta.enemies]] as const) {
    for (const c of list as string[]) {
      const icon = await loadChampionIcon(path.join(root, 'ddragon', 'icons'), sd.version, sd.championIds.get(c) ?? c);
      templates.push(buildTemplate(c, team, icon, img.width, p));
    }
  }
  const t0 = Date.now();
  const { detections, candidates } = detect(img, templates, p);
  console.log(`frame ${nearest} ms, ${img.width}px, ${Date.now() - t0} ms, candidates ${candidates.length}`);
  for (const c of candidates) console.log(`  cand ${c.team} (${c.x},${c.y}) votes ${c.votes.toFixed(2)}`);
  for (const d of detections) console.log(`  DET ${d.champion.padEnd(10)} ${d.team} (${Math.round(d.x * img.width)},${Math.round(d.y * img.width)}) score ${d.score.toFixed(3)} margin ${d.margin.toFixed(3)}`);

  // Render: 2x upscale, candidate crosses (yellow), detections circles (green ORDER / orange CHAOS) with index dots.
  const S = 2;
  const big: Rgba = resize(img, img.width * S, img.height * S);
  const set = (x: number, y: number, r: number, g: number, b: number) => {
    if (x < 0 || y < 0 || x >= big.width || y >= big.height) return;
    const i = (y * big.width + x) * 4; big.data[i] = r; big.data[i + 1] = g; big.data[i + 2] = b; big.data[i + 3] = 255;
  };
  for (const c of candidates) for (let k = -4; k <= 4; k++) { set(c.x * S + k, c.y * S, 255, 255, 0); set(c.x * S, c.y * S + k, 255, 255, 0); }
  detections.forEach((d, n) => {
    const R = p.ringR * img.width * S + 3;
    const col = d.team === 'ORDER' ? [60, 255, 120] : [255, 150, 30];
    for (let a = 0; a < 360; a += 2) { const t = (a * Math.PI) / 180; set(Math.round(d.x * big.width + Math.cos(t) * R), Math.round(d.y * big.width + Math.sin(t) * R), col[0]!, col[1]!, col[2]!); }
    for (let k = 0; k <= n; k++) for (let q = 0; q < 3; q++) for (let w = 0; w < 3; w++) set(Math.round(d.x * big.width - R) + k * 5 + q, Math.round(d.y * big.width - R) - 6 + w, 255, 255, 255);
  });
  writeFileSync(out, encodePng(big));
}
main().catch(e => { console.error(e); process.exit(1); });
