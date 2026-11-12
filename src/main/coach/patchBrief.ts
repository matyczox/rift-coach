import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { researchEffort, researchModel } from './research';
import path from 'node:path';
import { callModel } from './llm';

/**
 * Once per patch: Claude (with web search/fetch) reads the official patch notes
 * and current meta and writes a compact brief. Every in-game coach call carries
 * it, so advice reflects the live patch rather than training-data knowledge.
 */
export interface PatchBrief {
  patch: string;                 // e.g. "16.22"
  createdAt: number;
  /** ≤ ~1200 chars: what changed and what is strong/weak right now. */
  summary: string;
  /** Champion -> one-line note (buffs/nerfs, current build/role). */
  champions: Record<string, string>;
  /** Notable item/system changes, one line each. */
  items: string[];
  sources: string[];
}

const SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['summary', 'champions', 'items', 'sources'],
  properties: {
    summary: { type: 'string', maxLength: 1400 },
    champions: { type: 'object', additionalProperties: { type: 'string', maxLength: 160 } },
    items: { type: 'array', items: { type: 'string', maxLength: 160 }, maxItems: 12 },
    sources: { type: 'array', items: { type: 'string' }, maxItems: 8 },
  },
};

/** "16.22.1" -> "16.22" */
export const patchOf = (ddragonVersion: string) => ddragonVersion.split('.').slice(0, 2).join('.');

export function loadCachedBrief(dir: string, patch: string): PatchBrief | null {
  const f = path.join(dir, `patch-${patch}.json`);
  if (!existsSync(f)) return null;
  try { return JSON.parse(readFileSync(f, 'utf8')); } catch { return null; }
}

export async function buildPatchBrief(opts: { dir: string; patch: string; focusChampions: string[]; model?: string }): Promise<PatchBrief> {
  const cached = loadCachedBrief(opts.dir, opts.patch);
  if (cached) return cached;
  const system = `Jesteś analitykiem meta League of Legends. Masz dostęp do wyszukiwarki i pobierania stron.
Znajdź oficjalne notatki do patcha ${opts.patch} (leagueoflegends.com) oraz aktualne statystyki meta (np. strony ze statystykami winrate/pickrate) dla tego patcha.
Napisz po polsku zwięzły brief dla coacha, który doradza w trakcie gry:
- summary: najważniejsze zmiany i co jest teraz silne/słabe (role, junglerzy, itemy, obiektywy, zmiany mapy/timerów, jeśli są).
- champions: krótka notatka dla bohaterów zmienionych w patchu i dla: ${opts.focusChampions.join(', ')} (aktualny build/runy/siła).
- items: istotne zmiany przedmiotów.
- sources: adresy, z których korzystałeś.
Tylko fakty z tego patcha; jeśli czegoś nie znalazłeś, pomiń zamiast zgadywać.`;
  const r = await callModel<Omit<PatchBrief, 'patch' | 'createdAt'>>({
    model: opts.model ?? researchModel(),
    effort: researchEffort(),
    system,
    prompt: `Przygotuj brief dla patcha ${opts.patch}.`,
    schema: SCHEMA,
    timeoutMs: 240000,
    tools: ['WebSearch', 'WebFetch'],
  });
  const brief: PatchBrief = { patch: opts.patch, createdAt: Date.now(), ...r.value };
  mkdirSync(opts.dir, { recursive: true });
  writeFileSync(path.join(opts.dir, `patch-${opts.patch}.json`), JSON.stringify(brief, null, 2));
  return brief;
}

/** Compact text for the coach prompt, limited to champions in this game. */
export function briefForGame(b: PatchBrief, champions: string[]): string {
  const notes = champions.map(c => (b.champions[c] ? `${c}: ${b.champions[c]}` : null)).filter(Boolean);
  return [`Patch ${b.patch}: ${b.summary}`, ...notes, ...b.items.slice(0, 5)].join('\n');
}
