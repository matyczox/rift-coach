// Scores %TEMP%/rc-bench.jsonl. Automatic checks against the real game state:
//  - valid answer, latency
//  - call ≤ 70 chars (the top bar must be glanceable)
//  - no death review while alive; death count / gank claims consistent with the data
//  - item / ward ids that really exist; build items valid, not already owned
//  - research: plausible win rate, 18-value curve, tips, sources
// Prints a table per task and per config, and writes %TEMP%/rc-bench-summary.json.
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { loadStaticData } from '../../src/main/intel/ddragon';
import { WARD_SPOTS } from '../../src/shared/wards';

interface Row { model: string; effort: string; task: string; ms: number; value: any; error: string | null }

async function main() {
  const rows = readFileSync(path.join(process.env.TEMP!, 'rc-bench.jsonl'), 'utf8').trim().split('\n').filter(Boolean).map(l => JSON.parse(l) as Row);
  const truth = JSON.parse(readFileSync(path.join(process.env.TEMP!, 'rc-bench-truth.json'), 'utf8')) as Array<{ id: string; truth: { dead: boolean; deaths: number; gank: boolean | null; champs: string[] }; myItems: string[] }>;
  const sd = await loadStaticData(path.join(process.env.APPDATA!, 'Rift Coach', 'ddragon'));
  const itemNames = [...sd.items.values()].filter(i => i.completed || i.total >= 300).flatMap(i => [i.name, i.nameEn].filter(Boolean).map(n => n!.toLowerCase()));
  const validItem = (s: string | null) => !s || itemNames.some(n => s.toLowerCase() === n || s.toLowerCase().includes(n));

  const score = (r: Row): { s: number; notes: string[] } => {
    const notes: string[] = [];
    if (r.error || !r.value) return { s: 0, notes: [`błąd: ${r.error?.slice(0, 60)}`] };
    let s = 10;
    const v = r.value;
    if (r.task === 'live' || r.task === 'death' || r.task === 'late') {
      const tr = truth.find(t => t.id === r.task)!.truth;
      const text = [v.call, ...(v.deathReview ?? [])].join(' ');
      if (!v.call || v.call.length > 70) { s -= 2; notes.push('za długi call'); }
      if (v.deathReview && !tr.dead) { s -= 3; notes.push('analiza śmierci, choć żyje'); }
      if (!v.deathReview && tr.dead) { s -= 2; notes.push('brak analizy po śmierci'); }
      const m = /(\d+)\.?\s*(zgon|śmier)/i.exec(text) ?? /(zgon|śmier)\w*[^0-9]{0,12}(\d+)/i.exec(text);
      const n = m ? Number(m[1] && /^\d+$/.test(m[1]) ? m[1] : m[2]) : null;
      if (n !== null && n !== tr.deaths && n < 30) { s -= 3; notes.push(`zła liczba śmierci (${n} vs ${tr.deaths})`); }
      if (tr.gank === false && /\bgank\w*\b/i.test(text) && !/(nie|bez|brak)\W+(\w+\W+){0,2}gank/i.test(text)) { s -= 2; notes.push('twierdzi gank'); }
      if (!validItem(v.item)) { s -= 1; notes.push('zły przedmiot'); }
      if (v.ward && !WARD_SPOTS[v.ward.spot]) { s -= 1; notes.push('zły ward'); }
    } else if (r.task === 'build') {
      const items = (v.items ?? []) as Array<{ name: string; cost: number | null }>;
      const own = (truth.find(t => t.id === 'late')!.myItems ?? []).map(x => x.toLowerCase());
      if (items.length < 2) { s -= 3; notes.push('za mało przedmiotów'); }
      const bad = items.filter(i => i.cost === null).length;
      if (bad) { s -= 2 * bad; notes.push(`${bad} nieznanych`); }
      const dup = items.filter(i => own.includes(i.name.toLowerCase())).length;
      if (dup) { s -= 2 * dup; notes.push(`${dup} już posiadane`); }
    } else if (r.task === 'research') {
      if (v.winRate === null || v.winRate === undefined) { s -= 2; notes.push('brak winrate'); }
      else if (v.winRate < 0.42 || v.winRate > 0.6) { s -= 3; notes.push(`dziwny winrate ${v.winRate}`); }
      if (!Array.isArray(v.curve) || v.curve.length !== 18) { s -= 3; notes.push('zła krzywa'); }
      if (!v.tips?.length) { s -= 2; notes.push('brak wskazówek'); }
      if (!(v.mySpikes?.length >= 2)) { s -= 1; notes.push('mało spike’ów'); }
    }
    return { s: Math.max(0, s), notes };
  };

  const configs = [...new Set(rows.map(r => `${r.model}|${r.effort}`))];
  const tasks = ['live', 'death', 'late', 'build', 'research'];
  const summary: Array<{ model: string; effort: string; avg: number; avgMs: number; perTask: Record<string, { s: number; ms: number; notes: string[] }> }> = [];
  for (const c of configs) {
    const [model, effort] = c.split('|') as [string, string];
    const per: Record<string, { s: number; ms: number; notes: string[] }> = {};
    for (const t of tasks) {
      const r = rows.find(x => x.model === model && x.effort === effort && x.task === t);
      if (!r) continue;
      const sc = score(r);
      per[t] = { s: sc.s, ms: r.ms, notes: sc.notes };
    }
    const vals = Object.values(per);
    summary.push({ model, effort, avg: vals.reduce((a, b) => a + b.s, 0) / Math.max(1, vals.length), avgMs: vals.reduce((a, b) => a + b.ms, 0) / Math.max(1, vals.length), perTask: per });
  }
  summary.sort((a, b) => b.avg - a.avg || a.avgMs - b.avgMs);
  console.log('model              effort   avg  | live      death     late      build     research   (score/10, seconds)');
  for (const s of summary) {
    const cell = (t: string) => s.perTask[t] ? `${s.perTask[t]!.s.toString().padStart(2)}/${(s.perTask[t]!.ms / 1000).toFixed(0).padStart(3)}s` : '   -   ';
    console.log(`${s.model.padEnd(18)} ${s.effort.padEnd(7)} ${s.avg.toFixed(1).padStart(4)} | ${tasks.map(cell).join('  ')}`);
  }
  console.log('\nproblemy:');
  for (const s of summary) for (const [t, v] of Object.entries(s.perTask)) if (v.notes.length) console.log(`  ${s.model} ${s.effort} ${t}: ${v.notes.join(', ')}`);
  writeFileSync(path.join(process.env.TEMP!, 'rc-bench-summary.json'), JSON.stringify(summary, null, 1));
}
main().catch(e => { console.error(e); process.exit(1); });
