// Dev: post-game review from the recorded practice-tool snapshot (real events, deaths, items).
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { IntelTracker } from '../../src/main/intel/gameIntel';
import { loadStaticData } from '../../src/main/intel/ddragon';
import { MatchLog, buildReview } from '../../src/main/coach/review';
import type { AllGameData } from '../../src/main/sources/liveApi';

async function main() {
  const root = path.join(process.env.APPDATA!, 'Rift Coach');
  const sd = await loadStaticData(path.join(root, 'ddragon'));
  const d = JSON.parse(readFileSync(path.join(process.cwd(), 'tests/fixtures/allgamedata.json'), 'utf8')) as AllGameData;
  const tracker = new IntelTracker(() => sd.items, () => sd.championStats);
  const log = new MatchLog();
  // Replay the event list over time so the log sees deaths/objectives as they happened.
  const all = d.events.Events;
  const end = d.gameData.gameTime;
  for (let t = 60; t <= end; t += 30) {
    const snap = { ...d, events: { Events: all.filter(e => e.EventTime <= t) }, gameData: { ...d.gameData, gameTime: t } };
    log.update(tracker.update(snap));
  }
  const t0 = Date.now();
  const r = await buildReview(log, { model: process.argv[2] ?? 'claude-opus-5-5', win: null, dir: path.join(process.env.TEMP!, 'rc-review-test') });
  if (!r) return console.log('no review');
  console.log(`review in ${Date.now() - t0} ms (${r.model}) — ${r.champion} ${r.kda}, ${r.csPerMin} CS/min, ${r.moments.length} moments`);
  console.log('\nPodsumowanie:', r.summary);
  console.log('\nDobrze:'); r.good.forEach(x => console.log('  +', x));
  console.log('\nDo poprawy:'); r.improve.forEach((x, k) => console.log(`  ${k + 1}.`, x));
}
main().catch(e => { console.error('FAIL', String(e).slice(0, 400)); process.exit(1); });
