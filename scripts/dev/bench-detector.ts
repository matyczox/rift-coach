// Dev: detector timing full sweep vs regional sweep on harvested frames.
import { readFileSync, readdirSync } from 'node:fs';
import path from 'node:path';
import { decodePng } from '../../src/main/vision/png';
import { buildTemplate, detect, DEFAULT_PARAMS, type Template } from '../../src/main/vision/detect';
import { loadChampionIcon } from '../../src/main/vision/icons';
import { loadStaticData } from '../../src/main/intel/ddragon';

async function main() {
  const dir = process.argv[2]!;
  const root = path.join(process.env.APPDATA!, 'Rift Coach');
  const sd = await loadStaticData(path.join(root, 'ddragon'));
  const meta = JSON.parse(readFileSync(path.join(dir, 'meta.json'), 'utf8'));
  const files = readdirSync(path.join(dir, 'frames')).map(f => parseInt(f, 10)).filter(Number.isFinite).sort((a, b) => a - b).filter((_, i) => i % 40 === 0).slice(5, 105);
  const imgs = files.map(f => decodePng(readFileSync(path.join(dir, 'frames', `${f}.png`))));
  const templates: Template[] = [];
  for (const [team, l] of [['ORDER', meta.allies], ['CHAOS', meta.enemies]] as const) for (const c of l as string[]) templates.push(buildTemplate(c, team, await loadChampionIcon(path.join(root, 'ddragon', 'icons'), sd.version, sd.championIds.get(c) ?? c), imgs[0]!.width, DEFAULT_PARAMS));
  const decodeT0 = Date.now(); for (const f of files.slice(0, 20)) decodePng(readFileSync(path.join(dir, 'frames', `${f}.png`))); const decodeMs = (Date.now() - decodeT0) / 20;
  for (const mode of ['full', 'regional', 'none'] as const) {
    let ms = 0, found = 0;
    for (const img of imgs) {
      const prev = detect(img, templates, { ...DEFAULT_PARAMS, ownTeam: 'ORDER', sweep: 'none' }).detections;
      const hints = prev.map(d => ({ x: d.x, y: d.y, r: 0.08 }));
      const t0 = performance.now();
      const r = detect(img, templates, { ...DEFAULT_PARAMS, ownTeam: 'ORDER', sweep: mode === 'full' ? 'full' : mode === 'none' ? 'none' : hints }).detections;
      ms += performance.now() - t0; found += r.length;
    }
    console.log(`${mode.padEnd(8)} ${(ms / imgs.length).toFixed(1)} ms/frame, ${(found / imgs.length).toFixed(2)} detections/frame`);
  }
  console.log(`PNG decode ${decodeMs.toFixed(1)} ms/frame`);
}
main().catch(e => { console.error(e); process.exit(1); });
