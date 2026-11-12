import { BrowserWindow, desktopCapturer, ipcMain } from 'electron';
import { EventEmitter } from 'node:events';
import path from 'node:path';
import { CAPTURE } from '../shared/ipc';
import type { CaptureStatus, Rect } from '../shared/types';

export const GAME_WINDOW_TITLE = 'League of Legends (TM) Client';

export interface CropFrame { at: number; width: number; height: number; png: Buffer }
export interface SceneFrame { at: number; width: number; height: number; kind: 'png' | 'jpeg'; data: Buffer }

/**
 * Owns the hidden capture renderer. Finds the game window by exact title,
 * streams it, and emits 'frame' (minimap crop) and 'scene' (full frame).
 */
export class Capture extends EventEmitter {
  status: CaptureStatus = { state: 'off', windowWidth: null, windowHeight: null, fps: 0, lastFrameAt: null, error: null };
  private win: BrowserWindow | null = null;
  private finder: NodeJS.Timeout | null = null;
  private frameTimes: number[] = [];
  private pendingScene: ((s: SceneFrame | null) => void) | null = null;
  private cfg = { fps: 10, crop: null as Rect | null, sceneEverySec: 0, hpEveryMs: 0, hpColor: 'red' as 'red' | 'blue' };

  start(preload: string, rendererDir: string) {
    this.win = new BrowserWindow({
      show: false, width: 200, height: 200, skipTaskbar: true,
      webPreferences: { preload, backgroundThrottling: false, sandbox: false },
    });
    void this.win.loadFile(path.join(rendererDir, 'capture.html'));

    ipcMain.on(CAPTURE.status, (_e, s: { state: CaptureStatus['state']; width: number | null; height: number | null; error: string | null }) => {
      this.status = { ...this.status, state: s.state, windowWidth: s.width, windowHeight: s.height, error: s.error };
      this.emit('status', this.status);
    });
    ipcMain.on(CAPTURE.frame, (_e, f: { at: number; width: number; height: number; png: Uint8Array }) => {
      const now = Date.now();
      this.frameTimes.push(now);
      while (this.frameTimes.length && now - this.frameTimes[0]! > 2000) this.frameTimes.shift();
      this.status.fps = this.frameTimes.length / 2;
      this.status.lastFrameAt = f.at;
      this.emit('frame', { ...f, png: Buffer.from(f.png) } satisfies CropFrame);
    });
    ipcMain.on(CAPTURE.hp, (_e, h: { at: number; width: number; height: number; bars: unknown[] }) => this.emit('hp', h));
    ipcMain.on(CAPTURE.scene, (_e, s: { at: number; width: number; height: number; kind: 'png' | 'jpeg'; data: Uint8Array } | null) => {
      const scene = s ? { ...s, data: Buffer.from(s.data) } : null;
      if (scene?.kind === 'png' || !scene) { this.pendingScene?.(scene); this.pendingScene = null; return; }
      this.emit('scene', scene);
    });

    this.win.webContents.once('did-finish-load', () => {
      this.finder = setInterval(() => void this.findGame(), 2000);
      void this.findGame();
    });
  }

  configure(cfg: Partial<typeof this.cfg>) {
    this.cfg = { ...this.cfg, ...cfg };
    this.win?.webContents.send(CAPTURE.config, this.cfg);
  }

  grabScene(timeoutMs = 3000): Promise<SceneFrame | null> {
    if (this.status.state !== 'connected' || !this.win) return Promise.resolve(null);
    return new Promise(resolve => {
      const t = setTimeout(() => { this.pendingScene = null; resolve(null); }, timeoutMs);
      this.pendingScene = s => { clearTimeout(t); resolve(s); };
      this.win!.webContents.send(CAPTURE.grabScene);
    });
  }

  stop() {
    if (this.finder) clearInterval(this.finder);
    this.win?.destroy();
    this.win = null;
  }

  private async findGame() {
    if (this.status.state === 'connected' || !this.win) return;
    try {
      const sources = await desktopCapturer.getSources({ types: ['window'], thumbnailSize: { width: 0, height: 0 } });
      const game = sources.find(s => s.name === GAME_WINDOW_TITLE);
      if (!game) {
        if (this.status.state !== 'waiting') { this.status = { ...this.status, state: 'waiting', fps: 0, error: null }; this.emit('status', this.status); }
        return;
      }
      this.win.webContents.send(CAPTURE.start, { sourceId: game.id, ...this.cfg });
    } catch (e) {
      this.status = { ...this.status, state: 'error', error: String(e) };
      this.emit('status', this.status);
    }
  }
}
