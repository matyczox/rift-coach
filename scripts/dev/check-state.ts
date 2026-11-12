// Dev: what did the coach actually receive at given moments of the real replay (me = given champion)?
import { readFileSync } from 'node:fs';
import { gunzipSync, constants as zc } from 'node:zlib';
import path from 'node:path';
import { IntelTracker } from '../../src/main/intel/gameIntel';
import { loadStaticData } from '../../src/main/intel/ddragon';
import type { AllGameData } from '../../src/main/sources/liveApi';

async function main() {
  const [dir, champ, ...times] = process.argv.slice(2) as [string, string, ...string[]];
  const sd = await loadStaticData(path.join(process.env.APPDATA!, 'Rift Coach', 'ddragon'));
  const api = gunzipSync(readFileSync(path.join(dir, 'api.jsonl.gz')), { finishFlush: zc.Z_SYNC_FLUSH }).toString().trim().split('\n').map(l => JSON.parse(l) as { o: number; d: AllGameData });
  for (const ts of times) {
    const t = Number(ts);
    const s = api.reduce((b, a) => (Math.abs(a.o - t * 1000) < Math.abs(b.o - t * 1000) ? a : b));
    const me = s.d.allPlayers.find(p => p.championName === champ)!;
    const d = { ...s.d, activePlayer: { ...s.d.activePlayer, riotId: me.riotId, summonerName: me.summonerName } };
    const i = new IntelTracker(() => sd.items, () => sd.championStats).update({ ...d, gameData: { ...d.gameData, gameTime: t } });
    console.log(`t=${t}s (sample ${(s.o / 1000).toFixed(1)}s): dead=${i.me?.isDead} respawnIn=${i.me?.respawnIn} kda=${i.me?.kills}/${i.me?.deaths}/${i.me?.assists} myDeaths=${JSON.stringify(i.myDeaths.map(x => [Math.round(x.at), x.killer]))}`);
  }
}
main().catch(e => { console.error(e); process.exit(1); });
