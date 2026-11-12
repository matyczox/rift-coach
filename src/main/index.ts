import { app, BrowserWindow, globalShortcut, ipcMain, nativeImage, screen, shell } from 'electron';
import path from 'node:path';
import { writeFileSync, readdirSync, statSync, existsSync, readFileSync } from 'node:fs';
import os from 'node:os';
import { EVENT } from '../shared/ipc';
import type { AppStatus, MinimapCalibration, MinimapFrame, Rect, ReplayFile, Settings, SourceState } from '../shared/types';
import type { AlertState, CoachAdvice, GameIntel, MinimapGhost, OpponentProfile } from '../shared/intel';
import { Hud } from './hud';
import { composeHud, localAlerts } from './hudState';
import { IntelTracker } from './intel/gameIntel';
import { championIconUrl, loadStaticData, type StaticData } from './intel/ddragon';
import { Coach } from './coach/coach';
import { RiotClient, isPlatform } from './riot/client';
import { FedWatcher, TeamWatcher, dangerAt, fedChampions, objectiveCall, type SeenUnit } from './intel/situational';
import { adviseBuild, buildContext, BuildTrigger } from './coach/build';
import { WARD_SPOTS, wardPoint } from '../shared/wards';
import { adviseRunes, applyRunePage, loadRuneTrees } from './runes/runes';
import { MatchLog, buildReview, listReviews } from './coach/review';
import { setFastResearchModel, setResearchModel } from './coach/research';
import { detectBackends, usableModel, type Backends } from './coach/profiles';
import { ObsRecorder, obsDir, riotToMinimap, trainFromObservations } from './tracker/selfTrain';
import { assignRoles, laneOpponentFor, roleRates } from './runes/roles';
import type { BuildPanelState } from '../shared/intel';
import { fetchMatchup, laneOpponent, loadCachedMatchup, matchupForCoach, powerBar, powerScore, type MatchupBrief } from './intel/matchup';
import { VisionService, type VisionResult } from './vision/service';
import { CAMPS, CampTracker, type CampStatus } from './vision/camps';
import { matchBarsToChampions, type HealthBar } from './vision/healthbars';
import { loadChampionIcon } from './vision/icons';
import { loadWalkable } from './vision/terrain';
import { GridTerrain } from './tracker/terrain';
import { EnemyTracker, type EnemyInput } from './tracker/tracker';
import { Learner } from './tracker/learning';
import { trackerAlerts } from './tracker/alerts';
import { buildProfiles } from './riot/profiles';
import { briefForGame, buildPatchBrief, loadCachedBrief, patchOf, type PatchBrief } from './coach/patchBrief';
import { Harvester } from './harvest';
import { replayApi } from './sources/replayApi';
import { Capture, type CropFrame } from './capture';
import { findMinimap, scaleRect } from './calibration';
import { Recorder, pruneRecordings } from './recorder';
import { Replayer } from './replay';
import { SettingsStore } from './settings';
import { Lcu } from './sources/lcu';
import { LiveApi, normalizeChampionNames, toGameInfo, type AllGameData } from './sources/liveApi';
import { WindowTracker } from './winrect';

if (!app.requestSingleInstanceLock()) app.quit();

const RENDERER = path.join(__dirname, 'renderer');
const PRELOAD = path.join(__dirname, 'preload.js');
const SMOKE = process.argv.includes('--smoke');
/** Started by Windows at login: stay in the background until the user opens the window (desktop icon). */
const BACKGROUND = process.argv.includes('--background');

let control: BrowserWindow | null = null;
let quitting = false;
/** Which AI backends this PC has (Claude Code logged in / Codex installed). */
let backends: Backends = { claude: true, codex: true };
const pick = (configured: string, task: Parameters<typeof usableModel>[1]) => usableModel(configured, task, backends) ?? { model: configured, effort: 'low' as const };
const pickModel = (configured: string, task: Parameters<typeof usableModel>[1]) => pick(configured, task).model;
function applyModels() {
  const r = pick(settings.value.coach.researchModel, 'research'), d = pick(settings.value.coach.draftModel, 'draft');
  setResearchModel(r.model, r.effort);
  setFastResearchModel(d.model, d.effort);
}
function applyLoginItem(s: Settings) {
  if (!app.isPackaged && !process.defaultApp) return;
  app.setLoginItemSettings({ openAtLogin: s.launchAtLogin, path: process.execPath, args: process.defaultApp ? [path.resolve(process.argv[1] ?? '.'), '--background'] : ['--background'] });
}
let hud: Hud | null = null;
let staticData: StaticData | null = null;
const intelTracker = new IntelTracker(() => staticData?.items ?? null, () => staticData?.championStats ?? null);
let intel: GameIntel | null = null;
const coach = new Coach();
let lastAdvice: CoachAdvice | null = null;
let alerts: AlertState[] = [];
let ghosts: MinimapGhost[] = [];
const vision = new VisionService();
let enemyTracker: EnemyTracker | null = null;
let learner: Learner | null = null;
let visionRoster = '';
const campTracker = new CampTracker();
let campStatus: CampStatus[] = [];
let myPos: { x: number; y: number } | null = null;
let visibleFn: ((x: number, y: number) => number) | undefined;
const AUTO_FLASH = false;
const LEARN_MODES = new Set(['CLASSIC', 'SWIFTPLAY']);
let profiles: OpponentProfile[] = [];
let profilesStatus: AppStatus['profiles'] = { state: 'idle', count: 0, error: null };
let patchBrief: PatchBrief | null = null;
let patchBriefRequested = false;
let profilesGameKey = '';
let matchup: MatchupBrief | null = null;
let matchupKey = '';
let allies: SeenUnit[] = [];
let visibleEnemies: SeenUnit[] = [];
const fedWatcher = new FedWatcher();
const teamWatcher = new TeamWatcher();
const buildTrigger = new BuildTrigger();
let build: BuildPanelState | null = null;
let buildBusy = false;
let runesStatus: AppStatus['runes'] = { state: 'idle', label: null, error: null };
let runesKey = '';
let runesTimer: NodeJS.Timeout | null = null;
const matchLog = new MatchLog();
const obsRecorder = new ObsRecorder();
let obsGameKey = '';
let reviewDoneFor = '';
const reviewsDir = () => process.argv.find(a => a.startsWith('--reviews-dir='))?.slice(14) ?? path.join(app.getPath('userData'), 'reviews');
let settings: SettingsStore;
let recorder: Recorder;
let replay: Replayer | null = null;
let replayGame: AllGameData | null = null;
const live = new LiveApi();
const lcu = new Lcu();
const capture = new Capture();
const gameWindow = new WindowTracker();
let frameUntil = 0; // show the minimap outline until this time
let liveLostAt: number | null = null;
let harvester: Harvester;
let meProfile: import('../shared/types').PlayerProfile | null = null;

/** Name, level, icon and ranks of the logged-in player (League client). */
async function refreshMe() {
  if (lcu.state !== 'connected') return;
  try {
    const s = await lcu.get<{ gameName?: string; tagLine?: string; summonerLevel?: number; profileIconId?: number }>('/lol-summoner/v1/current-summoner');
    const rk = await lcu.get<{ queues?: Array<{ queueType: string; tier: string; division: string; leaguePoints: number; wins: number; losses: number }> }>('/lol-ranked/v1/current-ranked-stats').catch(() => null);
    const ranks = (rk?.queues ?? []).filter(q => (q.queueType === 'RANKED_SOLO_5x5' || q.queueType === 'RANKED_FLEX_SR') && q.tier && q.tier !== 'NONE')
      .map(q => ({ queue: q.queueType === 'RANKED_SOLO_5x5' ? 'solo' as const : 'flex' as const, tier: q.tier, division: q.division, lp: q.leaguePoints, wins: q.wins, losses: q.losses }));
    if (s?.gameName) meProfile = { gameName: s.gameName, tagLine: s.tagLine ?? '', level: s.summonerLevel ?? 0, iconId: s.profileIconId ?? 0, ranks };
  } catch { /* client busy; try later */ }
}
let replayApiState: SourceState = 'waiting';
let harvestQueue: number[] = [];
let harvestLaunching: number | null = null;
let harvestCancelled = false;

// ---------- status ----------

function status(): AppStatus {
  const gameData = replay ? replayGame : live.last;
  return {
    version: app.getVersion(),
    ddragonVersion: staticData?.version ?? null,
    me: meProfile,
    liveApi: live.state,
    lcu: lcu.state,
    lcuPhase: lcu.phase,
    game: gameData ? toGameInfo(gameData) : null,
    capture: capture.status,
    recording: recorder.status,
    replay: replay?.status ?? null,
    replayApi: replayApiState,
    coach: coach.status,
    lastAdvice,
    profiles: profilesStatus,
    runes: settings.value.autoRunes ? runesStatus : { state: 'off', label: null, error: null },
    harvest: { ...harvester.status, ...(harvestLaunching ? { state: 'launching' as const, gameId: harvestLaunching } : {}), queue: harvestQueue },
    settings: settings.value,
  };
}

const sendControl = (ch: string, v: unknown) => { if (control && !control.isDestroyed()) control.webContents.send(ch, v); };

function championIcon(champion: string) {
  if (!staticData) return '';
  return championIconUrl(staticData.version, staticData.championIds.get(champion) ?? champion.replace(/[^A-Za-z]/g, ''));
}

function currentPMe(): number | null {
  if (!intel?.me) return null;
  const opp = laneOpponent(intel);
  return opp ? powerScore(intel, opp, matchup).pMe : null;
}
/** Ward spots our team already sees (from the fog mask). */
function coveredSpots(i: GameIntel): string[] {
  if (!visibleFn) return [];
  const red = i.me?.team === 'CHAOS';
  return Object.keys(WARD_SPOTS).filter(id => { const p = wardPoint(id, red)!; return visibleFn!(p.x, p.y) > 0.6; });
}
/** Short camp facts for the coach: buff respawns (both sides) and the enemy jungler's last camp. */
function campSummary(i: GameIntel): string[] {
  const out: string[] = [];
  const myTeam = i.me?.team ?? 'ORDER';
  for (const st of campStatus) {
    const camp = CAMPS.find(c => c.id === st.id);
    if (!camp || camp.kind !== 'buff' || st.state !== 'down' || st.respawnAt === null) continue;
    const left = Math.round(st.respawnAt - i.gameTime);
    if (left > 0 && left < 90) out.push(`${camp.side === myTeam ? 'twój' : 'wrogi'} ${camp.id.endsWith('blue') ? 'niebieski' : 'czerwony'} buff za ${left} s`);
  }
  const last = [...campTracker.events].reverse().find(e => e.kind === 'cleared' && CAMPS.find(c => c.id === e.id)?.side !== myTeam && CAMPS.find(c => c.id === e.id)?.side !== 'neutral');
  if (last && i.gameTime - last.at < 60) out.push(`wrogi jungler wyczyścił ${campLabel(last.id)} ${Math.round(i.gameTime - last.at)} s temu`);
  return out;
}
const fedNames = () => (intel ? fedChampions(intel).filter(p => !p.isAlly).map(p => p.champion) : []);

/** Compose and push the in-game HUD (~10x/s). */
function pushOverlay() {
  if (!hud) return;
  const now = Date.now();
  alerts = alerts.filter(a => a.expiresAt > now);
  hud.push(composeHud({
    visible: settings.value.overlayVisible && !replay && replayApiState !== 'connected',
    intel, advice: lastAdvice, alerts, ghosts, profiles, championIcon,
    power: intel ? powerBar(intel, matchup, championIcon) : null,
    build,
    danger: intel ? dangerAt(intel, myPos, enemyTracker?.all ?? [], visibleEnemies, currentPMe(), fedNames()) : null,
    objective: intel ? objectiveCall(intel, enemyTracker?.all ?? [], allies, visibleEnemies) : null,
    fed: fedNames(),
    visionAt: visibleFn,
    me: myPos,
    showFrame: now < frameUntil, recording: recorder.status.active,
  }));
}

// ---------- calibration & geometry ----------

/** Calibration rect expressed in the current capture video's pixels. */
function cropRect(): Rect | null {
  const c = settings.value.calibration, w = capture.status.windowWidth, h = capture.status.windowHeight;
  if (!c || !w || !h) return null;
  return scaleRect(c.rect, c.windowWidth, c.windowHeight, w, h);
}

function placeOverlay() {
  if (!hud) return;
  const win = gameWindow.rect, c = settings.value.calibration;
  if (!win) { hud.setGameRect(null); return; }
  hud.setGameRect(screen.screenToDipRect(null, { x: win.x, y: win.y, width: win.width, height: win.height }));
  if (c) {
    const r = scaleRect(c.rect, c.windowWidth, c.windowHeight, win.width, win.height);
    hud.setMinimapRect(screen.screenToDipRect(null, { x: win.x + r.x, y: win.y + r.y, width: r.width, height: r.height }));
  } else hud.setMinimapRect(null);
  pushOverlay();
}

function applyCalibration(c: MinimapCalibration) {
  settings.update({ calibration: c });
  capture.configure({ crop: cropRect() });
  frameUntil = Date.now() + 4000;
  placeOverlay();
  setTimeout(pushOverlay, 4100);
}

async function autoCalibrate(): Promise<MinimapCalibration | null> {
  const scene = await capture.grabScene();
  if (!scene) return null;
  const img = nativeImage.createFromBuffer(scene.data);
  const { width, height } = img.getSize();
  const found = findMinimap(img.toBitmap(), width, height);
  if (!found) return null;
  const c: MinimapCalibration = { rect: found.rect, windowWidth: width, windowHeight: height, source: 'auto', updatedAt: Date.now() };
  applyCalibration(c);
  return c;
}

// ---------- recording ----------

const SUPPORTED_MODES = new Set(['CLASSIC', 'PRACTICETOOL', 'ARAM', 'URF', 'CHERRY', 'SWIFTPLAY']);

function startRecording() {
  if (recorder.status.active) return;
  recorder.start({ calibration: settings.value.calibration, appVersion: app.getVersion() });
  capture.configure({ sceneEverySec: settings.value.sceneEverySec });
  pushOverlay();
}

function stopRecording() {
  recorder.stop();
  capture.configure({ sceneEverySec: 0 });
  pushOverlay();
}

// ---------- frames ----------

let lastCrop: CropFrame | null = null;
function onLiveFrame(f: CropFrame) {
  lastCrop = f;
  harvester.frame(f);
  recorder.frame(f);
  // Live game only (not while harvesting a replay): detect + track.
  if (intel && enemyTracker && replayApiState !== 'connected' && visionRoster && live.last?.gameData.mapNumber === 11) {
    const hints = enemyTracker.all.filter(t => t.state === 'visible' && t.last).map(t => ({ x: t.last!.x, y: t.last!.y, team: t.team }));
    vision.push(f.at, f.png, hints);
  }
  if (!replay) sendFrame({ at: f.at, width: f.width, height: f.height, dataUrl: `data:image/png;base64,${f.png.toString('base64')}` });
}

let lastFrameSent = 0;
function sendFrame(f: MinimapFrame) {
  if (!control || control.isDestroyed() || !control.isVisible() || control.isMinimized()) return;
  if (f.at - lastFrameSent < 90) return; // <= ~10 fps to the UI
  lastFrameSent = f.at;
  sendControl(EVENT.minimapFrame, f);
}

// ---------- windows ----------

function createControl() {
  control = new BrowserWindow({
    width: 1180, height: 780, minWidth: 900, minHeight: 600, show: false,
    title: 'Rift Coach', backgroundColor: '#0c0f14', autoHideMenuBar: true,
    // Own dark title bar (Blitz-style); Windows keeps native min/max/close drawn over it.
    titleBarStyle: 'hidden', titleBarOverlay: { color: '#0c0f14', symbolColor: '#9aa3b2', height: 36 },
    icon: path.join(__dirname, '..', 'assets', 'icon.png'),
    webPreferences: { preload: PRELOAD, sandbox: false },
  });
  void control.loadFile(path.join(RENDERER, 'control.html'));
  // Do not steal focus from the game when the app starts mid-match.
  control.once('ready-to-show', () => { if (!SMOKE && !BACKGROUND) control?.showInactive(); });
  // Closing the window keeps the coach running in the background when it starts with Windows.
  control.on('close', e => { if (settings.value.launchAtLogin && !quitting) { e.preventDefault(); control?.hide(); } });
  control.on('closed', () => { control = null; app.quit(); });
  // External links (e.g. developer.riotgames.com) open in the default browser.
  control.webContents.setWindowOpenHandler(({ url }) => {
    if (/^https:\/\//.test(url)) void shell.openExternal(url);
    return { action: 'deny' };
  });
}

function createOverlay() {
  hud = new Hud(PRELOAD, RENDERER);
  placeOverlay();
}

// ---------- intel & coach ----------

function onGameData(d: AllGameData) {
  intel = intelTracker.update(d);
  // The fog model learns only from real Summoner's Rift games (ranked/normal/swiftplay).
  if (enemyTracker) enemyTracker.learningEnabled = LEARN_MODES.has(d.gameData.gameMode) && d.gameData.mapNumber === 11;
  matchLog.update(intel);
  maybeLoadProfiles(d);
  maybeLoadMatchup();
  void syncVisionRoster();
  const now = Date.now();
  for (const a of localAlerts(intel, now)) if (!alerts.some(x => x.text === a.text)) alerts.push(a);
  for (const a of fedWatcher.update(intel, now)) alerts.push(a);
  if (staticData && intel.me && !buildBusy && coach.status.state !== 'not-logged-in' && coach.status.state !== 'off' && buildTrigger.shouldRefresh(intel, now)) {
    buildBusy = true;
    const sdv = staticData;
    adviseBuild(buildContext(intel, sdv.items, profiles, patchBrief ? briefForGame(patchBrief, intel.players.map(p => p.champion)) : undefined, fedNames()), sdv.items,
      id => `https://ddragon.leagueoflegends.com/cdn/${sdv.version}/img/item/${id}.png`, pickModel(settings.value.coach.liveModel, 'build'))
      .then(b => { build = b; }, e => console.warn('build', e))
      .finally(() => { buildBusy = false; });
  }
  coach.setConfig({ enabled: settings.value.coach.enabled, liveModel: pickModel(settings.value.coach.liveModel, 'live'), reviewModel: pickModel(settings.value.coach.reviewModel, 'death'), liveEffort: pick(settings.value.coach.liveModel, 'live').effort, reviewEffort: pick(settings.value.coach.reviewModel, 'death').effort, periodSec: settings.value.coach.periodSec });
  const champions = intel.players.map(p => p.champion);
  if (staticData && !patchBrief) patchBrief = loadCachedBrief(briefDir(), patchOf(staticData.version));
  if (staticData && !patchBrief && !patchBriefRequested && coach.status.state === 'idle') {
    patchBriefRequested = true;
    buildPatchBrief({ dir: briefDir(), patch: patchOf(staticData.version), focusChampions: [...new Set(['Rumble', 'Akali', 'Ambessa', ...champions])] })
      .then(b => { patchBrief = b; }, e => { console.warn('patch brief failed', e); setTimeout(() => { patchBriefRequested = false; }, 10 * 60000); });
  }
  coach.tick({ intel, ghosts, alerts, profiles, patchBrief: patchBrief ? briefForGame(patchBrief, champions) : undefined, matchup: matchupForCoach(intel, matchup), coveredWardSpots: coveredSpots(intel), camps: campSummary(intel) });
}

/** Fetch enemy profiles once per game (needs a Riot API key). */
function maybeLoadProfiles(d: AllGameData) {
  const key = d.allPlayers.map(p => p.championName).join('|');
  if (key === profilesGameKey) return;
  profilesGameKey = key;
  profiles = [];
  const { apiKey, platform } = settings.value.riot;
  if (!apiKey) { profilesStatus = { state: 'no-key', count: 0, error: null }; return; }
  const me = intel?.me;
  const enemies = d.allPlayers.filter(p => p.team !== (me?.team ?? 'ORDER') && !p.isBot && p.riotId && p.riotId.includes('#'));
  if (!enemies.length) { profilesStatus = { state: 'idle', count: 0, error: 'brak graczy z Riot ID (boty?)' }; return; }
  profilesStatus = { state: 'loading', count: 0, error: null };
  const client = new RiotClient({ apiKey, platform: isPlatform(platform) ? platform : 'eun1', cacheDir: path.join(app.getPath('userData'), 'riot-cache') });
  const roleOf = (pos: string) => (['TOP', 'JUNGLE', 'MIDDLE', 'BOTTOM', 'UTILITY'].includes(pos) ? pos : 'UNKNOWN') as OpponentProfile['role'];
  buildProfiles(enemies.map(p => ({ riotId: p.riotId!, champion: p.championName, role: roleOf(p.position) })), {
    client,
    onProgress: pr => { profilesStatus = { state: 'loading', count: pr.playersDone, error: null }; },
  }).then(list => {
    if (profilesGameKey !== key) return;
    profiles = list;
    profilesStatus = { state: 'ready', count: list.filter(p => p.gamesAnalyzed > 0).length, error: null };
  }, e => { profilesStatus = { state: 'error', count: 0, error: String(e instanceof Error ? e.message : e) }; });
}

const CAMP_PL: Record<string, string> = { gromp: 'gromp', blue: 'niebieskim buffie', wolves: 'wilkach', raptors: 'raptorach', red: 'czerwonym buffie', krugs: 'krugach' };
const campLabel = (id: string) => CAMP_PL[id.replace(/^(blue|red)_/, '')] ?? id;

/** Game setting "relative team colours" (enemies always red on the minimap). Default on. */
function readRelativeColors(): boolean {
  for (const p of ['C:/Riot Games/League of Legends/Config/game.cfg', 'D:/Riot Games/League of Legends/Config/game.cfg']) {
    try { const m = /^RelativeTeamColors=(\d)/m.exec(readFileSync(p, 'utf8')); if (m) return m[1] === '1'; } catch { /* next */ }
  }
  return true;
}

/** Give the vision worker this game's 10 champions (icons from Data Dragon). */
async function syncVisionRoster() {
  if (!intel || !staticData) return;
  const myTeam = intel.me?.team ?? 'ORDER';
  const key = intel.players.map(p => p.champion).join('|') + myTeam;
  if (key === visionRoster) return;
  visionRoster = key;
  const dir = path.join(app.getPath('userData'), 'ddragon', 'icons');
  try {
    const champions = await Promise.all(intel.players.map(async p => ({
      champion: p.champion, team: p.team, icon: await loadChampionIcon(dir, staticData!.version, staticData!.championIds.get(p.champion) ?? p.champion),
    })));
    vision.setRoster(champions, myTeam, readRelativeColors());
    capture.configure({ hpEveryMs: 350, hpColor: readRelativeColors() ? 'red' : (myTeam === 'ORDER' ? 'red' : 'blue') });
    campTracker.reset();
    enemyTracker?.reset();
  } catch (e) { visionRoster = ''; console.warn('vision roster', e); }
}

function onVision(r: VisionResult) {
  if (!intel || !enemyTracker) return;
  const myTeam = intel.me?.team ?? 'ORDER';
  const meDet = intel.me ? r.detections.find(d => d.champion === intel!.me!.champion) : undefined;
  if (meDet) myPos = { x: meDet.x, y: meDet.y };
  const withVel = (prev: SeenUnit[], d: { champion: string; x: number; y: number }): SeenUnit => {
    const p = prev.find(u => u.champion === d.champion);
    return p ? { champion: d.champion, x: d.x, y: d.y, vx: (p.vx ?? 0) * 0.6 + (d.x - p.x) * 0.4, vy: (p.vy ?? 0) * 0.6 + (d.y - p.y) * 0.4 } : { champion: d.champion, x: d.x, y: d.y };
  };
  allies = r.detections.filter(d => d.team === myTeam).map(d => withVel(allies, d));
  visibleEnemies = r.detections.filter(d => d.team !== myTeam).map(d => withVel(visibleEnemies, d));
  for (const a of teamWatcher.update(intel, myPos, allies, visibleEnemies, currentPMe(), Date.now())) alerts.push(a);
  const mask = r.mask;
  const visible = visibleFn = mask ? (x: number, y: number) => mask[Math.min(63, Math.max(0, Math.floor(y * 64))) * 64 + Math.min(63, Math.max(0, Math.floor(x * 64)))]! : undefined;
  const enemies: EnemyInput[] = intel.players.filter(p => !p.isAlly).map(p => ({
    champion: p.champion, team: p.team, role: p.role, isDead: p.isDead, moveSpeed: p.moveSpeed,
    startTopSide: profiles.find(pr => pr.champion === p.champion)?.jungle?.startTopSide,
    itemGold: p.itemGold, hasFlash: p.summonerSpells.some(sp => /Flash|Błysk/i.test(sp)),
  }));
  const dragon = intel.objectives.find(o => o.kind === 'dragon'), baron = intel.objectives.find(o => o.kind === 'baron');
  const t = intel.gameTime;
  // Jungle camps: an enemy camp just died -> their jungler was there (a sighting for the tracker);
  // one of OUR camps died while our jungler is elsewhere -> invade.
  const n0 = campTracker.events.length;
  campStatus = campTracker.update(t, r.camps, visible);
  const campSightings: typeof r.detections = [];
  const enemyJg = intel.players.find(p => !p.isAlly && p.role === 'JUNGLE');
  for (const ev of campTracker.events.slice(n0)) {
    if (ev.kind !== 'cleared') continue;
    const camp = CAMPS.find(c => c.id === ev.id);
    if (!camp) continue;
    const fresh = ev.earliest === undefined || ev.at - ev.earliest <= 10;
    if (camp.side !== myTeam && camp.side !== 'neutral' && enemyJg && fresh && !r.detections.some(d => d.champion === enemyJg.champion))
      campSightings.push({ champion: enemyJg.champion, team: enemyJg.team, x: camp.x, y: camp.y, score: 0.7, margin: 0, votes: 0 });
    if (camp.side === myTeam) {
      const myJg = intel.players.find(p => p.isAlly && !p.isMe && p.role === 'JUNGLE');
      const myJgPos = myJg ? allies.find(a => a.champion === myJg.champion) : undefined;
      const ours = myJgPos && Math.hypot(myJgPos.x - camp.x, myJgPos.y - camp.y) < 0.08;
      const meThere = myPos && Math.hypot(myPos.x - camp.x, myPos.y - camp.y) < 0.08;
      if (!ours && !meThere) alerts.push({ kind: 'invade', text: `Invade: wróg na twoim ${campLabel(camp.id)}`, confidence: 0.85, expiresAt: Date.now() + 6000 });
    }
  }
  const tracks = enemyTracker.update({
    t, detections: [...r.detections.filter(d => d.team !== myTeam), ...campSightings], visible, enemies,
    dragonIn: dragon ? (dragon.alive ? 0 : (dragon.spawnsAt ?? t) - t) : null,
    baronIn: baron ? (baron.alive ? 0 : (baron.spawnsAt ?? t) - t) : null,
  });
  // Probable Flash -> 5 min timer; proven recall of my lane opponent / jungler -> short note.
  for (const tr of tracks) {
    // Auto-Flash is OFF: on a real replay 66 "Flashes" were dashes (Yone, Rakan, Hecarim, Malphite R) —
    // indistinguishable on the minimap. Flash timers come from the manual hotkey only.
    if (AUTO_FLASH && tr.flashSuspectAt !== null && tr.flashSuspectAt === t) intelTracker.markFlash(tr.champion, t);
    const p = intel.players.find(x => x.champion === tr.champion);
    if (tr.recalledAt === t && p && (p.role === 'JUNGLE' || p.role === intel.me?.role)) alerts.push({ kind: 'missing', text: `${tr.champion} w bazie (kupił)`, confidence: 1, expiresAt: Date.now() + 4000 });
  }
  ghosts = tracks.filter(tr => tr.state === 'fog' && tr.estimate && tr.fogSince !== null && t - tr.fogSince < 120).map(tr => ({
    champion: tr.champion, x: tr.estimate!.x, y: tr.estimate!.y, radius: tr.estimate!.radius,
    age: t - (tr.fogSince ?? t), confidence: tr.estimate!.confidence, iconUrl: championIcon(tr.champion),
  }));
  if (enemyTracker.learningEnabled) {
    const gk = intel.players.map(p => p.champion).join('|');
    if (gk !== obsGameKey) { obsGameKey = gk; obsRecorder.start(myTeam); }
    obsRecorder.add({
      t, d: r.detections.filter(d => d.team !== myTeam).map(d => [d.champion, +d.x.toFixed(4), +d.y.toFixed(4)] as [string, number, number]),
      mask: r.mask, e: enemies.map(e => [e.champion, e.role, e.isDead ? 1 : 0, e.moveSpeed, e.itemGold ?? 0] as [string, string, number, number, number]),
      dr: dragon ? (dragon.alive ? 0 : Math.round((dragon.spawnsAt ?? t) - t)) : null, br: baron ? (baron.alive ? 0 : Math.round((baron.spawnsAt ?? t) - t)) : null,
    });
  }
  const now = Date.now();
  for (const a of trackerAlerts(tracks, myPos, myTeam, t, now)) {
    const i = alerts.findIndex(x => x.kind === a.kind);
    if (i >= 0) alerts[i] = a; else alerts.push(a);
  }
}

/** Fetch (once per patch & pairing) the lane matchup brief: curve per level, spikes, tips. */
function maybeLoadMatchup() {
  if (!intel?.me || !staticData) return;
  const opp = laneOpponent(intel);
  if (!opp) return;
  const q = { patch: patchOf(staticData.version), me: intel.me.champion, enemy: opp.champion, role: intel.me.role };
  const k = `${q.patch}|${q.me}|${q.enemy}|${q.role}`;
  if (k === matchupKey) return;
  matchupKey = k;
  const dir = path.join(app.getPath('userData'), 'matchups');
  matchup = loadCachedMatchup(dir, q);
  if (matchup || coach.status.state === 'not-logged-in' || coach.status.state === 'off') return;
  fetchMatchup(dir, q).then(b => { if (matchupKey === k) matchup = b; }, e => { console.warn('matchup', e); matchupKey = ''; });
}

// ---------- automatic runes (champion select) ----------

interface ChampSelectSession {
  localPlayerCellId: number;
  myTeam: Array<{ cellId: number; championId: number; assignedPosition: string }>;
  theirTeam: Array<{ championId: number }>;
  actions: Array<Array<{ actorCellId: number; type: string; completed: boolean; championId: number }>>;
}

const ROLE_NAMES: Record<string, string> = { top: 'TOP', jungle: 'JUNGLE', middle: 'MIDDLE', bottom: 'BOTTOM', utility: 'UTILITY' };

function onChampSelect(session: ChampSelectSession) {
  if (!settings.value.autoRunes || !staticData || coach.status.state === 'not-logged-in') return;
  const me = session.myTeam.find(p => p.cellId === session.localPlayerCellId);
  if (!me || !me.championId) return;
  const locked = session.actions.flat().some(a => a.actorCellId === me.cellId && a.type === 'pick' && a.completed);
  if (!locked) return;
  const champion = staticData.championByKey.get(me.championId);
  if (!champion) return;
  const role = ROLE_NAMES[me.assignedPosition] ?? 'ARAM/BRAK ROLI';
  const enemies = session.theirTeam.map(p => staticData!.championByKey.get(p.championId)).filter((c): c is string => Boolean(c));
  const allies = session.myTeam.filter(p => p.cellId !== me.cellId).map(p => staticData!.championByKey.get(p.championId)).filter((c): c is string => Boolean(c));
  const key = `${champion}|${role}|${[...enemies].sort().join(',')}`;
  if (key === runesKey) return;
  runesKey = key;
  // Debounce: enemies often lock in quick succession.
  if (runesTimer) clearTimeout(runesTimer);
  runesTimer = setTimeout(() => void setRunes(key, champion, role, enemies, allies), 2500);
}

async function setRunes(key: string, champion: string, role: string, enemies: string[], allies: string[]) {
  if (!staticData) return;
  runesStatus = { state: 'working', label: `${champion} ${role.toLowerCase()}`, error: null };
  try {
    const dir = path.join(app.getPath('userData'), 'runes');
    const trees = await loadRuneTrees(path.join(app.getPath('userData'), 'ddragon'), staticData.version);
    // Who is my lane opponent? Joint role assignment of enemy picks on this patch's play rates.
    const rates = enemies.length ? await roleRates(dir, patchOf(staticData.version), enemies) : new Map();
    const opp = laneOpponentFor(role, assignRoles(enemies, rates));
    const opponent = opp && opp.p >= 0.6 ? opp : null;
    const plan = await adviseRunes({ cacheDir: dir, trees, patch: patchOf(staticData.version), champion, role, enemies, allies, opponent });
    if (key !== runesKey) return; // draft moved on; a newer request is coming
    const res = await applyRunePage(lcu, plan);
    runesStatus = res === 'no-slot'
      ? { state: 'no-slot', label: plan.label, error: 'Brak wolnej strony run — usuń jedną albo nazwij ją „Rift Coach”.' }
      : { state: 'set', label: plan.label, error: null };
  } catch (e) {
    runesStatus = { state: 'error', label: null, error: String(e instanceof Error ? e.message : e).slice(0, 160) };
    runesKey = ''; // allow a retry on the next session update
  }
}

/** After the game: post-game review by the review model (default Opus/Sonnet from settings). */
async function finishGame() {
  const key = matchLog.gameKey;
  if (!key || key === reviewDoneFor || !matchLog.last || matchLog.last.gameTime < 300) return;
  if (replayApiState === 'connected' || coach.status.state === 'not-logged-in') return;
  reviewDoneFor = key;
  let win: boolean | null = null;
  let gameId: number | null = null;
  try {
    const eog = await lcu.get<{ gameId?: number; localPlayer?: { teamId?: number }; teams?: Array<{ teamId: number; isWinningTeam: boolean }> }>('/lol-end-of-game/v1/eog-stats-block');
    const t = eog?.teams?.find(x => x.teamId === eog.localPlayer?.teamId);
    if (t) win = t.isWinningTeam;
    gameId = eog?.gameId ?? null;
  } catch { /* client didn't show EOG (practice tool) */ }
  void selfTrain(gameId);
  try {
    const r = await buildReview(matchLog, {
      model: pickModel(settings.value.coach.reviewModel, 'review'), effort: pick(settings.value.coach.reviewModel, 'review').effort, win, dir: reviewsDir(),
      patchBrief: patchBrief && matchLog.last ? briefForGame(patchBrief, matchLog.last.players.map(p => p.champion)) : undefined,
    });
    if (r) sendControl(EVENT.review, r);
  } catch (e) { console.warn('review', e); reviewDoneFor = ''; }
}

/** Save this game's observations and learn from the Riot timeline (true positions every minute). */
async function selfTrain(gameId: number | null) {
  const file = obsRecorder.save(obsDir(app.getPath('userData')), gameId);
  obsRecorder.log = null;
  if (!file || !gameId || !learner || !settings.value.riot.apiKey) return;
  try {
    const platform = isPlatform(settings.value.riot.platform) ? settings.value.riot.platform : 'eun1';
    const client = new RiotClient({ apiKey: settings.value.riot.apiKey, platform, cacheDir: path.join(app.getPath('userData'), 'riot-cache') });
    const matchId = `${platform.toUpperCase()}_${gameId}`;
    // The match appears in the API a minute or two after the game.
    let match = null, timeline = null;
    for (let i = 0; i < 6 && (!match || !timeline); i++) {
      await sleep(30000);
      match = await client.getMatch(matchId).catch(() => null);
      timeline = await client.getTimeline(matchId).catch(() => null);
    }
    if (!match || !timeline) return;
    const byPid = new Map(match.info.participants.map(p => [p.participantId, p]));
    const nameFix = (n: string) => staticData?.championIds && [...staticData.championIds.entries()].find(([, id]) => id === n)?.[0] || n;
    const truth = (champion: string, minute: number) => {
      const frame = timeline!.info.frames[minute];
      if (!frame) return null;
      for (const pf of Object.values(frame.participantFrames)) {
        const p = byPid.get(pf.participantId);
        if (p && nameFix(p.championName) === champion && pf.position) return riotToMinimap(pf.position);
      }
      return null;
    };
    const { loadObs } = await import('./tracker/selfTrain');
    const res = trainFromObservations(loadObs(file), new GridTerrain(128, loadWalkable(128)), learner, truth);
    learner.save();
    console.log('self-train', JSON.stringify(res));
  } catch (e) { console.warn('self-train', e); }
}

const briefDir = () => path.join(app.getPath('userData'), 'patch');

function markEnemyFlash() {
  if (!intel?.me) return;
  // Lane opponent by role; fall back to the closest-level enemy.
  const enemies = intel.players.filter(p => !p.isAlly);
  const target = enemies.find(p => p.role === intel!.me!.role && p.role !== 'UNKNOWN') ?? enemies[0];
  if (target) intelTracker.markFlash(target.champion, intel.gameTime);
}

function registerHotkeys(s: Settings) {
  globalShortcut.unregisterAll();
  const reg = (acc: string, fn: () => void) => { try { if (acc) globalShortcut.register(acc, fn); } catch { /* invalid accelerator */ } };
  reg(s.hotkeys.toggleOverlay, () => { settings.update({ overlayVisible: !settings.value.overlayVisible }); placeOverlay(); pushOverlay(); });
  reg(s.hotkeys.toggleRecording, () => (recorder.status.active ? stopRecording() : startRecording()));
  reg(s.hotkeys.enemyFlash, markEnemyFlash);
}

// ---------- replay harvesting ----------

const REPLAY_DIRS = [path.join(os.homedir(), 'Documents', 'League of Legends', 'Replays')];
const sleep = (ms: number) => new Promise(r => setTimeout(r, ms));

function listReplayFiles(): ReplayFile[] {
  const done = new Set<number>();
  for (const r of recorder.list() as Array<{ source?: string; gameId?: number; frames: number }>) {
    if (r.source === 'replay' && r.gameId && r.frames > 0) done.add(r.gameId);
  }
  const out: ReplayFile[] = [];
  for (const dir of REPLAY_DIRS) {
    if (!existsSync(dir)) continue;
    for (const name of readdirSync(dir)) {
      const m = /-(\d+)\.rofl$/i.exec(name);
      if (!m) continue;
      const st = statSync(path.join(dir, name));
      const gameId = Number(m[1]);
      out.push({ gameId, name, sizeMb: Math.round(st.size / 1e5) / 10, modifiedAt: st.mtimeMs, harvested: done.has(gameId) });
    }
  }
  return out.sort((a, b) => b.modifiedAt - a.modifiedAt);
}

async function waitReplayReady(timeoutMs: number) {
  const end = Date.now() + timeoutMs;
  while (Date.now() < end) {
    try { const pb = await replayApi.playback(); if (pb.length > 0 && pb.time > 0.5) return; } catch { /* loading */ }
    await sleep(1000);
  }
  throw new Error('Powtórka nie wystartowała (czy EnableReplayApi=1 jest w game.cfg?)');
}

/** Close the replay game process (identified by the Replay API itself). */
async function closeReplayProcess() {
  try {
    const { processID } = await replayApi.game();
    if (processID > 0) process.kill(processID);
  } catch { return; /* no replay open */ }
  for (let i = 0; i < 30; i++) { try { await replayApi.playback(); } catch { return; } await sleep(1000); }
}

/** Harvest the replay that is open now, with a fresh calibration for the replay HUD. */
async function harvestOpen(speed: number, gameId: number | null, extra: { passes?: ('blue' | 'red' | 'all')[]; confirmFirst?: boolean; sessionId?: string } = {}) {
  capture.configure({ fps: 30 });
  for (let i = 0; i < 20 && capture.status.state !== 'connected'; i++) await sleep(500);
  await autoCalibrate();
  try {
    await harvester.run({ gameId, speed, meta: { calibration: settings.value.calibration, appVersion: app.getVersion() }, ...extra });
  } finally {
    capture.configure({ fps: settings.value.minimapFps });
  }
}

async function harvestStart(opts: { gameIds?: number[]; speed?: number }) {
  if (harvester.running || harvestLaunching) throw new Error('Zbieranie już trwa');
  const speed = Math.min(16, Math.max(1, opts.speed ?? 8));
  harvestCancelled = false;
  if (!opts.gameIds?.length) { void harvestOpen(speed, null); return; }
  if (live.state === 'connected' && replayApiState !== 'connected') throw new Error('Trwa mecz — zbieranie z powtórek po meczu');
  harvestQueue = [...opts.gameIds];
  void (async () => {
    while (harvestQueue.length && !harvestCancelled) {
      const id = harvestQueue.shift()!;
      harvestLaunching = id;
      try {
        if (replayApiState === 'connected') await closeReplayProcess();
        await lcu.post(`/lol-replays/v1/rofls/${id}/watch`, { componentType: 'replay-button_match-history' });
        await waitReplayReady(180000);
        await sleep(3000); // let the HUD settle before calibrating
        harvestLaunching = null;
        await harvestOpen(speed, id);
      } catch (e) {
        harvester.status = { ...harvester.status, state: 'error', gameId: id, error: String(e) };
      } finally {
        harvestLaunching = null;
        if (!harvestCancelled) await closeReplayProcess();
      }
    }
    harvestQueue = [];
  })();
}

// ---------- IPC ----------

function closeReplay() {
  replay?.close();
  replay = null;
  replayGame = null;
}

function registerIpc() {
  ipcMain.handle('getStatus', () => status());
  ipcMain.handle('updateSettings', (_e, patch: Partial<Settings>) => {
    const before = settings.value;
    const next = settings.update(patch);
    applyModels();
    if (before.launchAtLogin !== next.launchAtLogin) applyLoginItem(next);
    if (JSON.stringify(before.hotkeys) !== JSON.stringify(next.hotkeys)) registerHotkeys(next);
    capture.configure({ fps: next.minimapFps, crop: cropRect(), sceneEverySec: recorder.status.active ? next.sceneEverySec : 0 });
    placeOverlay();
    return next;
  });
  ipcMain.handle('grabScene', async () => {
    const s = await capture.grabScene();
    return s ? { dataUrl: `data:image/png;base64,${s.data.toString('base64')}`, width: s.width, height: s.height } : null;
  });
  ipcMain.handle('autoCalibrate', () => autoCalibrate());
  ipcMain.handle('setCalibration', (_e, c: MinimapCalibration) => applyCalibration(c));
  ipcMain.handle('startRecording', () => startRecording());
  ipcMain.handle('stopRecording', () => stopRecording());
  ipcMain.handle('listRecordings', () => recorder.list());
  ipcMain.handle('deleteRecording', (_e, id: string) => {
    if (replay?.status.sessionId === id) closeReplay();
    recorder.delete(id);
  });
  ipcMain.handle('openRecordingsFolder', () => shell.openPath(recorder.root));
  ipcMain.handle('replayOpen', (_e, id: string) => {
    closeReplay();
    const r = new Replayer(recorder.root, id);
    r.on('frame', ({ offsetMs, png }: { offsetMs: number; png: Buffer }) => {
      const size = nativeImage.createFromBuffer(png).getSize();
      sendFrame({ at: Date.now(), offsetMs, width: size.width, height: size.height, dataUrl: `data:image/png;base64,${png.toString('base64')}` });
    });
    r.on('api', (raw: AllGameData) => { const d = staticData ? normalizeChampionNames(raw, id => staticData!.nameById.get(id)) : raw; replayGame = d; intel = intelTracker.update(d); });
    replay = r;
    r.seek(0);
    return r.status;
  });
  ipcMain.handle('replayPlay', () => replay?.play());
  ipcMain.handle('replayPause', () => replay?.pause());
  ipcMain.handle('replaySeek', (_e, ms: number) => replay?.seek(ms));
  ipcMain.handle('replaySpeed', (_e, s: number) => replay?.setSpeed(s));
  ipcMain.handle('replayClose', () => closeReplay());
  ipcMain.handle('listReplayFiles', () => listReplayFiles());
  ipcMain.handle('listReviews', () => listReviews(reviewsDir()));
  ipcMain.handle('harvestStart', (_e, opts: { gameIds?: number[]; speed?: number }) => harvestStart(opts ?? {}));
  ipcMain.handle('harvestContinue', () => harvester.continue());
  ipcMain.handle('harvestStop', () => { harvestCancelled = true; harvestQueue = []; harvester.stop(); });
}

// ---------- lifecycle ----------

app.whenReady().then(() => {
  settings = new SettingsStore(app.getPath('userData'));
  recorder = new Recorder(path.join(app.getPath('userData'), 'recordings'), () => settings.value.recordFps);
  harvester = new Harvester(recorder.root);
  applyModels();
  const refreshBackends = () => detectBackends().then(b => { backends = b; applyModels(); console.log('AI backends', JSON.stringify(b)); }, () => undefined);
  void refreshBackends();
  setInterval(() => void refreshBackends(), 5 * 60000);
  // Disk housekeeping at startup (thin old sessions, drop old live recordings).
  setTimeout(() => { try { const freed = pruneRecordings(recorder.root, { active: recorder.status.sessionId }); if (freed) console.log('pruned', Math.round(freed / 1e6), 'MB'); } catch (e) { console.warn('prune', e); } }, 30000);
  loadStaticData(path.join(app.getPath('userData'), 'ddragon')).then(d => { staticData = d; }, e => console.warn('ddragon', e));
  learner = new Learner(path.join(app.getPath('userData'), 'model', 'fog-model.json'));
  try { enemyTracker = new EnemyTracker(new GridTerrain(128, loadWalkable(128)), learner); } catch (e) { console.warn('terrain', e); }
  vision.start(path.join(__dirname, 'vision-worker.js'));
  vision.on('result', onVision);
  vision.on('error', e => console.warn('vision', e));
  setInterval(() => learner?.save(), 60000);
  coach.on('advice', (a: CoachAdvice) => {
    // Models sometimes write free text in 'item' — keep it only if it is a real shop item.
    if (a.item && staticData) {
      const want = a.item.toLowerCase();
      const hit = [...staticData.items.values()].find(i => i.completed && [i.name, i.nameEn].some(n => n && (n.toLowerCase() === want || want.includes(n.toLowerCase()))));
      a = { ...a, item: hit?.name ?? null };
    }
    lastAdvice = a;
    matchLog.advice(a);
  });
  live.on('state', s => { if (s !== 'connected') { intel = null; lastAdvice = null; alerts = []; ghosts = []; myPos = null; visionRoster = ''; build = null; allies = []; visibleEnemies = []; learner?.save(); } });

  registerIpc();
  createControl();
  createOverlay();
  registerHotkeys(settings.value);
  applyLoginItem(settings.value);

  live.on('data', (raw: AllGameData, at: number) => {
    // English champion names everywhere (the client may be localised).
    const d = staticData ? normalizeChampionNames(raw, id => staticData!.nameById.get(id)) : raw;
    liveLostAt = null;
    recorder.apiSample(d, at);
    if (replayApiState !== 'connected') onGameData(d);
    if (settings.value.autoRecord && replayApiState !== 'connected' && !recorder.status.active && SUPPORTED_MODES.has(d.gameData.gameMode)) startRecording();
  });
  live.on('state', s => {
    if (s !== 'connected') liveLostAt ??= Date.now();
  });
  lcu.on('state', (st: string) => { if (st === 'connected') setTimeout(() => void refreshMe(), 1500); });
  setInterval(() => void refreshMe(), 120000);
  lcu.on('phase', (p: string) => { recorder.lcuPhase(p); if (p === 'EndOfGame') setTimeout(() => void refreshMe(), 8000); if (p !== 'ChampSelect') runesKey = ''; if (p === 'PreEndOfGame' || p === 'EndOfGame') setTimeout(() => void finishGame(), 3000); });
  lcu.on('champSelect', (sess: ChampSelectSession) => onChampSelect(sess));

  capture.on('frame', onLiveFrame);
  capture.on('scene', s => recorder.scene(s));
  capture.on('hp', (h: { at: number; bars: HealthBar[] }) => {
    if (!intel || replayApiState === 'connected') return;
    // Candidates: alive enemies with their levels. Lane opponent gets a hint position when he
    // is next to me on the minimap (no camera projection yet -> level match first).
    const cands = intel.players.filter(p => !p.isAlly && !p.isDead).map(p => ({ champion: p.champion, level: p.level }));
    for (const m of matchBarsToChampions(h.bars, cands)) {
      if (m.champion) intelTracker.setHp(m.champion, m.bar.pct, intel.gameTime);
    }
    // Only one enemy bar on screen and only my lane opponent is near me on the minimap -> it's him.
    const opp = laneOpponent(intel);
    if (h.bars.length === 1 && opp && myPos && h.bars[0]!.level === null) {
      const nearMe = visibleEnemies.filter(e => Math.hypot(e.x - myPos!.x, e.y - myPos!.y) < 0.08);
      if (nearMe.length === 1 && nearMe[0]!.champion === opp.champion) intelTracker.setHp(opp.champion, h.bars[0]!.pct, intel.gameTime);
    }
  });
  capture.on('status', s => {
    if (s.state === 'connected') {
      capture.configure({ crop: cropRect() });
      if (!settings.value.calibration) setTimeout(() => void autoCalibrate(), 1500);
    }
  });
  gameWindow.on('rect', () => placeOverlay());

  capture.start(PRELOAD, RENDERER);
  capture.configure({ fps: settings.value.minimapFps, crop: null, sceneEverySec: 0 });
  live.start();
  lcu.start();
  gameWindow.start();

  setInterval(() => {
    replayApi.playback().then(() => { replayApiState = 'connected'; }, () => { replayApiState = 'waiting'; });
  }, 1000);

  setInterval(() => {
    sendControl(EVENT.status, status());
    pushOverlay();
    // Stop an auto recording ~15 s after the game API disappears (game over).
    if (recorder.status.active && settings.value.autoRecord && liveLostAt && Date.now() - liveLostAt > 15000) stopRecording();
    if (liveLostAt && Date.now() - liveLostAt > 20000 && matchLog.gameKey && matchLog.gameKey !== reviewDoneFor) void finishGame();
  }, 250);

  const harvestArg = process.argv.find(a => a.startsWith('--harvest-open'));
  if (harvestArg) {
    const limit = Number(process.argv.find(a => a.startsWith('--harvest-seconds='))?.slice(18) ?? 0);
    setTimeout(async () => {
      if (limit) setTimeout(() => harvester.stop(), limit * 1000);
      const pass = process.argv.find(a => a.startsWith('--harvest-pass='))?.slice(15) as 'blue' | 'red' | 'all' | undefined;
      const sessionId = process.argv.find(a => a.startsWith('--harvest-session='))?.slice(18);
      await harvestOpen(8, null, pass ? { passes: [pass], confirmFirst: true, sessionId } : { confirmFirst: true });
      console.log('HARVEST', JSON.stringify(harvester.status));
      app.quit();
    }, 3000);
  } else if (SMOKE) setTimeout(() => {
    const out = process.argv.find(a => a.startsWith('--smoke-frame='))?.slice(14);
    if (out && lastCrop) writeFileSync(out, lastCrop.png);
    const shot = process.argv.find(a => a.startsWith('--shot='))?.slice(7);
    if (shot && control) void control.webContents.capturePage().then(img => writeFileSync(shot, img.toPNG()));
    console.log('SMOKE', JSON.stringify(status()));
    app.quit();
  }, 12000);
});

app.on('second-instance', () => { control?.show(); control?.focus(); });
app.on('before-quit', () => { quitting = true; });
app.on('will-quit', () => {
  hud?.destroy();
  learner?.save();
  vision.stop();
  globalShortcut.unregisterAll();
  recorder?.stop();
  capture.stop();
  live.stop();
  lcu.stop();
  gameWindow.stop();
});
