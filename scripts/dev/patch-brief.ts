// Dev: build (or show cached) patch brief for the current Data Dragon patch.
import path from 'node:path';
import { loadStaticData } from '../../src/main/intel/ddragon';
import { buildPatchBrief, patchOf } from '../../src/main/coach/patchBrief';

async function main() {
  const root = path.join(process.env.APPDATA!, 'Rift Coach');
  const sd = await loadStaticData(path.join(root, 'ddragon'));
  const t = Date.now();
  const b = await buildPatchBrief({ dir: path.join(root, 'patch'), patch: patchOf(sd.version), focusChampions: ['Rumble', 'Akali', 'Ambessa'] });
  console.log('patch', b.patch, 'in', Date.now() - t, 'ms');
  console.log(JSON.stringify(b, null, 1));
}
main().catch(e => { console.error('FAIL', String(e).slice(0, 600)); process.exit(1); });
