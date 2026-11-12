// Dev: re-run the fog tracker on a game's recorded observations (model/observations/<gameId>.json.gz)
// and report what the minimap overlay showed: detection rate per enemy, ghosts on screen, overlaps,
// sizes, ghosts next to a visible copy of the same champion, recall triggers.
// usage: replay-obs <obs.json.gz>
import { readFileSync } from 'node:fs';
import { gunzipSync } from 'node:zlib';
import { GridTerrain } from '../../src/main/tracker/terrain';
import { loadWalkable } from '../../src/main/vision/terrain';
import { EnemyTracker, type EnemyInput } from '../../src/main/tracker/tracker';
import { Learner } from '../../src/main/tracker/learning';
import { unpackMask } from '../../src/main/tracker/selfTrain';
import type { Detection } from '../../src/main/vision/detect';

type Frame = { t: number; d: Array<[string, number, number]>; m: string; e: Array<[string, string, number, number, number]>; dr: number | null; br: number | null };

const obs = JSON.parse(gunzipSync(readFileSync(process.argv[2]!)).toString()) as { team: string; enemyTeam: string; frames: Frame[] };
const terrain = new GridTerrain(128, loadWalkable(128));
const tracker = new EnemyTracker(terrain, new Learner(null), 300);
tracker.learningEnabled = false;
const seen = new Map<string, { alive: number; det: number }>();
let ghostFrames = 0, ghostSum = 0, overlapFrames = 0, bigSum = 0, nearVisible = 0, maxGhosts = 0;
const radii: number[] = [];
let prevGold = new Map<string, number>();
const goldJumps: string[] = [];
for (const f of obs.frames) {
  const enemies: EnemyInput[] = f.e.map(([champion, role, dead, ms, gold]) => ({ champion, team: obs.enemyTeam as 'CHAOS', role: role as EnemyInput['role'], isDead: !!dead, moveSpeed: ms, itemGold: gold, hasFlash: true }));
  for (const [c, , dead, , gold] of f.e) {
    const s = seen.get(c) ?? { alive: 0, det: 0 };
    if (!dead) { s.alive++; if (f.d.some(d => d[0] === c)) s.det++; }
    seen.set(c, s);
    const pg = prevGold.get(c);
    if (pg !== undefined && gold - pg >= 250 && !f.d.some(d => d[0] === c)) goldJumps.push(`${c} ${Math.floor(f.t / 60)}:${String(Math.floor(f.t % 60)).padStart(2, '0')} +${gold - pg}`);
    prevGold.set(c, gold);
  }
  const detections: Detection[] = f.d.map(([champion, x, y]) => ({ champion, team: obs.enemyTeam as 'CHAOS', x, y, score: 0.9, margin: 0.5, votes: 1 }));
  const tracks = tracker.update({ t: f.t, detections, enemies, visible: unpackMask(f.m), dragonIn: f.dr, baronIn: f.br });
  const ghosts = tracks.filter(tr => tr.state === 'fog' && tr.estimate && tr.fogSince !== null && f.t - tr.fogSince < 120)
    .map(tr => ({ champion: tr.champion, x: tr.estimate!.x, y: tr.estimate!.y, radius: tr.estimate!.radius, age: f.t - tr.fogSince! }));
  if (ghosts.length) { ghostFrames++; ghostSum += ghosts.length; maxGhosts = Math.max(maxGhosts, ghosts.length); }
  for (const g of ghosts) { radii.push(g.radius); if (g.radius > 0.2) bigSum++; if (f.d.some(d => d[0] === g.champion)) nearVisible++; }
  let ov = false;
  for (let i = 0; i < ghosts.length; i++) for (let j = i + 1; j < ghosts.length; j++) {
    const a = ghosts[i]!, b = ghosts[j]!;
    if (Math.hypot(a.x - b.x, a.y - b.y) < (Math.min(a.radius, 0.28) + Math.min(b.radius, 0.28)) * 0.6) ov = true;
  }
  if (ov) overlapFrames++;
}
const n = obs.frames.length;
console.log(`klatki: ${n}`);
for (const [c, s] of seen) console.log(`  ${c.padEnd(10)} wykryty w ${(100 * s.det / Math.max(1, s.alive)).toFixed(0)}% klatek, gdy żył`);
radii.sort((a, b) => a - b);
console.log(`duchy: średnio ${(ghostSum / Math.max(1, ghostFrames)).toFixed(1)} naraz, max ${maxGhosts}; nachodzące na siebie w ${(100 * overlapFrames / n).toFixed(0)}% klatek`);
console.log(`promień: mediana ${(radii[Math.floor(radii.length / 2)] ?? 0).toFixed(2)}, ${(100 * bigSum / Math.max(1, radii.length)).toFixed(0)}% duchów > 0.20 mapy`);
console.log(`duch obok widocznego tego samego wroga: ${nearVisible}`);
console.log(`skoki złota >= 250 we mgle (stare wykrywanie powrotu): ${goldJumps.length}`);
console.log('  ' + goldJumps.slice(0, 12).join(' | '));
