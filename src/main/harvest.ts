import { EventEmitter } from 'node:events';
import { createWriteStream, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { createGzip, constants as zc } from 'node:zlib';
import path from 'node:path';
import type { HarvestStatus } from '../shared/types';
import type { CropFrame } from './capture';
import { replayApi, type Playback } from './sources/replayApi';
import { getJson } from './sources/http';
import { execFile } from 'node:child_process';

export type FogTeam = 'blue' | 'red' | 'all';

/**
 * Pick the fog team in the replay HUD (Mgla wojny: Niebieski / Czerwony / Wszystkie) with two clicks in
 * the replay window, then read the coloured label back to confirm (scripts/replay-fog.ps1).
 * Replays only: the harvester runs only while the Replay API is up, never in a live game.
 */
export function setReplayFog(team: FogTeam): Promise<boolean> {
  const script = path.join(__dirname, '..', 'scripts', 'replay-fog.ps1');
  if (!existsSync(script)) return Promise.resolve(false);
  return new Promise(res => execFile('powershell.exe', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', script, 'fog', team],
    { windowsHide: true, timeout: 20000 }, (err, out) => res(!err && /^OK fog/m.test(String(out)))));
}

/** Session sub-folder for each team's view. frames/ stays the default (blue) view for the player. */
// 'all' = HUD fog 'Wszystkie': union of both teams' vision -> every champion is drawn (full truth).
export const PASS_DIR: Record<FogTeam, string> = { blue: 'frames', red: 'red', all: 'all' };

/**
 * Collects training/evaluation data from an open replay.
 *
 * Which champions the minimap draws depends on the team picked in the replay
 * HUD ("Mgła wojny"), NOT on the Replay API fogOfWar flag (that only removes
 * the dark overlay). So we play the replay once per team view:
 *   blue pass -> all blue champions + red ones blue could see (= a blue player's view)
 *   red pass  -> all red champions + blue ones red could see
 * Ground truth for all 10 = blue champs from the blue pass + red champs from the red pass.
 * The HUD team can't be switched through the API and we never send input to the
 * game, so between passes the harvester waits for the user ('waiting-fog').
 *
 * Output (normal recording folder, frames named by game time in ms):
 *   frames/<gameMs>.png   blue-team view
 *   red/<gameMs>.png      red-team view
 *   api.jsonl.gz          {o: gameMs, d: allgamedata}
 *   meta.json             source: 'replay', passes done
 */
export class Harvester extends EventEmitter {
  status: HarvestStatus = { state: 'idle', gameId: null, pass: null, positionSec: 0, lengthSec: 0, frames: 0, error: null };
  private dir: string | null = null;
  private pb: { at: number; time: number; speed: number; paused: boolean } | null = null;
  private lastFrameGameMs = -1;
  private stopRequested = false;
  private resume: (() => void) | null = null;

  constructor(private root: string) { super(); }

  get running() { return this.status.state === 'running' || this.status.state === 'waiting-fog'; }

  /** Called for every captured minimap crop while running. */
  frame(f: CropFrame) {
    if (this.status.state !== 'running' || !this.dir || !this.pb || this.pb.paused || !this.status.pass) return;
    // Interpolate game time at capture moment from the last playback poll.
    const gameMs = Math.round((this.pb.time + ((f.at - this.pb.at) / 1000) * this.pb.speed) * 1000);
    if (gameMs <= this.lastFrameGameMs) return;
    this.lastFrameGameMs = gameMs;
    writeFileSync(path.join(this.dir, PASS_DIR[this.status.pass], `${gameMs}.png`), f.png);
    this.status.frames++;
  }

  /** User confirmed the HUD fog team is set for the pending pass. */
  continue() { this.resume?.(); }

  stop() { this.stopRequested = true; this.resume?.(); }

  /**
   * @param passes   team views to record, in order
   * @param confirmFirst  true if the HUD is already on passes[0] (no wait before the first pass)
   * @param sessionId  add passes to an existing harvest folder (e.g. a missing red pass)
   */
  async run(opts: { gameId: number | null; speed: number; meta: Record<string, unknown>; passes?: FogTeam[]; confirmFirst?: boolean; sessionId?: string }) {
    if (this.running) throw new Error('harvest already running');
    this.stopRequested = false;
    const passes = opts.passes ?? ['blue', 'red'];
    const first = await replayApi.playback(); // throws if no replay is open
    const id = opts.sessionId ?? `replay-${opts.gameId ?? 'unknown'}-${new Date().toISOString().replace(/[:.]/g, '-').slice(0, 19)}`;
    this.dir = path.join(this.root, path.basename(id));
    for (const p of passes) mkdirSync(path.join(this.dir, PASS_DIR[p]), { recursive: true });
    const metaFile = path.join(this.dir, 'meta.json');
    const prevMeta = existsSync(metaFile) ? JSON.parse(readFileSync(metaFile, 'utf8')) : {};
    const meta = {
      id, startedAt: Date.now(), durationMs: Math.round(first.length * 1000), frames: 0, bytes: 0, source: 'replay',
      gameId: opts.gameId, speed: opts.speed, gameMode: null as string | null, champion: null, allies: [] as string[], enemies: [] as string[],
      passesDone: [] as FogTeam[], ...prevMeta, ...opts.meta,
    };
    this.status = { state: 'running', gameId: opts.gameId, pass: null, positionSec: 0, lengthSec: first.length, frames: 0, error: null };

    const wantApi = !existsSync(path.join(this.dir, 'api.jsonl.gz'));
    const gz = wantApi ? createGzip({ flush: zc.Z_SYNC_FLUSH }) : null;
    if (gz) gz.pipe(createWriteStream(path.join(this.dir, 'api.jsonl.gz')));

    try {
      for (const [k, pass] of passes.entries()) {
        if (this.stopRequested) break;
        await replayApi.setPlayback({ paused: true }).catch(() => undefined);
        // Switch the replay HUD's fog team by itself; ask the user only if that fails.
        const switched = await setReplayFog(pass);
        if (!switched && (k > 0 || !opts.confirmFirst)) {
          this.status = { ...this.status, state: 'waiting-fog', pass, needFog: pass };
          this.emit('status', this.status);
          await new Promise<void>(r => { this.resume = r; });
          this.resume = null;
          if (this.stopRequested) break;
        }
        this.status = { ...this.status, state: 'running', pass, needFog: undefined };
        this.lastFrameGameMs = -1;
        await replayApi.setRender({ fogOfWar: true });
        await replayApi.seek(0);
        await new Promise(r => setTimeout(r, 400));
        await replayApi.setPlayback({ speed: opts.speed, paused: false });
        let lastApi = 0;
        for (;;) {
          if (this.stopRequested) break;
          const pb: Playback = await replayApi.playback();
          this.pb = { at: Date.now(), time: pb.time, speed: pb.speed, paused: pb.paused || pb.seeking };
          this.status.positionSec = pb.time;
          this.emit('status', this.status);
          if (gz && k === 0 && Date.now() - lastApi > 250) {
            lastApi = Date.now();
            getJson<{ gameData: { gameMode: string }; allPlayers: Array<{ championName: string; team: string }> }>('https://127.0.0.1:2999/liveclientdata/allgamedata')
              .then(d => {
                gz.write(JSON.stringify({ o: Math.round(pb.time * 1000), d }) + '\n');
                meta.gameMode = d.gameData.gameMode;
                meta.allies = d.allPlayers.filter(p => p.team === 'ORDER').map(p => p.championName);
                meta.enemies = d.allPlayers.filter(p => p.team === 'CHAOS').map(p => p.championName);
              })
              .catch(() => { /* replay seeking: try next poll */ });
          }
          if (pb.time >= pb.length - 0.5 || (pb.paused && pb.time > 5)) break;
          await new Promise(r => setTimeout(r, 120));
        }
        this.pb = null;
        await replayApi.setPlayback({ paused: true }).catch(() => undefined);
        if (!this.stopRequested && !meta.passesDone.includes(pass)) meta.passesDone.push(pass);
      }
      this.status = { ...this.status, state: this.stopRequested ? 'idle' : 'done', needFog: undefined };
    } catch (e) {
      this.status = { ...this.status, state: 'error', error: String(e), needFog: undefined };
    } finally {
      this.pb = null;
      gz?.end();
      meta.frames = (prevMeta.frames ?? 0) + this.status.frames;
      writeFileSync(metaFile, JSON.stringify(meta, null, 2));
      this.emit('status', this.status);
    }
  }
}
