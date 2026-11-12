import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { decodePng, type Rgba } from './png';

/**
 * The icon the GAME draws on the minimap (CommunityDragon `<champ>_circle_0.png`, default skin — the
 * minimap ignores skins). For most champions it equals the Data Dragon square, but not for all: Briar's
 * Data Dragon portrait matched her minimap icon at 0.25 (others 0.99), so she was never detected.
 */
async function loadGameCircleIcon(cacheDir: string, ddragonId: string): Promise<Rgba | null> {
  const id = ddragonId.toLowerCase();
  const file = path.join(cacheDir, `cd-${id}-0.png`), miss = `${file}.missing`;
  if (existsSync(file)) return decodePng(readFileSync(file));
  if (existsSync(miss)) return null;
  try {
    const res = await fetch(`https://raw.communitydragon.org/latest/game/assets/characters/${id}/hud/${id}_circle_0.png`, { signal: AbortSignal.timeout(10000) });
    if (!res.ok) { if (res.status === 404) writeFileSync(miss, ''); return null; }
    writeFileSync(file, Buffer.from(await res.arrayBuffer()));
    return decodePng(readFileSync(file));
  } catch { return null; }
}

/** Champion icon for template matching: the game's own minimap icon, else the Data Dragon square. */
export async function loadChampionIcon(cacheDir: string, version: string, ddragonId: string): Promise<Rgba> {
  mkdirSync(cacheDir, { recursive: true });
  const game = await loadGameCircleIcon(cacheDir, ddragonId);
  if (game) return game;
  const file = path.join(cacheDir, `${version}-${ddragonId}.png`);
  if (!existsSync(file)) {
    const res = await fetch(`https://ddragon.leagueoflegends.com/cdn/${version}/img/champion/${ddragonId}.png`, { signal: AbortSignal.timeout(10000) });
    if (!res.ok) throw new Error(`icon ${ddragonId}: HTTP ${res.status}`);
    writeFileSync(file, Buffer.from(await res.arrayBuffer()));
  }
  return decodePng(readFileSync(file));
}
