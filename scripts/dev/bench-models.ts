// Model × reasoning-effort benchmark on REAL match data (harvested replay, me = Rumble).
// Tasks: live call (3:30), death review (1:47), late game (30:00), build (30:00), web matchup research.
// Every call gets web access (WebSearch/WebFetch for Claude Code, web_search for Codex).
// Output: %TEMP%/rc-bench.jsonl (one line per call) — scored by bench-score.ts.
// usage: bench-models <sessionDir> [concurrency=5]
import { appendFileSync, readFileSync, writeFileSync, mkdtempSync } from 'node:fs';
import { gunzipSync, constants as zc } from 'node:zlib';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { IntelTracker } from '../../src/main/intel/gameIntel';
import { loadStaticData } from '../../src/main/intel/ddragon';
import { SCHEMA, buildSnapshot, gameSystemPrompt, type CoachContext } from '../../src/main/coach/coach';
import { callModel, type Effort } from '../../src/main/coach/llm';
import { adviseBuild, buildContext } from '../../src/main/coach/build';
import { fetchMatchup } from '../../src/main/intel/matchup';
import { briefForGame, loadCachedBrief } from '../../src/main/coach/patchBrief';
import type { AllGameData } from '../../src/main/sources/liveApi';
import type { GameIntel } from '../../src/shared/intel';

const ALL: Effort[] = ['low', 'medium', 'high', 'xhigh', 'max'];
const CLAUDE_LEVELS: Effort[] = ['low', 'medium', 'high', 'xhigh']; // no "max" for Claude models (user choice)
const CONFIGS: Array<{ model: string; effort: Effort }> = [
  ...CLAUDE_LEVELS.map(effort => ({ model: 'claude-haiku-5-5', effort })),
  ...CLAUDE_LEVELS.map(effort => ({ model: 'claude-sonnet-5-5', effort })),
  ...(['low', 'medium'] as Effort[]).map(effort => ({ model: 'claude-opus-5-5', effort })),
  ...ALL.map(effort => ({ model: 'gpt-6-luna', effort })),
  ...ALL.map(effort => ({ model: 'gpt-6.1-terra', effort })),
  ...ALL.map(effort => ({ model: 'gpt-6.1-sol', effort })),
];
const WEB = ['WebSearch', 'WebFetch'];
const OUT = path.join(process.env.TEMP!, 'rc-bench.jsonl');

async function pool<T>(items: T[], n: number, fn: (t: T) => Promise<void>) {
  let i = 0;
  await Promise.all(Array.from({ length: n }, async () => { while (i < items.length) { const k = i++; await fn(items[k]!); } }));
}

async function main() {
  const [dir, concArg] = process.argv.slice(2) as [string, string?];
  const root = path.join(process.env.APPDATA!, 'Rift Coach');
  const sd = await loadStaticData(path.join(root, 'ddragon'));
  const api = gunzipSync(readFileSync(path.join(dir, 'api.jsonl.gz')), { finishFlush: zc.Z_SYNC_FLUSH }).toString().trim().split('\n').map(l => JSON.parse(l) as { o: number; d: AllGameData });
  const brief = loadCachedBrief(path.join(root, 'patch'), '16.22');
  const at = (t: number): GameIntel => {
    const s = api.reduce((b, a) => (Math.abs(a.o - t * 1000) < Math.abs(b.o - t * 1000) ? a : b)).d;
    const me = s.allPlayers.find(p => p.championName === 'Rumble')!;
    const d = { ...s, activePlayer: { ...s.activePlayer, riotId: me.riotId, summonerName: me.summonerName, championStats: s.activePlayer?.championStats ?? {}, currentGold: s.activePlayer?.currentGold ?? 0, level: me.level } };
    return new IntelTracker(() => sd.items, () => sd.championStats).update({ ...d, gameData: { ...d.gameData, gameTime: t } });
  };
  const situations = [
    { id: 'live', t: 210, reason: 'periodic' },
    { id: 'death', t: 109, reason: 'death' },
    { id: 'late', t: 1800, reason: 'periodic' },
  ].map(s => {
    const intel = at(s.t);
    const ctx: CoachContext = { intel, ghosts: [], alerts: [], profiles: [], patchBrief: brief ? briefForGame(brief, intel.players.map(p => p.champion)) : undefined };
    return { ...s, intel, ctx, prompt: `Powód zapytania: ${s.reason}${s.reason === 'death' ? ' (gracz właśnie zginął — wypełnij deathReview)' : ''}\nStan meczu:\n${JSON.stringify(buildSnapshot(ctx, []))}`,
      truth: { dead: Boolean(intel.me?.isDead), deaths: intel.me?.deaths ?? 0, gank: intel.myDeaths[0]?.wasGank ?? null, champs: intel.players.map(p => p.champion) } };
  });
  writeFileSync(path.join(process.env.TEMP!, 'rc-bench-truth.json'), JSON.stringify(situations.map(s => ({ id: s.id, t: s.t, truth: s.truth, myItems: s.intel.me?.items.map(i => i.name) }))));
  // Resume: keep rows already measured (without errors), only run what's missing.
  const doneKeys = new Set<string>();
  const NL = String.fromCharCode(10);
  const prev = (() => { try { return readFileSync(OUT, 'utf8').split(NL).filter(Boolean); } catch { return [] as string[]; } })();
  const kept = prev.filter(l => { const r = JSON.parse(l); return !r.error && !(String(r.model).startsWith('claude-') && r.effort === 'max'); });
  for (const l of kept) { const r = JSON.parse(l); doneKeys.add(`${r.model}|${r.effort}|${r.task}`); }
  writeFileSync(OUT, kept.length ? kept.join(NL) + NL : '');
  const lateIntel = situations.find(s => s.id === 'late')!.intel;
  const jobs = CONFIGS.flatMap(c => [...situations.map(s => ({ c, task: s.id })), { c, task: 'build' }, { c, task: 'research' }]).filter(j => !doneKeys.has(`${j.c.model}|${j.c.effort}|${j.task}`));
  console.log('to run', jobs.length);
  let done = 0;
  await pool(jobs, Number(concArg ?? 5), async ({ c, task }) => {
    const t0 = Date.now();
    let value: unknown = null, error: string | null = null;
    try {
      if (task === 'build') {
        value = await adviseBuild(buildContext(lateIntel, sd.items, [], undefined, []), sd.items, id => String(id), c.model, c.effort, WEB);
      } else if (task === 'research') {
        value = await fetchMatchup(mkdtempSync(path.join(tmpdir(), 'rc-bm-')), { patch: '16.22', me: 'Rumble', enemy: 'Malphite', role: 'TOP' }, { model: c.model, effort: c.effort });
      } else {
        const s = situations.find(x => x.id === task)!;
        value = (await callModel({ model: c.model, effort: c.effort, system: gameSystemPrompt(s.ctx), prompt: s.prompt, schema: SCHEMA, tools: WEB, timeoutMs: 240000 })).value;
      }
    } catch (e) { error = String(e instanceof Error ? e.message : e).slice(0, 200); }
    appendFileSync(OUT, JSON.stringify({ model: c.model, effort: c.effort, task, ms: Date.now() - t0, value, error }) + '\n');
    done++;
    if (done % 10 === 0) console.log(`${done}/${jobs.length}`);
  });
  console.log('done', jobs.length);
}
main().catch(e => { console.error(e); process.exit(1); });
