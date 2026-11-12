// Ward spots and vision plants on Summoner's Rift, in minimap coordinates FOR
// THE BLUE SIDE (0,0 top-left, blue base bottom-left; my-* = blue jungle).
// Side-relative spots are mirrored (1-x, 1-y) for a red-side player.
//
// `value` (1-5) = how much it reveals: spots at path junctions or bushes that
// see several entrances score higher than corridor spots. `watches` = what it
// covers, so the app picks the best spot for the situation. Hand-made first
// version; refine from ward icons observed in harvested replays.

export type WardPurpose = 'dragon' | 'baron' | 'herald' | 'enemy-jungler' | 'gank-top' | 'gank-mid' | 'gank-bot' | 'invade' | 'defend-jungle';

export interface WardSpot {
  x: number; y: number;
  value: number;
  watches: WardPurpose[];
  /** Neutral spots (river/pits/lanes) are not mirrored for red side. */
  neutral?: boolean;
  /**
   * 'plant' = Scryer's Bloom spawn: hitting it reveals a big area for free.
   * 'faelight' = 2026 Faelight: a ward placed ON it gets extra vision range (reveal zone ~45 s).
   */
  kind?: 'ward' | 'plant' | 'faelight';
  /** Faelights: present from game start or only after the Elemental Rift transforms. */
  spawn?: 'start' | 'rift-transform';
  label: string;
}

export const WARD_SPOTS: Record<string, WardSpot> = {
  // Faelights (season 2026). POSITIONS APPROXIMATE — confirm from replays/patch brief.
  'faelight-river-top': { x: 0.24, y: 0.30, value: 5, watches: ['baron', 'herald', 'gank-top', 'enemy-jungler'], neutral: true, kind: 'faelight', spawn: 'start', label: 'Faelight (wyspa w rzece, top)' },
  'faelight-river-bot': { x: 0.76, y: 0.70, value: 5, watches: ['dragon', 'gank-bot', 'enemy-jungler'], neutral: true, kind: 'faelight', spawn: 'start', label: 'Faelight (wyspa w rzece, bot)' },
  'faelight-my-base': { x: 0.22, y: 0.78, value: 3, watches: ['defend-jungle', 'gank-mid'], kind: 'faelight', spawn: 'start', label: 'Faelight przy naszej bazie' },
  'faelight-enemy-base': { x: 0.78, y: 0.22, value: 4, watches: ['invade', 'enemy-jungler'], kind: 'faelight', spawn: 'start', label: 'Faelight przy bazie wroga' },
  'faelight-mid-top': { x: 0.44, y: 0.40, value: 4, watches: ['gank-mid', 'baron'], neutral: true, kind: 'faelight', spawn: 'rift-transform', label: 'Faelight (środek, top)' },
  'faelight-mid-bot': { x: 0.56, y: 0.60, value: 4, watches: ['gank-mid', 'dragon'], neutral: true, kind: 'faelight', spawn: 'rift-transform', label: 'Faelight (środek, bot)' },
  'dragon-pit-entrance': { x: 0.62, y: 0.66, value: 4, watches: ['dragon'], neutral: true, label: 'wejście do smoka' },
  'river-bush-bot': { x: 0.70, y: 0.60, value: 5, watches: ['dragon', 'gank-bot', 'enemy-jungler'], neutral: true, label: 'krzak w rzece (bot)' },
  'baron-pit-entrance': { x: 0.38, y: 0.34, value: 4, watches: ['baron', 'herald'], neutral: true, label: 'wejście do barona' },
  'river-bush-top': { x: 0.30, y: 0.40, value: 5, watches: ['baron', 'herald', 'gank-top', 'enemy-jungler'], neutral: true, label: 'krzak w rzece (top)' },
  'mid-river-bot': { x: 0.58, y: 0.56, value: 3, watches: ['gank-mid', 'dragon'], neutral: true, label: 'rzeka przy midzie (bot)' },
  'mid-river-top': { x: 0.42, y: 0.44, value: 3, watches: ['gank-mid', 'baron'], neutral: true, label: 'rzeka przy midzie (top)' },
  'my-tri-bush-bot': { x: 0.60, y: 0.86, value: 3, watches: ['gank-bot', 'defend-jungle'], label: 'tri-bush bot (nasz)' },
  'my-tri-bush-top': { x: 0.14, y: 0.40, value: 3, watches: ['gank-top', 'defend-jungle'], label: 'tri-bush top (nasz)' },
  'my-raptors': { x: 0.47, y: 0.60, value: 3, watches: ['defend-jungle', 'gank-mid'], label: 'raptory (nasze)' },
  'my-blue-buff': { x: 0.25, y: 0.47, value: 2, watches: ['defend-jungle'], label: 'niebieski buff (nasz)' },
  'my-red-buff': { x: 0.52, y: 0.74, value: 2, watches: ['defend-jungle'], label: 'czerwony buff (nasz)' },
  'enemy-tri-bush-bot': { x: 0.86, y: 0.60, value: 4, watches: ['enemy-jungler', 'invade'], label: 'tri-bush bot (wroga)' },
  'enemy-tri-bush-top': { x: 0.40, y: 0.14, value: 4, watches: ['enemy-jungler', 'invade'], label: 'tri-bush top (wroga)' },
  'enemy-raptors': { x: 0.53, y: 0.40, value: 4, watches: ['enemy-jungler', 'invade', 'gank-mid'], label: 'raptory (wroga)' },
  'enemy-blue-buff': { x: 0.75, y: 0.53, value: 3, watches: ['enemy-jungler', 'invade'], label: 'niebieski buff (wroga)' },
  'enemy-red-buff': { x: 0.48, y: 0.26, value: 3, watches: ['enemy-jungler', 'invade'], label: 'czerwony buff (wroga)' },
  'top-lane-bush': { x: 0.12, y: 0.22, value: 2, watches: ['gank-top'], neutral: true, label: 'krzak na topie' },
  'bot-lane-bush': { x: 0.78, y: 0.88, value: 2, watches: ['gank-bot'], neutral: true, label: 'krzak na bocie' },
  // Scryer's Bloom spawns (vision plants): one hit reveals a large cone/area.
  'plant-dragon': { x: 0.66, y: 0.62, value: 5, watches: ['dragon', 'enemy-jungler'], neutral: true, kind: 'plant', label: 'roślina wizji przy smoku' },
  'plant-baron': { x: 0.34, y: 0.38, value: 5, watches: ['baron', 'herald', 'enemy-jungler'], neutral: true, kind: 'plant', label: 'roślina wizji przy baronie' },
  'plant-my-jungle-bot': { x: 0.56, y: 0.70, value: 3, watches: ['defend-jungle', 'dragon'], kind: 'plant', label: 'roślina wizji (nasza dżungla, bot)' },
  'plant-my-jungle-top': { x: 0.30, y: 0.52, value: 3, watches: ['defend-jungle', 'gank-top'], kind: 'plant', label: 'roślina wizji (nasza dżungla, top)' },
};

export function wardPoint(spot: string, redSide: boolean): { x: number; y: number } | null {
  const p = WARD_SPOTS[spot];
  if (!p) return null;
  return redSide && !p.neutral ? { x: 1 - p.x, y: 1 - p.y } : { x: p.x, y: p.y };
}

/** Best spot for a purpose: highest value, ties broken by distance to `near` (normalized coords). */
export function bestWardSpot(purpose: WardPurpose, near?: { x: number; y: number }, redSide = false, riftTransformed = false): string | null {
  let best: { id: string; score: number } | null = null;
  for (const [id, s] of Object.entries(WARD_SPOTS)) {
    if (!s.watches.includes(purpose)) continue;
    if (s.spawn === 'rift-transform' && !riftTransformed) continue;
    const p = wardPoint(id, redSide)!;
    const dist = near ? Math.hypot(p.x - near.x, p.y - near.y) : 0;
    // Faelights win ties: same ward, more vision.
    const score = s.value + (s.kind === 'faelight' ? 0.5 : 0) - dist * 4;
    if (!best || score > best.score) best = { id, score };
  }
  return best?.id ?? null;
}

/**
 * Vision-aware ward choice. Score = spot value × relevance
 *   − already covered (current vision from the fog mask around the spot, incl. existing wards)
 *   + expected enemy presence near the spot (fog prediction circles) = information gain
 *   − distance from me (walking there costs time).
 * Returns null when every relevant spot is already in vision.
 */
export function smartWardSpot(opts: {
  purposes: WardPurpose[];
  visible?: (x: number, y: number) => number;
  ghosts?: Array<{ x: number; y: number; radius: number }>;
  me?: { x: number; y: number } | null;
  redSide?: boolean;
  riftTransformed?: boolean;
}): { id: string; covered: number; score: number } | null {
  let best: { id: string; covered: number; score: number } | null = null;
  for (const [id, s] of Object.entries(WARD_SPOTS)) {
    const relevance = opts.purposes.filter(p => s.watches.includes(p)).length;
    if (!relevance) continue;
    if (s.spawn === 'rift-transform' && !opts.riftTransformed) continue;
    const p = wardPoint(id, Boolean(opts.redSide))!;
    // Mean current vision in a ring around the spot (≈ ward vision radius on the minimap).
    let covered = 0, n = 0;
    if (opts.visible) {
      for (let k = 0; k < 12; k++) for (const r of [0.0, 0.03, 0.055]) {
        const a = (k / 12) * Math.PI * 2;
        covered += opts.visible(Math.min(0.999, Math.max(0, p.x + Math.cos(a) * r)), Math.min(0.999, Math.max(0, p.y + Math.sin(a) * r)));
        n++;
      }
      covered /= n;
    }
    if (covered > 0.6) continue; // already watched (ally/ward there)
    let info = 0;
    for (const g of opts.ghosts ?? []) {
      const d = Math.hypot(g.x - p.x, g.y - p.y);
      if (d < g.radius + 0.06) info += 1 - d / (g.radius + 0.06);
    }
    const dist = opts.me ? Math.hypot(p.x - opts.me.x, p.y - opts.me.y) : 0;
    const score = s.value * (0.7 + 0.3 * relevance) * (1 - covered) + (s.kind === 'faelight' ? 0.5 : 0) + 1.5 * info - 3 * dist;
    if (!best || score > best.score) best = { id, covered, score };
  }
  return best;
}
