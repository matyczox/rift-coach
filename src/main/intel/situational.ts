import type { AlertState, DangerState, GameIntel, ObjectiveCall, PlayerIntel } from '../../shared/intel';
import type { EnemyTrack } from '../tracker/tracker';

/** Minimap positions of objectives (normalized). */
const OBJ_POS: Record<string, { x: number; y: number }> = { dragon: { x: 0.67, y: 0.70 }, baron: { x: 0.33, y: 0.30 }, herald: { x: 0.33, y: 0.30 }, grubs: { x: 0.33, y: 0.30 } };

export interface Pos { x: number; y: number }
export interface SeenUnit { champion: string; x: number; y: number; vx?: number; vy?: number }

/**
 * Fed = clearly ahead of the lobby: big K-D or gold lead over the game average.
 * Thresholds scale a little with game time.
 */
export function fedChampions(intel: GameIntel): PlayerIntel[] {
  const avgGold = intel.players.reduce((s, p) => s + p.estGold, 0) / Math.max(1, intel.players.length);
  return intel.players.filter(p => {
    const kd = p.kills - p.deaths;
    const goldLead = p.estGold - avgGold;
    return (kd >= 4 && p.kills >= 5) || goldLead > 1500 + intel.gameTime * 0.8 || (kd >= 3 && goldLead > 1000);
  });
}

/** Alert when an enemy becomes fed (call once per update; keeps its own memory). */
export class FedWatcher {
  private known = new Set<string>();
  update(intel: GameIntel, now: number): AlertState[] {
    const out: AlertState[] = [];
    const fed = fedChampions(intel).filter(p => !p.isAlly);
    for (const p of fed) {
      if (this.known.has(p.champion)) continue;
      this.known.add(p.champion);
      const avg = intel.players.reduce((s, x) => s + x.estGold, 0) / intel.players.length;
      out.push({ kind: 'fed', text: `${p.champion} nafeedowany: ${p.kills}/${p.deaths}, +${(Math.round((p.estGold - avg) / 100) / 10).toFixed(1)}k`, confidence: 1, expiresAt: now + 9000 });
    }
    for (const c of [...this.known]) if (!fed.some(p => p.champion === c)) this.known.delete(c);
    return out;
  }
}

const ghostThreat = (t: EnemyTrack, me: Pos, r: number) => {
  if (t.state !== 'fog' || !t.estimate) return 0;
  const d = Math.hypot(t.estimate.x - me.x, t.estimate.y - me.y);
  if (d >= r + t.estimate.radius) return 0;
  const overlap = Math.min(1, (r + t.estimate.radius - d) / (2 * Math.min(r, t.estimate.radius) + 1e-6));
  // A huge circle (long in fog) says little.
  return overlap * Math.max(0.25, 1 - t.estimate.radius / 0.45);
};

/** Danger at my position: nearby visible enemies, fog circles reaching me, my HP, matchup, fed enemies. */
export function dangerAt(intel: GameIntel, me: Pos | null, tracks: EnemyTrack[], visibleEnemies: SeenUnit[], pMe: number | null, fed: string[]): DangerState | null {
  if (!intel.me || intel.me.isDead || !me) return null;
  const reasons: Array<{ w: number; text: string }> = [];
  let score = 0;
  for (const e of visibleEnemies) {
    const d = Math.hypot(e.x - me.x, e.y - me.y);
    if (d > 0.14) continue;
    const w = (1 - d / 0.14) * (fed.includes(e.champion) ? 1.6 : 1) * 0.45;
    score += w; reasons.push({ w, text: `${e.champion} blisko${fed.includes(e.champion) ? ' (fed)' : ''}` });
  }
  for (const t of tracks) {
    const w = ghostThreat(t, me, 0.12) * (fed.includes(t.champion) ? 1.5 : 1) * 0.35;
    if (w > 0.03) { score += w; reasons.push({ w, text: `${t.champion} we mgle blisko` }); }
  }
  const hp = intel.myStats && intel.myStats.maxHp ? intel.myStats.hp / intel.myStats.maxHp : 1;
  if (hp < 0.5) { const w = (0.5 - hp) * 0.8; score += w; reasons.push({ w, text: `Mało HP (${Math.round(hp * 100)}%) → back` }); }
  if (pMe !== null && pMe < 0.4 && score > 0.1) { const w = (0.4 - pMe) * 0.6; score += w; reasons.push({ w, text: 'słabszy w matchupie' }); }
  const missing = tracks.filter(t => t.state === 'fog').length;
  if (missing >= 3) { const w = 0.05 * missing; score += w; reasons.push({ w, text: `${missing} wrogów niewidocznych` }); }
  const level = Math.max(0, Math.min(1, score));
  const label: DangerState['label'] = level < 0.25 ? 'bezpiecznie' : level < 0.5 ? 'uważaj' : level < 0.75 ? 'niebezpiecznie' : 'uciekaj';
  return { level, label, reasons: reasons.sort((a, b) => b.w - a.w).slice(0, 2).map(r => r.text) };
}

/**
 * Go / contest / skip for the next objective within 60 s (or alive):
 * numbers alive, allies near it vs enemies near/likely near it, gold.
 */
export function objectiveCall(intel: GameIntel, tracks: EnemyTrack[], allies: SeenUnit[], visibleEnemies: SeenUnit[]): ObjectiveCall | null {
  const o = intel.objectives
    .filter(x => (x.kind === 'dragon' || x.kind === 'baron' || x.kind === 'herald' || x.kind === 'grubs') && (x.alive || (x.spawnsAt !== null && x.spawnsAt - intel.gameTime <= 60)))
    .sort((a, b) => (a.alive ? 0 : a.spawnsAt! - intel.gameTime) - (b.alive ? 0 : b.spawnsAt! - intel.gameTime))[0];
  if (!o) return null;
  const pos = OBJ_POS[o.kind]!;
  const near = (p: Pos, r: number) => Math.hypot(p.x - pos.x, p.y - pos.y) <= r;
  const allyAlive = intel.ally.aliveCount, enemyAlive = intel.enemy.aliveCount;
  const alliesNear = allies.filter(a => near(a, 0.2)).length;
  const enemiesSeenNear = visibleEnemies.filter(e => near(e, 0.2)).length;
  const enemiesLikelyNear = tracks.filter(t => t.state === 'fog' && t.estimate && near(t.estimate, 0.2 + t.estimate.radius * 0.5)).length;
  const enemyNear = enemiesSeenNear + 0.6 * enemiesLikelyNear;
  const goldK = intel.goldDiff / 1000;
  const score = (allyAlive - enemyAlive) * 0.9 + (alliesNear - enemyNear) * 0.6 + goldK * 0.25 + (o.kind === 'baron' ? -0.4 : 0);
  let verdict: ObjectiveCall['verdict'], reason: string;
  const deadEnemies = 5 - enemyAlive;
  if (score >= 1.2) { verdict = 'go'; reason = deadEnemies >= 2 ? `${deadEnemies} wrogów martwych` : alliesNear >= 3 ? `${alliesNear} waszych blisko` : 'przewaga'; }
  else if (score <= -0.8) { verdict = 'skip'; reason = allyAlive < enemyAlive ? `${enemyAlive}v${allyAlive}, oddaj` : 'wrogowie bliżej, oddaj'; }
  else { verdict = 'contest'; reason = 'wyrównane, tylko z wizją'; }
  const label = { dragon: 'Smok', baron: 'Baron', herald: 'Herold', grubs: 'Larwy' }[o.kind as 'dragon'] ?? o.label;
  return { objective: label, verdict, reason, inSec: o.alive ? 0 : Math.round(o.spawnsAt! - intel.gameTime), x: pos.x, y: pos.y };
}

/**
 * Teamplay prompts: my jungler next to my lane while I'm stronger → ask for the gank;
 * allies walking towards me → get ready to fight.
 */
export class TeamWatcher {
  private lastAsk = 0;
  private lastAlly = 0;
  update(intel: GameIntel, me: Pos | null, allies: SeenUnit[], visibleEnemies: SeenUnit[], pMe: number | null, now: number): AlertState[] {
    const out: AlertState[] = [];
    if (!intel.me || !me || intel.me.isDead) return out;
    const jungler = intel.players.find(p => p.isAlly && !p.isMe && p.role === 'JUNGLE');
    const jPos = jungler ? allies.find(a => a.champion === jungler.champion) : undefined;
    const enemyNearMe = visibleEnemies.some(e => Math.hypot(e.x - me.x, e.y - me.y) < 0.12);
    if (jPos && Math.hypot(jPos.x - me.x, jPos.y - me.y) < 0.18 && enemyNearMe && (pMe ?? 0.5) >= 0.42 && now - this.lastAsk > 30000) {
      this.lastAsk = now;
      out.push({ kind: 'gank-ask', text: `${jungler!.champion} obok → pinguj, wchodźcie`, confidence: 0.8, expiresAt: now + 6000 });
    }
    // Allies heading my way: within 0.25 and closing in.
    const coming = allies.filter(a => {
      if (a.champion === intel.me!.champion || a.vx === undefined || a.vy === undefined) return false;
      const dx = me.x - a.x, dy = me.y - a.y, d = Math.hypot(dx, dy);
      const sp = Math.hypot(a.vx, a.vy);
      return d < 0.25 && d > 0.05 && sp > 0.004 && (dx * a.vx + dy * a.vy) / (d * sp) > 0.8;
    });
    if (coming.length >= 1 && enemyNearMe && now - this.lastAlly > 20000) {
      this.lastAlly = now;
      out.push({ kind: 'ally', text: `${coming.map(c => c.champion).slice(0, 2).join(' i ')} ${coming.length > 1 ? 'idą' : 'idzie'} do ciebie`, confidence: 0.75, expiresAt: now + 6000 });
    }
    return out;
  }
}
