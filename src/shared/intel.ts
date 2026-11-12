// Contracts for game intel (System 1 derived state), opponent profiles and the
// Claude coach (System 2). Shared by main and the overlay renderers.

export type Team = 'ORDER' | 'CHAOS';
export type Role = 'TOP' | 'JUNGLE' | 'MIDDLE' | 'BOTTOM' | 'UTILITY' | 'UNKNOWN';

export interface PlayerIntel {
  champion: string;
  team: Team;
  isAlly: boolean;
  isMe: boolean;
  role: Role;
  level: number;
  kills: number; deaths: number; assists: number; cs: number; wardScore: number;
  isDead: boolean;
  respawnIn: number;               // seconds, 0 if alive
  /** Gold value of current items (shop prices). */
  itemGold: number;
  /** Gold of shop-only items (recall proof); evolving items excluded. */
  shopGold: number;
  /** Estimated total gold earned (items + estimated unspent). */
  estGold: number;
  items: Array<{ id: number; name: string; price: number }>;
  /** Completed legendary items (non-component, price >= 2200 or flagged). */
  completedItems: number;
  /** Power spikes reached recently (level 6/11/16, item completion), newest first. */
  spikes: Array<{ kind: 'level' | 'item'; label: string; at: number }>;
  summonerSpells: string[];
  /** Enemy HP fraction read from its health bar on screen (0..1), null if never seen. */
  hpPct: number | null;
  /** Game time the HP was last read (stale after a few seconds). */
  hpSeenAt: number | null;
  /** Movement speed: exact for me (Live API), estimated for others (base + items, soft-capped). */
  moveSpeed: number;
  /** Seconds until Flash is back, if we saw it used (manual hotkey) — null unknown. */
  flashIn: number | null;
}

export interface ObjectiveTimer {
  kind: 'dragon' | 'baron' | 'herald' | 'grubs' | 'atakhan' | 'inhibitor';
  label: string;            // e.g. "Smok (Chemtech)", "Inhib bot (wróg)"
  /** Game time it (re)spawns; null = alive/available now. */
  spawnsAt: number | null;
  alive: boolean;
}

export interface TeamIntel {
  kills: number;
  dragons: string[];        // dragon types taken
  towers: number;
  inhibitors: number;
  barons: number;
  heralds: number;
  estGold: number;
  aliveCount: number;
}

export interface GameIntel {
  gameTime: number;
  gameMode: string;
  me: PlayerIntel | null;
  players: PlayerIntel[];
  ally: TeamIntel;
  enemy: TeamIntel;
  /** Gold difference (ally - enemy), estimated. */
  goldDiff: number;
  objectives: ObjectiveTimer[];
  /** Recent notable events, newest first (max ~10), human readable Polish. */
  recent: Array<{ at: number; text: string }>;
  /** My deaths with context (killer, helpers), newest first. */
  myDeaths: Array<{ at: number; killer: string; assisters: string[]; enemiesInvolved: number; wasGank: boolean }>;
  /** Active Baron / Elder buffs. */
  buffs: Array<{ kind: 'baron' | 'elder'; team: 'ally' | 'enemy'; until: number }>;
  /** Current kill streak (kills since last death) per champion. */
  streaks: Record<string, number>;
  /** Game time of the last ace, if within the last 60 s. */
  ace: { team: 'ally' | 'enemy'; at: number } | null;
  /** My own stats from the active player block. */
  myStats: { gold: number; hp: number; maxHp: number; ad: number; ap: number; armor: number; mr: number; moveSpeed: number } | null;
}

// ---------- opponent profiles (Riot API, M5) ----------

export interface JunglePathing {
  /** Share of games starting on top-side / bot-side of their own jungle. */
  startTopSide: number;
  startBotSide: number;
  /** Median first-gank minute and its lane distribution (0..1). */
  firstGankMinute: number | null;
  firstGankLane: Partial<Record<'TOP' | 'MIDDLE' | 'BOTTOM', number>>;
  /** Share of games where they entered the enemy jungle before 4:00. */
  earlyInvadeRate: number;
  /** Share of first dragons / heralds they were present at. */
  dragonPresence: number;
  sampleGames: number;
}

export interface OpponentProfile {
  puuid: string;
  riotId: string;
  champion: string;
  role: Role;
  rank: string | null;            // "GOLD II 45 LP"
  gamesAnalyzed: number;
  /** 0..1, derived from early kills+deaths, solo kills, first blood rate. */
  aggression: number;
  firstBloodRate: number;
  deathsBefore10: number;         // per game average
  killsBefore15: number;          // per game average
  roamRate: number;               // laners: share of games with kills/assists in other lanes before 14:00
  championGames: number;          // games on this champion in the sample / season
  championWinRate: number | null;
  recentForm: { wins: number; losses: number; streak: number }; // streak >0 wins, <0 losses
  jungle: JunglePathing | null;
  /** One-line Polish summary for the loading screen, e.g. "agresywny, start dół, gank bot ~3:20". */
  summary: string;
  fetchedAt: number;
}

// ---------- coach (System 2) ----------

export type CoachTone = 'go' | 'wait' | 'danger' | 'farm';

export interface CoachAdvice {
  /** ≤ ~8 words, shown in the top bar. */
  call: string;
  tone: CoachTone;
  mapHint: { type: 'objective' | 'lane' | 'jungle' | 'base' | 'avoid'; target: string; x?: number; y?: number } | null;
  item: string | null;
  /** Where to place a ward now (spot name from WARD_SPOTS, relative to my side). */
  ward: { spot: string; reason: string } | null;
  /** Only when the player is dead: 3–5 short bullet points. */
  deathReview: string[] | null;
  ttlSec: number;
  /** Set by the app. */
  contextId: string;
  createdAt: number;
  gameTime: number;
  latencyMs: number;
}

export type CoachState = 'off' | 'not-logged-in' | 'idle' | 'thinking' | 'error';

export interface CoachStatus {
  state: CoachState;
  model: string;
  calls: number;
  lastError: string | null;
  lastLatencyMs: number | null;
}

// ---------- in-game overlay windows ----------

/** A fog-of-war prediction drawn on the minimap (System 1, filled in M3). */
export interface MinimapGhost {
  champion: string;
  /** Normalized minimap coordinates 0..1 (x right, y down). */
  x: number; y: number;
  /** Radius of the ~70% region, normalized to minimap width. */
  radius: number;
  /** Seconds since last seen. */
  age: number;
  confidence: number;      // 0..1
  iconUrl: string;
}

export interface MinimapHint {
  kind: 'objective' | 'move' | 'avoid' | 'invade' | 'ward';
  x: number; y: number;
  from?: { x: number; y: number };
  label?: string;
  expiresAt: number;
}

export interface TopBarState {
  call: string;
  tone: CoachTone;
  /** Optional small right-aligned tag, e.g. "Smok 0:48". */
  tag?: string;
  expiresAt: number;
}

export interface AlertState {
  kind: 'invade' | 'gank' | 'missing' | 'spike' | 'fed' | 'ally' | 'gank-ask';
  text: string;            // ≤ 6 words
  confidence: number;      // 0..1
  expiresAt: number;
}

export interface DeathCardState {
  title: string;           // e.g. "Zginąłeś · respawn 0:24"
  points: string[];        // 3–5
  respawnAt: number;       // game time
}

export interface LoadingCardState {
  enemies: Array<{ champion: string; role: Role; iconUrl: string; summary: string; rank: string | null }>;
}

/** Lane matchup power bar (me vs lane opponent). */
export interface PowerBarState {
  me: { champion: string; iconUrl: string; level: number };
  enemy: { champion: string; iconUrl: string; level: number };
  /** Estimated chance I win an all-in right now, 0..1. */
  pMe: number;
  /** Small line under the bar, e.g. "lvl 5 vs 6 · +450 g". */
  detail: string;
  /** Advice line, e.g. "Silniejszy od 6 lvl → wchodź po ulcie". */
  advice: string;
  /** Matchup winrate from meta stats (patch), if known. */
  metaWinRate: number | null;
  /** Lead over the lane opponent (me minus them): item gold, CS, level. */
  lane?: { gold: number; cs: number; level: number };
}

/** Next items to buy (AI build advisor), shown next to the minimap. */
export interface BuildPanelState {
  items: Array<{ name: string; iconUrl: string; why: string; cost: number | null }>;
  /** e.g. "wczesna gra", "środek gry", "późna gra" */
  phase: string;
  updatedAt: number;
}

/** Overall danger at my position right now. */
export interface DangerState {
  /** 0..1 */
  level: number;
  label: 'bezpiecznie' | 'uważaj' | 'niebezpiecznie' | 'uciekaj';
  /** Top 1–2 reasons, short Polish, e.g. "Hecarim we mgle blisko". */
  reasons: string[];
}

/** Go / contest / skip call for the next objective. */
export interface ObjectiveCall {
  objective: string;          // "Smok", "Baron", "Herold", "Larwy"
  verdict: 'go' | 'contest' | 'skip';
  reason: string;             // ≤ 6 words
  inSec: number | null;       // seconds to spawn; null/0 = alive
  x: number; y: number;       // minimap position of the objective
}

/** Everything the in-game overlay windows render. Pushed ~10x/s. */
export interface HudState {
  visible: boolean;
  gameTime: number;
  topBar: TopBarState | null;
  alert: AlertState | null;
  deathCard: DeathCardState | null;
  loadingCard: LoadingCardState | null;
  power: PowerBarState | null;
  build: BuildPanelState | null;
  danger: DangerState | null;
  objective: ObjectiveCall | null;
  /** Champions currently considered fed (stronger ghost colour, badges). */
  fed: string[];
  ghosts: MinimapGhost[];
  hints: MinimapHint[];
  timers: Array<{ kind: ObjectiveTimer['kind']; label: string; x: number; y: number; secondsLeft: number }>;
  /** Minimap overlay housekeeping. */
  showFrame: boolean;
  recording: boolean;
}
