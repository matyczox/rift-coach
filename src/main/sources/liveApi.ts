import { EventEmitter } from 'node:events';
import { getJson } from './http';
import type { GameInfo, SourceState } from '../../shared/types';

/** Subset of /liveclientdata/allgamedata we rely on; the raw JSON is kept too. */
export interface AllGameData {
  activePlayer: { riotId?: string; summonerName?: string; currentGold: number; level: number; championStats: Record<string, number> };
  allPlayers: Array<{
    championName: string; rawChampionName?: string; team: 'ORDER' | 'CHAOS'; position: string; isDead: boolean; respawnTimer: number;
    level: number; riotId?: string; summonerName?: string; isBot?: boolean;
    items: Array<{ itemID: number; displayName: string; price: number; slot: number; count: number }>;
    scores: { kills: number; deaths: number; assists: number; creepScore: number; wardScore: number };
    summonerSpells?: Record<string, { displayName: string }>;
  }>;
  events: { Events: Array<Record<string, unknown> & { EventID: number; EventName: string; EventTime: number }> };
  gameData: { gameMode: string; gameTime: number; mapNumber: number };
}

const URL_ALL = 'https://127.0.0.1:2999/liveclientdata/allgamedata';

/** Data Dragon id from rawChampionName ("game_character_displayname_MonkeyKing"), else the name without spaces/apostrophes. */
const ddId = (p: AllGameData['allPlayers'][number]) => /displayname_(.+)$/.exec(p.rawChampionName ?? '')?.[1] ?? p.championName.replace(/[^A-Za-z]/g, '');

export function toGameInfo(d: AllGameData): GameInfo {
  const meId = d.activePlayer.riotId ?? d.activePlayer.summonerName;
  const me = d.allPlayers.find(p => (p.riotId ?? p.summonerName) === meId);
  const myTeam = me?.team ?? 'ORDER';
  return {
    gameMode: d.gameData.gameMode,
    gameTime: d.gameData.gameTime,
    mapNumber: d.gameData.mapNumber,
    activeChampion: me?.championName ?? null,
    allies: d.allPlayers.filter(p => p.team === myTeam).map(p => p.championName),
    enemies: d.allPlayers.filter(p => p.team !== myTeam).map(p => p.championName),
    allyIds: d.allPlayers.filter(p => p.team === myTeam).map(ddId),
    enemyIds: d.allPlayers.filter(p => p.team !== myTeam).map(ddId),
  };
}

/**
 * Polls the in-game Live Client Data API.
 * Events: 'state' (SourceState), 'data' (AllGameData, receivedAt).
 */
export class LiveApi extends EventEmitter {
  state: SourceState = 'waiting';
  last: AllGameData | null = null;
  private timer: NodeJS.Timeout | null = null;
  private busy = false;

  constructor(private intervalMs = 500) { super(); }

  start() {
    if (this.timer) return;
    this.timer = setInterval(() => void this.tick(), this.intervalMs);
    void this.tick();
  }

  stop() {
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
    this.setState('off');
  }

  private async tick() {
    if (this.busy) return;
    this.busy = true;
    try {
      const data = await getJson<AllGameData>(URL_ALL);
      // During loading the API answers but gameData may be incomplete.
      if (!data?.gameData || !Array.isArray(data.allPlayers)) throw new Error('loading');
      this.last = data;
      this.setState('connected');
      this.emit('data', data, Date.now());
    } catch {
      this.last = null;
      this.setState('waiting');
    } finally {
      this.busy = false;
    }
  }

  private setState(s: SourceState) {
    if (s === this.state) return;
    this.state = s;
    this.emit('state', s);
  }
}

/**
 * The Live API gives LOCALISED champion names (e.g. "Nunu i Willump" in a Polish client).
 * Data Dragon / Riot API use English names. Rewrite championName to the English display
 * name using rawChampionName ("game_character_displayname_<DdragonId>").
 */
export function normalizeChampionNames(d: AllGameData, nameById: (id: string) => string | undefined): AllGameData {
  const fix = (p: AllGameData['allPlayers'][number]) => {
    const id = /displayname_(.+)$/.exec(p.rawChampionName ?? '')?.[1];
    const en = id ? nameById(id) : undefined;
    return en && en !== p.championName ? { ...p, championName: en } : p;
  };
  return { ...d, allPlayers: d.allPlayers.map(fix) };
}
