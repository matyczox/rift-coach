// Detector evaluation on a two-pass replay harvest.
//   recall:   in the blue-view pass every alive ORDER champion must be found; in the red pass every alive CHAOS one.
//   accuracy: an enemy found in one pass must sit where the other pass (its own team's view) has it at the same time.
// usage: eval-detector <sessionDir> [every=10] [key=value detector params...]
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { gunzipSync, constants as zc } from 'node:zlib';
import path from 'node:path';
import { decodePng } from '../../src/main/vision/png';
import { buildTemplate, detect, DEFAULT_PARAMS, type DetectParams, type Detection, type Template } from '../../src/main/vision/detect';
import { loadChampionIcon } from '../../src/main/vision/icons';
import { loadStaticData } from '../../src/main/intel/ddragon';

type Pass = { sub: string; ownTeam: 'ORDER' | 'CHAOS' };

async function main() {
  const [dir, everyArg, ...kv] = process.argv.slice(2) as [string, string | undefined, ...string[]];
  const every = Number(everyArg ?? 10);
  const p: DetectParams = { ...DEFAULT_PARAMS };
  for (const s of kv) { const [k, v] = s.split('='); (p as unknown as Record<string, number>)[k!] = Number(v); }
  const root = path.join(process.env.APPDATA!, 'Rift Coach');
  const sd = await loadStaticData(path.join(root, 'ddragon'));
  const meta = JSON.parse(readFileSync(path.join(dir, 'meta.json'), 'utf8'));
  const api = gunzipSync(readFileSync(path.join(dir, 'api.jsonl.gz')), { finishFlush: zc.Z_SYNC_FLUSH }).toString().trim().split('\n').map(l => JSON.parse(l) as { o: number; d: { allPlayers: Array<{ championName: string; isDead: boolean; team: string }> } });
  const aliveAt = (ms: number) => {
    let best = api[0]!;
    for (const a of api) if (Math.abs(a.o - ms) < Math.abs(best.o - ms)) best = a;
    // Skip frames near a death/respawn boundary (API sampled ~0.25–2 s apart).
    return new Set(best.d.allPlayers.filter(pl => !pl.isDead).map(pl => pl.championName));
  };

  const allPasses: Pass[] = [{ sub: 'frames', ownTeam: 'ORDER' as const }, { sub: 'red', ownTeam: 'CHAOS' as const }];
  const passes = allPasses.filter(ps => existsSync(path.join(dir, ps.sub)));
  let templates: Template[] | null = null;
  const results: Record<string, Map<number, Detection[]>> = {};
  const t0 = Date.now();

  for (const ps of passes) {
    const allFiles = readdirSync(path.join(dir, ps.sub)).map(f => parseInt(f, 10)).filter(Number.isFinite).sort((a, b) => a - b);
    const files = allFiles.filter((_, i) => i % every === 0);
    const det = new Map<number, Detection[]>();
    let own = 0, found = 0, frames = 0, ms = 0, flicker = 0, absent = 0;
    const missBy = new Map<string, number>();
    const neighbourCache = new Map<number, Detection[]>();
    const detectAt = (f: number) => {
      const hit = neighbourCache.get(f);
      if (hit) return hit;
      const d = detect(decodePng(readFileSync(path.join(dir, ps.sub, `${f}.png`))), templates!, { ...p, ownTeam: ps.ownTeam }).detections;
      neighbourCache.set(f, d);
      return d;
    };
    for (const f of files) {
      if (f < 60000) continue; // skip loading / fountain start
      const img = decodePng(readFileSync(path.join(dir, ps.sub, `${f}.png`)));
      if (!templates) {
        templates = [];
        for (const [team, list] of [['ORDER', meta.allies], ['CHAOS', meta.enemies]] as const)
          for (const c of list as string[]) templates.push(buildTemplate(c, team, await loadChampionIcon(path.join(root, 'ddragon', 'icons'), sd.version, sd.championIds.get(c) ?? [...sd.championIds].find(([k]) => k.split(/[ &]/)[0] === c.split(' ')[0])?.[1] ?? c), img.width, p));
      }
      const s = Date.now();
      const { detections } = detect(img, templates, { ...p, ownTeam: ps.ownTeam });
      ms += Date.now() - s;
      det.set(f, detections);
      const alive = aliveAt(f);
      const ownChamps = templates.filter(t => t.team === ps.ownTeam && alive.has(t.champion)).map(t => t.champion);
      frames++;
      own += ownChamps.length;
      for (const c of ownChamps) {
        if (detections.some(d => d.champion === c)) { found++; continue; }
        // Missed. Real detector flicker only if the champion is found in BOTH neighbouring frames
        // (~0.4 s apart) away from its fountain; otherwise it's genuinely not on the map
        // (fountain corner, under the camera box, stacked/covered for a while).
        const i = allFiles.indexOf(f);
        const prev = allFiles[i - 1], next = allFiles[i + 1];
        const fountain = ps.ownTeam === 'ORDER' ? { x: 0.04, y: 0.96 } : { x: 0.96, y: 0.04 };
        const onMap = (d?: Detection) => d && Math.hypot(d.x - fountain.x, d.y - fountain.y) > 0.08;
        const a = prev !== undefined ? detectAt(prev).find(d => d.champion === c) : undefined;
        const b = next !== undefined ? detectAt(next).find(d => d.champion === c) : undefined;
        if (onMap(a) && onMap(b)) {
          flicker++;
          missBy.set(c, (missBy.get(c) ?? 0) + 1);
          if (process.env.RC_MISS && (missBy.get(c) ?? 0) <= 3) console.log('   flicker-miss', c, f);
        } else absent++;
      }
    }
    results[ps.sub] = det;
    console.log(`[${ps.sub}] frames ${frames}, raw own-team recall ${(100 * found / Math.max(1, own)).toFixed(1)}% (${found}/${own}); flicker recall ${(100 * found / Math.max(1, found + flicker)).toFixed(1)}% (${flicker} flicker misses, ${absent} not on map/covered), avg ${(ms / Math.max(1, frames)).toFixed(0)} ms/frame`);
    console.log('   misses by champion:', [...missBy.entries()].sort((a, b) => b[1] - a[1]).map(([c, n]) => `${c} ${n}`).join(', ') || 'none');
  }

  // Cross-check positions between passes (needs both).
  if (results.frames && results.red) {
    const redTimes = [...results.red.keys()];
    const nearestRed = (ms: number) => redTimes.reduce((b, t) => (Math.abs(t - ms) < Math.abs(b - ms) ? t : b));
    let checked = 0, agree = 0, wrong = 0;
    const errors: string[] = [];
    for (const [ms, dets] of results.frames) {
      const rt = nearestRed(ms);
      if (Math.abs(rt - ms) > 400) continue;
      const truth = results.red.get(rt)!;
      for (const d of dets.filter(x => x.team === 'CHAOS')) {
        const t = truth.find(x => x.champion === d.champion);
        if (!t) continue;
        checked++;
        const dist = Math.hypot(t.x - d.x, t.y - d.y);
        if (dist < 0.03) agree++; else { wrong++; if (errors.length < 12) errors.push(`${(ms / 1000).toFixed(0)}s ${d.champion} off by ${(dist * 100).toFixed(1)}% (score ${d.score.toFixed(2)})`); }
      }
    }
    console.log(`[cross] enemy detections in blue view checked against red view: ${checked}, agree ${agree}, wrong ${wrong} (${(100 * wrong / Math.max(1, checked)).toFixed(1)}%)`);
    for (const e of errors) console.log('   ', e);
  }
  console.log(`total ${(Date.now() - t0) / 1000}s`);
}
main().catch(e => { console.error(e); process.exit(1); });
