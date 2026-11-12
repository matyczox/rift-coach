import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { IntelTracker, OBJ, estimateGoldEarned } from '../src/main/intel/gameIntel';
import { parseItems } from '../src/main/intel/ddragon';
import { buildSnapshot, Coach } from '../src/main/coach/coach';
import type { AllGameData } from '../src/main/sources/liveApi';
import type { CoachAdvice } from '../src/shared/intel';

const load = () => JSON.parse(readFileSync(path.join(__dirname, 'fixtures', 'allgamedata.json'), 'utf8')) as AllGameData;

const items = parseItems({
  data: {
    '1055': { name: 'Ostrze Dorana', gold: { total: 450, purchasable: true }, tags: ['Damage'] },
    '6333': { name: 'Taniec Śmierci', gold: { total: 3300, purchasable: true }, tags: ['Damage'] },
    '3174': { name: 'Opancerzone Natarcie', gold: { total: 1000, purchasable: true }, tags: ['Boots'] },
    '1036': { name: 'Długi Miecz', gold: { total: 350, purchasable: true }, into: ['6333'], tags: ['Damage'] },
  },
});

describe('ddragon items', () => {
  it('flags completed items and components', () => {
    expect(items.get(6333)!.completed).toBe(true);
    expect(items.get(1036)!.completed).toBe(false);
    expect(items.get(1055)!.completed).toBe(false);
    expect(items.get(3174)!.completed).toBe(true); // tier-2 boots
  });
});

describe('IntelTracker on a real practice-tool snapshot', () => {
  const d = load();
  const intel = new IntelTracker(() => items).update(d);

  it('identifies me and splits teams', () => {
    expect(intel.me?.champion).toBe('Ambessa');
    expect(intel.players.filter(p => p.isAlly)).toHaveLength(5);
    expect(intel.players.find(p => p.champion === 'Warwick')!.isAlly).toBe(false);
  });

  it('uses full item prices, not recipe costs', () => {
    const me = intel.me!;
    expect(me.items.find(i => i.id === 6333)!.price).toBe(3300);
    expect(me.completedItems).toBeGreaterThanOrEqual(1);
  });

  it('counts objectives from events (bot names map to teams)', () => {
    expect(intel.enemy.dragons.length + intel.ally.dragons.length).toBeGreaterThan(0);
    expect(intel.ally.kills + intel.enemy.kills).toBeGreaterThan(0);
    const dragon = intel.objectives.find(o => o.kind === 'dragon')!;
    expect(dragon.alive || (dragon.spawnsAt! > intel.gameTime)).toBe(true);
  });

  it('builds a compact coach snapshot', () => {
    const s = buildSnapshot({ intel, ghosts: [], alerts: [], profiles: [] }, []);
    expect(s.me?.champ).toBe('Ambessa');
    expect(s.enemies).toHaveLength(5);
    expect(JSON.stringify(s).length).toBeLessThan(6000);
  });
});

describe('spikes and timers', () => {
  it('records a level-6 spike between snapshots', () => {
    const d = load();
    const tr = new IntelTracker(() => items);
    const p = d.allPlayers.find(x => x.championName === 'Warwick')!;
    p.level = 5; tr.update(d);
    p.level = 6; d.gameData.gameTime += 2;
    const intel = tr.update(d);
    expect(intel.players.find(x => x.champion === 'Warwick')!.spikes[0]?.label).toBe('poziom 6');
  });

  it('dragon respawns 5 min after the last kill', () => {
    const d = load();
    d.events.Events = [{ EventID: 1, EventName: 'DragonKill', EventTime: 600, DragonType: 'Fire', KillerName: 'Bot Warwick' }];
    d.gameData.gameTime = 700;
    const intel = new IntelTracker(() => items).update(d);
    const dragon = intel.objectives.find(o => o.kind === 'dragon')!;
    expect(dragon.spawnsAt).toBe(600 + OBJ.dragonRespawn);
    expect(intel.enemy.dragons).toEqual(['Fire']);
  });

  it('gold estimate grows with time and farm', () => {
    expect(estimateGoldEarned(600, { cs: 80, kills: 2, assists: 1, wardScore: 5 })).toBeGreaterThan(estimateGoldEarned(300, { cs: 30, kills: 0, assists: 0, wardScore: 0 }));
  });
});

describe('coach freshness', () => {
  it('drops stale or wrong-state advice', () => {
    const intel = new IntelTracker(() => items).update(load());
    const base: CoachAdvice = { call: 'x', tone: 'go', mapHint: null, item: null, ward: null, deathReview: null, ttlSec: 20, contextId: 'c', createdAt: 0, gameTime: intel.gameTime, latencyMs: 0 };
    expect(Coach.isFresh(base, intel)).toBe(true);
    expect(Coach.isFresh({ ...base, gameTime: intel.gameTime - 30 }, intel)).toBe(false);
    expect(Coach.isFresh({ ...base, deathReview: ['a', 'b'] }, intel)).toBe(intel.me!.isDead);
  });
});

import { bestWardSpot, wardPoint } from '../src/shared/wards';
describe('ward spots', () => {
  it('picks the highest-value spot for an objective', () => {
    expect(bestWardSpot('dragon')).toBe('faelight-river-bot');
    expect(bestWardSpot('gank-mid', undefined, false, false)).not.toMatch(/faelight-mid/);
  });
  it('mirrors side-relative spots for red side but not neutral ones', () => {
    expect(wardPoint('my-raptors', true)).toEqual({ x: 1 - 0.47, y: 1 - 0.60 });
    expect(wardPoint('dragon-pit-entrance', true)).toEqual({ x: 0.62, y: 0.66 });
  });
});

import { smartWardSpot } from '../src/shared/wards';
describe('vision-aware wards', () => {
  it('skips spots already in vision and picks where the enemy probably is', () => {
    const none = smartWardSpot({ purposes: ['dragon'] });
    expect(none?.id).toBe('faelight-river-bot');
    // That area is already visible (ward/ally there) -> pick another dragon spot.
    const seen = (x: number, y: number) => (Math.hypot(x - 0.76, y - 0.70) < 0.08 ? 1 : 0);
    const alt = smartWardSpot({ purposes: ['dragon'], visible: seen });
    expect(alt?.id).not.toBe('faelight-river-bot');
    // Everything visible -> nothing to ward.
    expect(smartWardSpot({ purposes: ['dragon'], visible: () => 1 })).toBeNull();
  });
});

import { assignRoles, laneOpponentFor, type RoleRates } from '../src/main/runes/roles';
describe('draft role assignment', () => {
  const r = (t: number, j: number, m: number, b: number, u: number): RoleRates => ({ TOP: t, JUNGLE: j, MIDDLE: m, BOTTOM: b, UTILITY: u });
  const rates = new Map<string, RoleRates>([
    ['Warwick', r(0.22, 0.75, 0.01, 0, 0.02)],
    ['Lee Sin', r(0.03, 0.95, 0.02, 0, 0)],
    ['Darius', r(0.85, 0.08, 0.05, 0.01, 0.01)],
    ['Jinx', r(0, 0, 0.01, 0.98, 0.01)],
  ]);
  it('Warwick alone is most likely the jungler (too unsure to call him top)', () => {
    expect(laneOpponentFor('TOP', assignRoles(['Warwick', 'Jinx'], rates))!.p).toBeLessThan(0.6);
  });
  it('Warwick next to Lee Sin becomes the top laner', () => {
    const top = laneOpponentFor('TOP', assignRoles(['Warwick', 'Lee Sin', 'Jinx'], rates))!;
    expect(top.champion).toBe('Warwick');
    expect(top.p).toBeGreaterThan(0.6);
  });
  it('a real top laner wins the top slot', () => {
    expect(laneOpponentFor('TOP', assignRoles(['Warwick', 'Darius'], rates))?.champion).toBe('Darius');
  });
});

describe('death context and streaks from events', () => {
  it('knows who killed me, with helpers', () => {
    const intel = new IntelTracker(() => items).update(load());
    expect(intel.myDeaths.length).toBeGreaterThan(0);
    const last = intel.myDeaths[0]!;
    expect(last.killer).not.toBe('');
    expect(last.enemiesInvolved).toBeGreaterThanOrEqual(1);
  });
  it('tracks kill streaks and baron buff windows', () => {
    const d = load();
    d.gameData.gameTime = 1300;
    d.events.Events = [
      { EventID: 1, EventName: 'ChampionKill', EventTime: 1000, KillerName: 'Bot Warwick', VictimName: 'Bot Fizz', Assisters: [] },
      { EventID: 2, EventName: 'ChampionKill', EventTime: 1010, KillerName: 'Bot Warwick', VictimName: 'Bot Sona', Assisters: [] },
      { EventID: 3, EventName: 'BaronKill', EventTime: 1200, KillerName: 'Bot Warwick' },
    ];
    const intel = new IntelTracker(() => items).update(d);
    expect(intel.streaks.Warwick).toBe(2);
    expect(intel.buffs).toEqual([{ kind: 'baron', team: 'enemy', until: 1380 }]);
  });
});

describe('real-game event names', () => {
  it('maps player names in kill events to champions (death context, streaks)', () => {
    const d = load();
    const ww = d.allPlayers.find(p => p.championName === 'Warwick')!;
    (ww as { riotIdGameName?: string }).riotIdGameName = 'SomeJungler';
    ww.riotId = 'SomeJungler#EUNE';
    d.gameData.gameTime = 900;
    d.events.Events = [{ EventID: 1, EventName: 'ChampionKill', EventTime: 880, KillerName: 'SomeJungler', VictimName: 'playerAmbessa', Assisters: ['Bot Lulu'] }];
    const intel = new IntelTracker(() => items).update(d);
    expect(intel.myDeaths[0]).toMatchObject({ killer: 'Warwick', assisters: ['Lulu'], enemiesInvolved: 2, wasGank: true });
    expect(intel.streaks.Warwick).toBe(1);
  });
});

describe('event robustness', () => {
  it('ignores future and duplicated events', () => {
    const d = load();
    d.gameData.gameTime = 200;
    const kill = { EventID: 1, EventName: 'ChampionKill', EventTime: 150, KillerName: 'Bot Warwick', VictimName: 'playerAmbessa', Assisters: [] };
    d.events.Events = [kill, { ...kill, EventID: 9 }, { ...kill, EventID: 2, EventTime: 400 }];
    const intel = new IntelTracker(() => items).update(d);
    expect(intel.myDeaths).toHaveLength(1);
  });
});
