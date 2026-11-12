import { EventEmitter } from 'node:events';
import type { CoachAdvice, CoachStatus, GameIntel, MinimapGhost, OpponentProfile, AlertState } from '../../shared/intel';
import { claudeAuthStatus } from './claude';
import { callModel, type Effort } from './llm';
import { WARD_SPOTS } from '../../shared/wards';

/** What System 1 and the rest of the app know right now. */
export interface CoachContext {
  /** Item plan shown in the HUD build panel; the coach's item call must match it. */
  plannedBuild?: string[];
  /** Player profile text (habits, weak spots, lessons) for the per-game system prompt. */
  selfProfile?: string;
  intel: GameIntel;
  ghosts: MinimapGhost[];
  alerts: AlertState[];
  profiles: OpponentProfile[];
  /** Patch brief text for this game (current patch notes + meta), if available. */
  patchBrief?: string;
  /** Lane matchup: live all-in chance, meta win rate, spikes, tips. */
  matchup?: Record<string, unknown> | null;
  /** Jungle camp timers, e.g. "twój czerwony buff za 40 s", "wrogi jungler był na raptorach 0:20 temu". */
  camps?: string[];
  /** Ward spot ids already in our vision (allies/wards there) — do not suggest them. */
  coveredWardSpots?: string[];
}

export interface CoachConfig {
  enabled: boolean;
  liveModel: string;        // fast model during play
  reviewModel: string;      // deeper model for death review
  periodSec: number;        // calm-game cadence
  minGapSec: number;        // never call more often than this
  liveEffort?: Effort;
  reviewEffort?: Effort;
}

export const DEFAULT_COACH: CoachConfig = { enabled: true, liveModel: 'claude-sonnet-5-5', reviewModel: 'claude-sonnet-5-5', periodSec: 25, minGapSec: 20 };

export const SYSTEM = `Jesteś coachem League of Legends na poziomie challengera. Gracz gra TERAZ i może tylko zerknąć na ekran.
Dostajesz stan meczu (JSON): jego postać, itemy, złoto, wszyscy gracze, timery celów, przewidywane pozycje wrogów we mgle (z % pewności) i profile przeciwników.
Zasady:
- "call": MAKSYMALNIE 5 słów, po polsku: co zrobić w ciągu najbliższych 5–20 sekund, ZANIM to się stanie (np. "Smok teraz — 3 martwych", "Nie pushuj — jungler top", "Back po tej fali"). Bez lania wody, bez emotek.
- NIE opisuj tego, co gracz już widzi na ekranie (jego HP, złoto, timery, że ktoś zniknął) i NIE powtarzaj poprzednich porad (previousCalls) ani ich sensu innymi słowami. Ogólniki typu "farm pod wieżą", "uważaj na ganka" tylko gdy coś się zmieniło.
- Smoki, baron i herold są ważne — komentuj je (przed pojawieniem się i przy walce o nie).
- Bądź KONSEKWENTNY: nie zmieniaj decyzji z previousCalls (np. "Smok TAK" → "Odpuść smoka"), dopóki sytuacja wyraźnie się nie zmieni (ktoś zginął, wróg się pokazał, przegrana walka). Gdy zmieniasz zdanie, podaj powód w 2–3 słowach ("Odpuść — Yi przy smoku").
- Nie komentuj wszystkiego: jedna rzecz naraz, tylko to, co zmienia decyzję gracza.
- Gdy nie masz NOWEJ, konkretnej decyzji: "call": null (gracz nic nie zobaczy — to lepsze niż szum).
- "tone": go = zrób to teraz, wait = czekaj/przygotuj się, danger = zagrożenie, farm = spokojnie farm.
- "mapHint": gdzie iść/co unikać, albo null. target to np. "dragon", "baron", "herald", "top", "mid", "bot", "my-jungle", "enemy-jungle", "base".
- "item": następny przedmiot do kupienia przy najbliższym powrocie. Jeśli w danych jest "plannedBuild" (plan z panelu buildu, który gracz widzi na ekranie), wybierz z niego pierwszy przedmiot, którego gracz jeszcze nie ma — nie proponuj innego. Odejdź od planu tylko, gdy sytuacja wyraźnie tego wymaga (np. wróg nagle nafeedowany AP), i wtedy w "call" krótko napisz dlaczego. Gdy nie ma sensu nic mówić o przedmiotach: null.
- "ward": gdzie postawić ward TERAZ (spot z listy wardSpots, nazwy względem strony gracza: my-* = jego dżungla, enemy-* = wroga) i krótki powód, albo null. Wybieraj miejsca o wyższej wartości (więcej wizji); faelight-* to Faelighty (2026): ward postawiony NA nich daje dużo większy zasięg wizji — preferuj je, gdy dostępne (mid-* dopiero po przemianie mapy po 2. smoku); plant-* to rośliny wizji — uderzenie odsłania duży obszar za darmo. Proponuj ward przed objective (60–90 s przed smokiem/baronem), przy pushowaniu linii bez wizji, gdy jungler wroga zniknął.
- "deathReview": TYLKO gdy me.stan = "NIE ŻYJE" (jeśli "żyje", deathReview = null — także gdy zginął przed chwilą i już się odrodził): 3–5 krótkich punktów: dlaczego zginął (masz myDeath: kto zabił, kto asystował, czy to gank — używaj tego, nie zgaduj), co zrobić po respawnie (back/farm/gdzie iść), czy grać pasywnie czy agresywnie przez najbliższe minuty i dlaczego. Inaczej null.
- "ttlSec": jak długo porada jest aktualna (10–40).
Opieraj się na liczbach: przewaga złota, kto martwy i jak długo, timery, power spike'i, gdzie prawdopodobnie są wrogowie. Jeśli nic ważnego się nie dzieje — "call": null.`;

export const SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['call', 'tone', 'mapHint', 'item', 'ward', 'deathReview', 'ttlSec'],
  properties: {
    call: { anyOf: [{ type: 'null' }, { type: 'string', maxLength: 45 }] },
    tone: { type: 'string', enum: ['go', 'wait', 'danger', 'farm'] },
    mapHint: {
      anyOf: [{ type: 'null' }, {
        type: 'object', additionalProperties: false, required: ['type', 'target'],
        properties: { type: { type: 'string', enum: ['objective', 'lane', 'jungle', 'base', 'avoid'] }, target: { type: 'string' } },
      }],
    },
    item: { anyOf: [{ type: 'null' }, { type: 'string' }] },
    ward: { anyOf: [{ type: 'null' }, { type: 'object', additionalProperties: false, required: ['spot', 'reason'], properties: { spot: { type: 'string', enum: Object.keys(WARD_SPOTS) }, reason: { type: 'string', maxLength: 60 } } }] },
    deathReview: { anyOf: [{ type: 'null' }, { type: 'array', items: { type: 'string', maxLength: 120 }, minItems: 2, maxItems: 5 }] },
    ttlSec: { type: 'number', minimum: 5, maximum: 60 },
  },
};

const fmt = (s: number) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, '0')}`;

/**
 * Per-game context that doesn't change between calls (patch brief, enemy profiles,
 * ward spot list, map orientation). It goes into the system prompt, which Claude
 * caches, so it isn't paid in full on every call.
 */
export function gameSystemPrompt(ctx: CoachContext): string {
  const i = ctx.intel;
  const parts = [SYSTEM];
  parts.push(`Strona gracza: ${i.me?.team === 'CHAOS' ? 'czerwona (prawy górny róg)' : 'niebieska (lewy dolny róg)'}. Współrzędne fog x,y: 0..1, (0,0) = lewy górny róg minimapy.`);
  if (ctx.patchBrief) parts.push(`Brief patcha (aktualna meta):\n${ctx.patchBrief}`);
  const prof = ctx.profiles.filter(p => p.gamesAnalyzed > 0).map(p => `${p.champion}: ${p.summary}${p.rank ? ` (${p.rank})` : ''}`);
  if (prof.length) parts.push(`Profile wrogów z historii gier:\n${prof.join('\n')}`);
  if (ctx.selfProfile) parts.push(ctx.selfProfile);
  parts.push(`Miejsca na wardy (id: opis, wartość wizji):\n${Object.entries(WARD_SPOTS).map(([id, s]) => `${id}: ${s.label}, ${s.value}/5`).join('\n')}`);
  return parts.join('\n\n');
}

/** Compact, model-friendly snapshot (keeps tokens and latency down). */
export function buildSnapshot(ctx: CoachContext, previous: CoachAdvice[]) {
  const i = ctx.intel;
  const p = (x: GameIntel['players'][number]) => ({
    champ: x.champion, role: x.role, lvl: x.level, ms: x.moveSpeed, kda: `${x.kills}/${x.deaths}/${x.assists}`, cs: x.cs,
    gold: x.estGold, items: x.items.filter(it => it.price >= 900).map(it => it.name),
    ...(x.isDead ? { dead: `respawn ${x.respawnIn}s` } : {}),
    ...(x.spikes[0] && i.gameTime - x.spikes[0].at < 90 ? { spike: x.spikes[0].label } : {}),
    ...(x.flashIn ? { flashCd: Math.round(x.flashIn) } : {}),
    ...(!x.isAlly ? { spells: x.summonerSpells } : {}),
    ...(x.hpPct !== null && x.hpSeenAt !== null && i.gameTime - x.hpSeenAt <= 6 ? { hp: `${Math.round(x.hpPct * 100)}%` } : {}),
    ...((i.streaks[x.champion] ?? 0) >= 3 ? { killStreak: i.streaks[x.champion], shutdown: 'duży shutdown za zabicie' } : {}),
  });
  return {
    time: fmt(i.gameTime),
    mode: i.gameMode,
    me: i.me ? { stan: i.me.isDead ? 'NIE ŻYJE' : 'żyje', ...p(i.me), ...(i.myStats ? { goldNow: i.myStats.gold, hp: `${i.myStats.hp}/${i.myStats.maxHp}`, ad: i.myStats.ad, ap: i.myStats.ap, armor: i.myStats.armor, mr: i.myStats.mr } : {}) } : null,
    allies: i.players.filter(x => x.isAlly && !x.isMe).map(p),
    enemies: i.players.filter(x => !x.isAlly).map(x => {
      const prof = ctx.profiles.find(pr => pr.champion === x.champion);
      const ghost = ctx.ghosts.find(g => g.champion === x.champion);
      void prof;
      return { ...p(x), ...(ghost ? { fog: { x: +ghost.x.toFixed(2), y: +ghost.y.toFixed(2), r: +ghost.radius.toFixed(2), unseenSec: Math.round(ghost.age), conf: +ghost.confidence.toFixed(2) } } : {}) };
    }),
    goldDiff: i.goldDiff,
    score: { kills: `${i.ally.kills}-${i.enemy.kills}`, dragons: `${i.ally.dragons.length}-${i.enemy.dragons.length}`, towers: `${i.ally.towers}-${i.enemy.towers}`, barons: `${i.ally.barons}-${i.enemy.barons}` },
    // Only objectives that are up or spawn within 90 s: a far-off Baron made the model talk about it
    // ("przygotuj Barona") ten minutes before it exists.
    objectives: i.objectives
      .filter(o => o.alive || (o.spawnsAt !== null && o.spawnsAt - i.gameTime <= 90))
      .map(o => ({ what: o.label, ...(o.alive ? { status: 'żyje' } : { in: Math.round(o.spawnsAt! - i.gameTime) }) })),
    ...(i.buffs.length ? { buffs: i.buffs.map(b => `${b.kind === 'baron' ? 'Baron' : 'Starszy smok'} (${b.team === 'ally' ? 'my' : 'wróg'}) do ${fmt(b.until)}`) } : {}),
    ...(i.ace ? { ace: `${i.ace.team === 'ally' ? 'MY zrobiliśmy ace' : 'WRÓG zrobił ace'} ${Math.round(i.gameTime - i.ace.at)} s temu` } : {}),
    ...(i.me?.isDead && i.myDeaths[0] ? { myDeath: { killer: i.myDeaths[0].killer, assisters: i.myDeaths[0].assisters, enemiesInvolved: i.myDeaths[0].enemiesInvolved, gank: i.myDeaths[0].wasGank, deathsSoFar: i.myDeaths.length } } : {}),
    csDiffVsLaneOpponent: (() => { const o = i.players.find(p => !p.isAlly && p.role === i.me?.role && p.role !== 'UNKNOWN'); return o && i.me ? i.me.cs - o.cs : null; })(),
    alerts: ctx.alerts.map(a => `${a.text} (${Math.round(a.confidence * 100)}%)`),
    recent: i.recent.slice(0, 6).map(r => `${fmt(r.at)} ${r.text}`),
    previousCalls: previous.slice(0, 3).map(a => `${fmt(a.gameTime)} ${a.call}`),
    ...(ctx.matchup ? { laneMatchup: ctx.matchup } : {}),
    ...(ctx.plannedBuild?.length ? { plannedBuild: ctx.plannedBuild } : {}),
    ...(ctx.camps?.length ? { jungleCamps: ctx.camps } : {}),
    ...(ctx.coveredWardSpots?.length ? { alreadyWarded: ctx.coveredWardSpots } : {}),
  };
}

/**
 * Decides WHEN to ask Claude and keeps only fresh answers.
 * Emits 'advice' (CoachAdvice) and 'status' (CoachStatus).
 */
export class Coach extends EventEmitter {
  status: CoachStatus = { state: 'off', model: DEFAULT_COACH.liveModel, calls: 0, lastError: null, lastLatencyMs: null };
  config: CoachConfig = DEFAULT_COACH;
  private history: CoachAdvice[] = [];
  private inFlight = false;
  private lastCallAt = 0;            // wall clock
  private lastCallGameTime = -999;
  private lastSeen = { dead: false, events: 0, myLevel: 0, gameKey: '' };
  private pendingReason: string | null = null;
  private authCheckedAt = 0;

  setConfig(c: Partial<CoachConfig>) { this.config = { ...this.config, ...c }; if (!this.status.calls) this.status.model = this.config.liveModel; }

  async checkAuth() {
    this.authCheckedAt = Date.now();
    const a = await claudeAuthStatus();
    this.setState(a.loggedIn ? 'idle' : 'not-logged-in', a.error ?? null);
    return a.loggedIn;
  }

  /** Call on every intel update (~2/s). */
  tick(ctx: CoachContext) {
    if (!this.config.enabled) { this.setState('off'); return; }
    if (this.status.state === 'off' || (this.status.state === 'not-logged-in' && Date.now() - this.authCheckedAt > 60000)) {
      if (Date.now() - this.authCheckedAt > 5000) void this.checkAuth();
      return;
    }
    if (this.status.state === 'not-logged-in') return;

    const i = ctx.intel;
    const gameKey = i.players.map(p => p.champion).join('|');
    if (gameKey !== this.lastSeen.gameKey) { this.history = []; this.lastSeen = { dead: false, events: 0, myLevel: 0, gameKey }; }

    const reason = this.trigger(ctx);
    if (reason) this.pendingReason ??= reason;
    if (!this.pendingReason || this.inFlight) return;
    if (Date.now() - this.lastCallAt < this.config.minGapSec * 1000 && this.pendingReason !== 'death') return;
    const why = this.pendingReason;
    this.pendingReason = null;
    void this.ask(ctx, why);
  }

  private trigger(ctx: CoachContext): string | null {
    const i = ctx.intel, me = i.me;
    let reason: string | null = null;
    const dead = Boolean(me?.isDead);
    if (dead && !this.lastSeen.dead) reason = 'death';
    else if (!dead && this.lastSeen.dead) reason = 'respawn';
    this.lastSeen.dead = dead;

    const eventsNow = i.recent.length ? i.recent[0]!.at : 0;
    if (!reason && eventsNow > this.lastSeen.events && i.recent[0] && /Smok|Baron|Herald|Atakhan|Wieża|Inhibitor/.test(i.recent[0].text)) reason = 'event';
    this.lastSeen.events = Math.max(this.lastSeen.events, eventsNow);

    if (!reason && me && me.level >= 6 && this.lastSeen.myLevel < 6 && this.lastSeen.myLevel > 0) reason = 'level6';
    if (me) this.lastSeen.myLevel = me.level;

    const soon = i.objectives.find(o => o.spawnsAt && o.spawnsAt - i.gameTime < 60 && o.spawnsAt - i.gameTime > 45);
    if (!reason && soon && i.gameTime - this.lastCallGameTime > 15) reason = 'objective-soon';
    const newAlert = ctx.alerts.find(a => a.confidence > 0.6 && !this.seenAlerts.has(a.text));
    for (const a of ctx.alerts) this.seenAlerts.set(a.text, i.gameTime);
    for (const [t, at] of this.seenAlerts) if (i.gameTime - at > 60) this.seenAlerts.delete(t);
    if (!reason && newAlert && i.gameTime - this.lastCallGameTime > 15) reason = 'alert';
    if (!reason && i.gameTime - this.lastCallGameTime > this.config.periodSec) {
      // Quiet game: only ask again if something changed, or after 2× the period at most.
      const sig = this.signature(ctx);
      if (sig !== this.lastSignature || i.gameTime - this.lastCallGameTime > this.config.periodSec * 2) reason = 'periodic';
    }
    // While dead: one death review only. Other triggers (kills, alerts, periodic) would ask the live
    // model, which sees "NIE ŻYJE" and writes a second death review over the first.
    if (dead && reason !== 'death') return null;
    return reason;
  }

  private lastSignature = '';
  /** Alert texts already reacted to (game time last seen); an alert triggers a call only when new. */
  private seenAlerts = new Map<string, number>();
  /** Coarse game state: levels, kills, deaths, items, objective states, fog count, alerts. */
  private signature(ctx: CoachContext): string {
    const i = ctx.intel;
    return [
      i.players.map(p => `${p.level}.${p.kills}.${p.deaths}.${p.completedItems}.${p.isDead ? 1 : 0}`).join(','),
      i.objectives.map(o => `${o.kind}${o.alive ? 'A' : o.spawnsAt ? Math.floor((o.spawnsAt - i.gameTime) / 30) : '-'}`).join(','),
      ctx.ghosts.length, ctx.alerts.map(a => a.kind).join(','),
      Math.round((i.myStats?.gold ?? 0) / 500),
    ].join('|');
  }

  private async ask(ctx: CoachContext, reason: string) {
    this.lastSignature = this.signature(ctx);
    this.inFlight = true;
    this.lastCallAt = Date.now();
    this.lastCallGameTime = ctx.intel.gameTime;
    const isDeath = reason === 'death';
    const model = isDeath ? this.config.reviewModel : this.config.liveModel;
    this.status.model = model;
    this.setState('thinking');
    const snapshot = buildSnapshot(ctx, this.history);
    const contextId = `${Math.round(ctx.intel.gameTime)}-${reason}`;
    try {
      const r = await callModel<Omit<CoachAdvice, 'contextId' | 'createdAt' | 'gameTime' | 'latencyMs'>>({
        model, system: gameSystemPrompt(ctx), schema: SCHEMA,
        prompt: `Powód zapytania: ${reason}${isDeath ? ' (gracz właśnie zginął — wypełnij deathReview)' : ''}\nStan meczu:\n${JSON.stringify(snapshot)}`,
        // In-game: no extended thinking (51 s -> ~6 s). Death review may think a little.
        // 5.x models answer without a thinking phase; older aliases (haiku/sonnet 4.x) think for ~50 s unless capped.
        thinkingTokens: /-5-5|-5/.test(model) ? undefined : isDeath ? 2000 : 0,
        effort: (isDeath ? this.config.reviewEffort : this.config.liveEffort) ?? 'low',
        timeoutMs: isDeath ? 60000 : 25000,
      });
      this.status.calls++;
      this.status.lastLatencyMs = r.latencyMs;
      const call = (r.value.call ?? '').trim();
      // Silent (no new decision) or the same idea as a recent call: show nothing new. A death review still counts.
      if (!isDeath && (!call || this.history.slice(0, 3).some(h => similarCall(h.call, call)))) { this.setState('idle'); return; }
      const advice: CoachAdvice = { ...r.value, call, contextId, createdAt: Date.now(), gameTime: ctx.intel.gameTime, latencyMs: r.latencyMs };
      this.history.unshift(advice);
      this.history = this.history.slice(0, 10);
      this.emit('advice', advice);
      this.setState('idle');
    } catch (e) {
      this.setState('error', String(e instanceof Error ? e.message : e));
      if (/auth|login|zalog/i.test(String(e))) void this.checkAuth();
    } finally {
      this.inFlight = false;
    }
  }

  /** Drop advice that no longer fits the game (too old or the player died/respawned since). */
  static isFresh(a: CoachAdvice, intel: GameIntel): boolean {
    const age = intel.gameTime - a.gameTime;
    if (age > a.ttlSec || age < -5) return false;
    const dead = Boolean(intel.me?.isDead);
    if (a.deathReview && !dead) return false;
    return true;
  }

  private setState(state: CoachStatus['state'], error: string | null = this.status.lastError) {
    const changed = state !== this.status.state || error !== this.status.lastError;
    this.status = { ...this.status, state, lastError: state === 'error' ? error : state === 'idle' ? null : error };
    if (changed) this.emit('status', this.status);
  }
}

/** Two calls say the same thing (shared content words), e.g. "Farm pod wieżą" vs "Farmuj pod wieżą — gank". */
export function similarCall(a: string, b: string): boolean {
  const words = (x: string) => new Set(x.toLowerCase().normalize('NFD').replace(/[\u0300-\u036f]/g, '').replace(/[^a-z0-9 ]/g, ' ').split(/\s+/).filter(w => w.length > 3).map(w => w.slice(0, 5)));
  const A = words(a), B = words(b);
  if (!A.size || !B.size) return false;
  let common = 0;
  for (const w of A) if (B.has(w)) common++;
  return common / Math.min(A.size, B.size) >= 0.6;
}
