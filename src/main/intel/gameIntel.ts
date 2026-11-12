import type { GameIntel, ObjectiveTimer, PlayerIntel, Role, Team, TeamIntel } from '../../shared/intel';
import type { AllGameData } from '../sources/liveApi';
import { estimateMoveSpeed, type ItemData } from './ddragon';

/**
 * Objective timings (seconds). These change between seasons — keep them in one
 * place. Season 2026: Atakhan removed, Baron back at 20:00 (sheepesports/altchar S1 2026 coverage).
 */
export const OBJ = {
  dragonFirst: 5 * 60,
  dragonRespawn: 5 * 60,
  elderRespawn: 6 * 60,
  grubsFirst: 8 * 60,
  heraldFirst: 15 * 60,
  baronFirst: 20 * 60,
  baronRespawn: 6 * 60,
  atakhanFirst: Infinity, // removed in 2026
  inhibRespawn: 5 * 60,
};

const DRAGON_PL: Record<string, string> = {
  Fire: 'Piekielny', Earth: 'Górski', Water: 'Oceaniczny', Air: 'Chmur', Chemtech: 'Chemtech', Hextech: 'Hextech', Elder: 'Starszy',
};

const ROLE_MAP: Record<string, Role> = { TOP: 'TOP', JUNGLE: 'JUNGLE', MIDDLE: 'MIDDLE', BOTTOM: 'BOTTOM', UTILITY: 'UTILITY' };

type LivePlayer = AllGameData['allPlayers'][number];
type LiveEvent = AllGameData['events']['Events'][number];

const nameOf = (p: { riotIdGameName?: string; riotId?: string; summonerName?: string }) =>
  (p as { riotIdGameName?: string }).riotIdGameName ?? p.riotId?.split('#')[0] ?? p.summonerName ?? '';

/** Passive gold + farm + takedowns: a rough "gold earned so far". */
export function estimateGoldEarned(gameTime: number, p: { cs: number; kills: number; assists: number; wardScore: number }) {
  const passive = Math.max(0, gameTime - 65) * 2.04;          // ~20.4 gold / 10 s after 1:05
  return 500 + passive + p.cs * 21 + p.kills * 300 + p.assists * 120 + p.wardScore * 2;
}

function roleOf(p: LivePlayer): Role {
  if (ROLE_MAP[p.position]) return ROLE_MAP[p.position]!;
  const spells = Object.values(p.summonerSpells ?? {}).map(s => s.displayName.toLowerCase());
  return spells.some(s => s.includes('smite') || s.includes('porażenie')) ? 'JUNGLE' : 'UNKNOWN';
}

/**
 * Stateful: turns Live API snapshots into GameIntel. Keeps history needed for
 * spikes (level/item thresholds crossed) and manual Flash timers.
 */
export class IntelTracker {
  private prev = new Map<string, { level: number; completed: number }>();
  private spikes = new Map<string, PlayerIntel['spikes']>();
  private flashUsedAt = new Map<string, number>();
  private gameId = '';
  private hp = new Map<string, { pct: number; at: number }>();

  /** HP fraction of a champion read from the screen (health bar), at game time t. */
  setHp(champion: string, pct: number, t: number) { this.hp.set(champion, { pct, at: t }); }

  constructor(
    private items: () => Map<number, ItemData> | null,
    private champStats: () => Map<string, { moveSpeed: number; attackRange: number }> | null = () => null,
  ) {}

  /** Manual hotkey: "enemy X used Flash". */
  markFlash(champion: string, gameTime: number) { this.flashUsedAt.set(champion, gameTime); }

  update(d: AllGameData): GameIntel {
    const t = d.gameData.gameTime;
    const key = d.allPlayers.map(p => p.championName).join('|');
    if (key !== this.gameId) { this.gameId = key; this.prev.clear(); this.spikes.clear(); this.flashUsedAt.clear(); this.hp.clear(); }

    const meName = d.activePlayer.riotId?.split('#')[0] ?? d.activePlayer.summonerName ?? '';
    const me = d.allPlayers.find(p => nameOf(p) === meName || p.riotId === d.activePlayer.riotId);
    const myTeam: Team = (me?.team as Team) ?? 'ORDER';
    const itemDb = this.items();

    const players: PlayerIntel[] = d.allPlayers.map(p => {
      const items = p.items.map(i => {
        const db = itemDb?.get(i.itemID);
        return { id: i.itemID, name: db?.name ?? i.displayName, price: db?.total ?? i.price, completed: db?.completed ?? false, shop: !!db?.purchasable };
      });
      const itemGold = items.reduce((s, i) => s + i.price, 0);
      // Gold of items that can only come from the shop: proof of a base visit. Items that evolve on
      // their own (jungle pet, support quest, transforms) are not in the shop and must not count.
      const shopGold = items.filter(i => i.shop).reduce((s, i) => s + i.price, 0);
      const completed = items.filter(i => i.completed).length;
      const sc = p.scores;
      const prev = this.prev.get(p.championName);
      const spikes = this.spikes.get(p.championName) ?? [];
      if (prev) {
        for (const lvl of [6, 11, 16]) if (prev.level < lvl && p.level >= lvl) spikes.unshift({ kind: 'level', label: `poziom ${lvl}`, at: t });
        if (completed > prev.completed) {
          const newest = items.filter(i => i.completed).at(-1);
          spikes.unshift({ kind: 'item', label: newest?.name ?? 'nowy przedmiot', at: t });
        }
      }
      this.prev.set(p.championName, { level: p.level, completed });
      this.spikes.set(p.championName, spikes.slice(0, 6));
      const flashAt = this.flashUsedAt.get(p.championName);
      const flashIn = flashAt === undefined ? null : Math.max(0, flashAt + 300 - t);
      return {
        champion: p.championName,
        team: p.team as Team,
        isAlly: p.team === myTeam,
        isMe: p === me,
        role: roleOf(p),
        level: p.level,
        kills: sc.kills, deaths: sc.deaths, assists: sc.assists, cs: sc.creepScore, wardScore: Math.round(sc.wardScore),
        isDead: p.isDead,
        respawnIn: p.isDead ? Math.max(0, Math.round(p.respawnTimer)) : 0,
        itemGold,
        shopGold,
        estGold: Math.round(Math.max(itemGold, estimateGoldEarned(t, { cs: sc.creepScore, kills: sc.kills, assists: sc.assists, wardScore: sc.wardScore }))),
        items: items.map(({ id, name, price }) => ({ id, name, price })),
        completedItems: completed,
        spikes: this.spikes.get(p.championName)!,
        summonerSpells: Object.values(p.summonerSpells ?? {}).map(s => s.displayName),
        hpPct: p === me && d.activePlayer.championStats?.maxHealth ? (d.activePlayer.championStats.currentHealth ?? 0) / d.activePlayer.championStats.maxHealth : (p.isDead ? 0 : this.hp.get(p.championName)?.pct ?? null),
        hpSeenAt: p === me ? t : this.hp.get(p.championName)?.at ?? null,
        moveSpeed: p === me && d.activePlayer.championStats?.moveSpeed
          ? Math.round(d.activePlayer.championStats.moveSpeed)
          : estimateMoveSpeed(this.champStats()?.get(p.championName)?.moveSpeed ?? 340, p.items.map(i => itemDb?.get(i.itemID) ?? { flatMs: 0, pctMs: 0 })),
        flashIn,
      };
    });

    const nameTeam = new Map<string, Team>();
    for (const p of d.allPlayers) nameTeam.set(nameOf(p), p.team as Team);
    const teamOfKiller = (n: unknown): Team | null => {
      const s = String(n ?? '');
      if (nameTeam.has(s)) return nameTeam.get(s)!;
      const bot = d.allPlayers.find(p => `Bot ${p.championName}` === s); // practice tool bots
      if (bot) return bot.team as Team;
      if (/T(Order|100)/i.test(s) || /_T1_/.test(s)) return 'ORDER';   // minions/turrets
      if (/T(Chaos|200)/i.test(s) || /_T2_/.test(s)) return 'CHAOS';
      return null;
    };

    const { ally, enemy, objectives, recent, myDeaths, buffs, streaks, ace } = this.events(d.events.Events, t, myTeam, teamOfKiller, players, meName, me?.championName ?? null, new Map(d.allPlayers.map(p => [nameOf(p), p.championName])));
    ally.estGold = players.filter(p => p.isAlly).reduce((s, p) => s + p.estGold, 0);
    enemy.estGold = players.filter(p => !p.isAlly).reduce((s, p) => s + p.estGold, 0);
    ally.aliveCount = players.filter(p => p.isAlly && !p.isDead).length;
    enemy.aliveCount = players.filter(p => !p.isAlly && !p.isDead).length;

    const s = d.activePlayer.championStats ?? {};
    return {
      gameTime: t,
      gameMode: d.gameData.gameMode,
      me: players.find(p => p.isMe) ?? null,
      players,
      ally, enemy,
      goldDiff: ally.estGold - enemy.estGold,
      objectives,
      recent,
      myDeaths, buffs, streaks, ace,
      myStats: me ? {
        gold: Math.round(d.activePlayer.currentGold), hp: Math.round(s.currentHealth ?? 0), maxHp: Math.round(s.maxHealth ?? 0),
        ad: Math.round(s.attackDamage ?? 0), ap: Math.round(s.abilityPower ?? 0), armor: Math.round(s.armor ?? 0),
        mr: Math.round(s.magicResist ?? 0), moveSpeed: Math.round(s.moveSpeed ?? 0),
      } : null,
    };
  }

  private events(events: LiveEvent[], t: number, myTeam: Team, teamOf: (n: unknown) => Team | null, players: PlayerIntel[], meName: string, meChamp: string | null, playerChamp: Map<string, string> = new Map()) {
    const blank = (): TeamIntel => ({ kills: 0, dragons: [], towers: 0, inhibitors: 0, barons: 0, heralds: 0, estGold: 0, aliveCount: 5 });
    const ally = blank(), enemy = blank();
    const side = (team: Team | null) => (team === null ? null : team === myTeam ? ally : enemy);
    const recent: GameIntel['recent'] = [];
    let lastDragon: number | null = null, lastBaron: number | null = null, heraldTaken = false, grubsTaken = 0, atakhanTaken = false;
    let elderUnlocked = false;
    const inhibs: Array<{ at: number; mine: boolean; name: string }> = [];
    const myDeaths: GameIntel['myDeaths'] = [];
    const buffs: GameIntel['buffs'] = [];
    const streaks: Record<string, number> = {};
    let ace: GameIntel['ace'] = null;
    const isMe = (n: unknown) => String(n ?? '') === meName || (meChamp !== null && String(n ?? '').replace(/^Bot /, '') === meChamp);
    // Real games name players (Riot ID game name) in events; practice tool uses 'Bot <Champion>'.
    const champOf = (n: unknown) => {
      const raw = String(n ?? '');
      if (playerChamp.has(raw)) return playerChamp.get(raw)!;
      const s = raw.replace(/^Bot /, '');
      return players.find(p => p.champion === s)?.champion ?? s;
    };

    // Robustness: ignore events from the future and duplicates (replays re-emit events after a seek).
    const seen = new Set<string>();
    const clean = events.filter(e => {
      if (e.EventTime > t + 1) return false;
      const k = `${e.EventName}|${Math.round(e.EventTime)}|${String(e.KillerName ?? '')}|${String(e.VictimName ?? '')}`;
      if (seen.has(k)) return false;
      seen.add(k);
      return true;
    }).sort((a, b) => a.EventTime - b.EventTime);
    for (const e of clean) {
      const killerTeam = teamOf(e.KillerName);
      const sd = side(killerTeam);
      const who = killerTeam === myTeam ? 'my' : 'wróg';
      switch (e.EventName) {
        case 'ChampionKill': {
          const killer = champOf(e.KillerName), victim = champOf(e.VictimName);
          streaks[killer] = (streaks[killer] ?? 0) + 1;
          streaks[victim] = 0;
          if (isMe(e.VictimName)) {
            const assisters = ((e.Assisters as string[] | undefined) ?? []).map(champOf);
            // A gank = the kill involved an enemy who isn't my lane opponent (jungler/roamer) or ≥2 enemies.
            const laneOpp = players.find(p => !p.isAlly && p.role === players.find(x => x.isMe)?.role)?.champion;
            const involved = [killer, ...assisters].filter(c => players.some(p => p.champion === c && !p.isAlly));
            myDeaths.unshift({ at: e.EventTime, killer, assisters, enemiesInvolved: involved.length, wasGank: involved.length >= 2 || (laneOpp !== undefined && !involved.includes(laneOpp)) });
          }
        }
          if (sd) sd.kills++;
          recent.unshift({ at: e.EventTime, text: `${champOf(e.KillerName)} zabił ${champOf(e.VictimName)}` });
          break;
        case 'DragonKill': {
          const type = String(e.DragonType ?? '');
          if (type === 'Elder') { if (killerTeam) buffs.push({ kind: 'elder', team: killerTeam === myTeam ? 'ally' : 'enemy', until: e.EventTime + 150 }); lastDragon = e.EventTime; recent.unshift({ at: e.EventTime, text: `Starszy smok: ${who}` }); break; }
          sd?.dragons.push(type);
          if (sd && sd.dragons.length >= 4) elderUnlocked = true;
          lastDragon = e.EventTime;
          recent.unshift({ at: e.EventTime, text: `Smok ${DRAGON_PL[type] ?? type}: ${who}` });
          break;
        }
        case 'BaronKill': if (sd) sd.barons++; lastBaron = e.EventTime; if (killerTeam) buffs.push({ kind: 'baron', team: killerTeam === myTeam ? 'ally' : 'enemy', until: e.EventTime + 180 }); recent.unshift({ at: e.EventTime, text: `Baron: ${who}` }); break;
        case 'HeraldKill': if (sd) sd.heralds++; heraldTaken = true; recent.unshift({ at: e.EventTime, text: `Herald: ${who}` }); break;
        case 'HordeKill': grubsTaken++; break;
        case 'Ace': {
          const acing = String(e.AcingTeam ?? '');
          const team: Team | null = acing === 'ORDER' || acing === 'CHAOS' ? acing : null;
          if (team) ace = { team: team === myTeam ? 'ally' : 'enemy', at: e.EventTime };
          break;
        }
        case 'AtakhanKill': atakhanTaken = true; recent.unshift({ at: e.EventTime, text: `Atakhan: ${who}` }); break;
        case 'TurretKilled': if (sd) sd.towers++; recent.unshift({ at: e.EventTime, text: `Wieża: ${who}` }); break;
        case 'InhibKilled': {
          if (sd) sd.inhibitors++;
          const name = String(e.InhibKilled ?? '');
          const lane = /_L0_|_L1_|_L2_/.exec(name)?.[0];
          const laneName = lane === '_L0_' ? 'bot' : lane === '_L1_' ? 'mid' : lane === '_L2_' ? 'top' : '';
          inhibs.push({ at: e.EventTime, mine: killerTeam !== myTeam, name: laneName });
          break;
        }
      }
    }

    const objectives: ObjectiveTimer[] = [];
    const dragonAt = lastDragon === null ? OBJ.dragonFirst : lastDragon + (elderUnlocked ? OBJ.elderRespawn : OBJ.dragonRespawn);
    objectives.push({ kind: 'dragon', label: elderUnlocked ? 'Starszy smok' : 'Smok', spawnsAt: dragonAt > t ? dragonAt : null, alive: dragonAt <= t });
    if (t < OBJ.heraldFirst + 5 * 60 && grubsTaken < 6) objectives.push({ kind: 'grubs', label: 'Larwy', spawnsAt: OBJ.grubsFirst > t ? OBJ.grubsFirst : null, alive: OBJ.grubsFirst <= t });
    if (!heraldTaken && t < OBJ.baronFirst - 10) objectives.push({ kind: 'herald', label: 'Herold', spawnsAt: OBJ.heraldFirst > t ? OBJ.heraldFirst : null, alive: OBJ.heraldFirst <= t });
    if (Number.isFinite(OBJ.atakhanFirst) && !atakhanTaken && t < OBJ.baronFirst + 60) objectives.push({ kind: 'atakhan', label: 'Atakhan', spawnsAt: OBJ.atakhanFirst > t ? OBJ.atakhanFirst : null, alive: OBJ.atakhanFirst <= t });
    const baronAt = lastBaron === null ? OBJ.baronFirst : lastBaron + OBJ.baronRespawn;
    objectives.push({ kind: 'baron', label: 'Baron', spawnsAt: baronAt > t ? baronAt : null, alive: baronAt <= t });
    for (const i of inhibs) {
      const back = i.at + OBJ.inhibRespawn;
      if (back > t) objectives.push({ kind: 'inhibitor', label: `Inhib ${i.name} (${i.mine ? 'nasz' : 'wroga'})`, spawnsAt: back, alive: false });
    }
    return {
      ally, enemy, objectives, recent: recent.slice(0, 10), myDeaths,
      buffs: buffs.filter(b => b.until > t), streaks,
      ace: ace && t - (ace as { at: number }).at < 60 ? ace : null,
    };
  }
}
