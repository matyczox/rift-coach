// Dev: replay a recorded LIVE game and ask the coach model exactly when the app would
// (every ~30 s, on the player's death, ~60 s before an objective), feeding back previous calls
// and dropping empty / repeated calls like Coach does. Also checks the 1v1 power bar against
// what happened in the next 45 s (kills between me and my lane opponent).
// usage: npx tsx scripts/dev/coach-sim.ts <recordingDir> [model=gpt-6.1-terra] [stepSec=30]
import { readFileSync, writeFileSync } from 'node:fs';
import { gunzipSync, constants as zc } from 'node:zlib';
import path from 'node:path';
import { IntelTracker } from '../../src/main/intel/gameIntel';
import { loadStaticData } from '../../src/main/intel/ddragon';
import { SCHEMA, buildSnapshot, gameSystemPrompt, sameObjectiveTopic, similarCall, type CoachContext } from '../../src/main/coach/coach';
import { callModel } from '../../src/main/coach/llm';
import { laneOpponent, powerScore } from '../../src/main/intel/matchup';
import type { AllGameData } from '../../src/main/sources/liveApi';
import type { CoachAdvice } from '../../src/shared/intel';

const [dir, model = 'gpt-6.1-terra', stepArg = '30'] = process.argv.slice(2) as [string, string?, string?];
const STEP = Number(stepArg);
const fmt = (t: number) => `${Math.floor(t / 60)}:${String(Math.floor(t % 60)).padStart(2, '0')}`;

async function main() {
  const root = path.join(process.env.APPDATA!, 'Rift Coach');
  const sd = await loadStaticData(path.join(root, 'ddragon'));
  const api = gunzipSync(readFileSync(path.join(dir, 'api.jsonl.gz')), { finishFlush: zc.Z_SYNC_FLUSH }).toString().trim().split('\n')
    .map(l => JSON.parse(l) as { o: number; d: AllGameData }).filter(a => a.d?.gameData?.gameTime > 0);
  const last = api[api.length - 1]!.d;
  const end = last.gameData.gameTime;
  const at = (t: number) => { let b = api[0]!; for (const a of api) { if (a.d.gameData.gameTime <= t) b = a; else break; } return b.d; };
  const events = last.events.Events;
  const it = new IntelTracker(() => sd.items, () => sd.championStats);

  const history: CoachAdvice[] = [];
  const log: unknown[] = [];
  const bar: Array<{ t: number; pMe: number; out: 'win' | 'loss' | null }> = [];
  let lastAsk = -99, wasDead = false;
  const askedObj = new Set<string>();
  let calls = 0, shown = 0, nulls = 0, dupes = 0, totalMs = 0;

  for (let t = 60; t <= end; t += 5) {
    const d = at(t);
    const intel = it.update(d);
    const me = intel.me;
    if (!me) continue;
    // Power bar vs outcome of the next 45 s.
    const opp = laneOpponent(intel);
    if (opp && !me.isDead && !opp.isDead && t % 15 === 0) {
      const myName = d.allPlayers.find(p => p.championName === me.champion)!;
      const oppName = d.allPlayers.find(p => p.championName === opp.champion)!;
      const nm = (p: typeof myName) => (p as { riotIdGameName?: string }).riotIdGameName ?? p.summonerName ?? "";
      const soon = events.filter(e => e.EventName === 'ChampionKill' && e.EventTime > t && e.EventTime <= t + 45);
      const won = soon.some(e => String(e.VictimName) === nm(oppName) && (String(e.KillerName) === nm(myName) || (e.Assisters as string[] | undefined)?.includes(nm(myName))));
      const lost = soon.some(e => String(e.VictimName) === nm(myName));
      bar.push({ t, pMe: powerScore(intel, opp, null).pMe, out: won && !lost ? 'win' : lost && !won ? 'loss' : null });
    }
    // When would the app ask?
    let reason: string | null = null;
    if (me.isDead && !wasDead) reason = 'death';
    const obj = intel.objectives.find(o => o.spawnsAt && o.spawnsAt - t <= 60 && o.spawnsAt - t > 0 && !askedObj.has(`${o.kind}@${o.spawnsAt}`));
    if (!reason && obj) { reason = 'objective-soon'; askedObj.add(`${obj.kind}@${obj.spawnsAt}`); }
    if (!reason && !me.isDead && t - lastAsk >= STEP) reason = 'periodic';
    wasDead = me.isDead;
    if (!reason || (me.isDead && reason !== 'death')) continue;
    lastAsk = t;
    const ctx: CoachContext = { intel, ghosts: [], alerts: [], profiles: [] };
    const prompt = `Powód zapytania: ${reason}${reason === 'death' ? ' (gracz właśnie zginął — wypełnij deathReview)' : ''}\nStan meczu:\n${JSON.stringify(buildSnapshot(ctx, history))}`;
    try {
      const r = await callModel<{ call: string | null; tone: string; ward: { spot: string } | null; item: string | null; deathReview: string[] | null }>({
        model, system: gameSystemPrompt(ctx), prompt, schema: SCHEMA, timeoutMs: 90000, effort: 'low',
      });
      calls++; totalMs += r.latencyMs;
      const v = r.value;
      const call = (v.call ?? '').trim();
      let verdict = 'SHOWN';
      if (reason !== 'death' && !call) { verdict = 'null'; nulls++; }
      else if (reason !== 'death' && history.slice(0, 3).some(h => similarCall(h.call ?? '', call) || (t - (h.gameTime ?? -999) < 120 && sameObjectiveTopic(h.call ?? '', call)))) { verdict = 'dupe'; dupes++; }
      else {
        shown++;
        history.unshift({ ...(v as unknown as CoachAdvice), call, gameTime: t } as CoachAdvice);
        history.splice(10);
      }
      const line = `${fmt(t).padStart(5)} ${reason.padEnd(14)} ${(r.latencyMs / 1000).toFixed(1).padStart(4)}s ${verdict.padEnd(5)} | ${call || '-'}${v.ward ? ` [ward ${v.ward.spot}]` : ''}${v.item ? ` [item ${v.item}]` : ''}`;
      console.log(line);
      if (v.deathReview) for (const b of v.deathReview) console.log(`${''.padEnd(34)}• ${b}`);
      log.push({ t, reason, ms: r.latencyMs, verdict, ...v, lvl: me.level, kda: `${me.kills}/${me.deaths}/${me.assists}`, goldDiff: Math.round(intel.goldDiff) });
    } catch (e) {
      console.log(`${fmt(t).padStart(5)} ${reason.padEnd(14)} ERROR ${String(e instanceof Error ? e.message : e).slice(0, 120)}`);
    }
  }

  // Bar calibration.
  const bins = [[0, 0.4], [0.4, 0.6], [0.6, 1.01]] as const;
  console.log('\n--- Pasek 1v1 vs co stało się w 45 s (kill na rywalu / twoja śmierć) ---');
  for (const [lo, hi] of bins) {
    const s = bar.filter(b => b.pMe >= lo && b.pMe < hi);
    const w = s.filter(b => b.out === 'win').length, l = s.filter(b => b.out === 'loss').length;
    console.log(`pMe ${lo.toFixed(1)}–${Math.min(1, hi).toFixed(1)}: ${String(s.length).padStart(3)} chwil, zabiłeś rywala ${w}, zginąłeś ${l}`);
  }
  console.log(`\nWywołań ${calls}, pokazane ${shown}, null ${nulls}, powtórki odrzucone ${dupes}, średnio ${(totalMs / Math.max(1, calls) / 1000).toFixed(1)} s`);
  writeFileSync(path.join(process.env.TEMP!, 'rc-coach-sim.json'), JSON.stringify({ log, bar }, null, 1));
}
main().catch(e => { console.error(e); process.exit(1); });
