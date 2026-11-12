import type { Rgba } from './png';
import { resize } from './png';

/**
 * Champion portrait detector for the LoL minimap.
 *
 * 1. Candidates: circle Hough transform on team-coloured ring pixels (red ring =
 *    CHAOS, blue/teal ring = ORDER) at the known portrait radius. Works with
 *    partially covered rings (overlapping portraits) because any visible arc votes.
 * 2. Identity: masked normalized cross-correlation of the portrait interior
 *    against the Data Dragon icons of the 5 champions of that team, with a small
 *    local search around the candidate centre.
 * 3. Assignment: each champion at most once, best scores first.
 *
 * All sizes are relative to the minimap width so any resolution works.
 */

export type Team = 'ORDER' | 'CHAOS';

export interface DetectParams {
  /** Ring radius / minimap width. */
  ringR: number;
  /** Portrait interior radius used for matching / minimap width. */
  innerR: number;
  /** Zoom applied to the ddragon icon before masking (icon art is cropped on the minimap). */
  zoom: number;
  /** Min fraction of ring samples that must vote for a centre. */
  minVotes: number;
  /** Min NCC to accept an identity. */
  minScore: number;
  /** Stricter threshold for the opposing team (false enemy sightings are costly: they kill fog predictions). */
  minScoreEnemy: number;
  /** The viewing team (whose champions are always drawn); enemies use minScoreEnemy. */
  ownTeam?: Team;
  /**
   * Which team has RED rings. Replays: CHAOS (absolute colours). Live games with the
   * "relative team colours" setting: the ENEMY team is red, whatever side you are on.
   */
  redTeam?: Team;
  /**
   * Where the expensive fallback sweep runs. 'full' (default) = whole minimap; an array =
   * only around these points (normalized, radius r) — e.g. last known positions. Live mode
   * uses regions on most frames and a full sweep about once a second.
   */
  sweep?: 'full' | 'none' | Array<{ x: number; y: number; r: number }>;
  /** Fallback sweep (portrait ring hidden under another portrait) is a blind search, so it needs a
   *  stricter match than ring-found candidates: otherwise minion waves / towers become champions. */
  fallbackMinScore?: number;
  fallbackSupport?: number;
  /** Local refinement radius in pixels. */
  refine: number;
}

export const DEFAULT_PARAMS: DetectParams = { ringR: 0.042, innerR: 0.032, zoom: 1.2, minVotes: 0.28, minScore: 0.45, minScoreEnemy: 0.6, refine: 2 };

export interface Template { champion: string; team: Team; size: number; /** zero-mean, unit-norm masked RGB vector */ vec: Float32Array; mask: Uint8Array }

export interface Candidate { x: number; y: number; team: Team; votes: number }

export interface Detection {
  champion: string;
  team: Team;
  /** Normalized minimap coords 0..1. */
  x: number; y: number;
  score: number;
  /** Margin over the second-best champion of the same team. */
  margin: number;
  votes: number;
}

const RED = 1, BLUE = 2;

export function ringClass(r: number, g: number, b: number): number {
  if (r > 140 && r > g * 1.5 && r > b * 1.4) return RED;
  if (b > 110 && b > r * 1.3 && g > r * 1.05) return BLUE;
  return 0;
}

/** Circle Hough on ring-coloured pixels; returns local maxima per team. */
export function findCandidates(img: Rgba, p: DetectParams): Candidate[] {
  const { width: W, height: H, data } = img;
  const R = p.ringR * W;
  const cls = new Uint8Array(W * H);
  for (let i = 0; i < W * H; i++) cls[i] = ringClass(data[i * 4]!, data[i * 4 + 1]!, data[i * 4 + 2]!);
  const out: Candidate[] = [];
  const angles = 48;
  const redTeam: Team = p.redTeam ?? 'CHAOS', blueTeam: Team = redTeam === 'CHAOS' ? 'ORDER' : 'CHAOS';
  for (const [team, c] of [[redTeam, RED], [blueTeam, BLUE]] as const) {
    const acc = new Float32Array(W * H);
    // Vote: each ring pixel votes for centres at distance R (±0.5 px band via 2 radii).
    for (let y = 0; y < H; y++) for (let x = 0; x < W; x++) {
      if (cls[y * W + x] !== c) continue;
      for (const rr of [R - 0.6, R + 0.6]) {
        for (let k = 0; k < angles; k++) {
          const t = (k / angles) * Math.PI * 2;
          const cx = Math.round(x + Math.cos(t) * rr), cy = Math.round(y + Math.sin(t) * rr);
          if (cx >= 0 && cy >= 0 && cx < W && cy < H) acc[cy * W + cx]! += 1;
        }
      }
    }
    // A full ring of circumference 2πR contributes ~2πR * 2 * angles/(2πR) ≈ 2*angles... normalise per centre:
    // count how many of `angles` directions around the centre hit a ring pixel instead (robust to thickness).
    const nms = Math.max(3, Math.round(R * 0.9));
    const peaks: Array<{ x: number; y: number; v: number }> = [];
    for (let y = 1; y < H - 1; y++) for (let x = 1; x < W - 1; x++) {
      const v = acc[y * W + x]!;
      if (v < angles * 0.15) continue;
      peaks.push({ x, y, v });
    }
    peaks.sort((a, b) => b.v - a.v);
    const kept: typeof peaks = [];
    for (const pk of peaks) {
      if (kept.some(k => Math.abs(k.x - pk.x) <= nms && Math.abs(k.y - pk.y) <= nms)) continue;
      // Ring support: fraction of directions with a ring pixel within R±1.5.
      let hits = 0;
      for (let k = 0; k < angles; k++) {
        const t = (k / angles) * Math.PI * 2;
        let hit = false;
        for (let dr = -1.5; dr <= 1.5 && !hit; dr += 0.75) {
          const xx = Math.round(pk.x + Math.cos(t) * (R + dr)), yy = Math.round(pk.y + Math.sin(t) * (R + dr));
          if (xx >= 0 && yy >= 0 && xx < W && yy < H && cls[yy * W + xx] === c) hit = true;
        }
        if (hit) hits++;
      }
      const votes = hits / angles;
      if (votes >= p.minVotes) { kept.push(pk); out.push({ x: pk.x, y: pk.y, team, votes }); }
      if (kept.length >= 40) break;
    }
  }
  return out;
}

/** Build a matching template from a ddragon square icon for a given minimap width. */
export function buildTemplate(champion: string, team: Team, icon: Rgba, minimapWidth: number, p: DetectParams): Template {
  const r = Math.max(4, Math.round(p.innerR * minimapWidth));
  const size = r * 2 + 1;
  // Crop the centre of the icon according to zoom, then resize to the portrait size.
  const crop = Math.round(icon.width / p.zoom);
  const off = Math.round((icon.width - crop) / 2);
  const cropped: Rgba = { width: crop, height: crop, data: new Uint8Array(crop * crop * 4) };
  for (let y = 0; y < crop; y++) cropped.data.set(icon.data.subarray(((y + off) * icon.width + off) * 4, ((y + off) * icon.width + off + crop) * 4), y * crop * 4);
  const small = resize(cropped, size, size);
  const mask = new Uint8Array(size * size);
  for (let y = 0; y < size; y++) for (let x = 0; x < size; x++) if ((x - r) ** 2 + (y - r) ** 2 <= (r - 0.5) ** 2) mask[y * size + x] = 1;
  return { champion, team, size, mask, vec: normalizedPatch(small.data, size, 0, 0, size, mask) };
}

/** Zero-mean, unit-norm RGB vector of masked pixels of a size×size patch at (x0,y0). */
function normalizedPatch(data: Uint8Array, stride: number, x0: number, y0: number, size: number, mask: Uint8Array): Float32Array {
  const n = mask.reduce((s, m) => s + m, 0);
  const v = new Float32Array(n * 3);
  let k = 0, sum = 0;
  for (let y = 0; y < size; y++) for (let x = 0; x < size; x++) {
    if (!mask[y * size + x]) continue;
    const i = ((y0 + y) * stride + (x0 + x)) * 4;
    for (let c = 0; c < 3; c++) { v[k] = data[i + c]!; sum += v[k]!; k++; }
  }
  const mean = sum / v.length;
  let norm = 0;
  for (let i = 0; i < v.length; i++) { v[i]! -= mean; norm += v[i]! * v[i]!; }
  norm = Math.sqrt(norm) || 1;
  for (let i = 0; i < v.length; i++) v[i]! /= norm;
  return v;
}

/**
 * Masked NCC without allocations. The template vector is zero-mean and unit-norm,
 * so NCC = Σ v·t / sqrt(Σv² − n·mean²) (the mean term cancels in the numerator).
 */
function ncc(img: Rgba, cx: number, cy: number, t: Template): number {
  const r = (t.size - 1) / 2;
  const x0 = cx - r, y0 = cy - r;
  if (x0 < 0 || y0 < 0 || x0 + t.size > img.width || y0 + t.size > img.height) return -1;
  const data = img.data, W = img.width, size = t.size, mask = t.mask, vec = t.vec;
  let dot = 0, sum = 0, sq = 0, k = 0;
  for (let y = 0; y < size; y++) {
    let i = ((y0 + y) * W + x0) * 4, m = y * size;
    for (let x = 0; x < size; x++, i += 4, m++) {
      if (!mask[m]) continue;
      const a = data[i]!, b = data[i + 1]!, c = data[i + 2]!;
      dot += a * vec[k]! + b * vec[k + 1]! + c * vec[k + 2]!;
      sum += a + b + c; sq += a * a + b * b + c * c;
      k += 3;
    }
  }
  const varTerm = sq - (sum * sum) / k;
  return varTerm > 1e-6 ? dot / Math.sqrt(varTerm) : -1;
}

/** Best score of a template around (cx, cy) within ±refine px. */
export function matchAround(img: Rgba, cx: number, cy: number, t: Template, refine: number): { score: number; x: number; y: number } {
  let best = { score: -1, x: cx, y: cy };
  for (let dy = -refine; dy <= refine; dy++) for (let dx = -refine; dx <= refine; dx++) {
    const s = ncc(img, cx + dx, cy + dy, t);
    if (s > best.score) best = { score: s, x: cx + dx, y: cy + dy };
  }
  return best;
}

/** Full detection on one minimap frame. `extraCenters` = e.g. tracker predictions to also try. */
export function detect(img: Rgba, templates: Template[], p: DetectParams = DEFAULT_PARAMS, extraCenters: Array<{ x: number; y: number; team: Team }> = []): { detections: Detection[]; candidates: Candidate[] } {
  const W = img.width;
  const candidates = findCandidates(img, p);
  const all = [...candidates, ...extraCenters.map(c => ({ x: Math.round(c.x * W), y: Math.round(c.y * W), team: c.team, votes: 0 }))];
  // Score every (candidate, champion of that team) pair.
  const pairs: Array<{ ci: number; t: Template; score: number; x: number; y: number; margin: number }> = [];
  all.forEach((c, ci) => {
    const scored = templates.filter(t => t.team === c.team).map(t => ({ t, ...matchAround(img, c.x, c.y, t, p.refine) })).sort((a, b) => b.score - a.score);
    for (const [k, s] of scored.entries()) pairs.push({ ci, t: s.t, score: s.score, x: s.x, y: s.y, margin: k === 0 ? s.score - (scored[1]?.score ?? -1) : 0 });
  });
  pairs.sort((a, b) => b.score - a.score);
  const usedC = new Set<number>(), usedT = new Set<string>();
  const detections: Detection[] = [];
  for (const pr of pairs) {
    const need = p.ownTeam && pr.t.team !== p.ownTeam ? p.minScoreEnemy : p.minScore;
    if (pr.score < need || usedC.has(pr.ci) || usedT.has(pr.t.champion)) continue;
    // Two candidates on the same spot (ring Hough + tracker hint): skip duplicates.
    if (detections.some(d => Math.hypot(d.x * W - pr.x, d.y * W - pr.y) < p.ringR * W * 0.8)) { usedC.add(pr.ci); continue; }
    usedC.add(pr.ci); usedT.add(pr.t.champion);
    detections.push({ champion: pr.t.champion, team: pr.t.team, x: pr.x / W, y: pr.y / W, score: pr.score, margin: pr.margin, votes: all[pr.ci]!.votes });
  }

  // Fallback sweep for champions still missing: portraits whose ring is mostly
  // covered by another portrait don't make it through the Hough step. Try every
  // position (stride 2) that has ring colour of the right team within ~R.
  const missing = templates.filter(t => !usedT.has(t.champion));
  const sweep = p.sweep ?? 'full';
  if (missing.length && sweep !== 'none') {
    const inRegion = sweep === 'full' ? () => true : (x: number, y: number) => sweep.some(r => Math.hypot(x / W - r.x, y / W - r.y) <= r.r);
    const R = p.ringR * W, near = sweepMask(img, R);
    for (const t of missing) {
      const want = t.team === (p.redTeam ?? 'CHAOS') ? RED : BLUE;
      let best = { score: -1, x: 0, y: 0 };
      for (let y = 0; y < img.height; y += 2) for (let x = 0; x < W; x += 2) {
        if (!inRegion(x, y)) continue;
        if (near[y * W + x] !== want && near[y * W + x] !== 3) continue;
        const s = ncc(img, x, y, t);
        if (s > best.score) best = { score: s, x, y };
      }
      const fine = matchAround(img, best.x, best.y, t, 1);
      // Team-coloured ring around it is the strong cue (structures/pings don't have one);
      // a covered portrait still shows most of its ring.
      const support = ringSupport(img, fine.x, fine.y, R, want);
      if (process.env.RC_DEBUG) console.log('fallback', t.champion, fine, 'support', support.toFixed(2));
      const need = p.ownTeam && t.team !== p.ownTeam ? p.minScoreEnemy : p.minScore;
      if (fine.score < Math.max(need, p.fallbackMinScore ?? 0) || support < (p.fallbackSupport ?? 0.4)) continue;
      if (detections.some(d => Math.hypot(d.x * W - fine.x, d.y * W - fine.y) < R * 0.9)) continue;
      usedT.add(t.champion);
      detections.push({ champion: t.champion, team: t.team, x: fine.x / W, y: fine.y / W, score: fine.score, margin: 0, votes: 0 });
    }
  }
  return { detections, candidates };
}

/** Fraction of directions around (cx,cy) with a ring pixel of class c at radius R±1.5. */
export function ringSupport(img: Rgba, cx: number, cy: number, R: number, c: number): number {
  const { width: W, height: H, data } = img;
  let hits = 0;
  const n = 48;
  for (let k = 0; k < n; k++) {
    const t = (k / n) * Math.PI * 2;
    for (let dr = -1.5; dr <= 1.5; dr += 0.75) {
      const x = Math.round(cx + Math.cos(t) * (R + dr)), y = Math.round(cy + Math.sin(t) * (R + dr));
      if (x >= 0 && y >= 0 && x < W && y < H && ringClass(data[(y * W + x) * 4]!, data[(y * W + x) * 4 + 1]!, data[(y * W + x) * 4 + 2]!) === c) { hits++; break; }
    }
  }
  return hits / n;
}

/** Per pixel: which team ring colours occur within ~R (1 red, 2 blue, 3 both, 0 none). */
function sweepMask(img: Rgba, R: number): Uint8Array {
  const { width: W, height: H, data } = img;
  const out = new Uint8Array(W * H);
  const r = Math.ceil(R + 1);
  for (let y = 0; y < H; y += 2) for (let x = 0; x < W; x += 2) {
    const c = ringClass(data[(y * W + x) * 4]!, data[(y * W + x) * 4 + 1]!, data[(y * W + x) * 4 + 2]!);
    if (!c) continue;
    for (let dy = -r; dy <= r; dy += 2) for (let dx = -r; dx <= r; dx += 2) {
      const xx = x + dx, yy = y + dy;
      if (xx >= 0 && yy >= 0 && xx < W && yy < H) out[yy * W + xx]! |= c;
    }
  }
  return out;
}
