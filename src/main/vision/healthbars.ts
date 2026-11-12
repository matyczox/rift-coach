import type { Rgba } from './png';

/**
 * Reads champion health bars drawn above champions' heads.
 *
 * Only what the player can see: a bar that is on screen gives that champion's
 * HP fraction; anything off-screen / in fog is simply absent from the result.
 *
 * Geometry (measured on 2560x1440, scaled by frame height for other sizes):
 *   [lvl box 26px] gap [frame | HP fill area 126px x 12px | frame]
 *                              [resource bar below, ignored]
 * The HP fill area starts 3px right of the frame's grey left edge; segment
 * ticks are 1px black lines in the upper ~2/3 of the fill; the empty part is
 * near-black; shields are a light grey run right after the HP fill.
 */

export type BarColor = 'red' | 'blue';

export interface HealthBar {
  /** Left edge of the HP fill area (screen px). */
  x: number;
  /** Top row of the HP fill (screen px). */
  y: number;
  /** Width of the full HP fill area (screen px) — the 100% length. */
  width: number;
  /** HP fraction 0..1 of the full bar. */
  pct: number;
  /** Shield fraction 0..1 (grey run right after the HP fill). */
  shieldPct: number;
  /**
   * True when HP + shield fill the whole bar. League then rescales the bar to
   * HP+shield, so the real HP fraction may be HIGHER than pct (pct is a lower
   * bound; e.g. a full-HP champion with a 20% shield shows pct≈0.83).
   */
  shieldCompressed: boolean;
  /** Champion level read from the level box, or null when unreadable. */
  level: number | null;
  /** 0..1 — how much the shape matched a champion bar. */
  confidence: number;
}

export interface ReadOptions {
  enemyColor: BarColor;
  /** Minimum confidence to report (default 0.5). */
  minConfidence?: number;
  /**
   * Height in px of the full game window the frame comes from (default:
   * frame.height). Set it when passing a crop/sub-region so sizes still scale
   * correctly (bar geometry is proportional to window height).
   */
  screenHeight?: number;
}

// ---- geometry at 1440p --------------------------------------------------------
const REF_H = 1440;
const INNER_W = 125;          // HP fill area width (full bar = 125 fill px)
const FILL_H = 12;            // HP fill height (rows passing the colour test)
const RIGHT_FRAME = 128;      // innerLeft + this = right grey frame column
const LEFT_FRAME = 3;         // innerLeft - this = left grey frame column
const BORDER_TOP = 4;         // fillTop - this = top grey frame row
const BOX_L = 30, BOX_R = 7;  // level box interior: innerLeft-BOX_L .. innerLeft-BOX_R
const BOX_T = 5, BOX_B = 21;  // level box rows: fillTop-BOX_T .. fillTop+BOX_B

// ---- colour tests ---------------------------------------------------------------
function isRed(r: number, g: number, b: number) { return r >= 110 && r - g >= 60 && r - b >= 60; }
function isBlue(r: number, g: number, b: number) { return b >= 150 && b - r >= 90 && g >= 90 && g < b; }
function isShield(r: number, g: number, b: number) {
  const mx = Math.max(r, g, b), mn = Math.min(r, g, b);
  return mn >= 130 && mx - mn <= 45;
}
/** Grey frame colour (~(70,71,68)..(110,112,108)). */
function isFrame(r: number, g: number, b: number) {
  const mx = Math.max(r, g, b), mn = Math.min(r, g, b);
  return mn >= 45 && mx <= 140 && mx - mn <= 14;
}

type Pred = (r: number, g: number, b: number) => boolean;

/**
 * Find all champion health bars of the given colour.
 * In a live game with relative team colours: enemies 'red', allies 'blue'.
 * In replays colours are absolute: ORDER 'blue', CHAOS 'red'.
 */
export function readHealthBars(frame: Rgba, opts: ReadOptions): HealthBar[] {
  const { width: W, height: H, data } = frame;
  const s = (opts.screenHeight ?? H) / REF_H;
  const fill: Pred = opts.enemyColor === 'red' ? isRed : isBlue;
  const minConf = opts.minConfidence ?? 0.5;
  const fillH = FILL_H * s;
  const step = Math.max(2, Math.floor(fillH / 2));
  const innerW = Math.round(INNER_W * s);
  const boxL = Math.round(BOX_L * s);
  const out: HealthBar[] = [];
  const seen = new Set<number>();

  const px = (x: number, y: number) => (y * W + x) * 4;
  const isFillAt = (x: number, y: number) => {
    if (x < 0 || y < 0 || x >= W || y >= H) return false;
    const o = px(x, y); return fill(data[o]!, data[o + 1]!, data[o + 2]!);
  };

  // Coarse search: every `step` rows (always >=1 hit inside the 12px-tall
  // fill) and every 2nd column (a bar with >=2px of fill is still found; we back
  // up 1px to the exact run start). Candidates are verified at full res. The
  // bar must leave room for the level box on the left and the frame on the right,
  // so bars clipped by the screen edge are skipped.
  const xMin = boxL + 2, xMax = W - Math.round(RIGHT_FRAME * s) - 2;
  for (let y = 2; y < H - 2; y += step) {
    const rowO = px(0, y);
    let prevFill = false;
    for (let x = xMin; x < xMax; x += 2) {
      const o = rowO + x * 4;
      const f = fill(data[o]!, data[o + 1]!, data[o + 2]!);
      if (f && !prevFill) {
        const bar = verify(isFillAt(x - 1, y) ? x - 1 : x, y);
        if (bar) {
          // skip past this bar on this row
          x = bar.x + innerW;
          prevFill = false;
          continue;
        }
      }
      prevFill = f;
    }
  }
  out.sort((a, b) => a.y - b.y || a.x - b.x);
  return out;

  function verify(x0: number, y0: number): HealthBar | null {
    // Run start must be the fill-area left edge: the column left of it is not fill.
    if (isFillAt(x0 - 1, y0)) return null;
    // Cheap reject: the grey frame must sit just left of the fill on this row.
    if (!frameNear(x0 - Math.round(LEFT_FRAME * s), y0)) return null;
    // Vertical extent of the fill at x0+1 (avoid a tick column by using two columns).
    const cx = x0 + 1;
    let top = y0, bot = y0;
    const lim = Math.ceil(fillH * 1.6);
    while (top > 0 && y0 - top < lim && (isFillAt(cx, top - 1) || isFillAt(x0, top - 1))) top--;
    while (bot < H - 1 && bot - y0 < lim && (isFillAt(cx, bot + 1) || isFillAt(x0, bot + 1))) bot++;
    const h = bot - top + 1;
    if (h < fillH * 0.7 || h > fillH * 1.4) return null;
    const key = Math.round(x0 / 3) * 100000 + Math.round(top / 3);
    if (seen.has(key)) return null;

    // Shape checks -----------------------------------------------------------
    let score = 0, total = 0, worst = 1;
    // (1) left edge consistency: every fill row starts at x0 (±1).
    {
      let ok = 0, n = 0;
      for (let y = top; y <= bot; y++) { n++; if (isFillAt(x0, y) || isFillAt(x0 + 1, y)) { if (!isFillAt(x0 - 2, y)) ok++; } }
      score += ok / n; total++; worst = Math.min(worst, ok / n);
    }
    // (2) grey frame left of the fill and on the right end.
    const lf = x0 - Math.round(LEFT_FRAME * s), rf = x0 + Math.round(RIGHT_FRAME * s);
    {
      let ok = 0, n = 0;
      for (let y = top; y <= bot; y++) {
        n += 2;
        if (frameNear(lf, y)) ok++;
        if (frameNear(rf, y)) ok++;
      }
      score += ok / n; total++; worst = Math.min(worst, ok / n);
    }
    // (3) top grey border row spanning the bar.
    {
      const ty = top - Math.round(BORDER_TOP * s);
      let ok = 0, n = 0;
      for (let x = x0; x < x0 + innerW; x += 3) {
        n++;
        if (frameNearV(x, ty)) ok++;
      }
      score += ok / n; total++; worst = Math.min(worst, ok / n);
    }
    // (4) dark level box left of the bar.
    const bx0 = x0 - boxL, bx1 = x0 - Math.round(BOX_R * s);
    const by0 = top - Math.round(BOX_T * s), by1 = top + Math.round(BOX_B * s);
    {
      let dark = 0, n = 0;
      for (let y = by0; y <= by1; y += 2) for (let x = bx0 + 2; x <= bx1 - 2; x += 2) {
        if (y < 0 || y >= H) continue;
        const o = px(x, y); n++;
        const r = data[o]!, g = data[o + 1]!, b = data[o + 2]!;
        if (r + g + b < 140 || Math.min(r, g, b) > 150) dark++; // dark box or white digit
      }
      score += n ? dark / n : 0; total++; worst = Math.min(worst, n ? dark / n : 0);
    }

    // Measurement --------------------------------------------------------------
    // Use rows in the lower part of the fill (below the segment ticks).
    const xEnd = Math.min(W - 1, x0 + innerW - 1);
    let last = x0;
    let shieldEnd = -1;
    const rows = [top + Math.round(h * 0.7), top + Math.round(h * 0.8)];
    for (const ry of rows) {
      let l = x0, gap = 0;
      for (let x = x0; x <= xEnd; x++) {
        if (isFillAt(x, ry)) { l = x; gap = 0; } else if (++gap > Math.max(2, 3 * s)) break;
      }
      if (l > last) last = l;
    }
    // shield: grey run right after the HP fill
    for (const ry of rows) {
      let e = -1, gap = 0;
      for (let x = last + 1; x <= xEnd; x++) {
        const o = px(x, ry);
        if (isShield(data[o]!, data[o + 1]!, data[o + 2]!)) { e = x; gap = 0; } else if (++gap > Math.max(2, 3 * s)) break;
      }
      if (e > shieldEnd) shieldEnd = e;
    }
    // (5) fill solidity: between x0 and the fill end every column is fill in
    // most rows (ticks are 1px); (6) the empty remainder is near-black.
    {
      let ok = 0, n = 0;
      for (let x = x0; x <= last; x++) {
        n++;
        let c = 0;
        for (let y = top; y <= bot; y++) if (isFillAt(x, y)) c++;
        if (c >= h * 0.25) ok++;
      }
      const solid = n ? ok / n : 0;
      score += solid; total++; worst = Math.min(worst, solid);
      const e0 = Math.max(last, shieldEnd) + 2, e1 = xEnd - 1;
      if (e1 - e0 >= 3) {
        let dk = 0, m = 0;
        for (const ry of rows) for (let x = e0; x <= e1; x++) {
          const o = px(x, ry); m++;
          if (Math.max(data[o]!, data[o + 1]!, data[o + 2]!) <= 50) dk++;
        }
        score += dk / m; total++; worst = Math.min(worst, dk / m);
      }
    }
    const confidence = score / total;
    if (confidence < minConf || worst < 0.6) return null;
    seen.add(key);
    const pct = Math.min(1, (last - x0 + 1) / innerW);
    const shieldPct = shieldEnd > last ? Math.min(1 - pct, (shieldEnd - last) / innerW) : 0;
    const shieldCompressed = shieldPct >= 0.02 && pct + shieldPct >= 0.97;
    const bar: HealthBar = { x: x0, y: top, width: innerW, pct, shieldPct, shieldCompressed, level: null, confidence };
    bar.level = readLevel(frame, bar, s);
    out.push(bar);
    return bar;
  }

  function frameNear(x: number, y: number) {
    for (let d = -1; d <= 1; d++) {
      const xx = x + d; if (xx < 0 || xx >= W) continue;
      const o = px(xx, y); if (isFrame(data[o]!, data[o + 1]!, data[o + 2]!)) return true;
    }
    return false;
  }
  function frameNearV(x: number, y: number) {
    for (let d = -1; d <= 1; d++) {
      const yy = y + d; if (yy < 0 || yy >= H) continue;
      const o = px(x, yy); if (isFrame(data[o]!, data[o + 1]!, data[o + 2]!)) return true;
    }
    return false;
  }
}

// ---- level digits ------------------------------------------------------------------

/** Digit glyph grid size used for templates. */
export const GLYPH_W = 6, GLYPH_H = 9;

/**
 * Extract digit glyphs from the level box as GLYPH_W x GLYPH_H grids of 0..1
 * brightness. Returns [] when nothing digit-like is found.
 */
export function levelGlyphs(frame: Rgba, bar: Pick<HealthBar, 'x' | 'y'>, scale?: number): number[][] {
  const { width: W, height: H, data } = frame;
  const s = scale ?? H / REF_H;
  const x0 = bar.x - Math.round(BOX_L * s), x1 = bar.x - Math.round(BOX_R * s);
  const y0 = bar.y - Math.round(BOX_T * s), y1 = bar.y + Math.round(BOX_B * s);
  if (x0 < 0 || y0 < 0 || x1 >= W || y1 >= H) return [];
  const bw = x1 - x0 + 1, bh = y1 - y0 + 1;
  const v = new Float32Array(bw * bh);
  for (let y = 0; y < bh; y++) for (let x = 0; x < bw; x++) {
    const o = ((y0 + y) * W + x0 + x) * 4;
    const mn = Math.min(data[o]!, data[o + 1]!, data[o + 2]!);
    v[y * bw + x] = Math.max(0, Math.min(1, (mn - 70) / 130));
  }
  const on = (x: number, y: number) => v[y * bw + x]! > 0.45;
  // Columns / rows containing bright pixels.
  const colOn: boolean[] = [];
  for (let x = 0; x < bw; x++) { let c = 0; for (let y = 0; y < bh; y++) if (on(x, y)) c++; colOn.push(c > 0); }
  let rTop = -1, rBot = -1;
  for (let y = 0; y < bh; y++) { let c = 0; for (let x = 0; x < bw; x++) if (on(x, y)) c++; if (c > 0) { if (rTop < 0) rTop = y; rBot = y; } }
  if (rTop < 0) return [];
  const gh = rBot - rTop + 1;
  if (gh < 6 * s || gh > 14 * s) return [];
  // Split into column groups.
  const groups: Array<[number, number]> = [];
  for (let x = 0; x < bw; x++) {
    if (!colOn[x]) continue;
    let e = x; while (e + 1 < bw && colOn[e + 1]) e++;
    groups.push([x, e]); x = e;
  }
  const glyphs: number[][] = [];
  for (const [a, b] of groups) {
    const gw = b - a + 1;
    if (gw > gh) return []; // not a digit (or merged)
    // Fixed-aspect cell centred on the glyph so narrow '1' stays narrow.
    const cellW = gh * (GLYPH_W / GLYPH_H);
    const cx = (a + b + 1) / 2;
    const g: number[] = [];
    for (let gy = 0; gy < GLYPH_H; gy++) for (let gx = 0; gx < GLYPH_W; gx++) {
      // area sample
      const sx0 = cx - cellW / 2 + (gx * cellW) / GLYPH_W, sx1 = sx0 + cellW / GLYPH_W;
      const sy0 = rTop + (gy * gh) / GLYPH_H, sy1 = sy0 + gh / GLYPH_H;
      // exact pixel-overlap weights (keeps 1080p glyphs as sharp as possible)
      let acc = 0, n = 0;
      for (let yy = Math.floor(sy0); yy < Math.ceil(sy1); yy++) {
        const wy = Math.min(yy + 1, sy1) - Math.max(yy, sy0);
        for (let xx = Math.floor(sx0); xx < Math.ceil(sx1); xx++) {
          const w = wy * (Math.min(xx + 1, sx1) - Math.max(xx, sx0));
          n += w;
          if (xx >= 0 && xx < bw && yy >= 0 && yy < bh) acc += w * v[yy * bw + xx]!;
        }
      }
      g.push(n ? acc / n : 0);
    }
    glyphs.push(g);
  }
  return glyphs.length >= 1 && glyphs.length <= 2 ? glyphs : [];
}

/**
 * Digit templates (GLYPH_W x GLYPH_H, row-major, intensity 0..9), built from
 * hand-labelled replay bars (1440p + bilinear 1200p/1080p) by scripts/dev/hp-digits.ts. '3' never
 * appeared on screen in the dataset and is synthesised from '8'; '2' and '4'
 * come from only two samples each.
 */
const DIGIT_TEMPLATES: Record<string, string> = {
  '0': '048830484683861177840068840068850068861067584473058830', // n=21
  '1': '003840038950046850012850002850002850002850002850005960', // n=129
  '2': '168951784585850088750186010581002830017610179322699998', // n=6
  '3': '158861004486001176006672118882002277000068883377278862', // n=0 (synthetic)
  '4': '002992016992027492165292371293673495577897011393000393', // n=6
  '5': '289984384331471000586510357882001387210178763485389841', // n=55
  '6': '015850157300384210787773872277860058760058584385158841', // n=50
  '7': '888898432486000482000661003830005710028500049300169300', // n=48
  '8': '158861574486771176386672278882662277860068883377278862', // n=49
  '9': '268861662386840068840068773388267796001582014840168400', // n=27
};

/** Zero-mean, unit-norm copy (for normalised cross-correlation). */
function normalise(g: number[]): number[] {
  const m = g.reduce((x, y) => x + y, 0) / g.length;
  const c = g.map((v) => v - m);
  const n = Math.sqrt(c.reduce((x, y) => x + y * y, 0)) || 1;
  return c.map((v) => v / n);
}

const TEMPLATES: Array<{ d: number; g: number[] }> = Object.entries(DIGIT_TEMPLATES).map(([d, str]) => ({
  d: Number(d), g: normalise([...str].map((c) => Number(c) / 9)),
}));

/** Nearest digit by normalised cross-correlation (robust to blur/contrast at 1080p). */
export function classifyGlyph(glyph: number[]): { d: number; corr: number; margin: number } | null {
  const g = normalise(glyph);
  let best = -1, bc = -Infinity, second = -Infinity;
  for (const t of TEMPLATES) {
    let c = 0;
    for (let i = 0; i < g.length; i++) c += g[i]! * t.g[i]!;
    if (c > bc) { second = bc; bc = c; best = t.d; } else if (c > second) second = c;
  }
  return best < 0 ? null : { d: best, corr: bc, margin: bc - second };
}

const MIN_CORR = 0.6, MIN_MARGIN = 0.08;

/** Read the champion level (1..20) from the level box, null when unsure. */
export function readLevel(frame: Rgba, bar: Pick<HealthBar, 'x' | 'y'>, scale?: number): number | null {
  if (!TEMPLATES.length) return null;
  const glyphs = levelGlyphs(frame, bar, scale);
  if (!glyphs.length) return null;
  let n = 0;
  for (const g of glyphs) {
    const c = classifyGlyph(g);
    if (!c || c.corr < MIN_CORR || c.margin < MIN_MARGIN) return null;
    n = n * 10 + c.d;
  }
  return n >= 1 && n <= 20 ? n : null;
}

// ---- assignment ----------------------------------------------------------------------

export interface ChampionCandidate {
  champion: string;
  level: number;
  /**
   * Champion's projected GROUND position on screen (e.g. from the minimap /
   * last known world position), if known. The bar is expected ~barAbove bar
   * widths above it.
   */
  screen?: { x: number; y: number };
}

export interface Assignment<B> { bar: B; champion: string | null }

/**
 * Assign bars (all of one team colour) to that team's champions.
 * Level number first; among equal-level candidates (or bars with unread level)
 * screen-position proximity decides. Ambiguous bars stay unassigned (null).
 *
 * Options:
 *  - maxDist: max px between the bar's expected and actual position
 *    (default 3 bar widths ≈ 375px at 1440p).
 *  - barAbove: bar height above the ground point in bar widths (default 1.6;
 *    measured ≈200-220px above the feet at 1440p for Rumble/Yone; varies by champion).
 *  - ambiguityRatio: a match is kept only if best/second-best distance is
 *    below this (default 0.6).
 * A unique level match is also dropped when that champion's screen position is
 * given and is further than 2*maxDist away (stale levels / misread digit).
 */
export function matchBarsToChampions<B extends { x: number; y: number; width: number; level: number | null }>(
  bars: B[], candidates: ChampionCandidate[], opts: { maxDist?: number; ambiguityRatio?: number; barAbove?: number } = {},
): Array<Assignment<B>> {
  const barAbove = opts.barAbove ?? 1.6;
  const maxD = (b: B) => opts.maxDist ?? 3 * b.width;
  const ratio = opts.ambiguityRatio ?? 0.6;
  const result: Array<Assignment<B>> = bars.map((bar) => ({ bar, champion: null }));
  const taken = new Set<string>();
  const dist = (b: B, c: ChampionCandidate) => {
    if (!c.screen) return Infinity;
    // bar centre vs where the bar should be: above the projected ground point
    return Math.hypot(b.x + b.width / 2 - c.screen.x, b.y - (c.screen.y - barAbove * b.width));
  };

  // Pass 1: bars whose level matches exactly one candidate.
  // Pass 2: bars whose level matches several candidates → proximity.
  // Pass 3: bars with unread level → proximity among all remaining.
  const pending: number[] = [];
  const byLevel = new Map<number, number[]>();
  bars.forEach((b, i) => {
    if (b.level == null) { pending.push(i); return; }
    const arr = byLevel.get(b.level) ?? []; arr.push(i); byLevel.set(b.level, arr);
  });
  for (const [lvl, idxs] of byLevel) {
    const cands = candidates.filter((c) => c.level === lvl);
    if (cands.length === 1 && idxs.length === 1) {
      const b = bars[idxs[0]!]!, c = cands[0]!;
      if (c.screen && dist(b, c) > 2 * maxD(b)) { pending.push(...idxs); continue; } // contradicts the position prior
      result[idxs[0]!]!.champion = c.champion; taken.add(c.champion);
    } else if (cands.length >= 1) {
      assignByProximity(idxs, cands);
    } else {
      // level read but nobody has it (misread or stale levels) → treat as unknown level
      pending.push(...idxs);
    }
  }
  if (pending.length) assignByProximity(pending, candidates.filter((c) => !taken.has(c.champion)));
  return result;

  function assignByProximity(idxs: number[], cands: ChampionCandidate[]) {
    // Greedy over all (bar, cand) pairs by distance, with an ambiguity check.
    const pairs: Array<{ i: number; c: ChampionCandidate; d: number }> = [];
    for (const i of idxs) for (const c of cands) {
      if (taken.has(c.champion)) continue;
      const d = dist(bars[i]!, c);
      if (d <= maxD(bars[i]!)) pairs.push({ i, c, d });
    }
    pairs.sort((a, b) => a.d - b.d);
    const done = new Set<number>();
    for (const p of pairs) {
      if (done.has(p.i) || taken.has(p.c.champion)) continue;
      // ambiguous if another free candidate is nearly as close to this bar,
      // or another free bar is nearly as close to this candidate
      const rivalC = pairs.find((q) => q.i === p.i && q.c !== p.c && !taken.has(q.c.champion));
      const rivalB = pairs.find((q) => q.c === p.c && q.i !== p.i && !done.has(q.i));
      if ((rivalC && p.d > rivalC.d * ratio) || (rivalB && p.d > rivalB.d * ratio)) continue;
      result[p.i]!.champion = p.c.champion; taken.add(p.c.champion); done.add(p.i);
    }
  }
}
