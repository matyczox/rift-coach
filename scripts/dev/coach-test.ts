// Dev check: one real coach call (live + death review) on the recorded practice-tool snapshot.
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { IntelTracker } from '../../src/main/intel/gameIntel';
import { loadStaticData } from '../../src/main/intel/ddragon';
import { Coach, buildSnapshot } from '../../src/main/coach/coach';
import type { AllGameData } from '../../src/main/sources/liveApi';
import type { CoachAdvice } from '../../src/shared/intel';
import * as claudeMod from '../../src/main/coach/claude';

async function main() {
  const sd = await loadStaticData(path.join(process.env.APPDATA!, 'Rift Coach', 'ddragon'));
  const d = JSON.parse(readFileSync(path.join(process.cwd(), 'tests/fixtures/allgamedata.json'), 'utf8')) as AllGameData;
  const tracker = new IntelTracker(() => sd.items, () => sd.championStats);
  for (const dead of [false, true]) {
    const me = d.allPlayers.find(p => p.championName === 'Ambessa')!;
    me.isDead = dead; me.respawnTimer = dead ? 31 : 0;
    const intel = tracker.update(d);
    const coach = new Coach();
    await coach.checkAuth();
    const got = new Promise<CoachAdvice>((res, rej) => { coach.on('advice', res); coach.on('status', s => s.state === 'error' && rej(new Error(s.lastError ?? 'error'))); });
    const ctx = { intel, ghosts: [], alerts: [], profiles: [] };
    if (!dead) console.log('snapshot chars:', JSON.stringify(buildSnapshot(ctx, [])).length);
    coach.tick(ctx);
    const a = await got;
    console.log(dead ? '--- PO ŚMIERCI' : '--- W GRZE', `(${coach.status.model}, ${a.latencyMs} ms)`);
    console.log(JSON.stringify({ call: a.call, tone: a.tone, mapHint: a.mapHint, item: a.item, ward: a.ward, deathReview: a.deathReview, ttlSec: a.ttlSec }, null, 1));
  }
}
main().catch(e => { console.error('FAIL', e); process.exit(1); });
