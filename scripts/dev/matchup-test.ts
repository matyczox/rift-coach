// Dev: fetch a matchup brief and show the power bar for a few lane situations.
import path from 'node:path';
import { fetchMatchup, powerBar } from '../../src/main/intel/matchup';
import type { GameIntel, PlayerIntel } from '../../src/shared/intel';

const [me = 'Ambessa', enemy = 'Warwick', role = 'TOP', patch = '16.22'] = process.argv.slice(2);

function player(champion: string, isAlly: boolean, level: number, itemGold: number, completed: number): PlayerIntel {
  return { champion, team: isAlly ? 'ORDER' : 'CHAOS', isAlly, isMe: isAlly, role: role as PlayerIntel['role'], level, kills: 0, deaths: 0, assists: 0, cs: 0, wardScore: 0, isDead: false, respawnIn: 0, itemGold, estGold: itemGold, items: [], completedItems: completed, spikes: [], summonerSpells: [], moveSpeed: 345, flashIn: null, hpPct: null, hpSeenAt: null };
}

async function main() {
  const t = Date.now();
  const b = await fetchMatchup(process.env.RC_MATCHUP_DIR ?? path.join(process.env.APPDATA!, 'Rift Coach', 'matchups'), { patch, me, enemy, role });
  console.log(`brief in ${Date.now() - t} ms:`, JSON.stringify({ winRate: b.winRate, curve: b.curve.map(v => +v.toFixed(2)), mySpikes: b.mySpikes, enemySpikes: b.enemySpikes, tips: b.tips }, null, 1));
  const cases: Array<[string, number, number, number, number, number]> = [
    ['lvl 2 vs 2, równo', 2, 2, 500, 500, 1],
    ['lvl 3 vs 4', 3, 4, 900, 1200, 1],
    ['lvl 6 vs 5, +450 g', 6, 5, 1800, 1350, 1],
    ['lvl 9 vs 9, on ma item więcej', 9, 9, 4200, 5600, 0.8],
    ['lvl 11 vs 10, ja z przewagą', 11, 10, 7000, 5500, 1],
  ];
  for (const [label, lm, lo, gm, go, hp] of cases) {
    const intel = { gameTime: 600, gameMode: 'CLASSIC', me: null, players: [player(me, true, lm, gm, Math.floor(gm / 3000)), player(enemy, false, lo, go, Math.floor(go / 3000))],
      ally: {} as GameIntel['ally'], enemy: {} as GameIntel['enemy'], goldDiff: 0, objectives: [], recent: [], myDeaths: [], buffs: [], streaks: {}, ace: null,
      myStats: { gold: 0, hp: Math.round(1000 * hp), maxHp: 1000, ad: 0, ap: 0, armor: 0, mr: 0, moveSpeed: 345 } } as GameIntel;
    intel.me = intel.players[0]!;
    const bar = powerBar(intel, b, c => c)!;
    console.log(`${label.padEnd(32)} → ${Math.round(bar.pMe * 100)}% | ${bar.detail} | ${bar.advice}`);
  }
}
main().catch(e => { console.error('FAIL', String(e).slice(0, 500)); process.exit(1); });
