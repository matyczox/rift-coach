// Dev: does matching against the in-game circle icon of the player's SKIN (CommunityDragon) beat the
// Data Dragon square? Scores each champion's template at its true minimap position (full-view pass).
// usage: skin-icons-test <sessionDir> [frames=120]
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { gunzipSync, constants as zc } from 'node:zlib';
import path from 'node:path';
import { decodePng, type Rgba } from '../../src/main/vision/png';
import { buildTemplate, matchAround, DEFAULT_PARAMS } from '../../src/main/vision/detect';
import { loadChampionIcon } from '../../src/main/vision/icons';
import { loadStaticData } from '../../src/main/intel/ddragon';

async function circle(cache: string, ddId: string, skin: number): Promise<Rgba | null> {
  const f = path.join(cache, `cd-${ddId.toLowerCase()}-${skin}.png`);
  if (!existsSync(f)) {
    const r = await fetch(`https://raw.communitydragon.org/latest/game/assets/characters/${ddId.toLowerCase()}/hud/${ddId.toLowerCase()}_circle_${skin}.png`);
    if (!r.ok) return null;
    writeFileSync(f, Buffer.from(await r.arrayBuffer()));
  }
  return decodePng(readFileSync(f));
}

(async () => {
  const [dir, nArg] = process.argv.slice(2) as [string, string?];
  const root = path.join(process.env.APPDATA!, 'Rift Coach');
  const sd = await loadStaticData(path.join(root, 'ddragon'));
  const cache = path.join(root, 'ddragon', 'icons'); mkdirSync(cache, { recursive: true });
  const api = gunzipSync(readFileSync(path.join(dir, 'api.jsonl.gz')), { finishFlush: zc.Z_SYNC_FLUSH }).toString().trim().split('\n').map(l => JSON.parse(l));
  const players = api[api.length - 1].d.allPlayers as Array<{ championName: string; rawChampionName: string; skinID: number; team: string }>;
  const truth = JSON.parse(readFileSync(path.join(dir, 'detections-all.json'), 'utf8')) as Record<string, Array<{ c: string; x: number; y: number }>>;
  const files = readdirSync(path.join(dir, 'all')).map(f => parseInt(f, 10)).filter(Number.isFinite).sort((a, b) => a - b);
  const step = Math.max(1, Math.floor(files.length / Number(nArg ?? 120)));
  const sample = files.filter((_, i) => i % step === 0);
  const W0 = decodePng(readFileSync(path.join(dir, 'all', `${sample[0]}.png`))).width;
  for (const p of players) {
    const ddId = /displayname_(.+)$/.exec(p.rawChampionName)?.[1] ?? p.championName;
    const name = sd.championIds.get(p.championName) ? p.championName : [...sd.championIds].find(([, id]) => id === ddId)?.[0] ?? p.championName;
    const sq = await loadChampionIcon(cache, sd.version, ddId);
    const ci = await circle(cache, ddId, p.skinID) ?? await circle(cache, ddId, 0);
    const tSq = buildTemplate(name, p.team as 'ORDER', sq, W0, DEFAULT_PARAMS);
    const variants: Array<[string, ReturnType<typeof buildTemplate>]> = [['ddragon', tSq]];
    if (ci) for (const z of [1.0, 1.1, 1.2]) variants.push([`circle z${z}`, buildTemplate(name, p.team as 'ORDER', ci, W0, { ...DEFAULT_PARAMS, zoom: z })]);
    const sums = variants.map(() => 0); let n = 0;
    for (const ms of sample) {
      const tr = (truth[ms] ?? []).find(d => d.c === name);
      if (!tr) continue;
      const img = decodePng(readFileSync(path.join(dir, 'all', `${ms}.png`)));
      variants.forEach(([, t], i) => { sums[i]! += matchAround(img, Math.round(tr.x * img.width), Math.round(tr.y * img.width), t, 2).score; });
      n++;
    }
    console.log(`${name.padEnd(12)} skin ${String(p.skinID).padStart(2)}  n=${String(n).padStart(3)}  ` + variants.map(([k], i) => `${k}: ${(sums[i]! / Math.max(1, n)).toFixed(3)}`).join('  '));
  }
})().catch(e => { console.error(e); process.exit(1); });
