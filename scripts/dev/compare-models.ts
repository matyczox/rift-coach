// Same game situation -> several coach models. Prints latency and answers side by side.
// usage: compare-models [model ...]
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { IntelTracker } from '../../src/main/intel/gameIntel';
import { loadStaticData } from '../../src/main/intel/ddragon';
import { SCHEMA, buildSnapshot, gameSystemPrompt } from '../../src/main/coach/coach';
import { callModel } from '../../src/main/coach/llm';
import { briefForGame, loadCachedBrief } from '../../src/main/coach/patchBrief';
import type { AllGameData } from '../../src/main/sources/liveApi';

const MODELS = process.argv.slice(2).length ? process.argv.slice(2) : ['claude-haiku-5-5', 'claude-sonnet-5-5', 'claude-opus-5-5', 'gpt-6.1-sol', 'gpt-6-astra'];

async function main() {
  const root = path.join(process.env.APPDATA!, 'Rift Coach');
  const sd = await loadStaticData(path.join(root, 'ddragon'));
  const d = JSON.parse(readFileSync(path.join(process.cwd(), 'tests/fixtures/allgamedata.json'), 'utf8')) as AllGameData;
  const brief = loadCachedBrief(path.join(root, 'patch'), '16.22');
  const results: string[] = [];
  for (const dead of [false, true]) {
    const me = d.allPlayers.find(p => p.championName === 'Ambessa')!;
    me.isDead = dead; me.respawnTimer = dead ? 31 : 0;
    const intel = new IntelTracker(() => sd.items, () => sd.championStats).update(d);
    const ctx = { intel, ghosts: [], alerts: [], profiles: [], patchBrief: brief ? briefForGame(brief, intel.players.map(p => p.champion)) : undefined };
    const prompt = `Powód zapytania: ${dead ? 'death (gracz właśnie zginął — wypełnij deathReview)' : 'periodic'}\nStan meczu:\n${JSON.stringify(buildSnapshot(ctx, []))}`;
    console.log(`\n===== ${dead ? 'PO ŚMIERCI' : 'W GRZE'} =====`);
    for (const model of MODELS) {
      try {
        const r = await callModel<{ call: string; tone: string; item: string | null; ward: { spot: string } | null; deathReview: string[] | null }>({
          model, system: gameSystemPrompt(ctx), prompt, schema: SCHEMA, timeoutMs: 120000, effort: 'low',
          thinkingTokens: /-5-5/.test(model) || model.startsWith('gpt-') ? undefined : 0,
        });
        const v = r.value;
        const line = `${model.padEnd(18)} ${(r.latencyMs / 1000).toFixed(1).padStart(5)} s | ${v.call} | ton: ${v.tone} | item: ${v.item ?? '-'} | ward: ${v.ward?.spot ?? '-'}`;
        console.log(line);
        if (v.deathReview) for (const b of v.deathReview) console.log(`${''.padEnd(28)}• ${b}`);
        results.push(JSON.stringify({ model, dead, latencyMs: r.latencyMs, ...v }));
      } catch (e) {
        console.log(`${model.padEnd(18)} BŁĄD: ${String(e instanceof Error ? e.message : e).slice(0, 160)}`);
      }
    }
  }
  writeFileSync(path.join(process.env.TEMP!, 'rc-model-compare.jsonl'), results.join('\n'));
}
main().catch(e => { console.error(e); process.exit(1); });
