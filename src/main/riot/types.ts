// Minimal subsets of the Riot Web API DTOs we read (match-v5, account-v1,
// league-v4, champion-mastery-v4, spectator-v5). Fields we do not use are
// omitted; everything that may be missing in older payloads is optional.

export interface Position { x: number; y: number }

export interface AccountDto { puuid: string; gameName?: string; tagLine?: string }

export interface LeagueEntryDto {
  queueType: string;          // RANKED_SOLO_5x5 | RANKED_FLEX_SR | ...
  tier?: string;              // GOLD
  rank?: string;              // II
  leaguePoints?: number;
  wins?: number;
  losses?: number;
  puuid?: string;
}

export interface ChampionMasteryDto {
  puuid: string;
  championId: number;
  championLevel: number;
  championPoints: number;
  lastPlayTime?: number;
}

export interface MatchParticipantDto {
  puuid: string;
  participantId: number;
  teamId: number;             // 100 blue (ORDER), 200 red (CHAOS)
  championId: number;
  championName: string;       // internal name, e.g. "MonkeyKing"
  teamPosition?: string;      // TOP | JUNGLE | MIDDLE | BOTTOM | UTILITY | ''
  individualPosition?: string;
  win: boolean;
  kills: number;
  deaths: number;
  assists: number;
  firstBloodKill?: boolean;
  firstBloodAssist?: boolean;
  totalMinionsKilled?: number;
  neutralMinionsKilled?: number;
  visionScore?: number;
  gameEndedInEarlySurrender?: boolean;
  riotIdGameName?: string;
  riotIdTagline?: string;
  challenges?: { soloKills?: number; [k: string]: unknown };
}

export interface MatchDto {
  metadata: { matchId: string; participants: string[] };
  info: {
    gameCreation: number;
    /** Seconds when gameEndTimestamp is present (patch 11.20+), milliseconds before. */
    gameDuration: number;
    gameEndTimestamp?: number;
    gameStartTimestamp?: number;
    queueId: number;
    mapId?: number;
    participants: MatchParticipantDto[];
  };
}

export interface ParticipantFrameDto {
  participantId: number;
  position?: Position;
  level?: number;
  totalGold?: number;
  minionsKilled?: number;
  jungleMinionsKilled?: number;
}

export interface TimelineEventDto {
  type: string;               // CHAMPION_KILL | ELITE_MONSTER_KILL | WARD_PLACED | ...
  timestamp: number;          // ms since game start
  killerId?: number;          // 0 = minion/tower execute
  victimId?: number;
  assistingParticipantIds?: number[];
  position?: Position;
  killerTeamId?: number;
  monsterType?: string;       // DRAGON | RIFTHERALD | BARON_NASHOR | HORDE | ATAKHAN
  monsterSubType?: string;
  participantId?: number;
}

export interface TimelineFrameDto {
  timestamp: number;
  participantFrames: Record<string, ParticipantFrameDto>;
  events: TimelineEventDto[];
}

export interface TimelineDto {
  metadata: { matchId: string; participants: string[] };
  info: {
    frameInterval?: number;
    frames: TimelineFrameDto[];
    participants?: Array<{ participantId: number; puuid: string }>;
  };
}

export interface CurrentGameParticipantDto {
  puuid: string | null;
  teamId: number;
  championId: number;
  riotId?: string;            // "Name#TAG"
  spell1Id?: number;
  spell2Id?: number;
  bot?: boolean;
}

export interface CurrentGameInfoDto {
  gameId: number;
  gameMode?: string;
  gameQueueConfigId?: number;
  gameStartTime?: number;
  gameLength?: number;
  platformId?: string;
  participants: CurrentGameParticipantDto[];
}
