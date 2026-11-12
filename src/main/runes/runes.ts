import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { fastResearchEffort, fastResearchModel } from '../coach/research';
import path from 'node:path';
import { callModel, type Effort } from '../coach/llm';
import type { Lcu } from '../sources/lcu';

/**
 * Automatic rune pages during champion select (like Blitz):
 *  1. my champion locked -> best page for champion + role on this patch,
 *  2. lane opponent known -> page for the matchup (draft: Claude picks the likely
 *     lane opponent among enemy picks, roles are hidden).
 * Only ever touches ONE page named PAGE_NAME; never overwrites or deletes the user's pages.
 */
export const PAGE_NAME = 'Rift Coach';

export interface RunePlan {
  primaryStyleId: number;
  subStyleId: number;
  selectedPerkIds: number[];   // 4 primary, 2 secondary, 3 shards
  label: string;               // e.g. "Ambessa top vs Warwick"
  laneOpponent: string | null;
  why?: string;                // model's one-line reasoning (shown in the UI tooltip, used in tests)
}

/** Stat shard ids (2024+ shard rows). */
const SHARDS: Record<string, number> = {
  'adaptive force': 5008, 'attack speed': 5005, 'ability haste': 5007, 'move speed': 5010, 'movement speed': 5010,
  'health scaling': 5001, 'scaling health': 5001, 'health': 5011, 'tenacity and slow resist': 5013, 'tenacity': 5013,
};

interface RuneTree { id: number; name: string; slots: Array<{ runes: Array<{ id: number; name: string }> }> }

export async function loadRuneTrees(cacheDir: string, version: string): Promise<RuneTree[]> {
  mkdirSync(cacheDir, { recursive: true });
  const f = path.join(cacheDir, `${version}-runes-en.json`);
  if (!existsSync(f)) {
    const res = await fetch(`https://ddragon.leagueoflegends.com/cdn/${version}/data/en_US/runesReforged.json`, { signal: AbortSignal.timeout(10000) });
    if (!res.ok) throw new Error(`runes: HTTP ${res.status}`);
    writeFileSync(f, await res.text());
  }
  return JSON.parse(readFileSync(f, 'utf8'));
}

const norm = (s: string) => s.toLowerCase().replace(/[^a-z0-9]/g, '');

/** Map Claude's English rune names to a valid page; throws if something doesn't fit the rules. */
export function planFromNames(trees: RuneTree[], a: { primaryTree: string; keystone: string; primary: string[]; secondaryTree: string; secondary: string[]; shards: string[] }, label: string, laneOpponent: string | null): RunePlan {
  const tree = (n: string) => trees.find(t => norm(t.name) === norm(n)) ?? (() => { throw new Error(`tree ${n}`); })();
  const p = tree(a.primaryTree), s = tree(a.secondaryTree);
  if (p.id === s.id) throw new Error('secondary tree equals primary');
  const inTree = (t: RuneTree, n: string, slots: number[]) => {
    for (const i of slots) { const r = t.slots[i]?.runes.find(x => norm(x.name) === norm(n)); if (r) return { id: r.id, slot: i }; }
    throw new Error(`rune ${n} not in ${t.name}`);
  };
  const ks = inTree(p, a.keystone, [0]);
  const prim = a.primary.slice(0, 3).map(n => inTree(p, n, [1, 2, 3]));
  if (new Set(prim.map(x => x.slot)).size !== 3) throw new Error('primary runes must be one per row');
  const sec = a.secondary.slice(0, 2).map(n => inTree(s, n, [1, 2, 3]));
  if (sec.length !== 2 || sec[0]!.slot === sec[1]!.slot) throw new Error('secondary runes must be from two different rows');
  const shards = a.shards.slice(0, 3).map(n => SHARDS[n.toLowerCase().trim()] ?? (() => { throw new Error(`shard ${n}`); })());
  if (shards.length !== 3) throw new Error('need 3 shards');
  return {
    primaryStyleId: p.id, subStyleId: s.id,
    selectedPerkIds: [ks.id, ...prim.sort((x, y) => x.slot - y.slot).map(x => x.id), ...sec.sort((x, y) => x.slot - y.slot).map(x => x.id), ...shards],
    label, laneOpponent,
  };
}

const SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['laneOpponent', 'primaryTree', 'keystone', 'primary', 'secondaryTree', 'secondary', 'shards'],
  properties: {
    laneOpponent: { anyOf: [{ type: 'null' }, { type: 'string' }] },
    primaryTree: { type: 'string' }, keystone: { type: 'string' },
    primary: { type: 'array', items: { type: 'string' }, minItems: 3, maxItems: 3 },
    secondaryTree: { type: 'string' },
    secondary: { type: 'array', items: { type: 'string' }, minItems: 2, maxItems: 2 },
    shards: { type: 'array', items: { type: 'string' }, minItems: 3, maxItems: 3 },
    why: { type: 'string' },
  },
};

export interface RuneAsk { patch: string; champion: string; role: string; enemies: string[]; opponent: { champion: string; p: number } | null; trees?: RuneTree[] }

/** Rows of every tree, so the model can't put two runes from one row into the secondary tree. */
export const treeRows = (trees: RuneTree[]) => trees.map(t => `${t.name}: ${t.slots.map((sl, i) => `${i === 0 ? 'keystone' : `rząd ${i}`} [${sl.runes.map(r => r.name).join(' | ')}]`).join('; ')}`).join('\n');

export function runeSystemPrompt(opts: RuneAsk): string {
  const opp = opts.opponent?.champion;
  return `Jesteś ekspertem od run w League of Legends. Patch ${opts.patch}. Gracz: ${opts.champion}, rola ${opts.role}. Wrogowie: ${opts.enemies.join(', ') || 'jeszcze nikt'}.
${opp ? `Przeciwnik na linii (pewność ${Math.round(opts.opponent!.p * 100)}%): ${opp}. Ustaw laneOpponent = "${opp}".` : 'Przeciwnik na linii niepewny: laneOpponent = null.'}

MUSISZ sprawdzić dane w internecie. Nie odpowiadaj z pamięci, bo runy zmieniają się co patch. Postępuj tak:
1. Strona bazowa: najlepsza strona ${opts.champion} ${opts.role} na tym patchu (albo poprzednim, jeśli ten jest świeży) z dużej próbki: lolalytics.com, u.gg, op.gg albo mobalytics.gg, ranga Emerald+ lub Platinum+, tysiące gier. Keystone wybierz po winrate przy sensownej popularności (pickrate ≥ 5%), a nie tylko po popularności.
${opp ? `2. Matchup: sprawdź stronę ${opts.champion} vs ${opp} (np. lolalytics "vs", u.gg matchups). Keystone i główne drzewo zmieniaj względem bazy TYLKO, gdy dane matchupu mają dużą próbkę (≥ 300 gier dla tej runy) i wyraźnie wyższy winrate (≥ 2 pkt proc.). Małe próbki (Master+, kilkadziesiąt gier) i mało znane agregatory IGNORUJ.
3. Drobne zmiany pod przeciwnika są zawsze w porządku, jeśli mają jasny powód:
   - drugie drzewo i drobne runy: Second Wind / Doran's start przeciw poke, Bone Plating przeciw burstowi, Unflinching lub Tenacity przeciw dużej ilości CC;
   - odłamki: Health Scaling, Health, Move Speed.
   Nie wymyślaj egzotycznego keystone'a, którego nikt na tej postaci nie gra.` : '2. Możesz lekko dopasować drugie drzewo i odłamki do składu wrogów (np. dużo AP albo dużo CC), ale keystone zostaw z bazy.'}

Podaj ANGIELSKIE nazwy dokładnie jak w grze:
- primaryTree (Precision/Domination/Sorcery/Resolve/Inspiration), keystone, primary (3 runy z kolejnych rzędów);
- secondaryTree, secondary (2 runy z różnych rzędów);
- shards (3 odłamki, wybierz z: "Adaptive Force", "Attack Speed", "Ability Haste", "Move Speed", "Health Scaling", "Health", "Tenacity and Slow Resist").
${opts.trees ? `Drzewa i rzędy run na tym patchu (secondary: 2 runy z DWÓCH RÓŻNYCH rzędów, bez keystone'a):
${treeRows(opts.trees)}
` : ''}why: jedno-dwa zdania po polsku: źródło i próbka strony bazowej, co zmieniłeś pod przeciwnika i dlaczego.`;
}

/** Ask a model (web) for the page; cached per patch/champion/role/opponent. */
export async function adviseRunes(opts: {
  cacheDir: string; trees: RuneTree[]; patch: string; champion: string; role: string; enemies: string[]; allies: string[];
  /** Lane opponent from the joint role assignment (null = not sure enough -> generic page). */
  opponent: { champion: string; p: number } | null;
  /** Dev/test overrides. */
  model?: string; effort?: Effort; systemPrompt?: (o: RuneAsk) => string;
}): Promise<RunePlan> {
  const key = `${opts.patch}-${opts.champion}-${opts.role}-${[...opts.enemies].sort().join('_') || 'none'}`.replace(/[^\w.-]/g, '');
  const f = path.join(opts.cacheDir, `runes-${key}.json`);
  if (existsSync(f)) return JSON.parse(readFileSync(f, 'utf8'));
  const system = (opts.systemPrompt ?? runeSystemPrompt)({ ...opts, trees: opts.trees });
  let lastErr: unknown;
  for (let attempt = 0; attempt < 2; attempt++) {
    let r;
    try {
      r = await callModel<{ why?: string; laneOpponent: string | null; primaryTree: string; keystone: string; primary: string[]; secondaryTree: string; secondary: string[]; shards: string[] }>({
            // Draft model for both pages: with this prompt Terra high read real patch stats (op.gg / u.gg / Mobalytics)
      // while Opus low mostly failed to load the stat pages and fell back to memory (scripts/dev/runes-bench.ts).
      model: opts.model ?? fastResearchModel(), effort: opts.effort ?? fastResearchEffort(), system, prompt: attempt ? `Poprzednia odpowiedź była niepoprawna (${String(lastErr)}). Popraw nazwy/rzędy.` : 'Dobierz runy.',
      schema: SCHEMA, tools: ['WebSearch', 'WebFetch'], timeoutMs: 120000,
    });
    } catch (e) { lastErr = e; continue; }
    try {
      const plan = planFromNames(opts.trees, r.value, `${opts.champion} ${opts.role.toLowerCase()}${r.value.laneOpponent ? ` vs ${r.value.laneOpponent}` : ''}`, r.value.laneOpponent);
      plan.why = r.value.why;
      mkdirSync(opts.cacheDir, { recursive: true });
      writeFileSync(f, JSON.stringify(plan, null, 2));
      return plan;
    } catch (e) { lastErr = e; }
  }
  throw new Error(`Runy: ${String(lastErr)}`);
}

interface LcuPage { id: number; name: string; isEditable: boolean; current: boolean }

/** Write the plan into the "Rift Coach" page (create it if there's a free slot) and select it. */
export async function applyRunePage(lcu: Lcu, plan: RunePlan): Promise<'updated' | 'created' | 'no-slot'> {
  const pages = (await lcu.get<LcuPage[]>('/lol-perks/v1/pages')) ?? [];
  const body = { name: PAGE_NAME, primaryStyleId: plan.primaryStyleId, subStyleId: plan.subStyleId, selectedPerkIds: plan.selectedPerkIds, current: true };
  const mine = pages.find(p => p.name === PAGE_NAME && p.isEditable);
  if (mine) {
    await lcu.request('PUT', `/lol-perks/v1/pages/${mine.id}`, { ...body, id: mine.id });
    await lcu.request('PUT', '/lol-perks/v1/currentpage', mine.id);
    return 'updated';
  }
  const inv = await lcu.get<{ canAddCustomPage: boolean }>('/lol-perks/v1/inventory');
  if (!inv?.canAddCustomPage) return 'no-slot';
  await lcu.request('POST', '/lol-perks/v1/pages', body);
  return 'created';
}
