import { DiskCache } from './cache';
import { DEV_KEY_LIMITS, DeadlineError, RateLimiter, sleep, type RateLimit } from './limiter';
import type {
  AccountDto, ChampionMasteryDto, CurrentGameInfoDto, LeagueEntryDto, MatchDto, TimelineDto,
} from './types';

// Riot Web API client for the user's personal development key.
// - The key is only ever sent in the X-Riot-Token header; it is never logged,
//   never put in URLs and never included in error messages.
// - Rate limits are per routing value (host), so each host has its own limiter.
// - 404 → null. 429 → honour Retry-After (pauses the whole host), retry.
//   5xx / network errors → exponential backoff retry. 401/403 → RiotApiError.

export type Platform =
  | 'eun1' | 'euw1' | 'tr1' | 'ru' | 'me1'
  | 'na1' | 'br1' | 'la1' | 'la2'
  | 'kr' | 'jp1'
  | 'oc1' | 'sg2' | 'tw2' | 'vn2';
export type Region = 'europe' | 'americas' | 'asia' | 'sea';

export const PLATFORM_REGION: Record<Platform, Region> = {
  eun1: 'europe', euw1: 'europe', tr1: 'europe', ru: 'europe', me1: 'europe',
  na1: 'americas', br1: 'americas', la1: 'americas', la2: 'americas',
  kr: 'asia', jp1: 'asia',
  oc1: 'sea', sg2: 'sea', tw2: 'sea', vn2: 'sea',
};

/** Default tag line guess when a Riot ID comes without "#TAG". */
export const PLATFORM_DEFAULT_TAG: Record<Platform, string> = {
  eun1: 'EUNE', euw1: 'EUW', tr1: 'TR1', ru: 'RU', me1: 'ME1', na1: 'NA1', br1: 'BR1', la1: 'LAN',
  la2: 'LAS', kr: 'KR1', jp1: 'JP1', oc1: 'OCE', sg2: 'SG2', tw2: 'TW2', vn2: 'VN2',
};

export const QUEUES = { RANKED_SOLO: 420, RANKED_FLEX: 440, NORMAL_DRAFT: 400, SWIFTPLAY: 480 } as const;

export function isPlatform(s: string): s is Platform {
  return Object.prototype.hasOwnProperty.call(PLATFORM_REGION, s);
}

export class RiotApiError extends Error {
  constructor(readonly status: number, readonly endpoint: string, msg?: string) {
    super(msg ?? `Riot API ${status} on ${endpoint}`);
    this.name = 'RiotApiError';
  }
}

export class BudgetExceededError extends Error {
  constructor() { super('Riot request budget exhausted'); this.name = 'BudgetExceededError'; }
}

/**
 * Counts network requests (cache hits are free). Budgets can be nested: a
 * per-player budget with a global parent only grants when both have room.
 */
export class RequestBudget {
  used = 0;
  constructor(readonly max: number, private readonly parent?: RequestBudget) {}
  get remaining(): number {
    const own = this.max - this.used;
    return this.parent ? Math.min(own, this.parent.remaining) : own;
  }
  take(): boolean {
    if (this.used >= this.max) return false;
    if (this.parent && !this.parent.take()) return false;
    this.used++;
    return true;
  }
}

export interface CallOptions {
  /** Network requests must fit this budget, else BudgetExceededError (cache still served). */
  budget?: RequestBudget;
  /** Epoch ms; no request (incl. limiter waits / retries) runs past it → DeadlineError. */
  deadline?: number;
  signal?: AbortSignal;
}

export interface CacheTtls { account: number; league: number; matchIds: number; mastery: number }

export const DEFAULT_TTLS: CacheTtls = {
  account: 24 * 3600_000,     // Riot ID → puuid rarely changes
  league: 10 * 60_000,
  matchIds: 10 * 60_000,
  mastery: 60 * 60_000,
};

export interface RiotClientOptions {
  /** Personal development key (RGAPI-...). Required. */
  apiKey: string;
  /** Cache directory (e.g. app.getPath('userData')/riot-cache). null/undefined = memory only. */
  cacheDir?: string | null;
  platform?: Platform;              // default 'eun1'
  /** Override the regional routing; default PLATFORM_REGION[platform]. */
  region?: Region;
  requestTimeoutMs?: number;        // default 8000
  maxRetries?: number;              // default 3 (per request, for 429/5xx/network)
  limits?: RateLimit[];             // default dev-key limits
  ttl?: Partial<CacheTtls>;
  fetchImpl?: typeof fetch;
  now?: () => number;
}

export class RiotClient {
  readonly platform: Platform;
  readonly region: Region;
  /** Network requests actually sent (including retries). */
  requestCount = 0;

  private readonly apiKey: string;
  private readonly cache: DiskCache;
  private readonly limiters = new Map<string, RateLimiter>();
  private readonly inflight = new Map<string, Promise<unknown>>();
  private readonly ttl: CacheTtls;
  private readonly fetchImpl: typeof fetch;
  private readonly now: () => number;
  private readonly timeoutMs: number;
  private readonly maxRetries: number;
  private readonly limits: RateLimit[];

  constructor(opts: RiotClientOptions) {
    const key = (opts.apiKey ?? '').trim();
    if (!key) throw new Error('Riot API key missing');
    this.apiKey = key;
    this.platform = opts.platform ?? 'eun1';
    if (!isPlatform(this.platform)) throw new Error(`unknown platform ${String(this.platform)}`);
    this.region = opts.region ?? PLATFORM_REGION[this.platform];
    this.now = opts.now ?? Date.now;
    this.cache = new DiskCache(opts.cacheDir ?? null, this.now);
    this.ttl = { ...DEFAULT_TTLS, ...opts.ttl };
    this.fetchImpl = opts.fetchImpl ?? ((...a: Parameters<typeof fetch>) => fetch(...a));
    this.timeoutMs = opts.requestTimeoutMs ?? 8000;
    this.maxRetries = opts.maxRetries ?? 3;
    this.limits = opts.limits ?? DEV_KEY_LIMITS;
  }

  /** account-v1 supports americas/asia/europe only. */
  private get accountRegion(): Region { return this.region === 'sea' ? 'asia' : this.region; }

  limiterFor(host: string): RateLimiter {
    let l = this.limiters.get(host);
    if (!l) { l = new RateLimiter(this.limits, this.now); this.limiters.set(host, l); }
    return l;
  }

  // ---------------- endpoints ----------------

  async getAccountByRiotId(gameName: string, tagLine: string, o: CallOptions = {}): Promise<AccountDto | null> {
    const key = `${gameName.toLowerCase()}#${tagLine.toLowerCase()}`;
    return this.cached('account', key, this.ttl.account, o, () =>
      this.request<AccountDto>(this.accountRegion,
        `/riot/account/v1/accounts/by-riot-id/${encodeURIComponent(gameName)}/${encodeURIComponent(tagLine)}`, o));
  }

  /** Resolve "Name#TAG" (tag defaults to the platform's usual tag). */
  async resolveRiotId(riotId: string, o: CallOptions = {}): Promise<AccountDto | null> {
    const { gameName, tagLine } = parseRiotId(riotId, PLATFORM_DEFAULT_TAG[this.platform]);
    if (!gameName) return null;
    return this.getAccountByRiotId(gameName, tagLine, o);
  }

  async getLeagueEntries(puuid: string, o: CallOptions = {}): Promise<LeagueEntryDto[]> {
    const r = await this.cached('league', puuid, this.ttl.league, o, () =>
      this.request<LeagueEntryDto[]>(this.platform, `/lol/league/v4/entries/by-puuid/${encodeURIComponent(puuid)}`, o));
    return r ?? [];
  }

  async getMatchIds(
    puuid: string,
    q: { queue?: number; type?: 'ranked' | 'normal' | 'tourney' | 'tutorial'; start?: number; count?: number; startTime?: number } = {},
    o: CallOptions = {},
  ): Promise<string[]> {
    const params = new URLSearchParams();
    if (q.queue !== undefined) params.set('queue', String(q.queue));
    if (q.type) params.set('type', q.type);
    if (q.startTime !== undefined) params.set('startTime', String(Math.floor(q.startTime)));
    params.set('start', String(q.start ?? 0));
    params.set('count', String(Math.max(1, Math.min(100, q.count ?? 20))));
    const qs = params.toString();
    const r = await this.cached('matchids', `${puuid}?${qs}`, this.ttl.matchIds, o, () =>
      this.request<string[]>(this.region, `/lol/match/v5/matches/by-puuid/${encodeURIComponent(puuid)}/ids?${qs}`, o));
    return r ?? [];
  }

  async getMatch(matchId: string, o: CallOptions = {}): Promise<MatchDto | null> {
    return this.cached('match', matchId, null, o, () =>
      this.request<MatchDto>(this.region, `/lol/match/v5/matches/${encodeURIComponent(matchId)}`, o));
  }

  async getTimeline(matchId: string, o: CallOptions = {}): Promise<TimelineDto | null> {
    return this.cached('timeline', matchId, null, o, () =>
      this.request<TimelineDto>(this.region, `/lol/match/v5/matches/${encodeURIComponent(matchId)}/timeline`, o));
  }

  async getChampionMastery(puuid: string, championId: number, o: CallOptions = {}): Promise<ChampionMasteryDto | null> {
    return this.cached('mastery', `${puuid}_${championId}`, this.ttl.mastery, o, () =>
      this.request<ChampionMasteryDto>(this.platform,
        `/lol/champion-mastery/v4/champion-masteries/by-puuid/${encodeURIComponent(puuid)}/by-champion/${championId}`, o));
  }

  /** spectator-v5: the live game (10 participants with puuid/champion) or null when not in game. Not cached. */
  async getActiveGame(puuid: string, o: CallOptions = {}): Promise<CurrentGameInfoDto | null> {
    if (o.budget && !o.budget.take()) throw new BudgetExceededError();
    return this.request<CurrentGameInfoDto>(this.platform, `/lol/spectator/v5/active-games/by-summoner/${encodeURIComponent(puuid)}`, o);
  }

  // ---------------- plumbing ----------------

  /**
   * Cache-first fetch with in-flight de-duplication. Only non-null results are
   * stored (a 404 may be temporary, e.g. a match still being processed).
   */
  private async cached<T>(ns: string, key: string, ttl: number | null, o: CallOptions, load: () => Promise<T | null>): Promise<T | null> {
    const hit = await this.cache.get<T>(ns, key, ttl);
    if (hit !== undefined) return hit;
    const id = `${ns}/${key}`;
    const pending = this.inflight.get(id) as Promise<T | null> | undefined;
    if (pending) return pending;
    if (o.budget && !o.budget.take()) throw new BudgetExceededError();
    const p = (async () => {
      try {
        const v = await load();
        if (v !== null) await this.cache.set(ns, key, v);
        return v;
      } finally { this.inflight.delete(id); }
    })();
    this.inflight.set(id, p);
    return p;
  }

  private async request<T>(host: string, pathAndQuery: string, o: CallOptions): Promise<T | null> {
    const limiter = this.limiterFor(host);
    const url = `https://${host}.api.riotgames.com${pathAndQuery}`;
    const endpoint = pathAndQuery.split('?')[0]!.replace(/\/[^/]{30,}(?=\/|$)/g, '/{id}'); // no puuids in errors
    let attempt = 0;
    for (;;) {
      await limiter.acquire(o.deadline, o.signal);
      const remaining = o.deadline !== undefined ? o.deadline - this.now() : Infinity;
      if (remaining <= 0) throw new DeadlineError();
      const timeout = AbortSignal.timeout(Math.max(1, Math.min(this.timeoutMs, remaining)));
      const signal = o.signal ? AbortSignal.any([o.signal, timeout]) : timeout;
      let res: Response | null = null;
      let netErr: unknown = null;
      this.requestCount++;
      try {
        res = await this.fetchImpl(url, { headers: { 'X-Riot-Token': this.apiKey, Accept: 'application/json' }, signal });
      } catch (e) {
        if (o.signal?.aborted) throw o.signal.reason ?? e;
        netErr = e;
      }

      if (res) {
        if (res.status === 200) return (await res.json()) as T;
        if (res.status === 404) return null;
        if (res.status === 401 || res.status === 403) {
          throw new RiotApiError(res.status, endpoint, `Riot API key rejected (${res.status}) — expired or invalid dev key?`);
        }
        if (res.status === 429) {
          const ra = Number(res.headers.get('retry-after'));
          const waitMs = Number.isFinite(ra) && ra > 0 ? ra * 1000 : 1000 * 2 ** attempt;
          // application/method limits block the whole host; "service" limits are per request
          if (res.headers.get('x-rate-limit-type') !== 'service') limiter.pause(waitMs);
          if (attempt >= this.maxRetries) throw new RiotApiError(429, endpoint);
          attempt++;
          await this.backoff(waitMs, o);
          continue;
        }
        if (res.status < 500) throw new RiotApiError(res.status, endpoint);
      }
      // 5xx or network/timeout error
      if (attempt >= this.maxRetries) {
        if (res) throw new RiotApiError(res.status, endpoint);
        throw netErr instanceof Error ? new Error(`Riot request failed on ${endpoint}: ${netErr.name}`) : new Error(`Riot request failed on ${endpoint}`);
      }
      await this.backoff(500 * 2 ** attempt, o);
      attempt++;
    }
  }

  private async backoff(ms: number, o: CallOptions): Promise<void> {
    if (o.deadline !== undefined && this.now() + ms > o.deadline) throw new DeadlineError();
    await sleep(ms, o.signal);
  }
}

export function parseRiotId(riotId: string, defaultTag: string): { gameName: string; tagLine: string } {
  const s = riotId.trim();
  const i = s.lastIndexOf('#');
  if (i < 0) return { gameName: s, tagLine: defaultTag };
  return { gameName: s.slice(0, i).trim(), tagLine: s.slice(i + 1).trim() || defaultTag };
}
