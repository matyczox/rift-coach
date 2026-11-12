import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';

/** Static data from Riot's Data Dragon CDN, cached on disk per patch. */
export interface ItemData {
  id: number;
  name: string;
  total: number;           // full shop price
  /** Final item (nothing builds from it) and not a consumable/trinket/boots-tier-1. */
  completed: boolean;
  tags: string[];
  /** English name (models often answer in English). */
  nameEn?: string;
  /** Movement speed bonuses from item stats. */
  flatMs: number;
  pctMs: number;
}

export interface StaticData {
  version: string;
  items: Map<number, ItemData>;
  /** Champion display name (as in Live API championName) -> ddragon id ("Twisted Fate" -> "TwistedFate"). */
  championIds: Map<string, string>;
  /** Champion display name -> base stats. */
  championStats: Map<string, { moveSpeed: number; attackRange: number }>;
  /** Numeric champion key (LCU championId) -> display name. */
  championByKey: Map<number, string>;
  /** Ddragon id ("MonkeyKing") -> English display name ("Wukong"). */
  nameById: Map<string, string>;
}

const CDN = 'https://ddragon.leagueoflegends.com';

export function championIconUrl(version: string, ddragonId: string) {
  return `${CDN}/cdn/${version}/img/champion/${ddragonId}.png`;
}

export function parseItems(json: { data: Record<string, { name: string; gold: { total: number; purchasable: boolean }; into?: string[]; tags?: string[]; consumed?: boolean; stats?: Record<string, number>; maps?: Record<string, boolean>; inStore?: boolean; requiredAlly?: string }> }): Map<number, ItemData> {
  const items = new Map<number, ItemData>();
  for (const [id, it] of Object.entries(json.data)) {
    // Summoner's Rift shop items only (Arena/other-mode copies use 6-digit ids or other maps).
    if (Number(id) >= 100000 || (it.maps && it.maps['11'] === false) || it.inStore === false || it.requiredAlly) continue;
    const tags = it.tags ?? [];
    const isBoots = tags.includes('Boots');
    const completed = it.gold.purchasable && !it.consumed && !(it.into && it.into.length) && !tags.includes('Trinket')
      && !tags.includes('Consumable') && (isBoots ? it.gold.total >= 900 : it.gold.total >= 2000);
    items.set(Number(id), { id: Number(id), name: it.name, total: it.gold.total, completed, tags, flatMs: it.stats?.FlatMovementSpeedMod ?? 0, pctMs: it.stats?.PercentMovementSpeedMod ?? 0 });
  }
  return items;
}

async function fetchJson<T>(url: string): Promise<T> {
  const res = await fetch(url, { signal: AbortSignal.timeout(10000) });
  if (!res.ok) throw new Error(`${url}: HTTP ${res.status}`);
  return res.json() as Promise<T>;
}

/** Load current patch data (network once per patch, then disk). Falls back to the newest cache offline. */
export async function loadStaticData(cacheDir: string, locale = 'pl_PL'): Promise<StaticData> {
  mkdirSync(cacheDir, { recursive: true });
  const latestFile = path.join(cacheDir, 'latest.txt');
  let version: string;
  try {
    version = (await fetchJson<string[]>(`${CDN}/api/versions.json`))[0]!;
    writeFileSync(latestFile, version);
  } catch {
    if (!existsSync(latestFile)) throw new Error('Data Dragon unreachable and no cache');
    version = readFileSync(latestFile, 'utf8').trim();
  }
  const file = path.join(cacheDir, `${version}-${locale}.json`);
  let raw: { items: unknown; champions: unknown; itemsEn?: unknown };
  if (existsSync(file)) raw = JSON.parse(readFileSync(file, 'utf8'));
  else {
    const [items, champions, itemsEn] = await Promise.all([
      fetchJson(`${CDN}/cdn/${version}/data/${locale}/item.json`),
      fetchJson(`${CDN}/cdn/${version}/data/en_US/champion.json`),
      fetchJson(`${CDN}/cdn/${version}/data/en_US/item.json`),
    ]);
    raw = { items, champions, itemsEn };
    writeFileSync(file, JSON.stringify(raw));
  }
  const champs = (raw.champions as { data: Record<string, { id: string; key: string; name: string; stats: { movespeed: number; attackrange: number } }> }).data;
  const championByKey = new Map<number, string>();
  const nameById = new Map<string, string>();
  const championStats = new Map<string, { moveSpeed: number; attackRange: number }>();
  const championIds = new Map<string, string>();
  for (const c of Object.values(champs)) {
    championIds.set(c.name, c.id); championIds.set(c.id, c.id);
    championStats.set(c.name, { moveSpeed: c.stats.movespeed, attackRange: c.stats.attackrange });
    championByKey.set(Number(c.key), c.name);
    nameById.set(c.id, c.name);
  }
  const items = parseItems(raw.items as Parameters<typeof parseItems>[0]);
  if (!raw.itemsEn) { try { raw.itemsEn = await fetchJson(`${CDN}/cdn/${version}/data/en_US/item.json`); writeFileSync(file, JSON.stringify(raw)); } catch { /* offline */ } }
  for (const [id, it] of Object.entries((raw.itemsEn as { data?: Record<string, { name: string }> } | undefined)?.data ?? {})) { const x = items.get(Number(id)); if (x) x.nameEn = it.name; }
  return { version, items, championIds, championStats, championByKey, nameById };
}

/** League movement speed soft caps applied to the raw value. */
export function softCapMoveSpeed(raw: number): number {
  if (raw > 490) return raw * 0.5 + 230;
  if (raw > 415) return raw * 0.8 + 83;
  if (raw < 220) return raw * 0.5 + 110;
  return raw;
}

/** Out-of-combat move speed estimate from base speed and item bonuses (no runes/summoners/buffs). */
export function estimateMoveSpeed(base: number, items: Array<Pick<ItemData, 'flatMs' | 'pctMs'>>): number {
  const flat = items.reduce((s, i) => s + i.flatMs, 0);
  const pct = items.reduce((s, i) => s + i.pctMs, 0);
  return Math.round(softCapMoveSpeed((base + flat) * (1 + pct)));
}
