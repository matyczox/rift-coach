// Evaluate readHealthBars + matchBarsToChampions on the replay dataset.
// Replay colours are absolute: ORDER bars blue, CHAOS bars red.
// Truth: side-panel HP bars (and the selected champion's stats-panel HP bar).
// Identity truth for grading assignment: HP oracle (see hp-truth.ts).
// usage: node hp-eval.cjs [--verbose] [--scale 0.75]
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { decodePng, resize } from '../../src/main/vision/png';
import { readHealthBars, matchBarsToChampions, type HealthBar } from '../../src/main/vision/healthbars';
import { DATASET, loadIndex, panelTruth, selectedTruth, teams, hpOracle } from './hp-truth';

const verbose = process.argv.includes('--verbose');
// --scale 0.75 → bilinear-downscale frames (2560x1440 → 1920x1080) before reading bars; truth stays full-res
const si = process.argv.indexOf('--scale');
const scale = si > 0 ? Number(process.argv[si + 1]) : 1;
const index = loadIndex();
let frames = 0, barsTotal = 0, framesWithBars = 0, ms = 0, msMax = 0;
let assigned = 0, unassigned = 0, graded = 0, wrong = 0, noTruth = 0;
let lvlRead = 0, lvlNull = 0, lvlWrong = 0;
const errs: number[] = [], oracleErrs: number[] = [], selErrs: number[] = [], panelVsSel: number[] = [];
let suspicious = 0, compressed = 0, tied = 0;
const perFrame: number[] = [];

for (const info of index) {
  const img = decodePng(readFileSync(join(DATASET, info.file)));
  frames++;
  const truth = panelTruth(img, info);
  const sel = selectedTruth(img);
  if (sel != null && truth.has(info.selected)) panelVsSel.push(Math.abs(sel - truth.get(info.selected)!));
  const t = teams(info);
  const frame = scale === 1 ? img : resize(img, Math.round(img.width * scale), Math.round(img.height * scale));
  // warm-up + timing of both colour passes (what main would do: one pass per colour of interest)
  const t0 = performance.now();
  const blue = readHealthBars(frame, { enemyColor: 'blue' });
  const red = readHealthBars(frame, { enemyColor: 'red' });
  const dt = performance.now() - t0;
  if (frames > 3) { ms += dt; msMax = Math.max(msMax, dt); }
  const n = blue.length + red.length;
  barsTotal += n; perFrame.push(n); if (n) framesWithBars++;

  for (const [bars, team] of [[blue, t.ORDER], [red, t.CHAOS]] as Array<[HealthBar[], string[]]>) {
    const cands = team.filter((c) => !info.dead.includes(c)).map((c) => ({ champion: c, level: info.levels[c]! }));
    const res = matchBarsToChampions(bars, cands);
    for (const { bar, champion } of res) {
      // shield-compressed bars only give a lower bound → no oracle identity
      const oracle = bar.shieldCompressed ? null : hpOracle(bar.pct, team, truth);
      const err = (tv: number) => bar.shieldCompressed ? Math.max(0, bar.pct - tv) : Math.abs(bar.pct - tv);
      if (bar.shieldCompressed) compressed++;
      const near = Math.min(...team.filter((c) => truth.has(c)).map((c) => Math.abs(truth.get(c)! - bar.pct)));
      if (!(near <= 0.15)) suspicious++;
      if (oracle) {
        if (bar.level == null) lvlNull++; else { lvlRead++; if (bar.level !== info.levels[oracle]) { lvlWrong++; if (verbose) console.log(`LEVEL ${info.file} bar L${bar.level} oracle ${oracle} index L${info.levels[oracle]}`); } }
        oracleErrs.push(Math.abs(bar.pct - truth.get(oracle)!));
      }
      let line = `${info.file} ${bar === undefined ? '' : ''}(${bar.x},${bar.y}) ${(bar.pct * 100).toFixed(1)}% L${bar.level} -> ${champion ?? '-'}`;
      if (!champion) { unassigned++; if (bar.level != null && cands.filter((c) => c.level === bar.level).length > 1) tied++; if (verbose) console.log(line, 'oracle', oracle); continue; }
      assigned++;
      const tv = truth.get(champion);
      if (tv == null) noTruth++; else errs.push(err(tv));
      line += ` truth ${tv == null ? '?' : (tv * 100).toFixed(1)} oracle ${oracle}`;
      if (oracle) { graded++; if (oracle !== champion) { wrong++; line += '  WRONG'; } }
      if (champion === info.selected && sel != null) { selErrs.push(err(sel)); line += ` sel ${(sel * 100).toFixed(1)}`; }
      if (verbose || line.includes('WRONG')) console.log(line);
    }
  }
}

const mean = (a: number[]) => a.reduce((x, y) => x + y, 0) / Math.max(1, a.length);
const within = (a: number[], t: number) => (100 * a.filter((e) => e <= t).length) / Math.max(1, a.length);
const pc = (x: number) => (x * 100).toFixed(2) + '%';
console.log('\n=== hp-eval ===');
console.log(`frames ${frames}, frames with >=1 bar ${framesWithBars}, bars ${barsTotal} (mean ${(barsTotal / frames).toFixed(2)}/frame, max ${Math.max(...perFrame)})`);
console.log(`shield-compressed bars (pct is a lower bound; error = max(0, pct - truth)): ${compressed}`);
console.log(`bars with no team champion within 15% HP (likely false positives): ${suspicious}`);
console.log(`assignment: assigned ${assigned}, unassigned ${unassigned} (${tied} because 2+ team champions share the read level and no screen positions are given); graded by HP-oracle ${graded}, wrong ${wrong} (${((100 * wrong) / Math.max(1, graded)).toFixed(1)}%)`);
console.log(`level digits (bars with oracle identity): read ${lvlRead}, wrong ${lvlWrong}, unread ${lvlNull}`);
console.log(`HP error vs panel truth of ASSIGNED champion: n=${errs.length} mean ${pc(mean(errs))}, within5 ${within(errs, 0.05).toFixed(1)}%, within10 ${within(errs, 0.1).toFixed(1)}%, max ${pc(Math.max(0, ...errs))}`);
console.log(`HP error vs oracle champion (measurement only): n=${oracleErrs.length} mean ${pc(mean(oracleErrs))}`);
console.log(`HP error vs selected-champion stats bar: n=${selErrs.length} mean ${pc(mean(selErrs))}, within5 ${within(selErrs, 0.05).toFixed(1)}%`);
console.log(`(truth sanity: side panel vs stats bar for selected champ: n=${panelVsSel.length} mean ${pc(mean(panelVsSel))})`);
console.log(`runtime (both colours, ${Math.round(2560 * scale)}x${Math.round(1440 * scale)}): mean ${(ms / Math.max(1, frames - 3)).toFixed(2)} ms, max ${msMax.toFixed(2)} ms`);
