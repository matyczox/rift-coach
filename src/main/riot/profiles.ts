import type { OpponentProfile, Role } from '../../shared/intel';
import { analyzeGame, buildProfile, normChampion, type GameStats } from './analyze';
import { BudgetExceededError, RequestBudget, RiotApiError, type CallOptions, type RiotClient } from './client';
import type { LeagueEntryDto, MatchDto } from './types';

// Orchestration: 5 enemies → OpponentProfile[] within the dev-key limits.
//
// Request plan for a cold cache (defaults), per player:
//   account 1 (skipped when puuid is given, e.g. from spectator-v5)
//   league 1, match ids 1 (type=ranked; +1 type=normal only if ranked < matchCount)
//   matches: jungler ≤ 12, laner ≤ 7 network fetches (cached ones are free, up to matchCount)
//   timelines: jungler ≤ 10, laner ≤ 5
//   mastery ≤ 1 (only when the champion is absent from the sample)
// Typical cold total: 5+5+5 + 12+4·7 + 10+4·5 + ≤5 = 85–90 ≤ maxRequests (95),
// which stays under 100 req / 120 s, so no long limiter waits.
// Everything is additionally bounded by a hard deadline (default 85 s).

export interface ProfilePlayer {
  riotId: string;
  champion: string;
  role: Role;
  /** Known puuid (spectator-v5 / LCU) saves the account lookup. */
  puuid?: string;
  /** Numeric champion id (enables the mastery lookup). */
  championId?: number;
}

export interface ProfileProgress {
  riotId: string;
  stage: 'account' | 'matches' | 'timelines' | 'done' | 'error';
  playersDone: number;
  playersTotal: number;
  requestsUsed: number;
}

export interface BuildProfilesOptions {
  client: RiotClient;
  /** Match ids considered per player (default 20). */
  matchCount?: number;
  /** Queues kept (default ranked solo 420, flex 440, normal draft 400, swiftplay 480). */
  queues?: number[];
  /** Max match downloads (network) per player. */
  maxMatchFetch?: { jungle: number; laner: number };
  /** Max timeline downloads (network) per player. */
  maxTimelines?: { jungle: number; laner: number };
  /** Hard cap on network requests for the whole run (default 95). */
  maxRequests?: number;
  /** Hard wall-clock cap (default 85 000 ms). */
  timeoutMs?: number;
  fetchMastery?: boolean;          // default true
  onProgress?: (p: ProfileProgress) => void;
  signal?: AbortSignal;
  now?: () => number;
}

const DEFAULT_QUEUES = [420, 440, 400, 480];
const RANKED = new Set([420, 440]);

export async function buildProfiles(players: ProfilePlayer[], opts: BuildProfilesOptions): Promise<OpponentProfile[]> {
  const now = opts.now ?? Date.now;
  const start = now();
  const deadline = start + (opts.timeoutMs ?? 85_000);
  const client = opts.client;
  const queues = new Set(opts.queues ?? DEFAULT_QUEUES);
  const matchCount = opts.matchCount ?? 20;
  const caps = {
    match: opts.maxMatchFetch ?? { jungle: 12, laner: 7 },
    tl: opts.maxTimelines ?? { jungle: 10, laner: 5 },
  };
  // Never plan more requests than the regional limiter can grant before the
  // deadline (e.g. a re-run shortly after a previous one), minus a safety margin.
  const limiterRoom = client.limiterFor(client.region).availableBy(deadline) - 3;
  const global = new RequestBudget(Math.max(0, Math.min(opts.maxRequests ?? 95, limiterRoom)));
  const base: CallOptions = { budget: global, deadline, signal: opts.signal };
  let done = 0;
  const report = (riotId: string, stage: ProfileProgress['stage']) => {
    try { opts.onProgress?.({ riotId, stage, playersDone: done, playersTotal: players.length, requestsUsed: global.used }); } catch { /* ignore */ }
  };

  // Champion name → id learned from any fetched match (for mastery lookups).
  const champIds = new Map<string, number>();

  const one = async (pl: ProfilePlayer): Promise<OpponentProfile> => {
    const isJungle = pl.role === 'JUNGLE';
    let puuid = pl.puuid ?? '';
    let rank: string | null = null;
    const games: GameStats[] = [];
    let masteryPoints: number | null | undefined;
    try {
      report(pl.riotId, 'account');
      if (!puuid) {
        const acc = await client.resolveRiotId(pl.riotId, base);
        if (!acc) throw new Error('account not found');
        puuid = acc.puuid;
      }
      const leagueP = client.getLeagueEntries(puuid, base).then(e => { rank = formatRank(e); }).catch(() => undefined);

      report(pl.riotId, 'matches');
      const ids = await listMatchIds(client, puuid, queues, matchCount, base);
      const matchBudget = new RequestBudget(isJungle ? caps.match.jungle : caps.match.laner, global);
      const matches: MatchDto[] = [];
      // Fetch newest first; small parallelism, the limiter does the pacing.
      await forEachLimit(ids, 4, async id => {
        try {
          const m = await client.getMatch(id, { ...base, budget: matchBudget });
          if (m && queues.has(m.info.queueId)) matches.push(m);
        } catch (e) { if (isFatal(e)) throw e; }
      });
      for (const m of matches) {
        for (const p of m.info.participants) champIds.set(normChampion(p.championName), p.championId);
      }
      matches.sort((a, b) => endOf(b) - endOf(a));

      report(pl.riotId, 'timelines');
      const want = pickTimelineGames(matches, puuid, pl);
      const tlBudget = new RequestBudget(isJungle ? caps.tl.jungle : caps.tl.laner, global);
      const timelines = new Map<string, Awaited<ReturnType<RiotClient['getTimeline']>>>();
      await forEachLimit(want, 3, async m => {
        try {
          const t = await client.getTimeline(m.metadata.matchId, { ...base, budget: tlBudget });
          if (t) timelines.set(m.metadata.matchId, t);
        } catch (e) { if (isFatal(e)) throw e; }
      });
      for (const m of matches) {
        const g = analyzeGame(m, timelines.get(m.metadata.matchId) ?? null, puuid);
        if (g) games.push(g);
      }

      const champ = normChampion(pl.champion);
      if ((opts.fetchMastery ?? true) && !games.some(g => normChampion(g.championName) === champ)) {
        const cid = pl.championId ?? champIds.get(champ);
        if (cid !== undefined) {
          try {
            const m = await client.getChampionMastery(puuid, cid, base);
            masteryPoints = m ? m.championPoints : null;
          } catch { /* optional */ }
        }
      }
      await leagueP;
      done++;
      report(pl.riotId, 'done');
    } catch {
      done++;
      report(pl.riotId, 'error');
    }
    return buildProfile({ puuid, riotId: pl.riotId, champion: pl.champion, role: pl.role, rank, games, masteryPoints, now: now() });
  };

  // Hard stop: even if something hangs, return what we have shortly after the deadline.
  const results: Array<OpponentProfile | null> = players.map(() => null);
  const work = Promise.all(players.map((pl, i) => one(pl).then(p => { results[i] = p; })));
  let timer: ReturnType<typeof setTimeout> | undefined;
  const guard = new Promise<void>(r => { timer = setTimeout(r, Math.max(0, deadline - now()) + 3000); });
  await Promise.race([work, guard]);
  if (timer) clearTimeout(timer);
  return results.map((r, i) => r ?? buildProfile({
    puuid: players[i]!.puuid ?? '', riotId: players[i]!.riotId, champion: players[i]!.champion,
    role: players[i]!.role, rank: null, games: [], now: now(),
  }));
}

/** Key rejected / aborted: stop this player. Anything else on a single match is skipped. */
function isFatal(e: unknown): boolean {
  return (e instanceof RiotApiError && (e.status === 401 || e.status === 403)) || (e instanceof Error && e.name === 'AbortError');
}

/** Budget/deadline/404-ish problems are expected: skip the item, keep going. */
function isSoft(e: unknown): boolean {
  return e instanceof BudgetExceededError || (e instanceof Error && e.name === 'DeadlineError');
}

async function listMatchIds(client: RiotClient, puuid: string, queues: Set<number>, count: number, o: CallOptions): Promise<string[]> {
  const wantRanked = [...queues].some(q => RANKED.has(q));
  const wantNormal = [...queues].some(q => !RANKED.has(q));
  let ids: string[] = [];
  if (queues.size === 1) {
    ids = await client.getMatchIds(puuid, { queue: [...queues][0]!, count }, o);
  } else {
    if (wantRanked) ids = await client.getMatchIds(puuid, { type: 'ranked', count }, o);
    if (wantNormal && ids.length < count) {
      try {
        const more = await client.getMatchIds(puuid, { type: 'normal', count }, o);
        ids = [...new Set([...ids, ...more])];
      } catch (e) { if (!isSoft(e)) throw e; }
    }
  }
  // Match ids are "<PLATFORM>_<increasing number>": sort newest first.
  return ids.sort((a, b) => matchNum(b) - matchNum(a)).slice(0, count);
}

function matchNum(id: string): number {
  const n = Number(id.slice(id.lastIndexOf('_') + 1));
  return Number.isFinite(n) ? n : 0;
}

function endOf(m: MatchDto): number {
  return m.info.gameEndTimestamp ?? m.info.gameCreation;
}

/**
 * Which games deserve a timeline: jungler → games played as JUNGLE; laner →
 * games in the same lane (else any lane game). Same champion first, then recency.
 */
export function pickTimelineGames(matches: MatchDto[], puuid: string, pl: Pick<ProfilePlayer, 'role' | 'champion'>): MatchDto[] {
  const champ = normChampion(pl.champion);
  const rows = matches.map(m => ({ m, p: m.info.participants.find(x => x.puuid === puuid) })).filter(r => r.p);
  const lanePos = (pos: string | undefined) => pos === 'UTILITY' ? 'BOTTOM' : pos;
  let eligible = rows.filter(r => pl.role === 'UNKNOWN' ? !!r.p!.teamPosition
    : lanePos(r.p!.teamPosition) === lanePos(pl.role));
  if (!eligible.length && pl.role !== 'JUNGLE') eligible = rows.filter(r => r.p!.teamPosition && r.p!.teamPosition !== 'JUNGLE');
  return eligible
    .sort((a, b) => Number(normChampion(b.p!.championName) === champ) - Number(normChampion(a.p!.championName) === champ) || endOf(b.m) - endOf(a.m))
    .map(r => r.m);
}

export function formatRank(entries: LeagueEntryDto[]): string | null {
  const e = entries.find(x => x.queueType === 'RANKED_SOLO_5x5') ?? entries.find(x => x.queueType === 'RANKED_FLEX_SR');
  if (!e || !e.tier) return null;
  const apex = ['MASTER', 'GRANDMASTER', 'CHALLENGER'].includes(e.tier);
  const lp = `${e.leaguePoints ?? 0} LP`;
  const flex = e.queueType === 'RANKED_FLEX_SR' ? ' (flex)' : '';
  return `${e.tier}${apex || !e.rank ? '' : ` ${e.rank}`} ${lp}${flex}`;
}

async function forEachLimit<T>(items: T[], limit: number, fn: (x: T) => Promise<void>): Promise<void> {
  let i = 0;
  const workers = Array.from({ length: Math.min(limit, items.length) }, async () => {
    while (i < items.length) { const x = items[i++]!; await fn(x); }
  });
  await Promise.all(workers);
}
