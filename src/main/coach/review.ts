import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import type { CoachAdvice, GameIntel } from '../../shared/intel';
import type { MatchReview } from '../../shared/types';
import { callModel, type Effort } from './llm';

/**
 * Collects the key moments of a game while it is played, then asks a strong model
 * for a post-game review (what went well, what to improve). Stored per game.
 */
export class MatchLog {
  gameKey = '';
  startedAt = 0;
  champion = '';
  role = '';
  moments: MatchReview['moments'] = [];
  goldCurve: MatchReview['goldCurve'] = [];
  last: GameIntel | null = null;
  gameMode = '';
  private seenDeaths = 0;
  private seenKills = 0;
  private seenRecent = new Set<string>();
  private lastCurveAt = -999;

  update(i: GameIntel) {
    const key = i.players.map(p => p.champion).join('|');
    if (key !== this.gameKey) this.reset(key, i);
    this.last = i;
    this.gameMode = i.gameMode;
    const me = i.me;
    if (!me) return;
    if (i.gameTime - this.lastCurveAt >= 30) { this.goldCurve.push({ t: Math.round(i.gameTime), diff: Math.round(i.goldDiff) }); this.lastCurveAt = i.gameTime; }
    if (i.myDeaths.length > this.seenDeaths) {
      for (const d of i.myDeaths.slice(0, i.myDeaths.length - this.seenDeaths).reverse())
        this.moments.push({ t: Math.round(d.at), kind: 'death', text: `Śmierć: ${d.killer}${d.assisters.length ? ` + ${d.assisters.join(', ')}` : ''}${d.wasGank ? ' (gank)' : ''}` });
      this.seenDeaths = i.myDeaths.length;
    }
    if (me.kills > this.seenKills) { this.moments.push({ t: Math.round(i.gameTime), kind: 'kill', text: `Kill (${me.kills}/${me.deaths}/${me.assists})` }); this.seenKills = me.kills; }
    for (const r of i.recent) {
      const k = `${Math.round(r.at)}|${r.text}`;
      if (this.seenRecent.has(k)) continue;
      this.seenRecent.add(k);
      if (/Smok|Baron|Herald|Herold|Wieża|Inhib|Starszy/.test(r.text)) this.moments.push({ t: Math.round(r.at), kind: 'objective', text: r.text });
    }
  }

  advice(a: CoachAdvice) {
    if (a.deathReview) return; // already covered by the death moment
    this.moments.push({ t: Math.round(a.gameTime), kind: 'advice', text: a.call });
  }

  private reset(key: string, i: GameIntel) {
    this.gameKey = key; this.startedAt = Date.now(); this.moments = []; this.goldCurve = [];
    this.seenDeaths = i.myDeaths.length; this.seenKills = i.me?.kills ?? 0; this.seenRecent = new Set(i.recent.map(r => `${Math.round(r.at)}|${r.text}`));
    this.lastCurveAt = -999; this.champion = i.me?.champion ?? ''; this.role = i.me?.role ?? '';
  }
}

const SCHEMA = {
  type: 'object', additionalProperties: false, required: ['summary', 'good', 'improve'],
  properties: {
    summary: { type: 'string', maxLength: 400 },
    good: { type: 'array', items: { type: 'string', maxLength: 140 }, minItems: 1, maxItems: 4 },
    improve: { type: 'array', items: { type: 'string', maxLength: 160 }, minItems: 2, maxItems: 5 },
  },
};

export async function buildReview(log: MatchLog, opts: { model: string; effort?: Effort; win: boolean | null; dir: string; patchBrief?: string }): Promise<MatchReview | null> {
  const i = log.last;
  if (!i?.me || i.gameTime < 300) return null; // remakes / practice blips
  if (log.gameMode === 'CHERRY') return null;  // Arena: different game, no Rift review
  const me = i.me;
  const lane = i.players.find(p => !p.isAlly && p.role === me.role && p.role !== 'UNKNOWN');
  const facts = {
    mode: log.gameMode === 'PRACTICETOOL' ? 'Practice Tool (trening — NIE oceniaj farmy/CS ani złota, skup się na walkach i decyzjach)' : log.gameMode === 'ARAM' ? 'ARAM (jedna linia — nie oceniaj farmy ani wizji)' : log.gameMode,
    champion: me.champion, role: me.role, result: opts.win === null ? 'nieznany' : opts.win ? 'wygrana' : 'przegrana',
    durationMin: +(i.gameTime / 60).toFixed(1), kda: `${me.kills}/${me.deaths}/${me.assists}`, cs: me.cs, csPerMin: +(me.cs / (i.gameTime / 60)).toFixed(1),
    items: me.items.map(x => x.name), laneOpponent: lane ? { champion: lane.champion, kda: `${lane.kills}/${lane.deaths}/${lane.assists}`, cs: lane.cs, items: lane.items.map(x => x.name) } : null,
    deaths: i.myDeaths.slice().reverse().map(d => ({ at: `${Math.floor(d.at / 60)}:${String(Math.floor(d.at % 60)).padStart(2, '0')}`, killer: d.killer, helpers: d.assisters, gank: d.wasGank })),
    goldCurve: log.goldCurve.filter((_, k) => k % 2 === 0),
    moments: log.moments.slice(-60),
  };
  const r = await callModel<{ summary: string; good: string[]; improve: string[] }>({
    model: opts.model, effort: opts.effort, schema: SCHEMA, timeoutMs: 180000,
    system: `Jesteś coachem League of Legends. Na podstawie danych z meczu napisz krótką, konkretną analizę dla gracza (po polsku, bez lania wody):
- summary: max 3 zdania — jak przebiegł mecz i co zdecydowało.
- good: 1–4 rzeczy zrobione dobrze (konkretnie, z odniesieniem do danych).
- improve: 2–5 najważniejszych rzeczy do poprawy, NAJWAŻNIEJSZA PIERWSZA; każda jako praktyczna wskazówka (np. "3 z 6 śmierci to ganki przed 10 min na wysoko popchniętej fali — wardeuj tri-bush przy 2:45 i 6:00").
Opieraj się tylko na danych; nie wymyślaj zdarzeń.${opts.patchBrief ? `\n\nKontekst patcha:\n${opts.patchBrief}` : ''}`,
    prompt: JSON.stringify(facts),
  });
  const review: MatchReview = {
    id: new Date(log.startedAt).toISOString().replace(/[:.]/g, '-').slice(0, 19),
    createdAt: Date.now(), champion: me.champion, role: me.role, win: opts.win,
    durationSec: Math.round(i.gameTime), kda: facts.kda, csPerMin: facts.csPerMin,
    summary: r.value.summary, good: r.value.good, improve: r.value.improve,
    moments: log.moments, goldCurve: log.goldCurve, model: opts.model,
  };
  mkdirSync(opts.dir, { recursive: true });
  writeFileSync(path.join(opts.dir, `${review.id}.json`), JSON.stringify(review, null, 2));
  return review;
}

export function listReviews(dir: string): MatchReview[] {
  if (!existsSync(dir)) return [];
  return readdirSync(dir).filter(f => f.endsWith('.json')).flatMap(f => { try { return [JSON.parse(readFileSync(path.join(dir, f), 'utf8')) as MatchReview]; } catch { return []; } })
    .sort((a, b) => b.createdAt - a.createdAt);
}
