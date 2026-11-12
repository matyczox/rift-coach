// Run the detector over every frame of a harvested replay and cache results to <session>/detections-<sub>.json.
// usage: detect-cache <sessionDir> [sub=frames|red] [every=1]
import { readFileSync, readdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { decodePng } from '../../src/main/vision/png';
import { buildTemplate, detect, DEFAULT_PARAMS, type Template } from '../../src/main/vision/detect';
import { loadChampionIcon } from '../../src/main/vision/icons';
import { loadStaticData } from '../../src/main/intel/ddragon';

async function main() {
  const [dir, sub = 'frames', everyArg = '1'] = process.argv.slice(2) as [string, string?, string?];
  const every = Number(everyArg);
  const ownTeam = sub === 'red' ? 'CHAOS' : 'ORDER';
  const root = path.join(process.env.APPDATA!, 'Rift Coach');
  const sd = await loadStaticData(path.join(root, 'ddragon'));
  const meta = JSON.parse(readFileSync(path.join(dir, 'meta.json'), 'utf8'));
  const files = readdirSync(path.join(dir, sub)).map(f => parseInt(f, 10)).filter(Number.isFinite).sort((a, b) => a - b).filter((_, i) => i % every === 0);
  let templates: Template[] | null = null;
  const out: Record<number, Array<{ c: string; t: string; x: number; y: number; s: number }>> = {};
  const t0 = Date.now();
  for (const [i, f] of files.entries()) {
    const img = decodePng(readFileSync(path.join(dir, sub, `${f}.png`)));
    if (!templates) {
      templates = [];
      for (const [team, list] of [['ORDER', meta.allies], ['CHAOS', meta.enemies]] as const)
        for (const c of list as string[]) templates.push(buildTemplate(c, team, await loadChampionIcon(path.join(root, 'ddragon', 'icons'), sd.version, sd.championIds.get(c) ?? [...sd.championIds].find(([k]) => k.split(/[ &]/)[0] === c.split(' ')[0])?.[1] ?? c), img.width, DEFAULT_PARAMS));
    }
    out[f] = detect(img, templates, { ...DEFAULT_PARAMS, ownTeam }).detections.map(d => ({ c: d.champion, t: d.team, x: +d.x.toFixed(4), y: +d.y.toFixed(4), s: +d.score.toFixed(3) }));
    if (i % 500 === 0) console.log(`${sub}: ${i}/${files.length} (${((Date.now() - t0) / 1000).toFixed(0)} s)`);
  }
  writeFileSync(path.join(dir, `detections-${sub}.json`), JSON.stringify(out));
  console.log(`done ${sub}: ${files.length} frames in ${((Date.now() - t0) / 1000).toFixed(0)} s`);
}
main().catch(e => { console.error(e); process.exit(1); });
