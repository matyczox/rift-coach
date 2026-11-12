// Dev: real AI build advice for the recorded practice-tool snapshot.
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { IntelTracker } from '../../src/main/intel/gameIntel';
import { loadStaticData } from '../../src/main/intel/ddragon';
import { adviseBuild, buildContext } from '../../src/main/coach/build';
import { fedChampions } from '../../src/main/intel/situational';
import { briefForGame, loadCachedBrief } from '../../src/main/coach/patchBrief';
import type { AllGameData } from '../../src/main/sources/liveApi';

async function main() {
  const root = path.join(process.env.APPDATA!, 'Rift Coach');
  const sd = await loadStaticData(path.join(root, 'ddragon'));
  const d = JSON.parse(readFileSync(path.join(process.cwd(), 'tests/fixtures/allgamedata.json'), 'utf8')) as AllGameData;
  const intel = new IntelTracker(() => sd.items, () => sd.championStats).update(d);
  const fed = fedChampions(intel).filter(p => !p.isAlly).map(p => p.champion);
  const brief = loadCachedBrief(path.join(root, 'patch'), '16.22');
  console.log('me', intel.me?.champion, intel.me?.items.map(i => i.name).join(', '), '| fed enemies:', fed.join(', ') || 'none');
  const t = Date.now();
  const b = await adviseBuild(buildContext(intel, sd.items, [], brief ? briefForGame(brief, intel.players.map(p => p.champion)) : undefined, fed), sd.items, id => `${id}`);
  console.log(`build in ${Date.now() - t} ms, phase: ${b.phase}`);
  for (const it of b.items) console.log(`  ${it.name} (${it.cost ?? '?'} g, icon id ${it.iconUrl || 'NOT FOUND'}) — ${it.why}`);
}
main().catch(e => { console.error('FAIL', String(e).slice(0, 400)); process.exit(1); });
