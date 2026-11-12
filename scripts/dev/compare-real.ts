// Compare coach models on REAL match situations (harvested replay of a ranked/normal game).
// Perspective: the user's champion in that game. Ghosts come from the tracker run on the
// cached blue-view detections up to each moment.
// usage: compare-real <sessionDir> <myChampion> [models...]
import { readFileSync, writeFileSync } from 'node:fs';
import { gunzipSync, constants as zc } from 'node:zlib';
import path from 'node:path';
import { IntelTracker } from '../../src/main/intel/gameIntel';
import { loadStaticData } from '../../src/main/intel/ddragon';
import { SCHEMA, buildSnapshot, gameSystemPrompt, type CoachContext } from '../../src/main/coach/coach';
import { callModel } from '../../src/main/coach/llm';
import { briefForGame, loadCachedBrief } from '../../src/main/coach/patchBrief';
import { loadWalkable } from '../../src/main/vision/terrain';
import { GridTerrain } from '../../src/main/tracker/terrain';
import { EnemyTracker, type EnemyInput } from '../../src/main/tracker/tracker';
import { Learner } from '../../src/main/tracker/learning';
import type { AllGameData } from '../../src/main/sources/liveApi';
import type { MinimapGhost } from '../../src/shared/intel';

const [dir, myChamp, ...rest] = process.argv.slice(2) as [string, string, ...string[]];
const MODELS = rest.length ? rest : ['gpt-6.1-sol', 'gpt-6-astra', 'gpt-6-luna', 'claude-haiku-5-5', 'claude-sonnet-5-5', 'claude-opus-5-5'];

async function main() {
  const root = path.join(process.env.APPDATA!, 'Rift Coach');
  const sd = await loadStaticData(path.join(root, 'ddragon'));
  const api = gunzipSync(readFileSync(path.join(dir, 'api.jsonl.gz')), { finishFlush: zc.Z_SYNC_FLUSH }).toString().trim().split('\n').map(l => JSON.parse(l) as { o: number; d: AllGameData });
  const blue = JSON.parse(readFileSync(path.join(dir, 'detections-frames.json'), 'utf8')) as Record<string, Array<{ c: string; t: string; x: number; y: number; s: number }>>;
  const blueTimes = Object.keys(blue).map(Number).sort((a, b) => a - b);
  const brief = loadCachedBrief(path.join(root, 'patch'), '16.22');
  const asMe = (d: AllGameData): AllGameData => {
    const me = d.allPlayers.find(p => p.championName === myChamp)!;
    return { ...d, activePlayer: { ...d.activePlayer, riotId: me.riotId, summonerName: me.summonerName, championStats: d.activePlayer?.championStats ?? {}, currentGold: d.activePlayer?.currentGold ?? 0, level: me.level } };
  };
  const sampleAt = (t: number) => api.reduce((b, a) => (Math.abs(a.o - t * 1000) < Math.abs(b.o - t * 1000) ? a : b)).d;

  // Pick situations from the real game.
  const intelT = new IntelTracker(() => sd.items, () => sd.championStats);
  const meName = sampleAt(300).allPlayers.find(p => p.championName === myChamp)! as { riotIdGameName?: string; summonerName?: string };
  const deathEv = sampleAt(2300).events.Events.find(e => e.EventName === 'ChampionKill' && String(e.VictimName) === (meName.riotIdGameName ?? meName.summonerName));
  let dragonT = 0;
  for (let t = 400; t < 1800; t += 10) {
    const i = intelT.update({ ...asMe(sampleAt(t)), gameData: { ...sampleAt(t).gameData, gameTime: t } });
    const dr = i.objectives.find(o => o.kind === 'dragon');
    if (dr && dr.spawnsAt && dr.spawnsAt - t <= 60 && dr.spawnsAt - t > 40) { dragonT = t; break; }
  }
  const situations = [
    { name: 'Laning 3:30', t: 210, reason: 'periodic' },
    { name: `Przed smokiem (${Math.floor(dragonT / 60)}:${String(dragonT % 60).padStart(2, '0')})`, t: dragonT, reason: 'objective-soon' },
    ...(deathEv ? [{ name: `Twoja śmierć (${Math.floor(deathEv.EventTime / 60)}:${String(Math.floor(deathEv.EventTime % 60)).padStart(2, '0')})`, t: Math.round(deathEv.EventTime + 2), reason: 'death' }] : []),
    { name: 'Późna gra 30:00', t: 1800, reason: 'periodic' },
  ];

  const terrain = new GridTerrain(128, loadWalkable(128));
  const out: unknown[] = [];
  for (const s of situations) {
    // Tracker up to this moment for fog ghosts.
    const tracker = new EnemyTracker(terrain, new Learner(null), 200);
    const it = new IntelTracker(() => sd.items, () => sd.championStats);
    let intel = it.update({ ...asMe(sampleAt(30)), gameData: { ...sampleAt(30).gameData, gameTime: 30 } });
    for (const ms of blueTimes) {
      if (ms > s.t * 1000) break;
      if (ms % 1000 > 500) continue; // ~2 updates/s is enough here
      const d = sampleAt(ms / 1000);
      intel = it.update({ ...asMe(d), gameData: { ...d.gameData, gameTime: ms / 1000 } });
      const enemies: EnemyInput[] = intel.players.filter(p => !p.isAlly).map(p => ({ champion: p.champion, team: p.team, role: p.role, isDead: p.isDead, moveSpeed: p.moveSpeed, itemGold: p.itemGold }));
      tracker.update({ t: ms / 1000, detections: blue[ms]!.filter(x => x.t !== intel.me!.team).map(x => ({ champion: x.c, team: x.t as 'ORDER' | 'CHAOS', x: x.x, y: x.y, score: x.s, margin: 0, votes: 0 })), enemies, dragonIn: null, baronIn: null });
    }
    const ghosts: MinimapGhost[] = tracker.all.filter(tr => tr.state === 'fog' && tr.estimate && tr.fogSince !== null).map(tr => ({ champion: tr.champion, x: tr.estimate!.x, y: tr.estimate!.y, radius: tr.estimate!.radius, age: s.t - tr.fogSince!, confidence: tr.estimate!.confidence, iconUrl: '' }));
    const ctx: CoachContext = { intel, ghosts, alerts: [], profiles: [], patchBrief: brief ? briefForGame(brief, intel.players.map(p => p.champion)) : undefined };
    const prompt = `Powód zapytania: ${s.reason}${s.reason === 'death' ? ' (gracz właśnie zginął — wypełnij deathReview)' : ''}\nStan meczu:\n${JSON.stringify(buildSnapshot(ctx, []))}`;
    console.log(`\n===== ${s.name} — ${intel.me?.champion} lvl ${intel.me?.level}, ${intel.me?.kills}/${intel.me?.deaths}/${intel.me?.assists}, złoto drużyn ${intel.goldDiff >= 0 ? '+' : ''}${Math.round(intel.goldDiff)}, we mgle: ${ghosts.map(g => g.champion).join(', ') || '-'} =====`);
    const results = await Promise.all(MODELS.map(async model => {
      try {
        const r = await callModel<{ call: string; tone: string; item: string | null; ward: { spot: string } | null; mapHint: { target: string } | null; deathReview: string[] | null }>({
          model, system: gameSystemPrompt(ctx), prompt, schema: SCHEMA, timeoutMs: 150000, effort: (process.env.RC_EFFORT as 'low' | 'medium' | 'high' | undefined) ?? 'low',
          thinkingTokens: /-5-5/.test(model) || model.startsWith('gpt-') ? undefined : 0,
        });
        return { model, ms: r.latencyMs, v: r.value };
      } catch (e) { return { model, ms: 0, err: String(e instanceof Error ? e.message : e).slice(0, 140) }; }
    }));
    for (const r of results) {
      if ('err' in r) { console.log(`${r.model.padEnd(18)} BŁĄD ${r.err}`); continue; }
      const v = r.v!;
      console.log(`${r.model.padEnd(18)} ${(r.ms / 1000).toFixed(1).padStart(5)} s | ${v.call} | ${v.tone} | cel: ${v.mapHint?.target ?? '-'} | item: ${v.item ?? '-'} | ward: ${v.ward?.spot ?? '-'}`);
      if (v.deathReview) for (const b of v.deathReview) console.log(`${''.padEnd(26)}• ${b}`);
      out.push({ situation: s.name, ...r });
    }
  }
  writeFileSync(path.join(process.env.TEMP!, 'rc-compare-real.json'), JSON.stringify(out, null, 1));
}
main().catch(e => { console.error(e); process.exit(1); });
