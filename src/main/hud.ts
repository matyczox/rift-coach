// In-game HUD overlay windows (minimap layer, top bar, death card, loading card).
//
// Every window is frameless, transparent, click-through, never focusable and
// shown with showInactive() only. Each window loads its own renderer
// (dist/renderer/hud-*.html) and receives the full HudPayload on HUD_CHANNEL.
//
// Preload: the HUD windows need dist/hud-preload.js (window.hud.onState). The
// constructor uses `preloadPath` if it already points at hud-preload.js,
// otherwise it uses a sibling `hud-preload.js` next to it (falls back to the
// given path if that file does not exist).
import { BrowserWindow, type Rectangle } from 'electron';
import fs from 'node:fs';
import path from 'node:path';
import type { HudState } from '../shared/intel';
import { HUD_CHANNEL, isLive, objectiveUrgent, type HudPayload, type HudWindowKind } from '../renderer/hud/types';

export interface HudRect { x: number; y: number; width: number; height: number }

export interface HudOptions {
  /** Keep every window hidden (offscreen rendering) — for scripts/hud-demo.cjs. */
  demo?: boolean;
}

export interface HudCapture { kind: HudWindowKind; bounds: Rectangle; png: Buffer }

const FILES: Record<HudWindowKind, string> = {
  minimap: 'hud-minimap.html',
  topbar: 'hud-topbar.html',
  death: 'hud-death.html',
  loading: 'hud-loading.html',
  power: 'hud-power.html',
  build: 'hud-build.html',
};
const KINDS = Object.keys(FILES) as HudWindowKind[];
/** Keep a window mapped this long after its content goes away (renderer fades out). */
const HIDE_DELAY_MS = 180;

interface Slot {
  kind: HudWindowKind;
  win: BrowserWindow;
  loaded: boolean;
  wanted: boolean;
  hideTimer: NodeJS.Timeout | null;
  bounds: Rectangle | null;
}

export class Hud {
  private slots = new Map<HudWindowKind, Slot>();
  private game: HudRect | null = null;
  private minimap: HudRect | null = null;
  private last: HudState | null = null;
  private readonly demo: boolean;
  private destroyed = false;
  private loadPromises: Promise<void>[] = [];

  constructor(preloadPath: string, rendererDir: string, opts: HudOptions = {}) {
    this.demo = !!opts.demo;
    const preload = resolvePreload(preloadPath);
    for (const kind of KINDS) {
      const win = new BrowserWindow({
        show: false,
        frame: false,
        transparent: true,
        backgroundColor: '#00000000',
        hasShadow: false,
        resizable: false,
        movable: false,
        minimizable: false,
        maximizable: false,
        fullscreenable: false,
        focusable: false,
        skipTaskbar: true,
        alwaysOnTop: true,
        thickFrame: false,
        width: 100,
        height: 100,
        paintWhenInitiallyHidden: true,
        webPreferences: {
          preload,
          sandbox: false,
          contextIsolation: true,
          nodeIntegration: false,
          backgroundThrottling: false,
          spellcheck: false,
          ...(this.demo ? { offscreen: true } : {}),
        },
      });
      win.setAlwaysOnTop(true, 'screen-saver');
      win.setIgnoreMouseEvents(true);
      win.setMenu(null);
      if (this.demo) win.webContents.setFrameRate(30);
      const slot: Slot = { kind, win, loaded: false, wanted: false, hideTimer: null, bounds: null };
      this.slots.set(kind, slot);
      this.loadPromises.push(new Promise<void>((resolve) => {
        win.webContents.on('did-finish-load', () => {
          slot.loaded = true;
          this.sendTo(slot);
          resolve();
        });
      }));
      void win.loadFile(path.join(rendererDir, FILES[kind]));
    }
  }

  /** Game client area in DIP screen coords (null = game not visible -> hide all). */
  setGameRect(r: HudRect | null): void {
    this.game = r ? { ...r } : null;
    this.layout();
    this.update();
  }

  /** Minimap square in DIP screen coords (null = unknown). */
  setMinimapRect(r: HudRect | null): void {
    this.minimap = r ? { ...r } : null;
    this.layout();
    this.update();
  }

  push(state: HudState): void {
    this.last = state;
    this.update();
  }

  /** Resolves once every HUD renderer has loaded. */
  whenReady(): Promise<void> {
    return Promise.all(this.loadPromises).then(() => undefined);
  }

  /** Demo/debug: PNG snapshots of the windows that currently have content. */
  async capture(): Promise<HudCapture[]> {
    const out: HudCapture[] = [];
    for (const slot of this.slots.values()) {
      if (!slot.wanted || !slot.bounds || slot.win.isDestroyed()) continue;
      const img = await slot.win.webContents.capturePage();
      out.push({ kind: slot.kind, bounds: { ...slot.bounds }, png: img.toPNG() });
    }
    return out;
  }

  destroy(): void {
    this.destroyed = true;
    for (const slot of this.slots.values()) {
      if (slot.hideTimer) clearTimeout(slot.hideTimer);
      if (!slot.win.isDestroyed()) slot.win.destroy();
    }
    this.slots.clear();
  }

  // ---------------------------------------------------------------------------

  private scale(): number {
    const h = this.game?.height ?? 1440;
    return Math.min(1.6, Math.max(0.55, h / 1440));
  }

  private layout(): void {
    const g = this.game;
    if (!g) return;
    const s = this.scale();
    const set = (kind: HudWindowKind, r: HudRect | null) => {
      const slot = this.slots.get(kind);
      if (!slot || !r) return;
      const b: Rectangle = {
        x: Math.round(r.x), y: Math.round(r.y),
        width: Math.max(1, Math.round(r.width)), height: Math.max(1, Math.round(r.height)),
      };
      const o = slot.bounds;
      if (o && o.x === b.x && o.y === b.y && o.width === b.width && o.height === b.height) return;
      slot.bounds = b;
      if (!slot.win.isDestroyed()) slot.win.setBounds(b);
    };

    // Top bar + alert pill: centered, ~6% of game height below the top edge.
    const tw = 600 * s, th = 150 * s;
    set('topbar', { x: g.x + (g.width - tw) / 2, y: g.y + g.height * 0.06, width: tw, height: th });

    // Death card: right-center.
    const dw = 380 * s, dh = 380 * s;
    set('death', { x: g.x + g.width - dw - g.width * 0.02, y: g.y + g.height * 0.42 - dh / 2, width: dw, height: dh });

    // Lane power bar: right side, centered ~32% of game height (above the death card).
    const pw = 280 * s, ph = 190 * s; // power bar + danger row, left of the enemy portraits column
    set('power', { x: g.x + g.width - pw - g.width * 0.075, y: g.y + g.height * 0.5 - ph / 2, width: pw, height: ph });

    // Loading card: centered.
    const lw = 720 * s, lh = 480 * s;
    set('loading', { x: g.x + (g.width - lw) / 2, y: g.y + (g.height - lh) / 2, width: lw, height: lh });

    set('minimap', this.minimap);

    // Build panel: directly left of the minimap, bottom-aligned with it. Without a
    // calibrated minimap, assume the default bottom-right minimap (~23.7% of height).
    const mm = this.minimap ?? (() => {
      const side = g.height * 0.237;
      return { x: g.x + g.width - side, y: g.y + g.height - side, width: side, height: side };
    })();
    // Build: left of the minimap, bottom-aligned, narrow (no descriptions) with a clear gap from its edge.
    const bw = 190 * s, bh = 150 * s;
    set('build', { x: mm.x - bw - 14 * s, y: mm.y + mm.height - bh, width: bw, height: bh });
  }

  private wants(kind: HudWindowKind, st: HudState): boolean {
    if (!this.game || !st.visible) return false;
    switch (kind) {
      case 'minimap':
        return !!this.minimap && (
          st.ghosts.length > 0 || st.showFrame || st.recording || !!st.objective ||
          st.hints.some((h) => isLive(h.expiresAt, st.gameTime)) ||
          st.timers.some((t) => t.secondsLeft <= 60 && t.secondsLeft > 0));
      case 'topbar':
        return (!!st.topBar && isLive(st.topBar.expiresAt, st.gameTime)) ||
          (!!st.alert && isLive(st.alert.expiresAt, st.gameTime)) ||
          (!!st.objective && objectiveUrgent(st.objective));
      case 'death':
        return !!st.deathCard;
      case 'loading':
        return !!st.loadingCard;
      case 'power':
        return !!(st.power || st.danger) && !st.deathCard;
      case 'build':
        return !!st.build && st.build.items.length > 0;
    }
  }

  private update(): void {
    if (this.destroyed) return;
    const st = this.last;
    for (const slot of this.slots.values()) {
      const want = !!st && this.wants(slot.kind, st);
      slot.wanted = want;
      this.sendTo(slot);
      if (this.demo) continue; // never map windows in demo mode
      const win = slot.win;
      if (win.isDestroyed()) continue;
      if (want) {
        if (slot.hideTimer) { clearTimeout(slot.hideTimer); slot.hideTimer = null; }
        if (!win.isVisible() && slot.bounds) {
          win.showInactive();
          win.setAlwaysOnTop(true, 'screen-saver');
        }
      } else if (win.isVisible() && !slot.hideTimer) {
        slot.hideTimer = setTimeout(() => {
          slot.hideTimer = null;
          if (!slot.wanted && !win.isDestroyed()) win.hide();
        }, this.game ? HIDE_DELAY_MS : 0);
      }
    }
  }

  private sendTo(slot: Slot): void {
    if (!slot.loaded || !this.last || slot.win.isDestroyed()) return;
    // A window without content gets visible:false so its renderer fades out.
    const state: HudState = slot.wanted ? this.last : { ...this.last, visible: false };
    const payload: HudPayload = { state, scale: this.scale() };
    slot.win.webContents.send(HUD_CHANNEL, payload);
  }
}

function resolvePreload(given: string): string {
  if (path.basename(given) === 'hud-preload.js') return given;
  const sibling = path.join(path.dirname(given), 'hud-preload.js');
  return fs.existsSync(sibling) ? sibling : given;
}
