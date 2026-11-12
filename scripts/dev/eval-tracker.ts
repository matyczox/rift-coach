// End-to-end evaluation of fog predictions on a two-pass replay harvest.
// Input = what a BLUE player sees (blue-view detections + fog mask from blue frames).
// Truth = red-team view detections (all red champions always drawn there).
// For every red champion in fog: error of the ghost centre vs truth, whether truth is inside
// the ghost radius, compared with the naive baseline "still where last seen".
// usage: eval-tracker <sessionDir> [noFog]
import { readFileSync } from 'node:fs';
import { gunzipSync, constants as zc } from 'node:zlib';
import path from 'node:path';
import { decodePng } from '../../src/main/vision/png';
import { loadReference } from '../../src/main/vision/background';
import { readVisionMask } from '../../src/main/vision/fog';
import { CAMPS, CampTracker, readCamps } from '../../src/main/vision/camps';
import { loadWalkable } from '../../src/main/vision/terrain';
import { GridTerrain } from '../../src/main/tracker/terrain';
import { EnemyTracker, type EnemyInput } from '../../src/main/tracker/tracker';
import { Learner } from '../../src/main/tracker/learning';
import type { Detection } from '../../src/main/vision/detect';
import { IntelTracker } from '../../src/main/intel/gameIntel';
import { loadStaticData } from '../../src/main/intel/ddragon';
import type { AllGameData } from '../../src/main/sources/liveApi';

type Cached = Record<string, Array<{ c: string; t: string; x: number; y: number; s: number }>>;

function mergeTruth(a: Cached, b: Cached): Cached {
  const bt = Object.keys(b).map(Number).sort((x, y) => x - y);
  const out: Cached = {};
  for (const [k, da] of Object.entries(a)) {
    const t = Number(k);
    const nb = bt.reduce((best, x) => Math.abs(x - t) < Math.abs(best - t) ? x : best, bt[0]!);
    const db = Math.abs(nb - t) <= 400 ? b[nb] ?? [] : [];
    const champs = new Set([...da, ...db].filter(d => d.t === 'CHAOS').map(d => d.c));
    out[k] = [];
    for (const c of champs) {
      const x = da.find(d => d.c === c), y = db.find(d => d.c === c);
      if (x && y) { if (Math.hypot(x.x - y.x, x.y - y.y) <= 0.04) out[k]!.push({ ...x, x: (x.x + y.x) / 2, y: (x.y + y.y) / 2 }); }
      else { const z = (x ?? y)!; if (z.s >= 0.75) out[k]!.push(z); }
    }
  }
  // Physics filter: a champion can't jump across the map in a second (minion waves next to a tower can
  // look like a portrait while the real one hides under a teammate). Allow jumps into the CHAOS fountain (recall).
  const last = new Map<string, { t: number; x: number; y: number }>();
  for (const k of Object.keys(out).map(Number).sort((p, q) => p - q)) {
    out[k] = out[k]!.filter(d => {
      const prev = last.get(d.c);
      const fountain = d.x > 0.86 && d.y < 0.14;
      if (prev && !fountain) {
        const dt = (k - prev.t) / 1000;
        if (dt < 15 && Math.hypot(d.x - prev.x, d.y - prev.y) > 0.05 + 0.08 * dt) return false;
      }
      last.set(d.c, { t: k, x: d.x, y: d.y });
      return true;
    });
  }
  return out;
}

async function main() {
  const [dir, flag] = process.argv.slice(2) as [string, string?];
  const useFog = flag !== 'noFog';
  const root = path.join(process.env.APPDATA!, 'Rift Coach');
  const sd = await loadStaticData(path.join(root, 'ddragon'));
  const blue = JSON.parse(readFileSync(path.join(dir, 'detections-frames.json'), 'utf8')) as Cached;
  // Truth: red-team view, full view, or both merged (TRUTH=both): a champion counts only where the two
  // passes agree (stacked portraits hide different champions in each view), or where one pass is confident.
  const load = (sub: string) => JSON.parse(readFileSync(path.join(dir, `detections-${sub}.json`), 'utf8')) as Cached;
  const red: Cached = process.env.TRUTH === 'both' ? mergeTruth(load('red'), load('all')) : load(process.env.TRUTH ?? 'red');
  const api = gunzipSync(readFileSync(path.join(dir, 'api.jsonl.gz')), { finishFlush: zc.Z_SYNC_FLUSH }).toString().trim().split('\n').map(l => JSON.parse(l) as { o: number; d: AllGameData });
  const redTimes = Object.keys(red).map(Number).sort((a, b) => a - b);
  const blueTimes = Object.keys(blue).map(Number).sort((a, b) => a - b);
  const nearest = (arr: number[], t: number) => { let lo = 0, hi = arr.length - 1; while (lo < hi) { const m = (lo + hi) >> 1; if (arr[m]! < t) lo = m + 1; else hi = m; } const a = arr[lo]!, b = arr[lo - 1]; return b !== undefined && Math.abs(b - t) < Math.abs(a - t) ? b : a; };

  const terrain = new GridTerrain(128, loadWalkable(128));
  // LEARNER_FILE: train a persistent model across sessions (e.g. the shipped base model).
  const learner = new Learner(process.env.LEARNER_FILE ?? null);
  const tracker = new EnemyTracker(terrain, learner, 300);
  const intelT = new IntelTracker(() => sd.items, () => sd.championStats);
  let ref: ReturnType<typeof loadReference> | null = null;

  const buckets = [[0, 5], [5, 15], [15, 30], [30, 60], [60, 120]] as const;
  const stats = buckets.map(() => ({ n: 0, errPred: 0, errBase: 0, inside: 0, better: 0, radius: 0 }));
  const Ks = [0.4, 0.6, 0.8, 1.0];
  const cal = buckets.map(() => Ks.map(() => ({ inside: 0, radius: 0 })));
  const speedOf = new Map<string, number>();
  const perChamp = new Map<string, { n: number; e: number; big: string[] }>();
  let apiIdx = 0;
  const campT = new CampTracker();
  let campImg: ReturnType<typeof decodePng> | null = null;
  const t0 = Date.now();

  for (const ms of blueTimes) {
    if (ms < 30000) continue;
    while (apiIdx < api.length - 1 && api[apiIdx + 1]!.o <= ms) apiIdx++;
    const intel = intelT.update({ ...api[apiIdx]!.d, gameData: { ...api[apiIdx]!.d.gameData, gameTime: ms / 1000 } });
    const enemies: EnemyInput[] = intel.players.filter(p => p.team === 'CHAOS').map(p => ({
      champion: p.champion, team: 'CHAOS', role: p.role, isDead: p.isDead, moveSpeed: p.moveSpeed,
      ...(process.env.NO_ITEMS ? {} : { itemGold: p.itemGold }), hasFlash: true,
    }));
    for (const e of enemies) speedOf.set(e.champion, e.moveSpeed);
    let visible: ((x: number, y: number) => number) | undefined;
    if (useFog) {
      const img = decodePng(readFileSync(path.join(dir, 'frames', `${ms}.png`)));
      campImg = img;
      ref ??= loadReference(img.width);
      const mask = readVisionMask(img, ref, 64);
      visible = (x, y) => { const gx = Math.min(63, Math.max(0, Math.floor(x * 64))), gy = Math.min(63, Math.max(0, Math.floor(y * 64))); return mask[gy * 64 + gx]!; };
    }
    const dets: Detection[] = blue[ms]!.filter(d => d.t === 'CHAOS').map(d => ({ champion: d.c, team: 'CHAOS', x: d.x, y: d.y, score: d.s, margin: 0, votes: 0 }));
    if (process.env.CAMPS && campImg) {
      const n0 = campT.events.length;
      campT.update(ms / 1000, readCamps(campImg), visible);
      const jg = intel.players.find(p => p.team === 'CHAOS' && p.role === 'JUNGLE');
      for (const ev of campT.events.slice(n0)) {
        const camp = CAMPS.find(c => c.id === ev.id);
        if (ev.kind === 'cleared' && camp && camp.side === 'CHAOS' && jg && (ev.earliest === undefined || ev.at - ev.earliest <= 10) && !dets.some(d => d.champion === jg.champion)) dets.push({ champion: jg.champion, team: 'CHAOS', x: camp.x, y: camp.y, score: 0.7, margin: 0, votes: 0 });
      }
    }
    const dragon = intel.objectives.find(o => o.kind === 'dragon'), baron = intel.objectives.find(o => o.kind === 'baron');
    const tracks = tracker.update({
      t: ms / 1000, detections: dets, visible, enemies,
      dragonIn: dragon ? (dragon.alive ? 0 : dragon.spawnsAt! - ms / 1000) : null,
      baronIn: baron ? (baron.alive ? 0 : baron.spawnsAt! - ms / 1000) : null,
    });

    if (process.env.RC_FLASH) for (const tr of tracks) if (tr.flashSuspectAt === ms / 1000) console.log('flash?', tr.champion, (ms / 1000).toFixed(1) + 's');
    const rt = nearest(redTimes, ms);
    if (process.env.RC_RECALL) for (const tr of tracks) if (tr.recalledAt === ms / 1000) { const tt = red[rt]!.find(d => d.c === tr.champion); console.log('recall', tr.champion, (ms / 1000).toFixed(0) + 's', 'truth', tt ? '(' + tt.x.toFixed(2) + ',' + tt.y.toFixed(2) + ')' : 'not visible', 'gold', intel.players.find(p => p.champion === tr.champion)?.itemGold); }
    if (Math.abs(rt - ms) > 300) continue;
    for (const tr of tracks) {
      if (tr.state !== 'fog' || !tr.estimate || !tr.last || tr.fogSince === null) continue;
      const truth = red[rt]!.find(d => d.c === tr.champion && d.t === 'CHAOS');
      if (!truth) continue; // in their fountain / covered in the red view too
      const age = ms / 1000 - tr.fogSince;
      const b = buckets.findIndex(([lo, hi]) => age >= lo && age < hi);
      if (b < 0) continue;
      const s = stats[b]!;
      const ep = Math.hypot(truth.x - tr.estimate.x, truth.y - tr.estimate.y), eb = Math.hypot(truth.x - tr.last.x, truth.y - tr.last.y);
      s.n++; s.errPred += ep; s.errBase += eb; s.radius += tr.estimate.radius;
      if (ep <= tr.estimate.radius) s.inside++;
      if (ep < eb) s.better++;
      if (process.env.RC_PERCHAMP && b === 0) { const pc = perChamp.get(tr.champion) ?? { n: 0, e: 0, big: [] as string[] }; pc.n++; pc.e += eb; if (eb > 0.15 && pc.big.length < 4) pc.big.push((ms / 1000).toFixed(0) + "s last(" + tr.last.x.toFixed(2) + "," + tr.last.y.toFixed(2) + ") truth(" + truth.x.toFixed(2) + "," + truth.y.toFixed(2) + ")"); perChamp.set(tr.champion, pc); }
      Ks.forEach((k, ki) => { const r = Math.min(0.45, Math.max(tr.estimate!.radius, k * ((speedOf.get(tr.champion) ?? 345) / 14870) * age)); cal[b]![ki]!.radius += r; if (ep <= r) cal[b]![ki]!.inside++; });
    }
  }

  console.log(`fog mask: ${useFog ? 'on' : 'off'}; ${blueTimes.length} frames in ${((Date.now() - t0) / 1000).toFixed(0)} s`);
  console.log('fog age   samples  err model  err baseline  model better  truth inside circle  avg radius');
  buckets.forEach(([lo, hi], i) => {
    const s = stats[i]!;
    if (!s.n) return;
    console.log(`${String(lo).padStart(3)}-${String(hi).padEnd(4)}s ${String(s.n).padStart(7)}   ${(s.errPred / s.n * 100).toFixed(1).padStart(6)}%      ${(s.errBase / s.n * 100).toFixed(1).padStart(6)}%       ${(100 * s.better / s.n).toFixed(0).padStart(4)}%          ${(100 * s.inside / s.n).toFixed(0).padStart(4)}%          ${(s.radius / s.n * 100).toFixed(1)}%`);
  });
  if (process.env.RC_PERCHAMP) for (const [c, v] of perChamp) console.log('0-5s', c, v.n, (v.e / v.n * 100).toFixed(1) + '%', v.big.join(' | '));
  console.log('calibrated radius r = max(pf radius, K * speed * age): inside% / avg radius');
  buckets.forEach(([lo, hi], i) => { const s = stats[i]!; if (!s.n) return; console.log(`${lo}-${hi}s  ` + Ks.map((k, ki) => `K=${k}: ${(100 * cal[i]![ki]!.inside / s.n).toFixed(0)}% r=${(100 * cal[i]![ki]!.radius / s.n).toFixed(0)}%`).join('  ')); });
  console.log('learner', JSON.stringify(learner.summary()));
  learner.save();
}
main().catch(e => { console.error(e); process.exit(1); });
