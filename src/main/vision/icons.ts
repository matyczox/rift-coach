import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { decodePng, type Rgba } from './png';

/** Champion square icons from Data Dragon, cached on disk (decoded RGBA). */
export async function loadChampionIcon(cacheDir: string, version: string, ddragonId: string): Promise<Rgba> {
  mkdirSync(cacheDir, { recursive: true });
  const file = path.join(cacheDir, `${version}-${ddragonId}.png`);
  if (!existsSync(file)) {
    const res = await fetch(`https://ddragon.leagueoflegends.com/cdn/${version}/img/champion/${ddragonId}.png`, { signal: AbortSignal.timeout(10000) });
    if (!res.ok) throw new Error(`icon ${ddragonId}: HTTP ${res.status}`);
    writeFileSync(file, Buffer.from(await res.arrayBuffer()));
  }
  return decodePng(readFileSync(file));
}
