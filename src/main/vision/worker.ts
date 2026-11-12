// Vision worker thread: minimap PNG in -> detections + fog mask out.
import { parentPort } from 'node:worker_threads';
import { decodePng, type Rgba } from './png';
import { buildTemplate, detect, DEFAULT_PARAMS, type Detection, type Team, type Template } from './detect';
import { loadReference } from './background';
import { readVisionMask } from './fog';
import { readCamps, type CampReading } from './camps';

export type WorkerIn =
  | { type: 'roster'; champions: Array<{ champion: string; team: Team; icon: Rgba }>; ownTeam: Team; relativeColors: boolean }
  | { type: 'frame'; id: number; at: number; png: Uint8Array; hints: Array<{ x: number; y: number; team: Team }> };

export type WorkerOut =
  | { type: 'result'; id: number; at: number; width: number; detections: Detection[]; mask: Float32Array | null; camps: CampReading[]; ms: number }
  | { type: 'error'; id: number; error: string };

let roster: Array<{ champion: string; team: Team; icon: Rgba }> = [];
let ownTeam: Team = 'ORDER';
let relativeColors = true;
let templates: Template[] = [];
let templatesFor = 0;
let reference: Rgba | null = null;
let frameNo = 0;

parentPort!.on('message', (m: WorkerIn) => {
  if (m.type === 'roster') { roster = m.champions; ownTeam = m.ownTeam; relativeColors = m.relativeColors; templatesFor = 0; return; }
  try {
    const t0 = performance.now();
    const img = decodePng(m.png);
    if (templatesFor !== img.width && roster.length) {
      templates = roster.map(r => buildTemplate(r.champion, r.team, r.icon, img.width, DEFAULT_PARAMS));
      templatesFor = img.width;
    }
    if (!reference || reference.width !== img.width) {
      try { reference = loadReference(img.width); } catch { reference = null; }
    }
    // Relative team colours (game setting): enemies are red whatever side you are on; otherwise red = CHAOS.
    const redTeam: Team = relativeColors ? (ownTeam === 'ORDER' ? 'CHAOS' : 'ORDER') : 'CHAOS';
    // Full fallback sweep once a second at 5 fps; otherwise only around recent positions (3-5x faster).
    const sweep = frameNo++ % 5 === 0 ? 'full' as const : m.hints.map(h => ({ x: h.x, y: h.y, r: 0.08 }));
    const detections = templates.length ? detect(img, templates, { ...DEFAULT_PARAMS, ownTeam, redTeam, sweep }, m.hints).detections : [];
    const mask = reference ? readVisionMask(img, reference, 64) : null;
    const camps = readCamps(img);
    const out: WorkerOut = { type: 'result', id: m.id, at: m.at, width: img.width, detections, mask, camps, ms: performance.now() - t0 };
    parentPort!.postMessage(out);
  } catch (e) {
    parentPort!.postMessage({ type: 'error', id: m.id, error: String(e) } satisfies WorkerOut);
  }
});
