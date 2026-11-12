// Dev: build the player profile with the saved Riot key and the LCU puuid; prints only aggregates.
import { readFileSync } from 'node:fs';
import https from 'node:https';
import path from 'node:path';
import { RiotClient, isPlatform } from '../../src/main/riot/client';
import { buildSelfProfile, selfProfileForCoach } from '../../src/main/riot/selfProfile';
import { listReviews } from '../../src/main/coach/review';

function lcuGet<T>(p: string): Promise<T> {
  const [, , port, pass] = readFileSync('C:/Riot Games/League of Legends/lockfile', 'utf8').split(':');
  return new Promise((res, rej) => https.get({ host: '127.0.0.1', port: Number(port), path: p, rejectUnauthorized: false, headers: { Authorization: 'Basic ' + Buffer.from('riot:' + pass).toString('base64') } },
    r => { let d = ''; r.on('data', c => d += c); r.on('end', () => { try { res(JSON.parse(d)); } catch (e) { rej(e); } }); }).on('error', rej));
}

async function main() {
  const root = path.join(process.env.APPDATA!, 'Rift Coach');
  const s = JSON.parse(readFileSync(path.join(root, 'settings.json'), 'utf8'));
  const me = await lcuGet<{ gameName: string; tagLine: string }>('/lol-summoner/v1/current-summoner');
  const client = new RiotClient({ apiKey: s.riot.apiKey, platform: isPlatform(s.riot.platform) ? s.riot.platform : 'eun1', cacheDir: path.join(root, 'riot-cache') });
  const t = Date.now();
  const acc = await client.resolveRiotId(`${me.gameName}#${me.tagLine}`);
  const p = await buildSelfProfile(client, acc!.puuid, listReviews(path.join(root, 'reviews')));
  console.log(`built in ${((Date.now() - t) / 1000).toFixed(1)} s`);
  console.log(selfProfileForCoach(p));
}
main().catch(e => { console.error('FAIL', String(e).replace(/RGAPI-[\w-]+/g, 'RGAPI-***').slice(0, 300)); process.exit(1); });
