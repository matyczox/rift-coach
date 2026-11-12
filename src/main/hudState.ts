import type { AlertState, CoachAdvice, GameIntel, HudState, LoadingCardState, MinimapGhost, MinimapHint, OpponentProfile, PowerBarState, BuildPanelState, DangerState, ObjectiveCall } from '../shared/intel';
import { Coach } from './coach/coach';
import { smartWardSpot, wardPoint, WARD_SPOTS, type WardPurpose } from '../shared/wards';

/** Fixed points on the minimap, normalized 0..1 (blue base bottom-left; the minimap is never flipped). */
export const MAP_POINTS: Record<string, { x: number; y: number }> = {
  dragon: { x: 0.67, y: 0.70 },
  baron: { x: 0.33, y: 0.30 },
  herald: { x: 0.33, y: 0.30 },
  grubs: { x: 0.33, y: 0.30 },
  atakhan: { x: 0.5, y: 0.5 },
  top: { x: 0.10, y: 0.10 },
  mid: { x: 0.50, y: 0.50 },
  bot: { x: 0.90, y: 0.90 },
  base: { x: 0.06, y: 0.94 },
  'my-jungle': { x: 0.27, y: 0.62 },
  'enemy-jungle': { x: 0.73, y: 0.38 },
};

/** Quick local alerts that don't need the vision system. */
export function localAlerts(intel: GameIntel, now: number): AlertState[] {
  const out: AlertState[] = [];
  const me = intel.me;
  if (!me) return out;
  // Lane opponent just spiked and I haven't matched it.
  for (const p of intel.players) {
    if (p.isAlly || p.role !== me.role || p.role === 'UNKNOWN') continue;
    const s = p.spikes[0];
    if (!s || intel.gameTime - s.at > 45) continue;
    const behind = s.kind === 'level' ? me.level < p.level : me.completedItems < p.completedItems;
    if (behind) out.push({ kind: 'spike', text: `${p.champion}: ${s.label}`, confidence: 1, expiresAt: now + 8000 });
  }
  // Buff windows and aces: tell when to defend / when to push.
  const fmtT = (s: number) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, '0')}`;
  for (const b of intel.buffs) {
    const name = b.kind === 'baron' ? 'Baron' : 'Starszy';
    out.push(b.team === 'enemy'
      ? { kind: 'missing', text: `Wróg ma ${name} do ${fmtT(b.until)} — broń pod wieżą`, confidence: 1, expiresAt: now + 3000 }
      : { kind: 'ally', text: `Macie ${name} do ${fmtT(b.until)} — push`, confidence: 1, expiresAt: now + 3000 });
  }
  if (intel.ace) out.push(intel.ace.team === 'ally'
    ? { kind: 'ally', text: 'ACE — push / Baron teraz', confidence: 1, expiresAt: now + 3000 }
    : { kind: 'missing', text: 'Wróg zrobił ACE — cofnij się', confidence: 1, expiresAt: now + 3000 });
  for (const p of intel.players) {
    const s = intel.streaks[p.champion] ?? 0;
    if (!p.isAlly && !p.isDead && s >= 4) out.push({ kind: 'fed', text: `${p.champion}: ${s} killi z rzędu — duży shutdown`, confidence: 0.9, expiresAt: now + 3000 });
  }
  return out;
}

export interface HudInputs {
  visible: boolean;
  intel: GameIntel | null;
  advice: CoachAdvice | null;
  alerts: AlertState[];
  ghosts: MinimapGhost[];
  profiles: OpponentProfile[];
  championIcon: (champion: string) => string;
  showFrame: boolean;
  recording: boolean;
  power?: PowerBarState | null;
  build?: BuildPanelState | null;
  danger?: DangerState | null;
  objective?: ObjectiveCall | null;
  fed?: string[];
  /** P(point is in our vision now), from the fog mask (includes existing wards). */
  visionAt?: (x: number, y: number) => number;
  me?: { x: number; y: number } | null;
}

const fmt = (s: number) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, '0')}`;

export function composeHud(i: HudInputs): HudState {
  const now = Date.now();
  const intel = i.intel;
  const advice = i.advice && intel && Coach.isFresh(i.advice, intel) ? i.advice : null;
  const gameTime = intel?.gameTime ?? 0;

  const nextObj = intel?.objectives
    .filter(o => o.spawnsAt && o.spawnsAt - gameTime <= 90 && o.kind !== 'inhibitor')
    .sort((a, b) => a.spawnsAt! - b.spawnsAt!)[0];

  const hints: MinimapHint[] = [];
  if (advice?.mapHint) {
    const p = MAP_POINTS[advice.mapHint.target] ?? (advice.mapHint.x !== undefined ? { x: advice.mapHint.x, y: advice.mapHint.y! } : null);
    if (p) hints.push({ kind: advice.mapHint.type === 'avoid' ? 'avoid' : advice.mapHint.type === 'objective' ? 'objective' : 'move', x: p.x, y: p.y, expiresAt: now + 15000 });
  }

  // Wards: coach suggestion, else a local rule 60–90 s before dragon/baron.
  const red = intel?.me?.team === 'CHAOS';
  if (advice?.ward) {
    const p = wardPoint(advice.ward.spot, red);
    if (p) hints.push({ kind: 'ward', x: p.x, y: p.y, label: WARD_SPOTS[advice.ward.spot]?.kind === 'plant' ? 'roślina' : WARD_SPOTS[advice.ward.spot]?.kind === 'faelight' ? 'faelight' : 'ward', expiresAt: now + 15000 });
  } else if (intel && !intel.me?.isDead) {
    // Vision-aware: skip spots our team already sees (allies, existing wards), prefer
    // spots where an enemy probably is (fog circles) and that are close to me.
    const transformed = (intel.ally.dragons.length + intel.enemy.dragons.length) >= 2;
    const obj = intel.objectives.find(o => (o.kind === 'dragon' || o.kind === 'baron') && o.spawnsAt && o.spawnsAt - gameTime <= 90 && o.spawnsAt - gameTime >= 30);
    const enemyJg = i.ghosts.find(g => intel.players.some(p => !p.isAlly && p.role === 'JUNGLE' && p.champion === g.champion) && g.age > 20);
    let purposes: WardPurpose[] | null = null;
    if (obj) purposes = [obj.kind === 'dragon' ? 'dragon' : 'baron'];
    else if (enemyJg && i.me) {
      const lane: WardPurpose = i.me.x + i.me.y < 0.75 ? 'gank-top' : i.me.x + i.me.y > 1.25 ? 'gank-bot' : 'gank-mid';
      purposes = ['enemy-jungler', lane];
    }
    if (purposes) {
      const pick = smartWardSpot({ purposes, visible: i.visionAt, ghosts: i.ghosts, me: i.me, redSide: red, riftTransformed: transformed });
      const p = pick ? wardPoint(pick.id, red) : null;
      if (p && pick) hints.push({ kind: 'ward', x: p.x, y: p.y, label: WARD_SPOTS[pick.id]!.kind === 'plant' ? 'roślina' : WARD_SPOTS[pick.id]!.kind === 'faelight' ? 'faelight' : 'ward', expiresAt: now + 3000 });
    }
  }

  const dead = Boolean(intel?.me?.isDead);
  const loadingCard: LoadingCardState | null = intel && gameTime < 25 && i.profiles.length
    ? { enemies: i.profiles.map(p => ({ champion: p.champion, role: p.role, iconUrl: i.championIcon(p.champion), summary: p.summary, rank: p.rank })) }
    : null;

  return {
    visible: i.visible && Boolean(intel),
    gameTime,
    topBar: advice && !dead ? { call: advice.call, tone: advice.tone, tag: nextObj ? `${nextObj.label.toLowerCase()} ${fmt(nextObj.spawnsAt! - gameTime)}` : undefined, expiresAt: now + 3000 } : null,
    alert: i.alerts.filter(a => a.expiresAt > now).sort((a, b) => b.confidence - a.confidence)[0] ?? null,
    deathCard: dead && intel?.me ? {
      title: `Nie żyjesz · respawn ${fmt(intel.me.respawnIn)}`,
      points: advice?.deathReview ?? ['Analizuję…'],
      respawnAt: gameTime + intel.me.respawnIn,
    } : null,
    loadingCard,
    power: !dead && !loadingCard ? i.power ?? null : null,
    build: i.build ?? null,
    danger: !dead ? i.danger ?? null : null,
    objective: i.objective ?? null,
    fed: i.fed ?? [],
    ghosts: i.ghosts,
    hints,
    timers: (intel?.objectives ?? [])
      .filter(o => o.spawnsAt && o.spawnsAt - gameTime <= 60 && MAP_POINTS[o.kind])
      .map(o => ({ kind: o.kind, label: o.label.toLowerCase(), ...MAP_POINTS[o.kind]!, secondsLeft: Math.round(o.spawnsAt! - gameTime) })),
    showFrame: i.showFrame,
    recording: i.recording,
  };
}
