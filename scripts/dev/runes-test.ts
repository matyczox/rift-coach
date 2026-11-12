// Dev: rune advice for a draft situation (does NOT write to the League client).
import path from 'node:path';
import { loadStaticData } from '../../src/main/intel/ddragon';
import { adviseRunes, loadRuneTrees } from '../../src/main/runes/runes';
import { assignRoles, laneOpponentFor, roleRates } from '../../src/main/runes/roles';
import { patchOf } from '../../src/main/coach/patchBrief';

async function main() {
  const root = path.join(process.env.APPDATA!, 'Rift Coach');
  const sd = await loadStaticData(path.join(root, 'ddragon'));
  const trees = await loadRuneTrees(path.join(root, 'ddragon'), sd.version);
  const name = new Map<number, string>();
  for (const t of trees) { name.set(t.id, t.name); for (const s of t.slots) for (const r of s.runes) name.set(r.id, r.name); }
  const shards: Record<number, string> = { 5008: 'Adaptive', 5005: 'AS', 5007: 'Haste', 5010: 'MS', 5001: 'HP scaling', 5011: 'HP', 5013: 'Tenacity' };
  for (const enemies of [['Warwick', 'Jinx'], ['Warwick', 'Lee Sin', 'Jinx']]) {
    const rates = await roleRates(path.join(process.env.TEMP!, 'rc-runes-' + (process.env.RC_RESEARCH_MODEL ?? 'default')), patchOf(sd.version), enemies);
    const opp = laneOpponentFor('TOP', assignRoles(enemies, rates));
    console.log('\nprzydział ról: przeciwnik na topie', opp?.champion, Math.round((opp?.p ?? 0) * 100) + '%', '| stawki Warwick:', JSON.stringify(rates.get('Warwick')));
    const t = Date.now();
    const p = await adviseRunes({ cacheDir: path.join(process.env.TEMP!, 'rc-runes-' + (process.env.RC_RESEARCH_MODEL ?? 'default')), trees, patch: patchOf(sd.version), champion: 'Ambessa', role: 'TOP', enemies, allies: [], opponent: opp && opp.p >= 0.6 ? opp : null });
    console.log(`\nwrogowie: ${enemies.join(', ') || '(jeszcze nikt)'} → ${p.label} (${Date.now() - t} ms)`);
    console.log(`  ${name.get(p.primaryStyleId)}: ${p.selectedPerkIds.slice(0, 4).map(i => name.get(i)).join(', ')}`);
    console.log(`  ${name.get(p.subStyleId)}: ${p.selectedPerkIds.slice(4, 6).map(i => name.get(i)).join(', ')}`);
    console.log(`  odłamki: ${p.selectedPerkIds.slice(6).map(i => shards[i]).join(', ')}`);
  }
}
main().catch(e => { console.error('FAIL', String(e).slice(0, 400)); process.exit(1); });
