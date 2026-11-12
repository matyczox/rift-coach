import type { AlertState } from '../../shared/intel';
import type { EnemyTrack } from './tracker';

/**
 * Invade / gank / missing alerts from fog predictions.
 * `me` = my champion's minimap position (from the detector), team = my team.
 */
export function trackerAlerts(tracks: EnemyTrack[], me: { x: number; y: number } | null, myTeam: 'ORDER' | 'CHAOS', gameTime: number, now: number): AlertState[] {
  const out: AlertState[] = [];
  const fog = tracks.filter(t => t.state === 'fog' && t.estimate && t.fogSince !== null);
  const near = (t: EnemyTrack, p: { x: number; y: number }, r: number) => {
    const e = t.estimate!;
    const d = Math.hypot(e.x - p.x, e.y - p.y);
    // Probability-ish: overlap of the ghost area with a radius-r circle around p.
    if (d <= Math.max(0.0001, r - e.radius)) return 1;
    if (d >= r + e.radius) return 0;
    return Math.max(0, Math.min(1, (r + e.radius - d) / (2 * Math.min(r, e.radius) + 1e-6)));
  };

  if (me) {
    // My jungle quadrant (blue: bottom-left of the mid diagonal, red: top-right).
    const inMyJungle = myTeam === 'ORDER' ? me.x + me.y > 0.98 && me.x < 0.62 && me.y > 0.38 : me.x + me.y < 1.02 && me.x > 0.38 && me.y < 0.62;
    const jungler = fog.find(t => t.role === 'JUNGLE');
    if (inMyJungle && jungler) {
      const p = near(jungler, me, 0.14);
      if (p > 0.45) out.push({ kind: 'invade', text: 'Możliwy invade', confidence: p, expiresAt: now + 4000 });
    }
    // Gank: any fogged enemy whose area reaches me.
    let best = 0;
    for (const t of fog) best = Math.max(best, near(t, me, 0.11));
    if (!out.length && best > 0.5) out.push({ kind: 'gank', text: 'Możliwy gank', confidence: best, expiresAt: now + 4000 });
  }

  // Many enemies unseen for a while (mid/late game).
  const longMissing = fog.filter(t => gameTime - (t.fogSince ?? gameTime) > 20).length;
  if (gameTime > 12 * 60 && longMissing >= 3 && !out.length) out.push({ kind: 'missing', text: `${longMissing} wrogów niewidocznych`, confidence: Math.min(1, longMissing / 5), expiresAt: now + 4000 });
  return out;
}
