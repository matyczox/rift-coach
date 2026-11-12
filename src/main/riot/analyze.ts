import type { JunglePathing, OpponentProfile, Role } from '../../shared/intel';
import type { MatchDto, MatchParticipantDto, Position, TimelineDto, TimelineEventDto } from './types';

// Pure functions: match-v5 match + timeline JSON → opponent profile metrics.
//
// Summoner's Rift coordinates (match-v5 timeline): x right, y up, roughly
// 0..14870 x 0..14980. Blue (teamId 100, ORDER) base is bottom-left, red (200,
// CHAOS) top-right. Mid lane runs along x = y, the river along x + y ≈ 14820.
// Top lane hugs the left and top edges, bot lane the bottom and right edges.
//
// Jungle quadrants (lane-relative, identical for both teams):
//   own jungle   blue: x + y below the river     red: x + y above the river
//   top side     x < y (between top lane and mid) for BOTH teams
//   bot side     x > y
// "Mirroring" for red side therefore applies to which half is *own* jungle
// (and which is *enemy* jungle for invades); the top/bot label is the lane the
// quadrant borders, which is what matters to the coach ("starts bot → early bot
// gank"). E.g. blue's blue buff (3800,7900) and red's red buff (7100,10900)
// are both top-side starts; red's blue buff (11000,7000) is bot-side.
// (If you normalise red positions by the 180° rotation (x,y)→(W−x,W−y) instead,
// top and bot swap — classifyJungleSide handles this explicitly.)

export const MAP_SIZE = 14870;
export const RIVER_SUM = 14820;
const RIVER_HALF = 900;          // half-width of the river band, in x+y units
export const DRAGON_PIT: Position = { x: 9866, y: 4414 };
export const BARON_PIT: Position = { x: 5007, y: 10471 };
const BASE_EDGE = 4600;          // blue base: x,y < 4600; red base: x,y > MAP_SIZE-4600-~70

export type Lane = 'TOP' | 'MIDDLE' | 'BOTTOM';
export type TeamId = 100 | 200;

const MIN = 60_000;

// ---------------- geometry ----------------

export function inBase(p: Position): TeamId | null {
  if (p.x < BASE_EDGE && p.y < BASE_EDGE) return 100;
  if (p.x > RIVER_SUM - BASE_EDGE && p.y > RIVER_SUM - BASE_EDGE) return 200;
  return null;
}

function topArea(p: Position, m: number): boolean {
  const { x, y } = p;
  return (x < 1900 + m && y > 4500 - m)         // left edge
    || (y > 12900 - m && x < 10300 + m)          // top edge
    || (x < 3800 + m && y > 11000 - m);          // top-left corner (lane bend)
}

/**
 * Whether `p` lies in the given lane corridor (outside both bases). `margin`
 * widens the corridor (units), useful for fights at the lane's edge.
 * Strips are narrow enough to exclude gromp/krugs which sit close to lanes.
 */
export function inLaneArea(p: Position, lane: Lane, margin = 0): boolean {
  if (inBase(p) !== null) return false;
  switch (lane) {
    case 'TOP': return topArea(p, margin);
    case 'BOTTOM': return topArea({ x: p.y, y: p.x }, margin); // mirror over x = y
    case 'MIDDLE': {
      const s = p.x + p.y;
      return Math.abs(p.x - p.y) < 1200 + margin && s > 6600 && s < 23000;
    }
  }
}

export function classifyLane(p: Position, margin = 0): Lane | null {
  if (inLaneArea(p, 'TOP', margin)) return 'TOP';
  if (inLaneArea(p, 'BOTTOM', margin)) return 'BOTTOM';
  if (inLaneArea(p, 'MIDDLE', margin)) return 'MIDDLE';
  return null;
}

/** Which team's jungle `p` is in (not lane, not base, not river), or null. */
export function jungleOwner(p: Position): TeamId | null {
  if (inBase(p) !== null || classifyLane(p) !== null) return null;
  const s = p.x + p.y;
  if (s < RIVER_SUM - RIVER_HALF) return 100;
  if (s > RIVER_SUM + RIVER_HALF) return 200;
  return null;
}

/**
 * Side of `team`'s own jungle that `p` is in: 'top' (borders top lane) or
 * 'bot', or null if `p` is not in that team's jungle. The own-jungle test is
 * mirrored per team; the side label is lane-relative (x<y = top) for both.
 */
export function classifyJungleSide(p: Position, team: TeamId): 'top' | 'bot' | null {
  if (jungleOwner(p) !== team) return null;
  // Normalise to the team's own perspective (own base bottom-left)...
  const n = team === 100 ? p : { x: MAP_SIZE - p.x, y: MAP_SIZE - p.y };
  // ...where for blue x<y borders top lane, but for red the 180° rotation
  // maps their top-lane side onto x>y — so the comparison flips.
  const towardTop = team === 100 ? n.x < n.y : n.x > n.y;
  return towardTop ? 'top' : 'bot';
}

export function dist(a: Position, b: Position): number {
  return Math.hypot(a.x - b.x, a.y - b.y);
}

// ---------------- timeline helpers ----------------

export function findParticipant(match: MatchDto, puuid: string): MatchParticipantDto | null {
  return match.info.participants.find(p => p.puuid === puuid) ?? null;
}

/** participantId of `puuid` in a timeline (falls back to the match participant id). */
export function timelineParticipantId(tl: TimelineDto, puuid: string, fallback?: number): number | null {
  const fromInfo = tl.info.participants?.find(p => p.puuid === puuid)?.participantId;
  if (fromInfo) return fromInfo;
  const i = tl.metadata.participants.indexOf(puuid);
  if (i >= 0) return i + 1;
  return fallback ?? null;
}

export function involved(ev: TimelineEventDto, pid: number): boolean {
  return ev.killerId === pid || ev.victimId === pid || (ev.assistingParticipantIds?.includes(pid) ?? false);
}

function* events(tl: TimelineDto, type?: string): Generator<TimelineEventDto> {
  for (const f of tl.info.frames) for (const e of f.events ?? []) if (!type || e.type === type) yield e;
}

function sortedEvents(tl: TimelineDto, type: string): TimelineEventDto[] {
  return [...events(tl, type)].sort((a, b) => a.timestamp - b.timestamp);
}

/** Positions of `pid` per frame: [{t, pos}] ascending. */
function track(tl: TimelineDto, pid: number): Array<{ t: number; pos: Position }> {
  const out: Array<{ t: number; pos: Position }> = [];
  for (const f of tl.info.frames) {
    const pf = f.participantFrames?.[String(pid)];
    if (pf?.position) out.push({ t: f.timestamp, pos: pf.position });
  }
  return out.sort((a, b) => a.t - b.t);
}

/** Position of `pid` at frame minute `m` (frame closest to m*60s, within 20 s). */
export function positionAtMinute(tl: TimelineDto, pid: number, m: number): Position | null {
  let best: { d: number; pos: Position } | null = null;
  for (const s of track(tl, pid)) {
    const d = Math.abs(s.t - m * MIN);
    if (d <= 20_000 && (!best || d < best.d)) best = { d, pos: s.pos };
  }
  return best?.pos ?? null;
}

/** Linear interpolation of the per-minute track (coarse, but fine for "near the pit"). */
export function positionAt(tl: TimelineDto, pid: number, t: number): Position | null {
  const tr = track(tl, pid);
  if (!tr.length) return null;
  let prev = tr[0]!;
  if (t <= prev.t) return prev.pos;
  for (const s of tr) {
    if (s.t >= t) {
      const k = s.t === prev.t ? 0 : (t - prev.t) / (s.t - prev.t);
      return { x: prev.pos.x + (s.pos.x - prev.pos.x) * k, y: prev.pos.y + (s.pos.y - prev.pos.y) * k };
    }
    prev = s;
  }
  return prev.pos;
}

// ---------------- per-game metrics ----------------

export interface EarlyStats {
  kills15: number;
  deaths15: number;
  deaths10: number;
  /** Kills before 15:00 with no assisting participants. */
  soloKills15: number;
}

export function earlyStats(tl: TimelineDto, pid: number): EarlyStats {
  const r: EarlyStats = { kills15: 0, deaths15: 0, deaths10: 0, soloKills15: 0 };
  for (const e of events(tl, 'CHAMPION_KILL')) {
    if (e.timestamp >= 15 * MIN) continue;
    if (e.killerId === pid) {
      r.kills15++;
      if (!e.assistingParticipantIds || e.assistingParticipantIds.length === 0) r.soloKills15++;
    }
    if (e.victimId === pid) {
      r.deaths15++;
      if (e.timestamp < 10 * MIN) r.deaths10++;
    }
  }
  return r;
}

export function isSoloKill(e: TimelineEventDto): boolean {
  return e.type === 'CHAMPION_KILL' && !!e.killerId && e.killerId > 0 && (!e.assistingParticipantIds || e.assistingParticipantIds.length === 0);
}

export function laneOfPosition(teamPosition: string | undefined): Lane | null {
  switch (teamPosition) {
    case 'TOP': return 'TOP';
    case 'MIDDLE': return 'MIDDLE';
    case 'BOTTOM': case 'UTILITY': return 'BOTTOM';
    default: return null;
  }
}

/**
 * Laner roamed in this game: took part (kill, death or assist) in a champion
 * kill between 1:30 and 14:00 located outside their own lane corridor
 * (+1000 units tolerance) and outside their own base. Level-1 fights before
 * minions arrive (< 1:30) are not roams.
 */
export function roamedInGame(tl: TimelineDto, pid: number, lane: Lane, team: TeamId): boolean {
  for (const e of events(tl, 'CHAMPION_KILL')) {
    if (e.timestamp < 90_000 || e.timestamp >= 14 * MIN || !e.position || !involved(e, pid)) continue;
    if (inBase(e.position) === team) continue;
    if (!inLaneArea(e.position, lane, 1000)) return true;
  }
  return false;
}

export interface JungleGameStats {
  startSide: 'top' | 'bot' | null;
  firstGank: { minute: number; lane: Lane } | null;
  invaded: boolean;
  /** null when no dragon was taken in the game. */
  dragonPresent: boolean | null;
}

const GANK_AFTER = 150_000;   // 2:30
const GANK_UNTIL = 15 * MIN;
const INVADE_BEFORE = 4 * MIN;
const DRAGON_RADIUS = 2500;

export function analyzeJungleGame(tl: TimelineDto, pid: number, team: TeamId): JungleGameStats {
  // Start side: minute-2 position (at/just after the first camp), else minute 1.
  let startSide: 'top' | 'bot' | null = null;
  for (const m of [2, 1]) {
    const p = positionAtMinute(tl, pid, m);
    const side = p ? classifyJungleSide(p, team) : null;
    if (side) { startSide = side; break; }
  }

  // First gank: earliest kill/death/assist in a lane corridor (+600 tolerance)
  // or first per-minute position inside a lane corridor, after 2:30.
  let firstGank: { t: number; lane: Lane } | null = null;
  const consider = (t: number, lane: Lane | null) => {
    if (lane && t >= GANK_AFTER && t < GANK_UNTIL && (!firstGank || t < firstGank.t)) firstGank = { t, lane };
  };
  for (const e of events(tl, 'CHAMPION_KILL')) {
    if (e.position && involved(e, pid)) consider(e.timestamp, classifyLane(e.position, 600));
  }
  for (const s of track(tl, pid)) consider(s.t, classifyLane(s.pos));

  // Invade: any position (frame or own kill event) in the enemy jungle before 4:00.
  const enemy: TeamId = team === 100 ? 200 : 100;
  let invaded = track(tl, pid).some(s => s.t > 0 && s.t < INVADE_BEFORE && jungleOwner(s.pos) === enemy);
  if (!invaded) {
    for (const e of events(tl, 'CHAMPION_KILL')) {
      if (e.timestamp < INVADE_BEFORE && e.position && involved(e, pid) && jungleOwner(e.position) === enemy) { invaded = true; break; }
    }
  }

  // Dragon presence at the first dragon of the game (either team).
  let dragonPresent: boolean | null = null;
  const dragon = sortedEvents(tl, 'ELITE_MONSTER_KILL').find(e => e.monsterType === 'DRAGON');
  if (dragon) {
    if (dragon.killerId === pid || dragon.assistingParticipantIds?.includes(pid)) dragonPresent = true;
    else {
      const p = positionAt(tl, pid, dragon.timestamp);
      dragonPresent = !!p && dist(p, dragon.position ?? DRAGON_PIT) <= DRAGON_RADIUS;
    }
  }

  const fg = firstGank as { t: number; lane: Lane } | null;
  return { startSide, firstGank: fg ? { minute: fg.t / MIN, lane: fg.lane } : null, invaded, dragonPresent };
}

export interface GameStats {
  matchId: string;
  queueId: number;
  /** Epoch ms of game end (for recency ordering). */
  endedAt: number;
  durationSec: number;
  win: boolean;
  championName: string;
  championId: number;
  teamPosition: string;
  teamId: TeamId;
  kills: number; deaths: number; assists: number;
  csPerMin: number;
  fbParticipation: boolean;
  /** Whole-game solo kills from match challenges (if present). */
  challengeSoloKills: number | null;
  early: EarlyStats | null;           // needs timeline
  roamed: boolean | null;             // needs timeline + lane role
  jungle: JungleGameStats | null;     // needs timeline + JUNGLE role
}

export function durationSeconds(match: MatchDto): number {
  const d = match.info.gameDuration;
  return match.info.gameEndTimestamp !== undefined ? d : d / 1000;
}

/** True for remakes / early surrenders that should not count. */
export function isRemake(match: MatchDto, p?: MatchParticipantDto | null): boolean {
  return durationSeconds(match) < 300 || !!p?.gameEndedInEarlySurrender;
}

export function analyzeGame(match: MatchDto, timeline: TimelineDto | null, puuid: string): GameStats | null {
  const p = findParticipant(match, puuid);
  if (!p || isRemake(match, p)) return null;
  const durationSec = durationSeconds(match);
  const team: TeamId = p.teamId === 200 ? 200 : 100;
  const endedAt = match.info.gameEndTimestamp ?? (match.info.gameCreation + durationSec * 1000);
  const g: GameStats = {
    matchId: match.metadata.matchId,
    queueId: match.info.queueId,
    endedAt,
    durationSec,
    win: !!p.win,
    championName: p.championName,
    championId: p.championId,
    teamPosition: p.teamPosition ?? '',
    teamId: team,
    kills: p.kills, deaths: p.deaths, assists: p.assists,
    csPerMin: ((p.totalMinionsKilled ?? 0) + (p.neutralMinionsKilled ?? 0)) / Math.max(1, durationSec / 60),
    fbParticipation: !!(p.firstBloodKill || p.firstBloodAssist),
    challengeSoloKills: typeof p.challenges?.soloKills === 'number' ? p.challenges.soloKills : null,
    early: null, roamed: null, jungle: null,
  };
  if (timeline) {
    const pid = timelineParticipantId(timeline, puuid, p.participantId);
    if (pid) {
      g.early = earlyStats(timeline, pid);
      const lane = laneOfPosition(p.teamPosition);
      if (lane) g.roamed = roamedInGame(timeline, pid, lane, team);
      if (p.teamPosition === 'JUNGLE') g.jungle = analyzeJungleGame(timeline, pid, team);
    }
  }
  return g;
}

// ---------------- aggregation ----------------

/**
 * Aggression 0..1 (documented formula):
 *   A = 0.45 · min(1, (kills15 + deaths15) / 6)   early involvement per game
 *     + 0.30 · min(1, soloKills15 / 1.0)          solo kills before 15:00 per game
 *     + 0.25 · min(1, firstBloodRate / 0.4)       FB kill/assist share (baseline ≈ 0.2)
 * Deaths count as aggression: dying early usually means fighting early.
 */
export function aggressionScore(involvement15: number, soloKills15: number, firstBloodRate: number): number {
  const a = 0.45 * Math.min(1, involvement15 / 6)
    + 0.30 * Math.min(1, soloKills15 / 1.0)
    + 0.25 * Math.min(1, firstBloodRate / 0.4);
  return round(Math.max(0, Math.min(1, a)), 3);
}

export function recentForm(games: Array<{ win: boolean; endedAt: number }>): OpponentProfile['recentForm'] {
  const sorted = [...games].sort((a, b) => b.endedAt - a.endedAt);
  const wins = sorted.filter(g => g.win).length;
  let streak = 0;
  for (const g of sorted) {
    if (streak === 0) streak = g.win ? 1 : -1;
    else if (g.win && streak > 0) streak++;
    else if (!g.win && streak < 0) streak--;
    else break;
  }
  return { wins, losses: sorted.length - wins, streak };
}

export function normChampion(s: string): string {
  const n = s.toLowerCase().replace(/[^a-z0-9]/g, '');
  const alias: Record<string, string> = { wukong: 'monkeyking', nunuwillump: 'nunu', renataglasc: 'renata' };
  return alias[n] ?? n;
}

export function median(xs: number[]): number | null {
  if (!xs.length) return null;
  const s = [...xs].sort((a, b) => a - b);
  const m = s.length >> 1;
  return s.length % 2 ? s[m]! : (s[m - 1]! + s[m]!) / 2;
}

function round(x: number, d = 2): number { const k = 10 ** d; return Math.round(x * k) / k; }
const avg = (xs: number[]) => xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : 0;

export function aggregateJungle(games: GameStats[]): JunglePathing | null {
  const js = games.map(g => g.jungle).filter((j): j is JungleGameStats => !!j);
  if (!js.length) return null;
  const sided = js.filter(j => j.startSide);
  const top = sided.filter(j => j.startSide === 'top').length;
  const ganks = js.map(j => j.firstGank).filter((g): g is { minute: number; lane: Lane } => !!g);
  const laneDist: Partial<Record<Lane, number>> = {};
  for (const g of ganks) laneDist[g.lane] = (laneDist[g.lane] ?? 0) + 1;
  for (const k of Object.keys(laneDist) as Lane[]) laneDist[k] = round(laneDist[k]! / ganks.length, 3);
  const withDragon = js.filter(j => j.dragonPresent !== null);
  const med = median(ganks.map(g => g.minute));
  return {
    startTopSide: sided.length ? round(top / sided.length, 3) : 0,
    startBotSide: sided.length ? round((sided.length - top) / sided.length, 3) : 0,
    firstGankMinute: med === null ? null : round(med, 2),
    firstGankLane: laneDist,
    earlyInvadeRate: round(js.filter(j => j.invaded).length / js.length, 3),
    dragonPresence: withDragon.length ? round(withDragon.filter(j => j.dragonPresent).length / withDragon.length, 3) : 0,
    sampleGames: js.length,
  };
}

export interface ProfileInput {
  puuid: string;
  riotId: string;
  champion: string;
  role: Role;
  rank: string | null;
  games: GameStats[];
  /** Mastery points on `champion` if fetched (null = never played / unknown). */
  masteryPoints?: number | null;
  now?: number;
}

export function buildProfile(inp: ProfileInput): OpponentProfile {
  const games = inp.games;
  const n = games.length;
  const champ = normChampion(inp.champion);
  const onChamp = games.filter(g => normChampion(g.championName) === champ);
  const champWins = onChamp.filter(g => g.win).length;
  const fbRate = n ? games.filter(g => g.fbParticipation).length / n : 0;

  // Early stats: timeline games when available, otherwise a rough estimate
  // scaling whole-game numbers by the share of the game that was before 10/15 min.
  const tl = games.filter(g => g.early);
  let k15: number, d15: number, d10: number, solo15: number;
  if (tl.length) {
    k15 = avg(tl.map(g => g.early!.kills15));
    d15 = avg(tl.map(g => g.early!.deaths15));
    d10 = avg(tl.map(g => g.early!.deaths10));
    solo15 = avg(tl.map(g => g.early!.soloKills15));
  } else {
    const f = (g: GameStats, min: number) => Math.min(1, (min * 60) / Math.max(1, g.durationSec));
    k15 = avg(games.map(g => g.kills * f(g, 15)));
    d15 = avg(games.map(g => g.deaths * f(g, 15)));
    d10 = avg(games.map(g => g.deaths * f(g, 10)));
    solo15 = avg(games.map(g => (g.challengeSoloKills ?? 0) * f(g, 15)));
  }

  const roamGames = games.filter(g => g.roamed !== null && (inp.role === 'JUNGLE' || inp.role === 'UNKNOWN' || sameLane(g.teamPosition, inp.role)));
  const roamSample = roamGames.length ? roamGames : games.filter(g => g.roamed !== null);
  const roamRate = inp.role === 'JUNGLE' || !roamSample.length ? 0 : roamSample.filter(g => g.roamed).length / roamSample.length;

  const jungleGames = games.filter(g => g.jungle);
  const jungle = inp.role === 'JUNGLE' || (inp.role === 'UNKNOWN' && jungleGames.length * 2 >= tl.length && jungleGames.length > 0)
    ? aggregateJungle(jungleGames) : null;

  const profile: OpponentProfile = {
    puuid: inp.puuid,
    riotId: inp.riotId,
    champion: inp.champion,
    role: inp.role,
    rank: inp.rank,
    gamesAnalyzed: n,
    aggression: n ? aggressionScore(k15 + d15, solo15, fbRate) : 0,
    firstBloodRate: round(fbRate, 3),
    deathsBefore10: round(d10),
    killsBefore15: round(k15),
    roamRate: round(roamRate, 3),
    championGames: onChamp.length,
    championWinRate: onChamp.length ? round(champWins / onChamp.length, 3) : null,
    recentForm: recentForm(games),
    jungle,
    summary: '',
    fetchedAt: inp.now ?? Date.now(),
  };
  const laneGames = games.filter(g => g.teamPosition !== 'JUNGLE');
  profile.summary = summarize(profile, {
    csPerMin: avg((laneGames.length ? laneGames : games).map(g => g.csPerMin)),
    roamSample: roamSample.length,
    masteryPoints: inp.masteryPoints,
  });
  return profile;
}

function sameLane(teamPosition: string, role: Role): boolean {
  const a = laneOfPosition(teamPosition);
  return a !== null && a === laneOfPosition(role);
}

// ---------------- summary ----------------

const LANE_SHORT: Record<Lane, string> = { TOP: 'top', MIDDLE: 'mid', BOTTOM: 'bot' };

/** Polish plural: 1 → one, 2-4 (not 12-14) → few, else many. */
export function plPlural(n: number, one: string, few: string, many: string): string {
  if (n === 1) return one;
  const d = n % 10, h = n % 100;
  return d >= 2 && d <= 4 && (h < 12 || h > 14) ? few : many;
}

export function fmtMinute(min: number): string {
  const s = Math.round(min * 60);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}

export interface SummaryExtras { csPerMin?: number; roamSample?: number; masteryPoints?: number | null }

/**
 * One short Polish line (≤ ~6 words) with the 1–3 most decision-relevant facts,
 * e.g. "agresywny, start dół, gank bot ~3:20" or "pasywny farmer, 2 gry na postaci".
 */
export function summarize(p: OpponentProfile, x: SummaryExtras = {}): string {
  if (p.gamesAnalyzed === 0) return 'brak danych';
  // [order in the output line, priority, text]
  const facts: Array<{ order: number; prio: number; text: string }> = [];
  const enough = p.gamesAnalyzed >= 3;
  const isJungle = p.role === 'JUNGLE';

  if (enough) {
    if (p.aggression >= 0.6) facts.push({ order: 0, prio: isJungle ? 0.8 : 0.9, text: 'agresywny' });
    else if (p.aggression <= 0.3) {
      const farmer = !isJungle && (x.csPerMin ?? 0) >= 7;
      facts.push({ order: 0, prio: isJungle ? 0.6 : 0.75, text: farmer ? 'pasywny farmer' : 'pasywny' });
    }
  }

  const j = p.jungle;
  if (j && j.sampleGames >= 3) {
    if (j.startBotSide >= 0.6) facts.push({ order: 1, prio: 0.9 + j.startBotSide / 100, text: 'start dół' });
    else if (j.startTopSide >= 0.6) facts.push({ order: 1, prio: 0.9 + j.startTopSide / 100, text: 'start góra' });
    const lanes = Object.entries(j.firstGankLane) as Array<[Lane, number]>;
    const top = lanes.sort((a, b) => b[1] - a[1])[0];
    if (top && top[1] >= 0.5 && j.firstGankMinute !== null) {
      facts.push({ order: 2, prio: 0.85, text: `gank ${LANE_SHORT[top[0]]} ~${fmtMinute(j.firstGankMinute)}` });
    }
    if (j.earlyInvadeRate >= 0.4) facts.push({ order: 3, prio: 0.7, text: 'invaduje' });
  }

  if (!isJungle && (x.roamSample ?? 0) >= 3 && p.roamRate >= 0.4) facts.push({ order: 4, prio: 0.85, text: 'roamuje' });

  if (p.championGames === 0) {
    if (x.masteryPoints === null || (x.masteryPoints !== undefined && x.masteryPoints < 20_000)) {
      facts.push({ order: 5, prio: 0.82, text: 'nowa postać' });
    }
  } else if (p.championGames <= 2 && p.gamesAnalyzed >= 5) {
    facts.push({ order: 5, prio: 0.8, text: `${p.championGames} ${plPlural(p.championGames, 'gra', 'gry', 'gier')} na postaci` });
  } else if (p.championGames >= 6 && p.championGames / p.gamesAnalyzed >= 0.6) {
    facts.push({ order: 5, prio: 0.55, text: 'main postaci' });
  }

  const s = p.recentForm.streak;
  if (s <= -3) facts.push({ order: 6, prio: 0.65, text: `${-s} ${plPlural(-s, 'porażka', 'porażki', 'porażek')} z rzędu` });
  else if (s >= 4) facts.push({ order: 6, prio: 0.5, text: `${s} ${plPlural(s, 'wygrana', 'wygrane', 'wygranych')} z rzędu` });

  let chosen = facts.sort((a, b) => b.prio - a.prio).slice(0, 3);
  const words = (fs: typeof chosen) => fs.reduce((n, f) => n + f.text.split(' ').length, 0);
  while (chosen.length > 1 && words(chosen) > 6) chosen = chosen.slice(0, -1);
  if (!chosen.length) return enough ? 'bez wyraźnych wzorców' : 'mało danych';
  return chosen.sort((a, b) => a.order - b.order).map(f => f.text).join(', ');
}
