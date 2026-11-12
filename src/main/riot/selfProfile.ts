// Profile of the PLAYER (not the enemies): habits from their recent Summoner's Rift games
// (Riot match-v5 + timelines) and recurring lessons from the app's own post-game reviews.
// The coach gets it in the per-game system prompt, so its calls aim at the player's weak spots.
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import type { MatchReview, PlayerSelfProfile } from '../../shared/types';
import { earlyStats, findParticipant, timelineParticipantId } from './analyze';
import type { RiotClient } from './client';
import type { MatchDto } from './types';

const QUEUES = new Set([420, 440, 400, 480, 490]); // ranked solo/flex, draft, swiftplay, quickplay (Summoner's Rift)
const MATCHES = 15, TIMELINES = 8;

const minutes = (m: MatchDto) => (m.info.gameEndTimestamp ? m.info.gameDuration : m.info.gameDuration / 1000) / 60;
const ROLE_PL: Record<string, string> = { TOP: 'top', JUNGLE: 'jungle', MIDDLE: 'mid', BOTTOM: 'ADC', UTILITY: 'support' };

/** Recent games -> habits. Throws only on fatal API errors (bad key). */
export async function buildSelfProfile(client: RiotClient, puuid: string, reviews: MatchReview[]): Promise<PlayerSelfProfile> {
  const ids = await client.getMatchIds(puuid, { count: 30 });
  const matches: MatchDto[] = [];
  for (const id of ids) {
    if (matches.length >= MATCHES) break;
    const m = await client.getMatch(id).catch(() => null);
    if (m && QUEUES.has(m.info.queueId) && minutes(m) >= 10) matches.push(m);
  }
  let deaths10 = 0, tlCount = 0;
  for (const m of matches.slice(0, TIMELINES)) {
    const tl = await client.getTimeline(m.metadata.matchId).catch(() => null);
    const p = findParticipant(m, puuid);
    const pid = tl && p ? timelineParticipantId(tl, puuid, p.participantId) : null;
    if (tl && pid) { deaths10 += earlyStats(tl, pid).deaths10; tlCount++; }
  }

  let wins = 0, k = 0, d = 0, a = 0, cs = 0, mins = 0, vision = 0, kpSum = 0;
  const roles = new Map<string, number>(), champs = new Map<string, { games: number; wins: number }>();
  for (const m of matches) {
    const p = findParticipant(m, puuid)!;
    const t = minutes(m);
    if (p.win) wins++;
    k += p.kills; d += p.deaths; a += p.assists; mins += t;
    cs += (p.totalMinionsKilled ?? 0) + (p.neutralMinionsKilled ?? 0);
    vision += (p as { visionScore?: number }).visionScore ?? 0;
    const teamKills = m.info.participants.filter(x => x.teamId === p.teamId).reduce((s, x) => s + x.kills, 0);
    kpSum += teamKills ? (p.kills + p.assists) / teamKills : 0;
    if (p.teamPosition) roles.set(p.teamPosition, (roles.get(p.teamPosition) ?? 0) + 1);
    const c = champs.get(p.championName) ?? { games: 0, wins: 0 };
    c.games++; if (p.win) c.wins++;
    champs.set(p.championName, c);
  }
  const n = matches.length;
  const mainRole = [...roles.entries()].sort((x, y) => y[1] - x[1])[0]?.[0] ?? null;
  const profile: PlayerSelfProfile = {
    games: n, wins,
    mainRole,
    champions: [...champs.entries()].sort((x, y) => y[1].games - x[1].games).slice(0, 5).map(([name, c]) => ({ name, games: c.games, wins: c.wins })),
    kda: n ? +((k + a) / Math.max(1, d)).toFixed(2) : 0,
    deathsPerGame: n ? +(d / n).toFixed(1) : 0,
    csPerMin: mins ? +(cs / mins).toFixed(1) : 0,
    visionPerMin: mins ? +(vision / mins).toFixed(2) : 0,
    killParticipation: n ? +(kpSum / n).toFixed(2) : 0,
    deathsBefore10: tlCount ? +(deaths10 / tlCount).toFixed(1) : null,
    weaknesses: [], lessons: [], summary: '', updatedAt: Date.now(),
  };
  // Weak spots, thresholds per role (supports and junglers don't farm like laners).
  const laner = mainRole && !['UTILITY', 'JUNGLE'].includes(mainRole);
  if (laner && profile.csPerMin < 6.5) profile.weaknesses.push(`niska farma (${profile.csPerMin} CS/min)`);
  if (profile.deathsBefore10 !== null && profile.deathsBefore10 >= 1.5) profile.weaknesses.push(`dużo śmierci przed 10. minutą (${profile.deathsBefore10}/mecz)`);
  if (profile.deathsPerGame >= 7) profile.weaknesses.push(`dużo śmierci w meczu (${profile.deathsPerGame}/mecz)`);
  if (profile.visionPerMin < (mainRole === 'UTILITY' ? 1.5 : 0.6)) profile.weaknesses.push(`mało wizji (${profile.visionPerMin}/min)`);
  if (profile.killParticipation < 0.45) profile.weaknesses.push(`mały udział w walkach (${Math.round(profile.killParticipation * 100)}% KP)`);
  profile.lessons = lessonsFrom(reviews);
  profile.summary = summarize(profile);
  return profile;
}

/** The most important point of the last few post-game reviews (newest first, no duplicates). */
export function lessonsFrom(reviews: MatchReview[]): string[] {
  const out: string[] = [];
  for (const r of [...reviews].sort((x, y) => y.createdAt - x.createdAt).slice(0, 8)) {
    const l = r.improve[0]?.trim();
    if (l && !out.some(o => o.slice(0, 40) === l.slice(0, 40))) out.push(l);
    if (out.length >= 4) break;
  }
  return out;
}

function summarize(p: PlayerSelfProfile): string {
  const role = p.mainRole ? ROLE_PL[p.mainRole] ?? p.mainRole : 'różne role';
  const top = p.champions.slice(0, 3).map(c => c.name).join(', ');
  return `${p.games} ostatnich gier na SR (${p.wins}W ${p.games - p.wins}L), głównie ${role}${top ? `, postacie: ${top}` : ''}. ` +
    `KDA ${p.kda}, ${p.csPerMin} CS/min, ${p.deathsPerGame} śmierci/mecz${p.deathsBefore10 !== null ? ` (${p.deathsBefore10} przed 10. min)` : ''}, KP ${Math.round(p.killParticipation * 100)}%.`;
}

/** Text block for the coach's per-game system prompt. */
export function selfProfileForCoach(p: PlayerSelfProfile | null): string | undefined {
  if (!p || (!p.games && !p.lessons.length)) return undefined;
  const lines = [`Profil gracza (Ty): ${p.summary}`];
  if (p.weaknesses.length) lines.push(`Jego słabe strony: ${p.weaknesses.join('; ')}.`);
  if (p.lessons.length) lines.push(`Najważniejsze uwagi z jego ostatnich analiz:\n- ${p.lessons.join('\n- ')}`);
  lines.push('Gdy sytuacja dotyczy jego słabej strony (np. wychodzi daleko bez wizji, a często ginie od ganków), przypomnij mu o tym krótko w poradzie.');
  return lines.join('\n');
}

export function loadSelfProfile(file: string): PlayerSelfProfile | null {
  try { return existsSync(file) ? JSON.parse(readFileSync(file, 'utf8')) as PlayerSelfProfile : null; } catch { return null; }
}
export function saveSelfProfile(file: string, p: PlayerSelfProfile) { writeFileSync(file, JSON.stringify(p, null, 2)); }
