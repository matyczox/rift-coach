// Dev: evaluate readCamps + CampTracker on a two-pass harvest session.
// - runs the tracker on frames/ (blue view, fog), red/ (red view, fog) and truth/ (fog-free = reference)
// - matches clears between views, checks every clear against the hourglass that must follow it,
//   and checks who (champion detections in detections-frames.json / detections-red.json) stood next to the camp.
// usage: camps-eval <sessionDir> [every=1]
import { readFileSync, readdirSync, existsSync } from 'node:fs';
import path from 'node:path';
import { decodePng } from '../../src/main/vision/png';
import { loadReference } from '../../src/main/vision/background';
import { readVisionMask } from '../../src/main/vision/fog';
import { CAMPS, CampTracker, readCamps, type CampEvent } from '../../src/main/vision/camps';

const [dir, everyArg] = process.argv.slice(2) as [string, string?];
const every = Number(everyArg ?? 1);
const ref = loadReference(341) // run from the repo root;
const fmt = (s: number) => `${Math.floor(s / 60)}:${String(Math.round(s % 60)).padStart(2, '0')}`;
const endMs = (JSON.parse(readFileSync(path.join(dir, 'meta.json'), 'utf8')) as { durationMs: number }).durationMs - 6000;

interface Run { events: CampEvent[]; fogClears: number; respawnErr: number[]; /** death time implied by the hourglass schedule */ hg: Map<CampEvent, number> }
function run(sub: string, useFog: boolean): Run {
  const files = readdirSync(path.join(dir, sub)).map(f => parseInt(f)).sort((a, b) => a - b).filter(t => t < endMs);
  const tr = new CampTracker();
  let fogClears = 0;
  const respawnErr: number[] = [];
  const pending = new Map<string, number>();
  const open = new Map<string, { e: CampEvent; down: number | null }>();
  const hg = new Map<CampEvent, number>();
  for (let i = 0; i < files.length; i += every) {
    const t = files[i]!;
    const img = decodePng(readFileSync(path.join(dir, sub, `${t}.png`)));
    const readings = readCamps(img);
    const mask = useFog ? readVisionMask(img, ref, 64) : null;
    const vis = mask ? (x: number, y: number) => mask[Math.min(63, Math.floor(y * 64)) * 64 + Math.min(63, Math.floor(x * 64))]! : () => 1;
    const n0 = tr.events.length;
    const status = tr.update(t / 1000, readings, vis);
    for (const e of tr.events.slice(n0)) {
      const c = CAMPS.find(c => c.id === e.id)!;
      if (e.kind === 'cleared') { pending.set(e.id, e.respawnAt!); if (vis(c.x, c.y) < 0.6) fogClears++; }
      if (e.kind === 'respawned' && pending.has(e.id)) { respawnErr.push(e.at - pending.get(e.id)!); pending.delete(e.id); }
      if (e.kind === 'retracted') pending.delete(e.id);
      if (e.kind === 'cleared') open.set(e.id, { e, down: status.find(s => s.id === e.id)!.downSince });
    }
    for (const [id, o] of open) {
      const st = status.find(s => s.id === id)!;
      if (st.state !== 'down') { open.delete(id); continue; }
      if (st.downSince !== o.down && st.downSince !== null) { hg.set(o.e, st.downSince); open.delete(id); }
    }
  }
  return { events: tr.events, fogClears, respawnErr, hg };
}

type Det = { c: string; t: string; x: number; y: number; s: number };
const loadDets = (f: string): Array<[number, Det[]]> => existsSync(f) ? Object.entries(JSON.parse(readFileSync(f, 'utf8')) as Record<string, Det[]>).map(([k, v]) => [Number(k) / 1000, v] as [number, Det[]]).sort((a, b) => a[0] - b[0]) : [];
const detsBlue = loadDets(path.join(dir, 'detections-frames.json')), detsRed = loadDets(path.join(dir, 'detections-red.json'));
const meta = JSON.parse(readFileSync(path.join(dir, 'meta.json'), 'utf8')) as { allies: string[]; enemies: string[] };

/** Closest approach of each champion to (x,y) within [t0,t1]; own-team detections from each team's own view. */
function nearby(x: number, y: number, t0: number, t1: number): Array<{ c: string; d: number }> {
  const best = new Map<string, number>();
  for (const [dets, team] of [[detsBlue, 'ORDER'], [detsRed, 'CHAOS']] as const) {
    for (const [t, list] of dets) {
      if (t < t0 || t > t1) continue;
      for (const d of list) if (d.t === team) { const dist = Math.hypot(d.x - x, d.y - y); if (dist < (best.get(d.c) ?? 9)) best.set(d.c, dist); }
    }
  }
  return [...best.entries()].map(([c, d]) => ({ c, d })).sort((a, b) => a.d - b.d);
}

const views = { truth: run('truth', false), frames: run('frames', true), red: run('red', true) };
const clears = (r: Run) => r.events.filter(e => e.kind === 'cleared');
for (const [name, r] of Object.entries(views)) {
  const cl = clears(r);
  const bySide = { ORDER: 0, CHAOS: 0, neutral: 0 } as Record<string, number>;
  for (const e of cl) bySide[e.side]!++;
  const retr = r.events.filter(e => e.kind === 'retracted').length;
  const err = r.respawnErr.map(Math.abs).sort((a, b) => a - b);
  console.log(`${name.padEnd(6)} clears ${cl.length} (ORDER ${bySide.ORDER}, CHAOS ${bySide.CHAOS}, scuttle ${bySide.neutral}); cleared while camp fogged: ${r.fogClears}; retracted ${retr}; respawned ${r.events.filter(e => e.kind === 'respawned').length}; |respawn - predicted| median ${err[err.length >> 1]?.toFixed(1)} s, p90 ${err[Math.floor(err.length * 0.9)]?.toFixed(1)} s, max ${err[err.length - 1]?.toFixed(1)} s (n ${err.length})`);
}

// Match fogged-view clears against truth clears (same camp; strict |dt| <= 5 s, loose <= 20 s: the moment the icon
// vanishes is often hidden under the clearing champion's portrait, which is drawn differently with/without fog).
for (const name of ['frames', 'red'] as const) for (const win of [5, 20]) {
  const truth = clears(views.truth), pred = clears(views[name]);
  const key = (r: Run, e: CampEvent) => r.hg.get(e) ?? e.at;
  const used = new Set<CampEvent>();
  let tp = 0; const dts: number[] = [];
  for (const p of pred) {
    const m = truth.find(t => !used.has(t) && t.id === p.id && Math.abs(key(views.truth, t) - key(views[name], p)) <= win);
    if (m) { used.add(m); tp++; dts.push(key(views[name], p) - key(views.truth, m)); }
    else if (win === 20) console.log(`  ${name} extra clear ${p.id} @${fmt(p.at)}`);
  }
  if (win === 20) for (const t of truth) if (!used.has(t)) console.log(`  ${name} missed clear ${t.id} @${fmt(t.at)}`);
  console.log(`${name} (±${win}s): vs truth precision ${(tp / pred.length).toFixed(3)} recall ${(tp / truth.length).toFixed(3)} (tp ${tp}, pred ${pred.length}, truth ${truth.length}), mean dt ${(dts.reduce((a, b) => a + b, 0) / Math.max(1, dts.length)).toFixed(2)} s`);
}
// How well does the vanishing icon time the kill? Compare with the hourglass-implied death time.
for (const [name, r] of Object.entries(views)) {
  const q = (a: number[]) => { const b = a.map(Math.abs).sort((x, y) => x - y); return `median ${b[b.length >> 1]?.toFixed(1)} p90 ${b[Math.floor(b.length * 0.9)]?.toFixed(1)}`; };
  const ea: number[] = [], ee: number[] = [], em: number[] = [];
  for (const [e, d] of r.hg) { ea.push(e.at - d); ee.push(e.earliest! - d); em.push((e.at + e.earliest!) / 2 - d); }
  console.log(`${name}: clear time vs hourglass-implied death (n ${r.hg.size}/${clears(r).length}): at ${q(ea)} s | earliest ${q(ee)} s | midpoint ${q(em)} s`);
}
// False flips: a clear followed by 'respawned' far sooner than any respawn timer allows.
for (const [name, r] of Object.entries(views)) {
  let flips = 0;
  r.events.forEach((e, i) => {
    if (e.kind !== 'cleared') return;
    const back = r.events.slice(i + 1).find(x => x.id === e.id);
    if (back && back.kind === 'respawned' && back.at - e.at < 60) { flips++; console.log(`  ${name} flip ${e.id} cleared @${fmt(e.at)} back @${fmt(back.at)}`); }
  });
  console.log(`${name}: false flips (cleared, then back up < 60 s later): ${flips}`);
}

// Who cleared it: champions near the camp within [at-12, at+1] s (own-team views see their own team always).
const jg = meta.enemies.includes('Hecarim') ? 'Hecarim' : meta.enemies[1]!;
const R = 0.07;
const stat: Record<string, { n: number; hec: number; anyEnemy: number; anyAlly: number; nobody: number }> = {};
console.log(`\nclears in blue fogged view (frames/) with champions within ${R} (~${Math.round(R * 341)} px) of the camp in [at-12s, at+1s]:`);
for (const e of clears(views.frames)) {
  const c = CAMPS.find(c => c.id === e.id)!;
  const near = nearby(c.x, c.y, e.at - 12, e.at + 1).filter(n => n.d <= R);
  const s = (stat[e.side] ??= { n: 0, hec: 0, anyEnemy: 0, anyAlly: 0, nobody: 0 });
  s.n++;
  if (near.some(n => n.c === jg)) s.hec++;
  if (near.some(n => meta.enemies.includes(n.c))) s.anyEnemy++;
  if (near.some(n => meta.allies.includes(n.c))) s.anyAlly++;
  if (!near.length) s.nobody++;
  console.log(`  ${fmt(e.at).padStart(5)} ${e.id.padEnd(13)} ${near.map(n => `${n.c}(${n.d.toFixed(3)})`).join(' ') || '-'}`);
}
for (const [side, s] of Object.entries(stat)) console.log(`${side}: ${s.n} clears; ${jg} near ${s.hec}; any enemy near ${s.anyEnemy}; any ally near ${s.anyAlly}; nobody detected ${s.nobody}`);

// Chance baseline: how often is the jungler within R of a random camp of each side at a random time.
const rnd = (seed => () => (seed = (seed * 16807) % 2147483647) / 2147483647)(42);
for (const side of ['ORDER', 'CHAOS'] as const) {
  const camps = CAMPS.filter(c => c.side === side);
  let hit = 0; const N = 400;
  for (let i = 0; i < N; i++) {
    const t = 90 + rnd() * (endMs / 1000 - 100), c = camps[Math.floor(rnd() * camps.length)]!;
    if (nearby(c.x, c.y, t - 12, t + 1).some(n => n.c === jg && n.d <= R)) hit++;
  }
  console.log(`baseline: ${jg} within ${R} of a random ${side} camp in a random 13 s window: ${(hit / N * 100).toFixed(1)}%`);
}
