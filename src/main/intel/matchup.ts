import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { researchEffort, researchModel } from '../coach/research';
import path from 'node:path';
import type { GameIntel, PlayerIntel, PowerBarState } from '../../shared/intel';
import { callModel, type Effort } from '../coach/llm';

/**
 * Lane matchup knowledge, fetched once per (patch, my champion, enemy champion, role)
 * by Claude with web access (meta stats + matchup guides), cached on disk.
 * `curve[i]` = my advantage at level i+1, from -1 (he wins hard) to +1 (I win hard).
 */
export interface MatchupBrief {
  patch: string;
  me: string;
  enemy: string;
  role: string;
  /** My win rate in this matchup on the current patch (0..1), null if not found. */
  winRate: number | null;
  curve: number[];          // 18 values
  mySpikes: string[];       // e.g. "lvl 6 (R)", "Eclipse"
  enemySpikes: string[];
  tips: string[];           // ≤ 3 short Polish tips
  createdAt: number;
}

const SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['winRate', 'curve', 'mySpikes', 'enemySpikes', 'tips'],
  properties: {
    winRate: { anyOf: [{ type: 'null' }, { type: 'number', minimum: 0, maximum: 1 }] },
    curve: { type: 'array', items: { type: 'number', minimum: -1, maximum: 1 }, minItems: 18, maxItems: 18 },
    mySpikes: { type: 'array', items: { type: 'string', maxLength: 40 }, maxItems: 5 },
    enemySpikes: { type: 'array', items: { type: 'string', maxLength: 40 }, maxItems: 5 },
    tips: { type: 'array', items: { type: 'string', maxLength: 90 }, maxItems: 3 },
  },
};

const key = (b: { patch: string; me: string; enemy: string; role: string }) =>
  `${b.patch}-${b.role}-${b.me}-vs-${b.enemy}`.replace(/[^\w.-]/g, '');

export function loadCachedMatchup(dir: string, q: { patch: string; me: string; enemy: string; role: string }): MatchupBrief | null {
  const f = path.join(dir, `${key(q)}.json`);
  if (!existsSync(f)) return null;
  try { return JSON.parse(readFileSync(f, 'utf8')); } catch { return null; }
}

export async function fetchMatchup(dir: string, q: { patch: string; me: string; enemy: string; role: string }, override: { model?: string; effort?: Effort } = {}): Promise<MatchupBrief> {
  const cached = loadCachedMatchup(dir, q);
  if (cached) return cached;
  const system = `Jesteś analitykiem matchupów League of Legends. Masz wyszukiwarkę i pobieranie stron.
Sprawdź matchup ${q.me} vs ${q.enemy} na linii ${q.role} na patchu ${q.patch} (strony ze statystykami matchupów, np. lolalytics/u.gg/op.gg, oraz poradniki matchupów).
Zwróć:
- winRate: winrate ${q.me} przeciw ${q.enemy} (0..1) albo null, jeśli nie znalazłeś,
- curve: 18 liczb, przewaga ${q.me} na poziomach 1..18 w walce 1v1 przy równych itemach (-1 = ${q.enemy} wyraźnie silniejszy, 0 = równo, +1 = ${q.me} wyraźnie silniejszy). Uwzględnij spike'i z umiejętności (np. 2 lvl, 3 lvl, 6 lvl ult, 11, 16).
- mySpikes / enemySpikes: kluczowe momenty siły (poziomy, przedmioty).
- tips: maks. 3 krótkie wskazówki po polsku, jak grać ten matchup (kiedy tradować, czego unikać).
Opieraj się na źródłach; gdzie brak danych, oceń ostrożnie na podstawie kitów.`;
  const r = await callModel<Omit<MatchupBrief, 'patch' | 'me' | 'enemy' | 'role' | 'createdAt'>>({
    model: override.model ?? researchModel(), effort: override.effort ?? researchEffort(), system, prompt: `Matchup: ${q.me} vs ${q.enemy}, ${q.role}, patch ${q.patch}.`, schema: SCHEMA,
    tools: ['WebSearch', 'WebFetch'], timeoutMs: 180000,
  });
  const curve = Array.from({ length: 18 }, (_, i) => Math.max(-1, Math.min(1, Number(r.value.curve?.[i] ?? 0))));
  const brief: MatchupBrief = { ...q, ...r.value, curve, createdAt: Date.now() };
  mkdirSync(dir, { recursive: true });
  writeFileSync(path.join(dir, `${key(q)}.json`), JSON.stringify(brief, null, 2));
  return brief;
}

/** Enemy playing my role (lane opponent); null if roles unknown. */
export function laneOpponent(intel: GameIntel): PlayerIntel | null {
  const me = intel.me;
  if (!me || me.role === 'UNKNOWN') return null;
  return intel.players.find(p => !p.isAlly && p.role === me.role) ?? null;
}

const sigmoid = (x: number) => 1 / (1 + Math.exp(-x));

/**
 * Live all-in chance. Score (log-odds-ish) =
 *   matchup curve at the current levels (main term)
 * + level difference (each level ≈ one ability rank + stats)
 * + item gold difference and completed-item difference
 * + my HP fraction (enemy HP is not in the Live API) + enemy Flash down.
 */
export function powerScore(intel: GameIntel, opp: PlayerIntel, brief: MatchupBrief | null): { pMe: number; score: number } {
  const me = intel.me!;
  const lvlMe = Math.min(18, Math.max(1, me.level)), lvlOpp = Math.min(18, Math.max(1, opp.level));
  // Curve at the average level, plus the slope effect of being at different levels.
  const curveAt = (l: number) => brief?.curve[l - 1] ?? 0;
  const curve = (curveAt(lvlMe) + curveAt(lvlOpp)) / 2;
  const levelDiff = lvlMe - lvlOpp;
  const goldDiff = (me.itemGold - opp.itemGold) / 1000;
  const itemsDiff = me.completedItems - opp.completedItems;
  const hp = intel.myStats && intel.myStats.maxHp > 0 ? intel.myStats.hp / intel.myStats.maxHp : 1;
  // Enemy HP from his health bar on screen, if read in the last 4 s; otherwise assume full.
  const oppHp = opp.hpPct !== null && opp.hpSeenAt !== null && intel.gameTime - opp.hpSeenAt <= 4 ? opp.hpPct : 1;
  const flash = opp.flashIn && opp.flashIn > 0 ? 0.25 : 0;
  // Enemy kill-threat summoners make all-ins worse for me.
  const threat = opp.summonerSpells.some(s => /Ignite|Podpalenie|Exhaust|Wyczerpanie/i.test(s)) ? -0.15 : 0;
  // Meta win rate shifts the baseline a little (50% → 0).
  const meta = brief?.winRate != null ? (brief.winRate - 0.5) * 4 : 0;
  const score = 1.6 * curve + 0.45 * levelDiff + 0.35 * goldDiff + 0.35 * itemsDiff + 1.5 * (hp - 1) - 1.5 * (oppHp - 1) + flash + threat + 0.5 * meta;
  return { pMe: sigmoid(score), score };
}

/** Next level (above the current one) where the matchup curve turns in my favour. */
export function nextGoodLevel(brief: MatchupBrief | null, level: number): number | null {
  if (!brief) return null;
  for (let l = level + 1; l <= 18; l++) if ((brief.curve[l - 1] ?? 0) > 0.15) return l;
  return null;
}

export function powerBar(intel: GameIntel, brief: MatchupBrief | null, icon: (c: string) => string): PowerBarState | null {
  const me = intel.me, opp = laneOpponent(intel);
  if (!me || !opp || me.isDead) return null;
  const { pMe } = powerScore(intel, opp, brief);
  const gold = Math.round((me.itemGold - opp.itemGold) / 50) * 50;
  const cs = me.cs - opp.cs;
  const detail = `lvl ${me.level} vs ${opp.level} · ${gold >= 0 ? '+' : ''}${gold} g · CS ${cs >= 0 ? '+' : ''}${cs}`;
  let advice: string;
  if (opp.isDead) advice = `${opp.champion} nie żyje → push / płytki`;
  else if (opp.hpPct !== null && opp.hpSeenAt !== null && intel.gameTime - opp.hpSeenAt <= 4 && opp.hpPct < 0.35 && pMe >= 0.55) advice = `${opp.champion} ${Math.round(opp.hpPct * 100)}% HP → all-in`;
  else if (pMe >= 0.6) advice = me.level > opp.level ? `Masz ${me.level}, on ${opp.level} → trade teraz` : 'Silniejszy teraz → wymieniaj';
  else if (pMe <= 0.4) {
    const nl = nextGoodLevel(brief, me.level);
    advice = nl ? `Czekaj na ${nl} lvl, farm bezpiecznie` : 'Słabszy → farm pod wieżą, nie wchodź';
  } else advice = 'Wyrównane → trade tylko z przewagą fali';
  return {
    me: { champion: me.champion, iconUrl: icon(me.champion), level: me.level },
    enemy: { champion: opp.champion, iconUrl: icon(opp.champion), level: opp.level },
    pMe, detail, advice, metaWinRate: brief?.winRate ?? null,
  };
}

/** Compact text for the coach prompt. */
export function matchupForCoach(intel: GameIntel, brief: MatchupBrief | null): Record<string, unknown> | null {
  const opp = laneOpponent(intel);
  if (!intel.me || !opp) return null;
  const { pMe } = powerScore(intel, opp, brief);
  return {
    vs: opp.champion, pWinAllIn: +pMe.toFixed(2), metaWinRate: brief?.winRate ?? null,
    myNextGoodLevel: nextGoodLevel(brief, intel.me.level), mySpikes: brief?.mySpikes, enemySpikes: brief?.enemySpikes, tips: brief?.tips,
  };
}
