// Standalone HUD preview — renders every HUD window OFFSCREEN (nothing is ever
// shown, focused or sent input) and saves PNGs + one composite.
//
//   node scripts/build.mjs
//   npx electron scripts/hud-demo.cjs [outDir]      (default: <tmp>/hud-demo)
//
// No single-instance lock: safe to run while the main app is running.
const { app, BrowserWindow } = require('electron');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const DIST = path.join(__dirname, '..', 'dist');
const outArg = process.argv.slice(2).find((a) => !a.startsWith('-') && !a.endsWith('.cjs') && a !== '.');
const OUT = path.resolve(outArg || path.join(os.tmpdir(), 'hud-demo'));
const DD = 'https://ddragon.leagueoflegends.com/cdn/16.22.1/img/champion/';
const DI = 'https://ddragon.leagueoflegends.com/cdn/16.22.1/img/item/';

// Separate profile so we never touch the main app's userData / locks.
app.setPath('userData', path.join(os.tmpdir(), 'rift-coach-hud-demo-profile'));
app.commandLine.appendSwitch('force-device-scale-factor', '1');

const GAME = { x: 0, y: 0, width: 2560, height: 1440 };
const MINIMAP = { x: 2205, y: 1083, width: 341, height: 341 };
const T = 852; // 14:12
const NOW = Date.now(); // expiresAt is Date.now() ms everywhere
const exp = (s) => NOW + s * 1000;

/** @type {import('../src/shared/intel').HudState} */
const STATE = {
  visible: true,
  gameTime: T,
  topBar: { call: 'Smok: TAK — 3 wrogów daleko', tone: 'go', tag: 'smok 0:48', expiresAt: exp(12) },
  alert: { kind: 'fed', text: 'Yone nafeedowany: 5/0', confidence: 0.9, expiresAt: exp(8) },
  deathCard: {
    title: 'Zginąłeś · respawn 0:24',
    respawnAt: T + 24,
    points: [
      'Wszedłeś w krzaki bez wizji — Warwick był tam 20 s.',
      'Flash wroga był gotowy, twój na cooldownie.',
      'Twoja drużyna była przy smoku, za daleko na pomoc.',
      'Następnym razem: ward w trójkąt przed wejściem.',
    ],
  },
  loadingCard: {
    enemies: [
      { champion: 'Darius', role: 'TOP', iconUrl: DD + 'Darius.png', rank: 'PLATINUM II', summary: 'agresywny na linii, 2 solo kille/mecz przed 10 min' },
      { champion: 'Warwick', role: 'JUNGLE', iconUrl: DD + 'Warwick.png', rank: 'GOLD I', summary: 'start dół, gank bot ~3:20, często invade (40%)' },
      { champion: 'Ahri', role: 'MIDDLE', iconUrl: DD + 'Ahri.png', rank: 'PLATINUM IV', summary: 'dużo roamuje po 6 lvl, 58% WR na Ahri' },
      { champion: 'Jinx', role: 'BOTTOM', iconUrl: DD + 'Jinx.png', rank: 'GOLD II', summary: 'pasywna, farmi, 1.4 śmierci przed 10 min' },
      { champion: 'Thresh', role: 'UTILITY', iconUrl: DD + 'Thresh.png', rank: null, summary: 'seria 4 porażek, mało wardów' },
    ],
  },
  ghosts: [
    { champion: 'Warwick', x: 0.30, y: 0.50, radius: 0.075, age: 9, confidence: 0.72, iconUrl: DD + 'Warwick.png' },
    { champion: 'Yone', x: 0.60, y: 0.40, radius: 0.10, age: 14, confidence: 0.5, iconUrl: DD + 'Yone.png' },
    { champion: 'Darius', x: 0.16, y: 0.16, radius: 0.13, age: 52, confidence: 0.35, iconUrl: DD + 'Darius.png' },
  ],
  hints: [
    { kind: 'objective', x: 0.665, y: 0.705, from: { x: 0.52, y: 0.60 }, expiresAt: exp(15) },
    { kind: 'invade', x: 0.23, y: 0.58, expiresAt: exp(10) },
    { kind: 'avoid', x: 0.80, y: 0.45, expiresAt: exp(10) },
    { kind: 'ward', x: 0.47, y: 0.27, label: 'ward', expiresAt: exp(60) },
    { kind: 'ward', x: 0.58, y: 0.86, label: 'roślina', expiresAt: exp(60) },
  ],
  timers: [
    { kind: 'dragon', label: 'smok', x: 0.665, y: 0.80, secondsLeft: 42 },
    { kind: 'baron', label: 'baron', x: 0.335, y: 0.235, secondsLeft: 57 },
    { kind: 'herald', label: 'herold', x: 0.33, y: 0.30, secondsLeft: 140 }, // > 60 s: hidden
  ],
  power: {
    me: { champion: 'Ambessa', iconUrl: DD + 'Ambessa.png', level: 6 },
    enemy: { champion: 'Warwick', iconUrl: DD + 'Warwick.png', level: 5 },
    pMe: 0.62,
    detail: 'lvl 6 vs 5 · +450 g',
    advice: 'Masz 6, on nie → trade teraz',
    metaWinRate: 0.49,
  },
  build: {
    phase: 'środek gry',
    updatedAt: NOW,
    items: [
      { name: 'Kaptur Śmierci Rabadona', iconUrl: DI + '3089.png', why: 'masz 1300 g na Needlessly — spike AP', cost: 3500 },
      { name: 'Klepsydra Zhonyi', iconUrl: DI + '3157.png', why: 'Yone i Hecarim mają all-in na ciebie', cost: 3250 },
      { name: 'Kostur Pustki', iconUrl: DI + '3135.png', why: '2 wrogów buduje odporność na magię', cost: 3000 },
    ],
  },
  danger: { level: 0.62, label: 'niebezpiecznie', reasons: ['Hecarim we mgle blisko', 'Yone fed'] },
  objective: { objective: 'Smok', verdict: 'go', reason: '2 wrogów martwych', inSec: 42, x: 0.67, y: 0.70 },
  fed: ['Yone'],
  showFrame: true,
  recording: true,
};

const wait = (ms) => new Promise((r) => setTimeout(r, ms));

async function composite(captures, name, crops) {
  // Hidden offscreen page that draws a busy fake game background + fake minimap
  // and the captured windows at their bounds, then captured as one PNG.
  const imgs = captures.map((c) =>
    `<img src="${c.kind}.png" style="position:absolute;left:${c.bounds.x}px;top:${c.bounds.y}px;width:${c.bounds.width}px;height:${c.bounds.height}px">`).join('');
  const html = `<!doctype html><html><head><meta charset="utf-8"><style>
html,body{margin:0;width:${GAME.width}px;height:${GAME.height}px;overflow:hidden;background:#11161b}
canvas{position:absolute;left:0;top:0}</style></head><body>
<canvas id="bg" width="${GAME.width}" height="${GAME.height}"></canvas>${process.env.HUD_BG ? `<img src="bg.png" style="position:absolute;left:0;top:0;width:${GAME.width}px;height:${GAME.height}px">` : ""}${imgs}
<script>
const c=document.getElementById('bg').getContext('2d');let s=7;const R=()=>(s=(s*16807)%2147483647)/2147483647;
c.fillStyle='#1d2a24';c.fillRect(0,0,${GAME.width},${GAME.height});
const cols=['#2f4a3a','#4a5a3c','#6d6a52','#26363f','#7f8a7a','#3b2f2a','#a8a48a','#1a2420'];
for(let i=0;i<900;i++){c.globalAlpha=0.25+R()*0.5;c.fillStyle=cols[Math.floor(R()*cols.length)];
c.beginPath();c.ellipse(R()*${GAME.width},R()*${GAME.height},20+R()*160,10+R()*90,R()*3,0,7);c.fill();}
c.globalAlpha=1;
// fake HUD chrome: scoreboard top-right, bottom ability bar
c.fillStyle='#0b0f13';c.fillRect(${GAME.width}-520,0,520,34);c.fillRect(${GAME.width / 2}-340,0,680,52);
c.fillRect(${GAME.width / 2}-420,${GAME.height}-170,840,170);
// fake minimap
const m=${JSON.stringify(MINIMAP)};c.fillStyle='#22382c';c.fillRect(m.x,m.y,m.width,m.height);
c.strokeStyle='#5d6b52';c.lineWidth=10;c.beginPath();
c.moveTo(m.x+20,m.y+m.height-20);c.lineTo(m.x+m.width-20,m.y+20);
c.moveTo(m.x+20,m.y+m.height-20);c.lineTo(m.x+20,m.y+20);c.lineTo(m.x+m.width-20,m.y+20);
c.moveTo(m.x+20,m.y+m.height-20);c.lineTo(m.x+m.width-20,m.y+m.height-20);c.lineTo(m.x+m.width-20,m.y+20);c.stroke();
c.strokeStyle='#2c4f63';c.lineWidth=14;c.beginPath();c.moveTo(m.x+40,m.y+40);c.lineTo(m.x+m.width-40,m.y+m.height-40);c.stroke();
c.strokeStyle='#e8e8e8';c.lineWidth=1.5;c.strokeRect(m.x+150,m.y+130,70,46);
</script></body></html>`;
  if (process.env.HUD_BG) fs.copyFileSync(process.env.HUD_BG, path.join(OUT, 'bg.png'));
  const file = path.join(OUT, 'composite.html');
  fs.writeFileSync(file, html);
  const win = new BrowserWindow({
    show: false, width: GAME.width, height: GAME.height, frame: false, focusable: false, skipTaskbar: true,
    paintWhenInitiallyHidden: true, useContentSize: true,
    webPreferences: { offscreen: true, sandbox: true, contextIsolation: true },
  });
  win.setContentSize(GAME.width, GAME.height);
  await win.loadFile(file);
  await wait(600);
  const img = await win.webContents.capturePage({ x: 0, y: 0, width: GAME.width, height: GAME.height });
  fs.writeFileSync(path.join(OUT, name), img.toPNG());
  // 1:1 crops of the composite for close review.
  const crop = (name, r) => fs.writeFileSync(path.join(OUT, name), img.crop(r).toPNG());
  for (const [n, r] of Object.entries(crops)) crop(n, r);
  win.destroy();
}

app.whenReady().then(async () => {
  fs.mkdirSync(OUT, { recursive: true });
  const { Hud } = require(path.join(DIST, 'hud.js'));
  const hud = new Hud(path.join(DIST, 'hud-preload.js'), path.join(DIST, 'renderer'), { demo: true });
  hud.setGameRect(GAME);
  hud.setMinimapRect(MINIMAP);
  await hud.whenReady();
  // Push at ~10 Hz like the real app, for ~2.6 s (lets the invade pulse settle and icons load).
  const t0 = Date.now();
  while (Date.now() - t0 < 2600) {
    hud.push(STATE);
    await wait(100);
  }
  // Pass 1 (dead): death card shown, power bar hidden by design.
  const dead = await hud.capture();
  // Pass 2 (laning): no death card -> power bar visible.
  for (let i = 0; i < 6; i++) { hud.push({ ...STATE, deathCard: null }); await wait(100); }
  const alive = await hud.capture();
  const save = (c) => { fs.writeFileSync(path.join(OUT, `${c.kind}.png`), c.png); console.log(`${c.kind}: ${JSON.stringify(c.bounds)}`); };
  for (const c of dead) save(c);
  for (const c of alive) if (c.kind === 'power') save(c);
  // Pass 3: objective spawning within 30 s -> echoed under the top bar.
  for (let i = 0; i < 4; i++) { hud.push({ ...STATE, deathCard: null, objective: { ...STATE.objective, inSec: 24 } }); await wait(100); }
  for (const c of await hud.capture()) if (c.kind === 'topbar') fs.writeFileSync(path.join(OUT, 'topbar-objective.png'), c.png);
  // Main composite = laning view (power bar, no death card); second = dead view.
  await composite(alive, 'hud-composite.png', {
    'crop-top.png': { x: 880, y: 40, width: 800, height: 200 },
    'crop-minimap.png': { x: MINIMAP.x - 20, y: MINIMAP.y - 20, width: MINIMAP.width + 20, height: MINIMAP.height + 20 },
    'crop-power.png': { x: 2080, y: 300, width: 480, height: 320 },
    'crop-build.png': { x: 1900, y: 1060, width: 660, height: 380 },
  });
  await composite(dead, 'hud-composite-dead.png', { 'crop-death.png': { x: 2080, y: 380, width: 480, height: 450 } });
  console.log(`composite: ${path.join(OUT, 'hud-composite.png')}`);
  hud.destroy();
  app.quit();
}).catch((e) => { console.error(e); app.exit(1); });

app.on('window-all-closed', () => { /* keep running until done */ });
