import { afterEach, describe, expect, it, vi } from 'vitest';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import {
  MAP_SIZE, aggregateJungle, analyzeGame, analyzeJungleGame, buildProfile, classifyJungleSide, classifyLane,
  earlyStats, fmtMinute, jungleOwner, plPlural, recentForm, roamedInGame, summarize, type GameStats,
} from '../src/main/riot/analyze';
import { RateLimiter, DeadlineError } from '../src/main/riot/limiter';
import { RequestBudget, RiotClient, BudgetExceededError, parseRiotId } from '../src/main/riot/client';
import { buildProfiles, formatRank, pickTimelineGames } from '../src/main/riot/profiles';
import type { MatchDto, MatchParticipantDto, Position, TimelineDto, TimelineEventDto } from '../src/main/riot/types';

// ---------------- synthetic fixtures (match-v5 schema subset) ----------------

const MIN = 60_000;
const P = (i: number) => `puuid-${i}`;

interface MatchOpts {
  id: string;
  /** participantId 1..10 of the profiled player; 1-5 blue (100), 6-10 red (200). */
  pid?: number;
  puuid?: string;
  champion?: string;
  championId?: number;
  position?: string;
  win?: boolean;
  kills?: number; deaths?: number; assists?: number;
  fbKill?: boolean; fbAssist?: boolean;
  end?: number;
  durationSec?: number;
  queueId?: number;
}

const POS = ['TOP', 'JUNGLE', 'MIDDLE', 'BOTTOM', 'UTILITY'];

function makeMatch(o: MatchOpts): MatchDto {
  const pid = o.pid ?? 2;
  const dur = o.durationSec ?? 1800;
  const end = o.end ?? 1_700_000_000_000;
  const participants: MatchParticipantDto[] = Array.from({ length: 10 }, (_, i) => {
    const id = i + 1;
    const me = id === pid;
    const teamId = id <= 5 ? 100 : 200;
    const myTeamWins = o.win ?? true;
    const win = (teamId === (pid <= 5 ? 100 : 200)) === myTeamWins;
    return {
      puuid: me ? (o.puuid ?? P(id)) : `other-${o.id}-${id}`,
      participantId: id,
      teamId,
      championId: me ? (o.championId ?? 19) : 100 + id,
      championName: me ? (o.champion ?? 'Warwick') : `Champ${id}`,
      teamPosition: me ? (o.position ?? POS[i % 5]) : POS[i % 5],
      win,
      kills: me ? (o.kills ?? 5) : 3,
      deaths: me ? (o.deaths ?? 5) : 3,
      assists: me ? (o.assists ?? 5) : 3,
      firstBloodKill: me ? !!o.fbKill : false,
      firstBloodAssist: me ? !!o.fbAssist : false,
      totalMinionsKilled: 180,
      neutralMinionsKilled: 20,
      challenges: { soloKills: 1 },
    };
  });
  return {
    metadata: { matchId: o.id, participants: participants.map(p => p.puuid) },
    info: {
      gameCreation: end - dur * 1000 - 60_000,
      gameDuration: dur,
      gameEndTimestamp: end,
      queueId: o.queueId ?? 420,
      mapId: 11,
      participants,
    },
  };
}

/** frames: minute → { participantId → position }, events placed into the frame of their minute. */
function makeTimeline(match: MatchDto, frames: Record<number, Record<number, Position>>, evs: TimelineEventDto[] = [], minutes = 16): TimelineDto {
  const out: TimelineDto['info']['frames'] = [];
  for (let m = 0; m <= minutes; m++) {
    const pf: TimelineDto['info']['frames'][number]['participantFrames'] = {};
    for (let id = 1; id <= 10; id++) {
      // default: everybody sits in their fountain
      const pos = frames[m]?.[id] ?? (id <= 5 ? { x: 554, y: 581 } : { x: 14302, y: 14390 });
      pf[String(id)] = { participantId: id, position: pos, level: 1 };
    }
    out.push({
      timestamp: m === 0 ? 0 : m * MIN + 37, // real frames drift a few ms
      participantFrames: pf,
      events: evs.filter(e => Math.floor(e.timestamp / MIN) === m),
    });
  }
  return {
    metadata: { matchId: match.metadata.matchId, participants: match.metadata.participants },
    info: {
      frameInterval: 60000,
      frames: out,
      participants: match.info.participants.map(p => ({ participantId: p.participantId, puuid: p.puuid })),
    },
  };
}

const kill = (t: number, killerId: number, victimId: number, pos: Position, assists?: number[]): TimelineEventDto => ({
  type: 'CHAMPION_KILL', timestamp: t, killerId, victimId, position: pos,
  ...(assists ? { assistingParticipantIds: assists } : {}),
});
const rot = (p: Position): Position => ({ x: MAP_SIZE - p.x, y: MAP_SIZE - p.y });

// Landmarks
const BLUE_BLUEBUFF = { x: 3800, y: 7900 };
const BLUE_REDBUFF = { x: 7800, y: 4000 };
const RED_REDBUFF = { x: 7100, y: 10900 };
const RED_BLUEBUFF = { x: 11000, y: 7000 };
const BOT_LANE = { x: 10500, y: 1200 };
const TOP_LANE = { x: 1200, y: 10500 };
const MID_LANE = { x: 7400, y: 7300 };
const DRAGON = { x: 9866, y: 4414 };

// ---------------- geometry ----------------

describe('map geometry', () => {
  it('classifies lanes and keeps camps near lanes out of lane corridors', () => {
    expect(classifyLane(TOP_LANE)).toBe('TOP');
    expect(classifyLane(BOT_LANE)).toBe('BOTTOM');
    expect(classifyLane(MID_LANE)).toBe('MIDDLE');
    expect(classifyLane({ x: 2090, y: 8430 })).toBeNull();   // blue gromp
    expect(classifyLane({ x: 8400, y: 2650 })).toBeNull();   // blue krugs
    expect(classifyLane({ x: 12700, y: 6400 })).toBeNull();  // red gromp
    expect(classifyLane({ x: 500, y: 500 })).toBeNull();     // fountain
    expect(jungleOwner(DRAGON)).toBeNull();                   // river
  });

  it('mirrors own-jungle halves per team; side labels stay lane-relative', () => {
    expect(classifyJungleSide(BLUE_BLUEBUFF, 100)).toBe('top');
    expect(classifyJungleSide(BLUE_REDBUFF, 100)).toBe('bot');
    expect(classifyJungleSide(RED_REDBUFF, 200)).toBe('top');
    expect(classifyJungleSide(RED_BLUEBUFF, 200)).toBe('bot');
    // a position in the other team's jungle is not "own" jungle
    expect(classifyJungleSide(RED_BLUEBUFF, 100)).toBeNull();
    expect(classifyJungleSide(BLUE_BLUEBUFF, 200)).toBeNull();
    // 180° rotation (red↔blue perspective) swaps top and bot
    expect(classifyJungleSide(rot(RED_BLUEBUFF), 100)).toBe('top');
    expect(classifyJungleSide(rot(BLUE_REDBUFF), 200)).toBe('top');
  });
});

// ---------------- jungle analysis ----------------

describe('jungle analysis', () => {
  // Red jungler (pid 7) starts at their blue buff (bot side), kills in bot lane at 3:20.
  const redMatch = makeMatch({ id: 'EUN1_1', pid: 7, position: 'JUNGLE' });
  const redTl = makeTimeline(redMatch, {
    1: { 7: { x: 11300, y: 7200 } },
    2: { 7: RED_BLUEBUFF },
    3: { 7: { x: 10800, y: 5600 } },          // red jungle, near river — not a lane
    4: { 7: { x: 11500, y: 3000 } },
  }, [
    kill(2 * MIN + 10_000, 4, 1, BOT_LANE),                  // lane fight before 2:30 without jungler
    kill(3 * MIN + 20_000, 7, 4, BOT_LANE, [9]),             // the gank
    kill(5 * MIN, 7, 3, MID_LANE),
  ]);

  it('detects red-side start side, first gank minute and lane', () => {
    const j = analyzeJungleGame(redTl, 7, 200);
    expect(j.startSide).toBe('bot');
    expect(j.firstGank?.lane).toBe('BOTTOM');
    expect(j.firstGank?.minute).toBeCloseTo(3 + 20 / 60, 5);
    expect(j.invaded).toBe(false);
    expect(j.dragonPresent).toBeNull();
  });

  it('mirrored game (blue side, everything rotated 180°) gives top start and top gank', () => {
    const blueMatch = makeMatch({ id: 'EUN1_2', pid: 2, position: 'JUNGLE' });
    const blueTl = makeTimeline(blueMatch, {
      1: { 2: rot({ x: 11300, y: 7200 }) },
      2: { 2: rot(RED_BLUEBUFF) },
      3: { 2: rot({ x: 10800, y: 5600 }) },
      4: { 2: rot({ x: 11500, y: 3000 }) },
    }, [kill(3 * MIN + 20_000, 2, 9, rot(BOT_LANE))]);
    const j = analyzeJungleGame(blueTl, 2, 100);
    expect(j.startSide).toBe('top');
    expect(j.firstGank?.lane).toBe('TOP');
    expect(j.firstGank?.minute).toBeCloseTo(3.333, 2);
  });

  it('falls back to the minute-1 position and detects gank from positions only', () => {
    const m = makeMatch({ id: 'EUN1_3', pid: 2, position: 'JUNGLE' });
    const tl = makeTimeline(m, {
      1: { 2: BLUE_REDBUFF },
      2: { 2: { x: 554, y: 581 } },             // recalled?? (fountain) → use minute 1
      3: { 2: { x: 6000, y: 3000 } },
      4: { 2: MID_LANE },                       // standing in mid at 4:00
    });
    const j = analyzeJungleGame(tl, 2, 100);
    expect(j.startSide).toBe('bot');
    expect(j.firstGank?.lane).toBe('MIDDLE');
    expect(j.firstGank?.minute).toBeCloseTo(4, 2);
  });

  it('detects early invades and dragon presence', () => {
    const m = makeMatch({ id: 'EUN1_4', pid: 2, position: 'JUNGLE' });
    const dragonKill: TimelineEventDto = { type: 'ELITE_MONSTER_KILL', timestamp: 6 * MIN + 30_000, killerId: 8, killerTeamId: 200, monsterType: 'DRAGON', monsterSubType: 'FIRE_DRAGON', position: DRAGON };
    const near = makeTimeline(m, {
      1: { 2: RED_BLUEBUFF },                    // level-1 invade into red's blue buff
      6: { 2: { x: 9000, y: 4000 } },
      7: { 2: { x: 9500, y: 5000 } },
    }, [dragonKill]);
    const a = analyzeJungleGame(near, 2, 100);
    expect(a.invaded).toBe(true);
    expect(a.dragonPresent).toBe(true);

    const far = makeTimeline(m, { 6: { 2: TOP_LANE }, 7: { 2: TOP_LANE } }, [dragonKill]);
    const b = analyzeJungleGame(far, 2, 100);
    expect(b.invaded).toBe(false);
    expect(b.dragonPresent).toBe(false);

    const assisted = makeTimeline(m, { 6: { 2: TOP_LANE } }, [{ ...dragonKill, killerId: 3, killerTeamId: 100, assistingParticipantIds: [2] }]);
    expect(analyzeJungleGame(assisted, 2, 100).dragonPresent).toBe(true);
  });

  it('aggregates pathing across games', () => {
    const g = (startSide: 'top' | 'bot', minute: number, lane: 'TOP' | 'BOTTOM', invaded = false) =>
      ({ jungle: { startSide, firstGank: { minute, lane }, invaded, dragonPresent: true } }) as unknown as GameStats;
    const j = aggregateJungle([g('bot', 3.2, 'BOTTOM'), g('bot', 3.5, 'BOTTOM', true), g('top', 4, 'TOP'), g('bot', 3.0, 'BOTTOM')])!;
    expect(j.startBotSide).toBe(0.75);
    expect(j.startTopSide).toBe(0.25);
    expect(j.firstGankMinute).toBeCloseTo(3.35, 5);
    expect(j.firstGankLane).toEqual({ BOTTOM: 0.75, TOP: 0.25 });
    expect(j.earlyInvadeRate).toBe(0.25);
    expect(j.sampleGames).toBe(4);
  });
});

// ---------------- laner / early stats ----------------

describe('early stats and roaming', () => {
  const m = makeMatch({ id: 'EUN1_10', pid: 3, position: 'MIDDLE' });
  const tl = makeTimeline(m, {}, [
    kill(3 * MIN, 3, 8, MID_LANE),                         // solo kill (no assisters field)
    kill(4 * MIN, 3, 8, MID_LANE, []),                     // solo kill (empty list)
    kill(6 * MIN, 3, 9, MID_LANE, [2]),                    // assisted kill
    kill(8 * MIN, 8, 3, MID_LANE),                         // death before 10
    kill(12 * MIN, 8, 3, MID_LANE, [7]),                   // death before 15, after 10
    kill(16 * MIN, 3, 8, MID_LANE),                        // after 15: ignored
  ]);

  it('counts solo kills, kills before 15 and deaths before 10', () => {
    expect(earlyStats(tl, 3)).toEqual({ kills15: 3, deaths15: 2, deaths10: 1, soloKills15: 2 });
  });

  it('flags a roam only for fights outside the own lane before 14:00', () => {
    expect(roamedInGame(tl, 3, 'MIDDLE', 100)).toBe(false);
    const roam = makeTimeline(m, {}, [kill(6 * MIN, 2, 6, TOP_LANE, [3])]);
    expect(roamedInGame(roam, 3, 'MIDDLE', 100)).toBe(true);
    const late = makeTimeline(m, {}, [kill(14 * MIN + 5000, 2, 6, TOP_LANE, [3])]);
    expect(roamedInGame(late, 3, 'MIDDLE', 100)).toBe(false);
    const lvl1 = makeTimeline(m, {}, [kill(70_000, 3, 6, RED_BLUEBUFF)]);
    expect(roamedInGame(lvl1, 3, 'MIDDLE', 100)).toBe(false);
  });

  it('analyzeGame wires match + timeline and skips remakes', () => {
    const g = analyzeGame(m, tl, P(3))!;
    expect(g.teamId).toBe(100);
    expect(g.early?.soloKills15).toBe(2);
    expect(g.roamed).toBe(false);
    expect(g.jungle).toBeNull();
    expect(g.csPerMin).toBeCloseTo(200 / 30, 5);
    expect(analyzeGame(makeMatch({ id: 'EUN1_11', pid: 3, durationSec: 200 }), null, P(3))).toBeNull();
    expect(analyzeGame(m, null, 'nobody')).toBeNull();
  });
});

// ---------------- aggregation & summary ----------------

describe('profile aggregation', () => {
  it('computes recent form from most recent first', () => {
    const f = recentForm([
      { win: true, endedAt: 1 }, { win: false, endedAt: 4 }, { win: false, endedAt: 3 }, { win: false, endedAt: 2 },
    ]);
    expect(f).toEqual({ wins: 1, losses: 3, streak: -3 });
    expect(recentForm([]).streak).toBe(0);
  });

  it('builds an aggressive bot-side jungler profile with a short Polish summary', () => {
    const games: GameStats[] = [];
    for (let i = 0; i < 8; i++) {
      const id = `EUN1_${100 + i}`;
      const m = makeMatch({ id, pid: 7, position: 'JUNGLE', champion: 'Warwick', win: i % 2 === 0, fbKill: i < 4, end: 1e12 + i * 1e7 });
      const tl = makeTimeline(m, { 2: { 7: RED_BLUEBUFF } }, [
        kill(3 * MIN + 20_000, 7, 4, BOT_LANE),
        kill(5 * MIN, 7, 3, BOT_LANE),
        kill(9 * MIN, 2, 7, MID_LANE),
        kill(11 * MIN, 7, 5, BOT_LANE, [8]),
      ]);
      games.push(analyzeGame(m, tl, P(7))!);
    }
    const p = buildProfile({ puuid: P(7), riotId: 'WW#EUNE', champion: 'Warwick', role: 'JUNGLE', rank: 'GOLD II 45 LP', games, now: 5 });
    expect(p.gamesAnalyzed).toBe(8);
    expect(p.championGames).toBe(8);
    expect(p.championWinRate).toBe(0.5);
    expect(p.firstBloodRate).toBe(0.5);
    expect(p.killsBefore15).toBe(3);
    expect(p.deathsBefore10).toBe(1);
    // 0.45·min(1,4/6) + 0.30·min(1,2) + 0.25·min(1,0.5/0.4) = 0.3 + 0.3 + 0.25
    expect(p.aggression).toBeCloseTo(0.85, 3);
    expect(p.jungle?.startBotSide).toBe(1);
    expect(p.jungle?.firstGankLane).toEqual({ BOTTOM: 1 });
    expect(p.roamRate).toBe(0);
    expect(p.summary).toBe('agresywny, start dół, gank bot ~3:20');
    expect(p.summary.split(' ').length).toBeLessThanOrEqual(6);
  });

  it('summarises a passive farmer on a new champion', () => {
    const games: GameStats[] = [];
    for (let i = 0; i < 6; i++) {
      const m = makeMatch({ id: `EUN1_${200 + i}`, pid: 4, position: 'BOTTOM', champion: i < 2 ? 'Jinx' : 'Caitlyn', kills: 4, deaths: 2 });
      m.info.participants[3]!.totalMinionsKilled = 230;
      games.push(analyzeGame(m, makeTimeline(m, {}, []), P(4))!);
    }
    const p = buildProfile({ puuid: P(4), riotId: 'x#y', champion: 'Jinx', role: 'BOTTOM', rank: null, games });
    expect(p.aggression).toBe(0);
    expect(p.championGames).toBe(2);
    expect(p.summary).toBe('pasywny farmer, 2 gry na postaci');
  });

  it('handles empty samples and Polish plurals/time formatting', () => {
    const p = buildProfile({ puuid: '', riotId: 'a#b', champion: 'Ahri', role: 'MIDDLE', rank: null, games: [] });
    expect(p.summary).toBe('brak danych');
    expect(p.championWinRate).toBeNull();
    expect(plPlural(1, 'gra', 'gry', 'gier')).toBe('gra');
    expect(plPlural(3, 'gra', 'gry', 'gier')).toBe('gry');
    expect(plPlural(12, 'gra', 'gry', 'gier')).toBe('gier');
    expect(plPlural(22, 'gra', 'gry', 'gier')).toBe('gry');
    expect(fmtMinute(3 + 20 / 60)).toBe('3:20');
    expect(summarize({ ...p, gamesAnalyzed: 5, aggression: 0.5, recentForm: { wins: 0, losses: 5, streak: -5 }, championGames: 3 }))
      .toBe('5 porażek z rzędu');
  });

  it('formats rank and parses Riot IDs', () => {
    expect(formatRank([{ queueType: 'RANKED_FLEX_SR', tier: 'SILVER', rank: 'I', leaguePoints: 3 }, { queueType: 'RANKED_SOLO_5x5', tier: 'GOLD', rank: 'II', leaguePoints: 45 }])).toBe('GOLD II 45 LP');
    expect(formatRank([{ queueType: 'RANKED_SOLO_5x5', tier: 'MASTER', rank: 'I', leaguePoints: 120 }])).toBe('MASTER 120 LP');
    expect(formatRank([])).toBeNull();
    expect(parseRiotId('Zażółć Gęślą#EUNE', 'X')).toEqual({ gameName: 'Zażółć Gęślą', tagLine: 'EUNE' });
    expect(parseRiotId('noTag', 'EUNE')).toEqual({ gameName: 'noTag', tagLine: 'EUNE' });
  });

  it('picks same-role, same-champion games first for timelines', () => {
    const ms = [
      makeMatch({ id: 'EUN1_1', pid: 2, position: 'JUNGLE', champion: 'LeeSin', end: 3e12 }),
      makeMatch({ id: 'EUN1_2', pid: 2, position: 'JUNGLE', champion: 'Warwick', end: 2e12 }),
      makeMatch({ id: 'EUN1_3', pid: 2, position: 'TOP', champion: 'Warwick', end: 4e12 }),
      makeMatch({ id: 'EUN1_4', pid: 2, position: 'JUNGLE', champion: 'Warwick', end: 1e12 }),
    ];
    expect(pickTimelineGames(ms, P(2), { role: 'JUNGLE', champion: 'Warwick' }).map(m => m.metadata.matchId)).toEqual(['EUN1_2', 'EUN1_4', 'EUN1_1']);
  });
});

// ---------------- limiter ----------------

describe('rate limiter', () => {
  afterEach(() => { vi.useRealTimers(); });

  it('grants 20 immediately, the 21st only after the 1 s window', async () => {
    vi.useFakeTimers({ now: 0 });
    const l = new RateLimiter(undefined, Date.now, 0);
    const granted: number[] = [];
    const all = Array.from({ length: 21 }, () => l.acquire().then(() => granted.push(Date.now())));
    await vi.advanceTimersByTimeAsync(0);
    expect(granted.length).toBe(20);
    await vi.advanceTimersByTimeAsync(999);
    expect(granted.length).toBe(20);
    await vi.advanceTimersByTimeAsync(1);
    await Promise.all(all);
    expect(granted.length).toBe(21);
    expect(granted[20]).toBe(1000);
  });

  it('honours the 100 / 120 s bucket too', async () => {
    vi.useFakeTimers({ now: 0 });
    const l = new RateLimiter(undefined, Date.now, 0);
    let n = 0;
    const all = Array.from({ length: 101 }, () => l.acquire().then(() => { n++; }));
    await vi.advanceTimersByTimeAsync(10_000);
    expect(n).toBe(100);                       // 5 s of 20/s bursts
    await vi.advanceTimersByTimeAsync(100_000);
    expect(n).toBe(100);                       // still inside 120 s of the first grant
    await vi.advanceTimersByTimeAsync(10_000);
    await Promise.all(all);
    expect(n).toBe(101);
  });

  it('rejects waits past the deadline without consuming a token, and pauses on Retry-After', async () => {
    vi.useFakeTimers({ now: 0 });
    const l = new RateLimiter([{ count: 1, windowMs: 1000 }], Date.now, 0);
    await l.acquire();
    await expect(l.acquire(500)).rejects.toBeInstanceOf(DeadlineError);
    expect(l.delay()).toBe(1000);
    await vi.advanceTimersByTimeAsync(1000);
    l.pause(3000);
    let ok = false;
    const p = l.acquire().then(() => { ok = true; });
    await vi.advanceTimersByTimeAsync(2999);
    expect(ok).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    await p;
    expect(ok).toBe(true);
  });
});

// ---------------- client: cache, retries, key hygiene ----------------

const KEY = 'RGAPI-test-key-0000';

function jsonRes(body: unknown, status = 200, headers: Record<string, string> = {}): Response {
  return new Response(status === 200 ? JSON.stringify(body) : '{}', { status, headers: { 'content-type': 'application/json', ...headers } });
}

describe('riot client', () => {
  const dirs: string[] = [];
  const tmp = () => { const d = mkdtempSync(path.join(tmpdir(), 'riot-cache-')); dirs.push(d); return d; };
  afterEach(() => { vi.useRealTimers(); for (const d of dirs.splice(0)) rmSync(d, { recursive: true, force: true }); });

  it('serves immutable matches from the disk cache across instances', async () => {
    const dir = tmp();
    const match = makeMatch({ id: 'EUN1_42' });
    const calls: string[] = [];
    const fetchImpl = (async (url: string, init?: RequestInit) => {
      calls.push(url);
      expect((init?.headers as Record<string, string>)['X-Riot-Token']).toBe(KEY);
      return jsonRes(match);
    }) as unknown as typeof fetch;
    const c1 = new RiotClient({ apiKey: KEY, cacheDir: dir, fetchImpl });
    expect((await c1.getMatch('EUN1_42'))?.metadata.matchId).toBe('EUN1_42');
    await c1.getMatch('EUN1_42');
    expect(calls).toEqual(['https://europe.api.riotgames.com/lol/match/v5/matches/EUN1_42']);
    const c2 = new RiotClient({ apiKey: KEY, cacheDir: dir, fetchImpl });
    await c2.getMatch('EUN1_42');
    expect(calls.length).toBe(1);
    expect(c2.requestCount).toBe(0);
  });

  it('expires TTL entries (match ids) and routes per platform/region', async () => {
    let now = 1_000_000;
    let calls = 0;
    const urls: string[] = [];
    const fetchImpl = (async (url: string) => { calls++; urls.push(url); return jsonRes(['EUN1_1']); }) as unknown as typeof fetch;
    const c = new RiotClient({ apiKey: KEY, cacheDir: tmp(), fetchImpl, now: () => now });
    await c.getMatchIds('abc', { queue: 420, count: 20 });
    await c.getMatchIds('abc', { queue: 420, count: 20 });
    expect(calls).toBe(1);
    expect(urls[0]).toBe('https://europe.api.riotgames.com/lol/match/v5/matches/by-puuid/abc/ids?queue=420&start=0&count=20');
    now += 11 * 60_000;
    await c.getMatchIds('abc', { queue: 420, count: 20 });
    expect(calls).toBe(2);
    await c.getLeagueEntries('abc');
    expect(urls[2]).toBe('https://eun1.api.riotgames.com/lol/league/v4/entries/by-puuid/abc');
    const na = new RiotClient({ apiKey: KEY, platform: 'na1', fetchImpl });
    expect(na.region).toBe('americas');
  });

  it('returns null on 404 without caching it, and de-duplicates in-flight requests', async () => {
    let calls = 0;
    const fetchImpl = (async () => { calls++; return jsonRes(null, 404); }) as unknown as typeof fetch;
    const c = new RiotClient({ apiKey: KEY, fetchImpl });
    const [a, b] = await Promise.all([c.getTimeline('EUN1_9'), c.getTimeline('EUN1_9')]);
    expect(a).toBeNull();
    expect(b).toBeNull();
    expect(calls).toBe(1);
    await c.getTimeline('EUN1_9');
    expect(calls).toBe(2);
  });

  it('retries 429 after Retry-After and 5xx with backoff; errors never contain the key', async () => {
    vi.useFakeTimers({ now: 0 });
    const seq = [jsonRes(null, 429, { 'retry-after': '2' }), jsonRes(null, 503), jsonRes({ puuid: 'p1' })];
    const times: number[] = [];
    const fetchImpl = (async () => { times.push(Date.now()); return seq.shift()!; }) as unknown as typeof fetch;
    const c = new RiotClient({ apiKey: KEY, fetchImpl });
    const p = c.getAccountByRiotId('Gracz', 'EUNE');
    await vi.advanceTimersByTimeAsync(10_000);
    expect(await p).toEqual({ puuid: 'p1' });
    expect(times[1]! - times[0]!).toBeGreaterThanOrEqual(2000);
    expect(times[2]! - times[1]!).toBeGreaterThanOrEqual(500);

    const bad = new RiotClient({ apiKey: KEY, maxRetries: 0, fetchImpl: (async () => jsonRes(null, 403)) as unknown as typeof fetch });
    const err: unknown = await bad.getMatch('EUN1_1').catch((e: unknown) => e);
    expect(err).toBeInstanceOf(Error);
    expect(String((err as Error).message)).not.toContain(KEY);
    expect((err as { status?: number }).status).toBe(403);
    expect(() => new RiotClient({ apiKey: '  ' })).toThrow(/missing/);
  });

  it('enforces request budgets but still serves cache hits', async () => {
    const fetchImpl = (async (url: string) => jsonRes(makeMatch({ id: url.split('/').pop()! }))) as unknown as typeof fetch;
    const c = new RiotClient({ apiKey: KEY, fetchImpl });
    const b = new RequestBudget(1);
    await c.getMatch('EUN1_1', { budget: b });
    await expect(c.getMatch('EUN1_2', { budget: b })).rejects.toBeInstanceOf(BudgetExceededError);
    expect((await c.getMatch('EUN1_1', { budget: b }))?.metadata.matchId).toBe('EUN1_1');
    const parent = new RequestBudget(1);
    const child = new RequestBudget(5, parent);
    expect(child.take()).toBe(true);
    expect(child.take()).toBe(false);
  });
});

// ---------------- orchestration ----------------

describe('buildProfiles', () => {
  it('builds 5 profiles from a cold cache within the request budget', async () => {
    const roles = ['TOP', 'JUNGLE', 'MIDDLE', 'BOTTOM', 'UTILITY'] as const;
    const players = roles.map((role, i) => ({ riotId: `Enemy${i}#EUNE`, champion: i === 1 ? 'Warwick' : `Champ${i}`, role }));
    // Each player has 20 ranked games; participant index i+1 (blue) or i+6 (red), role-matched.
    const matchFor = (pi: number, k: number): MatchDto => {
      const pid = (k % 2 ? 6 : 1) + pi;
      return makeMatch({
        id: `EUN1_${pi}${String(k).padStart(3, '0')}`, pid, puuid: `pu-${pi}`, position: roles[pi],
        champion: players[pi]!.champion, win: k % 3 !== 0, end: 2e12 + k * 1e7, queueId: k === 15 ? 450 : 420,
      });
    };
    let requests = 0;
    const fetchImpl = (async (url: string) => {
      requests++;
      const u = new URL(url);
      let m: RegExpMatchArray | null;
      if ((m = u.pathname.match(/by-riot-id\/Enemy(\d)\//))) return jsonRes({ puuid: `pu-${m[1]}` });
      if (u.pathname.includes('/league/v4/')) return jsonRes([{ queueType: 'RANKED_SOLO_5x5', tier: 'GOLD', rank: 'II', leaguePoints: 45 }]);
      if ((m = u.pathname.match(/by-puuid\/pu-(\d)\/ids/))) {
        if (u.searchParams.get('type') !== 'ranked') return jsonRes([]);
        return jsonRes(Array.from({ length: 20 }, (_, k) => `EUN1_${m![1]}${String(k).padStart(3, '0')}`));
      }
      if ((m = u.pathname.match(/matches\/EUN1_(\d)(\d{3})(\/timeline)?$/))) {
        const match = matchFor(Number(m[1]), Number(m[2]));
        if (!m[3]) return jsonRes(match);
        const pid = match.info.participants.find(p => p.puuid === `pu-${m![1]}`)!.participantId;
        const team = pid <= 5 ? 100 : 200;
        return jsonRes(makeTimeline(match, { 2: { [pid]: team === 100 ? BLUE_REDBUFF : RED_BLUEBUFF } }, [kill(3 * MIN + 20_000, pid, team === 100 ? 9 : 4, BOT_LANE)]));
      }
      if (u.pathname.includes('champion-mastery')) return jsonRes(null, 404);
      return jsonRes(null, 404);
    }) as unknown as typeof fetch;
    const client = new RiotClient({ apiKey: KEY, fetchImpl, limits: [{ count: 10_000, windowMs: 1000 }] });
    const progress: string[] = [];
    const profiles = await buildProfiles(players, { client, onProgress: p => progress.push(`${p.riotId}:${p.stage}`) });

    expect(profiles).toHaveLength(5);
    expect(requests).toBeLessThanOrEqual(95);
    expect(client.requestCount).toBe(requests);
    const ww = profiles[1]!;
    expect(ww.puuid).toBe('pu-1');
    expect(ww.rank).toBe('GOLD II 45 LP');
    expect(ww.gamesAnalyzed).toBe(11);          // 12 fetched, one ARAM (queue 450) dropped
    expect(ww.jungle?.sampleGames).toBe(10);
    expect(ww.jungle?.startBotSide).toBe(1);     // blue at red buff & red at blue buff are both bot side
    expect(ww.jungle?.firstGankLane).toEqual({ BOTTOM: 1 });
    expect(ww.summary).toContain('start dół');
    expect(profiles[0]!.gamesAnalyzed).toBe(6);   // 7 newest fetched, ARAM dropped
    expect(progress.filter(s => s.endsWith(':done'))).toHaveLength(5);

    // Warm run with no network allowance: everything (immutable or within TTL) comes from cache.
    const before = requests;
    const warm = await buildProfiles(players, { client, maxMatchFetch: { jungle: 0, laner: 0 }, maxTimelines: { jungle: 0, laner: 0 } });
    expect(requests).toBe(before);
    expect(warm.map(p => p.gamesAnalyzed)).toEqual(profiles.map(p => p.gamesAnalyzed));
    expect(warm[1]!.jungle?.sampleGames).toBe(10);
    // A normal warm run extends the sample with new downloads instead of re-fetching.
    const extended = await buildProfiles(players, { client });
    expect(extended[1]!.gamesAnalyzed).toBeGreaterThan(11);
  });

  it('shrinks the plan when the limiter has little room left', async () => {
    let requests = 0;
    const fetchImpl = (async () => { requests++; return jsonRes({ puuid: 'p' }); }) as unknown as typeof fetch;
    const client = new RiotClient({ apiKey: KEY, fetchImpl });
    const l = client.limiterFor('europe');
    for (let i = 0; i < 95; i++) l['log'].push(Date.now());   // 95 of 100 tokens used just now
    await buildProfiles([{ riotId: 'A#EUNE', champion: 'Ahri', role: 'MIDDLE' }], { client, timeoutMs: 5000 });
    expect(requests).toBeLessThanOrEqual(2);                   // only room for ~2 regional calls (+ platform)
  });

  it('returns partial profiles when a player cannot be resolved', async () => {
    const fetchImpl = (async () => jsonRes(null, 404)) as unknown as typeof fetch;
    const client = new RiotClient({ apiKey: KEY, fetchImpl });
    const r = await buildProfiles([{ riotId: 'Ghost#EUNE', champion: 'Ahri', role: 'MIDDLE' }], { client });
    expect(r[0]!.summary).toBe('brak danych');
    expect(r[0]!.gamesAnalyzed).toBe(0);
  });
});
