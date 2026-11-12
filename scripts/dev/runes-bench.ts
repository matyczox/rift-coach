// Dev: rune pages for several lane matchups, per model and prompt variant (does NOT touch the client).
// Usage: node runes-bench.js [old|new] [model:effort ...]
import path from 'node:path';
import { loadStaticData } from '../../src/main/intel/ddragon';
import { adviseRunes, loadRuneTrees, runeSystemPrompt } from '../../src/main/runes/runes';
import { patchOf } from '../../src/main/coach/patchBrief';
import type { Effort } from '../../src/main/coach/llm';

const MATCHUPS = [
  { champion: 'Ambessa', role: 'TOP', opp: 'Warwick', enemies: ['Warwick', 'Lee Sin', 'Jinx'] },
  { champion: 'Rumble', role: 'MIDDLE', opp: 'Yone', enemies: ['Yone', 'Vi', 'Kai\'Sa'] },
  { champion: 'Darius', role: 'TOP', opp: 'Teemo', enemies: ['Teemo', 'Graves', 'Lux'] },
  { champion: 'Ahri', role: 'MIDDLE', opp: 'Zed', enemies: ['Zed', 'Jarvan IV', 'Ezreal'] },
  { champion: 'Ezreal', role: 'BOTTOM', opp: 'Caitlyn', enemies: ['Caitlyn', 'Lux', 'Sejuani'] },
  { champion: 'Jax', role: 'TOP', opp: 'Malphite', enemies: ['Malphite', 'Kindred', 'Orianna'] },
];

async function main() {
  const variant = process.argv[2] ?? 'old';
  const models = (process.argv.slice(3).length ? process.argv.slice(3) : ['gpt-6.1-terra:high', 'claude-opus-5-5:low']).map(x => x.split(':') as [string, Effort]);
  const root = path.join(process.env.APPDATA!, 'Rift Coach');
  const sd = await loadStaticData(path.join(root, 'ddragon'));
  const trees = await loadRuneTrees(path.join(root, 'ddragon'), sd.version);
  const name = new Map<number, string>();
  for (const t of trees) { name.set(t.id, t.name); for (const s of t.slots) for (const r of s.runes) name.set(r.id, r.name); }
  const shard: Record<number, string> = { 5008: 'Adaptive', 5005: 'AS', 5007: 'Haste', 5010: 'MS', 5001: 'HPscale', 5011: 'HP', 5013: 'Tenac' };
  const jobs = models.flatMap(([model, effort]) => MATCHUPS.map(async m => {
    const t = Date.now();
    try {
      const p = await adviseRunes({
        cacheDir: path.join(process.env.TEMP!, `rc-rb-${variant}-${model}-${effort}`), trees, patch: patchOf(sd.version),
        champion: m.champion, role: m.role, enemies: m.enemies, allies: [], opponent: { champion: m.opp, p: 1 },
        model, effort, systemPrompt: runeSystemPrompt,
      });
      return { model, effort, m, ms: Date.now() - t, line: `${name.get(p.selectedPerkIds[0]!)} | ${p.selectedPerkIds.slice(1, 4).map(i => name.get(i)).join(', ')} | ${name.get(p.subStyleId)}: ${p.selectedPerkIds.slice(4, 6).map(i => name.get(i)).join(', ')} | ${p.selectedPerkIds.slice(6).map(i => shard[i]).join('/')}`, why: p.why ?? '' };
    } catch (e) { return { model, effort, m, ms: Date.now() - t, line: 'BŁĄD ' + String(e).slice(0, 120), why: '' }; }
  }));
  const res = await Promise.all(jobs);
  for (const m of MATCHUPS) {
    console.log(`\n## ${m.champion} ${m.role} vs ${m.opp}`);
    for (const r of res.filter(x => x.m === m)) console.log(`  ${r.model}:${r.effort} (${(r.ms / 1000).toFixed(0)} s)  ${r.line}\n      → ${r.why}`);
  }
}
main().catch(e => { console.error('FAIL', e); process.exit(1); });
