import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { fastResearchEffort, fastResearchModel as researchModel } from '../coach/research';
import path from 'node:path';
import { callModel } from '../coach/llm';

/**
 * Draft role inference. In champion select enemy roles (and in ranked their names)
 * are hidden, so we use each champion's role play rates on the current patch and
 * assign all enemy picks to roles JOINTLY: if they have Warwick and Lee Sin, Lee is
 * the jungler and Warwick is almost surely top.
 */
export const ROLES = ['TOP', 'JUNGLE', 'MIDDLE', 'BOTTOM', 'UTILITY'] as const;
export type DraftRole = typeof ROLES[number];
export type RoleRates = Record<DraftRole, number>;

/** Play-rate share per role for each champion, from the web, cached per patch & champion. */
export async function roleRates(dir: string, patch: string, champions: string[]): Promise<Map<string, RoleRates>> {
  mkdirSync(dir, { recursive: true });
  const file = path.join(dir, `roles-${patch}.json`);
  const cache: Record<string, RoleRates> = existsSync(file) ? JSON.parse(readFileSync(file, 'utf8')) : {};
  const missing = champions.filter(c => !cache[c]);
  if (missing.length) {
    const schema = {
      type: 'object', additionalProperties: false, required: ['rates'],
      properties: { rates: { type: 'object', additionalProperties: { type: 'object', additionalProperties: false, required: [...ROLES], properties: Object.fromEntries(ROLES.map(r => [r, { type: 'number', minimum: 0, maximum: 1 }])) } } },
    };
    const r = await callModel<{ rates: Record<string, RoleRates> }>({
      model: researchModel(), effort: fastResearchEffort(), schema, tools: ['WebSearch', 'WebFetch'], timeoutMs: 120000,
      system: `Podaj udział gier na każdej roli (TOP, JUNGLE, MIDDLE, BOTTOM, UTILITY) dla podanych bohaterów League of Legends na patchu ${patch}, na podstawie stron ze statystykami (np. lolalytics, u.gg, op.gg). Wartości 0..1 sumujące się do ~1. Klucz = nazwa bohatera dokładnie jak w zapytaniu.`,
      prompt: missing.join(', '),
    });
    for (const c of missing) {
      const v = r.value.rates[c];
      if (v) cache[c] = v;
    }
    writeFileSync(file, JSON.stringify(cache, null, 1));
  }
  const out = new Map<string, RoleRates>();
  for (const c of champions) if (cache[c]) out.set(c, cache[c]!);
  return out;
}

/**
 * P(champion plays role) for every enemy pick, marginalised over all injective
 * assignments of picks to roles, weighted by the product of play rates.
 * Handles 1–5 picks; roles already taken by nobody stay free.
 */
export function assignRoles(picks: string[], rates: Map<string, RoleRates>): Map<string, RoleRates> {
  const n = picks.length;
  const result = new Map<string, RoleRates>(picks.map(c => [c, { TOP: 0, JUNGLE: 0, MIDDLE: 0, BOTTOM: 0, UTILITY: 0 }]));
  if (!n) return result;
  let total = 0;
  const used = new Array<boolean>(5).fill(false), choice = new Array<number>(n);
  const rec = (i: number, w: number) => {
    if (w === 0) return;
    if (i === n) {
      total += w;
      for (let k = 0; k < n; k++) result.get(picks[k]!)![ROLES[choice[k]!]!] += w;
      return;
    }
    const r = rates.get(picks[i]!);
    for (let j = 0; j < 5; j++) {
      if (used[j]) continue;
      // Unknown champion: uniform; tiny floor so off-role picks are possible but unlikely.
      const p = r ? Math.max(0.005, r[ROLES[j]!]) : 0.2;
      used[j] = true; choice[i] = j;
      rec(i + 1, w * p);
      used[j] = false;
    }
  };
  rec(0, 1);
  if (total > 0) for (const v of result.values()) for (const k of ROLES) v[k] /= total;
  return result;
}

/** Most likely lane opponent for my role, with its probability. */
export function laneOpponentFor(myRole: string, assigned: Map<string, RoleRates>): { champion: string; p: number } | null {
  if (!(ROLES as readonly string[]).includes(myRole)) return null;
  let best: { champion: string; p: number } | null = null;
  for (const [c, r] of assigned) {
    const p = r[myRole as DraftRole];
    if (!best || p > best.p) best = { champion: c, p };
  }
  return best;
}
