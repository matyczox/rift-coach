// Demo: run the full pipeline (detector + fog mask + tracker) over a window of a harvested replay
// as a BLUE player would see it, and render snapshots with fog predictions vs where enemies really were.
// usage: demo-predictions <sessionDir> <fromSec> <toSec> <outDir> [snapshots=4]
import { mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { gunzipSync, constants as zc } from 'node:zlib';
import path from 'node:path';
import { decodePng, resize, type Rgba } from '../../src/main/vision/png';
import { encodePng } from '../../src/main/vision/pngEncode';
import { buildTemplate, detect, DEFAULT_PARAMS, type Template } from '../../src/main/vision/detect';
import { loadChampionIcon } from '../../src/main/vision/icons';
import { loadReference } from '../../src/main/vision/background';
import { readVisionMask } from '../../src/main/vision/fog';
import { loadWalkable } from '../../src/main/vision/terrain';
import { GridTerrain } from '../../src/main/tracker/terrain';
import { EnemyTracker, type EnemyInput, type EnemyTrack } from '../../src/main/tracker/tracker';
import { Learner } from '../../src/main/tracker/learning';
import { IntelTracker } from '../../src/main/intel/gameIntel';
import { loadStaticData } from '../../src/main/intel/ddragon';
import type { AllGameData } from '../../src/main/sources/liveApi';

async function main() {
  const [dir, fromS, toS, outDir, snapArg] = process.argv.slice(2) as [string, string, string, string, string?];
  const from = Number(fromS) * 1000, to = Number(toS) * 1000, wantSnaps = Number(snapArg ?? 4);
  mkdirSync(outDir, { recursive: true });
  const root = path.join(process.env.APPDATA!, 'Rift Coach');
  const sd = await loadStaticData(path.join(root, 'ddragon'));
  const meta = JSON.parse(readFileSync(path.join(dir, 'meta.json'), 'utf8'));
  const api = gunzipSync(readFileSync(path.join(dir, 'api.jsonl.gz')), { finishFlush: zc.Z_SYNC_FLUSH }).toString().trim().split('\n').map(l => JSON.parse(l) as { o: number; d: AllGameData });
  const list = (sub: string) => readdirSync(path.join(dir, sub)).map(f => parseInt(f, 10)).filter(Number.isFinite).sort((a, b) => a - b);
  const blueTimes = list('frames').filter(t => t >= from && t <= to), redTimes = list('red');
  const nearestRed = (t: number) => redTimes.reduce((b, x) => (Math.abs(x - t) < Math.abs(b - t) ? x : b));

  const icons = new Map<string, Rgba>();
  let templates: Template[] = [];
  const terrain = new GridTerrain(128, loadWalkable(128));
  const tracker = new EnemyTracker(terrain, new Learner(null), 300);
  const intelT = new IntelTracker(() => sd.items, () => sd.championStats);
  let ref: Rgba | null = null, apiIdx = 0, lastSnap = -1e9, snaps = 0;

  for (const ms of blueTimes) {
    const img = decodePng(readFileSync(path.join(dir, 'frames', `${ms}.png`)));
    if (!templates.length) {
      for (const [team, l] of [['ORDER', meta.allies], ['CHAOS', meta.enemies]] as const) for (const c of l as string[]) {
        const icon = await loadChampionIcon(path.join(root, 'ddragon', 'icons'), sd.version, sd.championIds.get(c) ?? c);
        icons.set(c, icon);
        templates.push(buildTemplate(c, team, icon, img.width, DEFAULT_PARAMS));
      }
      ref = loadReference(img.width);
    }
    while (apiIdx < api.length - 1 && api[apiIdx + 1]!.o <= ms) apiIdx++;
    const intel = intelT.update({ ...api[apiIdx]!.d, gameData: { ...api[apiIdx]!.d.gameData, gameTime: ms / 1000 } });
    const dets = detect(img, templates, { ...DEFAULT_PARAMS, ownTeam: 'ORDER' }).detections;
    const mask = readVisionMask(img, ref!, 64);
    const enemies: EnemyInput[] = intel.players.filter(p => p.team === 'CHAOS').map(p => ({ champion: p.champion, team: 'CHAOS', role: p.role, isDead: p.isDead, moveSpeed: p.moveSpeed }));
    const dragon = intel.objectives.find(o => o.kind === 'dragon');
    const tracks = tracker.update({
      t: ms / 1000, detections: dets.filter(d => d.team === 'CHAOS'), enemies,
      visible: (x, y) => mask[Math.min(63, Math.floor(y * 64)) * 64 + Math.min(63, Math.floor(x * 64))]!,
      dragonIn: dragon ? (dragon.alive ? 0 : dragon.spawnsAt! - ms / 1000) : null, baronIn: null,
    });

    // Snapshot when ≥1 enemy has been in fog for 8–60 s, at most every 25 s.
    const fogged = tracks.filter(t => t.state === 'fog' && t.estimate && t.fogSince !== null && ms / 1000 - t.fogSince >= 8 && ms / 1000 - t.fogSince < 60);
    if (fogged.length && ms - lastSnap > 25000 && snaps < wantSnaps) {
      lastSnap = ms; snaps++;
      const rt = nearestRed(ms);
      const truthDets = detect(decodePng(readFileSync(path.join(dir, 'red', `${rt}.png`))), templates, { ...DEFAULT_PARAMS, ownTeam: 'CHAOS' }).detections;
      const file = path.join(outDir, `demo-${snaps}-${Math.round(ms / 1000)}s.png`);
      writeFileSync(file, encodePng(render(img, fogged, truthDets.filter(d => d.team === 'CHAOS'), icons, ms / 1000)));
      const mm = (s: number) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, '0')}`;
      console.log(`\n${path.basename(file)} — czas gry ${mm(ms / 1000)}`);
      for (const t of fogged) {
        const truth = truthDets.find(d => d.champion === t.champion);
        const err = truth ? Math.hypot(truth.x - t.estimate!.x, truth.y - t.estimate!.y) : null;
        const base = truth && t.last ? Math.hypot(truth.x - t.last.x, truth.y - t.last.y) : null;
        console.log(`  ${t.champion.padEnd(9)} we mgle ${Math.round(ms / 1000 - t.fogSince!)} s | kółko r=${(t.estimate!.radius * 100).toFixed(0)}% mapy | ${truth ? `prawda ${err! <= t.estimate!.radius ? 'W KÓŁKU' : 'poza kółkiem'}, błąd środka ${(err! * 100).toFixed(0)}% vs "stoi gdzie był" ${(base! * 100).toFixed(0)}%` : 'prawda: niewidoczny też dla czerwonych (fontanna/zasłonięty)'}`);
      }
    }
  }
}

/** Blue view, 2x: ghost areas (red fill + outline), ghost portrait faded at the centre, true position (green ring) and a line between. */
function render(img: Rgba, fog: EnemyTrack[], truth: Array<{ champion: string; x: number; y: number }>, icons: Map<string, Rgba>, t: number): Rgba {
  const S = 2, out = resize(img, img.width * S, img.height * S), W = out.width;
  const blend = (x: number, y: number, r: number, g: number, b: number, a: number) => {
    if (x < 0 || y < 0 || x >= W || y >= out.height) return;
    const i = (y * W + x) * 4;
    out.data[i] = out.data[i]! * (1 - a) + r * a; out.data[i + 1] = out.data[i + 1]! * (1 - a) + g * a; out.data[i + 2] = out.data[i + 2]! * (1 - a) + b * a;
  };
  for (const tr of fog) {
    const e = tr.estimate!, cx = e.x * W, cy = e.y * W, R = e.radius * W;
    for (let y = Math.floor(cy - R); y <= cy + R; y++) for (let x = Math.floor(cx - R); x <= cx + R; x++) {
      const d = Math.hypot(x - cx, y - cy);
      if (d <= R) blend(x, y, 230, 60, 60, d > R - 2 ? 0.85 : 0.18);
    }
    // Faded portrait in the centre.
    const icon = icons.get(tr.champion), pr = Math.round(0.035 * W);
    if (icon) {
      const small = resize(icon, pr * 2, pr * 2);
      const fade = Math.max(0.25, 0.6 - (t - (tr.fogSince ?? t)) / 120);
      for (let y = 0; y < pr * 2; y++) for (let x = 0; x < pr * 2; x++) {
        if ((x - pr) ** 2 + (y - pr) ** 2 > pr * pr) continue;
        const i = (y * pr * 2 + x) * 4;
        blend(Math.round(cx - pr + x), Math.round(cy - pr + y), small.data[i]!, small.data[i + 1]!, small.data[i + 2]!, fade);
      }
    }
    const tt = truth.find(d => d.champion === tr.champion);
    if (tt) {
      const tx = tt.x * W, ty = tt.y * W;
      for (let a = 0; a < 360; a += 2) for (const rr of [16, 17, 18]) blend(Math.round(tx + Math.cos(a * Math.PI / 180) * rr), Math.round(ty + Math.sin(a * Math.PI / 180) * rr), 60, 255, 120, 1);
      const steps = Math.ceil(Math.hypot(tx - cx, ty - cy));
      for (let k = 0; k < steps; k += 3) blend(Math.round(cx + (tx - cx) * k / steps), Math.round(cy + (ty - cy) * k / steps), 255, 255, 255, 0.8);
    }
  }
  return out;
}
main().catch(e => { console.error(e); process.exit(1); });
