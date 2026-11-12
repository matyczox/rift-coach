import type { Rect } from '../shared/types';

type Px = Uint8Array | Buffer;

function luminance(px: Px, width: number, height: number): Float32Array {
  const lum = new Float32Array(width * height);
  for (let i = 0, j = 0; i < lum.length; i++, j += 4) lum[i] = (px[j]! + px[j + 1]! + px[j + 2]!) / 3;
  return lum;
}

/**
 * Edge test for the minimap frame: just inside the square there is a dark band,
 * just outside a brighter rim. Returns the fraction of sampled points along the
 * edge (inner 80%, skipping decorated corners) that look like that. Lanes and
 * walls inside the map only match in short stretches, the real frame along
 * nearly its whole length.
 */
function edgeFraction(lum: Float32Array, width: number, x0: number, y0: number, dx: number, dy: number, len: number, ox: number, oy: number): number {
  const L = (x: number, y: number) => lum[y * width + x]!;
  let hits = 0, n = 0;
  for (let k = 0; k < len; k += 2, n++) {
    const x = x0 + dx * k, y = y0 + dy * k;
    let out = 0, inn = 0;
    for (let d = 1; d <= 3; d++) { out += L(x - ox * d, y - oy * d); inn += L(x + ox * (d - 1), y + oy * (d - 1)); }
    out /= 3; inn /= 3;
    if (inn < 55 && out - inn > 12) hits++;
  }
  return n ? hits / n : 0;
}

function squareScore(lum: Float32Array, width: number, height: number, x0: number, y0: number, s: number): number {
  if (x0 < 3 || y0 < 3 || x0 + s + 3 >= width || y0 + s + 3 >= height) return 0;
  const inset = Math.round(s * 0.1), len = s - 2 * inset;
  return Math.min(
    edgeFraction(lum, width, x0, y0 + inset, 0, 1, len, 1, 0),          // left
    edgeFraction(lum, width, x0 + inset, y0, 1, 0, len, 0, 1),          // top
    edgeFraction(lum, width, x0 + s - 1, y0 + inset, 0, 1, len, -1, 0), // right
    edgeFraction(lum, width, x0 + inset, y0 + s - 1, 1, 0, len, 0, -1), // bottom
  );
}

/**
 * Locate the minimap square in a full game frame (bottom-right anchored).
 * Coarse grid search, then a fine search around the best candidate.
 * Ties go to the larger square, so a sub-square inside the map never wins.
 */
export function findMinimap(px: Px, width: number, height: number): { rect: Rect; score: number } | null {
  const lum = luminance(px, width, height);
  const minS = Math.round(height * 0.14), maxS = Math.round(height * 0.42);
  const maxMargin = Math.round(height * 0.04);

  const search = (sRange: [number, number], mrRange: [number, number], mbRange: [number, number], step: number) => {
    let best: { rect: Rect; score: number } | null = null;
    for (let s = sRange[0]; s <= sRange[1]; s += step) {
      for (let mr = mrRange[0]; mr <= mrRange[1]; mr += step) {
        for (let mb = mbRange[0]; mb <= mbRange[1]; mb += step) {
          const x0 = width - mr - s, y0 = height - mb - s;
          const score = squareScore(lum, width, height, x0, y0, s);
          // Prefer clearly better scores; among near-ties prefer the larger square.
          const better = !best || score > best.score + 0.02 || (score >= best.score - 0.02 && s > best.rect.width);
          if (better) best = { rect: { x: x0, y: y0, width: s, height: s }, score: best ? Math.max(score, best.score) : score };
        }
      }
    }
    return best;
  };

  const coarse = search([minS, maxS], [0, maxMargin], [0, maxMargin], 3);
  if (!coarse || coarse.score < 0.5) return null;
  const c = coarse.rect, mr = width - c.x - c.width, mb = height - c.y - c.height;
  const fine = search([c.width - 4, c.width + 4], [Math.max(0, mr - 4), mr + 4], [Math.max(0, mb - 4), mb + 4], 1);
  const best = fine && fine.score >= coarse.score - 0.02 ? fine : coarse;
  return { rect: best.rect, score: squareScore(lum, width, height, best.rect.x, best.rect.y, best.rect.width) };
}

/** Score an existing calibration on a new frame (0..1). */
export function verifyMinimap(px: Px, width: number, height: number, r: Rect): number {
  return squareScore(luminance(px, width, height), width, height, r.x, r.y, r.width);
}

/** Scale a calibration rect measured on one window size to another. */
export function scaleRect(r: Rect, fromW: number, fromH: number, toW: number, toH: number): Rect {
  if (fromW === toW && fromH === toH) return r;
  // The minimap scales with window height and stays anchored bottom-right.
  const k = toH / fromH;
  const right = fromW - (r.x + r.width), bottom = fromH - (r.y + r.height);
  const s = Math.round(r.width * k);
  return { x: Math.round(toW - right * k - s), y: Math.round(toH - bottom * k - s), width: s, height: s };
}
