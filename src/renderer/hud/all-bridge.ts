// Single-window HUD (Tauri): one transparent page over the whole game client area that hosts
// every HUD panel. The panel modules (minimap.ts, topbar.ts, ...) are reused unchanged: each
// looks up its elements by id and then calls window.hud.onState() once, at load time. This
// module runs first and
//   - scopes document.getElementById to the panel whose module is loading (several panels
//     use the same ids, e.g. #root), advancing to the next panel on every onState(),
//   - gives each panel its own state (visible:false when that panel has no content, like a
//     hidden panel),
//   - makes window.innerWidth/innerHeight report the minimap panel (minimap.ts sizes its
//     canvas from them).
import type { HudState } from '../../shared/intel';
import type { HudPayload, HudWindowKind } from './types';

/** Import order in all.ts — must match. */
export const KINDS: HudWindowKind[] = ['minimap', 'topbar', 'death', 'loading', 'power', 'build', 'notify'];

export interface HudMessage {
  state: HudState;
  /** Panels with content (missing = false). */
  wanted: Partial<Record<HudWindowKind, boolean>>;
  /** Minimap square as fractions of the game client area (null = not calibrated). */
  minimap: { x: number; y: number; width: number; height: number } | null;
  /** Panels moved by the player: top-left as fractions of the game client area. */
  layout?: Partial<Record<HudWindowKind, { x: number; y: number }>> | null;
}

export const panels = new Map<HudWindowKind, HTMLElement>(KINDS.map((k) => [k, document.getElementById(`p-${k}`)!]));
const callbacks = new Map<HudWindowKind, (p: HudPayload) => void>();

const realGetById = document.getElementById.bind(document);
let loading = 0;
document.getElementById = (id: string) => {
  const kind = KINDS[loading];
  const panel = kind ? panels.get(kind) : null;
  return panel ? panel.querySelector<HTMLElement>(`#${CSS.escape(id)}`) : realGetById(id);
};

const viewW = () => document.documentElement.clientWidth;
const viewH = () => document.documentElement.clientHeight;
const mm = () => panels.get('minimap')!;
Object.defineProperty(window, 'innerWidth', { configurable: true, get: () => mm().clientWidth || 1 });
Object.defineProperty(window, 'innerHeight', { configurable: true, get: () => mm().clientHeight || 1 });

// minimap.ts animates with requestAnimationFrame forever; while its panel has nothing to show
// run those frames at 4 Hz instead of 60 Hz (the game needs the CPU/GPU more than an empty canvas).
let minimapActive = false;
let minimapWanted = false;
const raf = window.requestAnimationFrame.bind(window);
window.requestAnimationFrame = (cb: FrameRequestCallback) =>
  minimapActive ? raf(cb) : window.setTimeout(() => cb(performance.now()), 250);

window.hud = {
  onState(cb) {
    const kind = KINDS[loading++];
    if (kind) callbacks.set(kind, cb);
    if (loading >= KINDS.length) document.getElementById = realGetById;
    return () => { if (kind) callbacks.delete(kind); };
  },
};

interface Box { x: number; y: number; width: number; height: number }

function place(el: HTMLElement, r: Box): boolean {
  // The panel being dragged keeps the spot under the cursor (a HUD push snapped it back to default).
  if (dragging && panels.get(dragging) === el) return false;
  const s = el.style;
  const v = [Math.round(r.x), Math.round(r.y), Math.max(1, Math.round(r.width)), Math.max(1, Math.round(r.height))].map((n) => `${n}px`);
  if (s.left === v[0] && s.top === v[1] && s.width === v[2] && s.height === v[3]) return false;
  s.left = v[0]!; s.top = v[1]!; s.width = v[2]!; s.height = v[3]!;
  return true;
}

/** Same placement as Hud.layout() in src/main/hud.ts, relative to the game client area. */
function layout(m: HudMessage, s: number): void {
  const W = viewW(), H = viewH();
  const tw = 600 * s, th = 150 * s;
  // AI calls + gank warning: bottom centre, right above the game's HUD (eyes stay near the champion).
  place(panels.get('topbar')!, { x: (W - tw) / 2, y: H * 0.845 - th, width: tw, height: th });
  const dw = 380 * s, dh = 380 * s;
  place(panels.get('death')!, { x: W - dw - W * 0.02, y: H * 0.42 - dh / 2, width: dw, height: dh });
  // Power/danger: compact, mid-height, left of the enemy portraits column on the right edge.
  // Gold / CS / lvl: one line right under the game's score (top right), where that kind of info lives.
  const pw = 320 * s, ph = 150 * s;
  place(panels.get('power')!, { x: W - pw - 14 * s, y: 74 * s, width: pw, height: ph });
  const lw = 720 * s, lh = 480 * s;
  place(panels.get('loading')!, { x: (W - lw) / 2, y: (H - lh) / 2, width: lw, height: lh });
  const mr = m.minimap ? { x: m.minimap.x * W, y: m.minimap.y * H, width: m.minimap.width * W, height: m.minimap.height * H } : null;
  if (mr && place(mm(), mr)) window.dispatchEvent(new Event('resize'));
  // Build panel: directly left of the minimap, bottom-aligned (default minimap without calibration).
  const side = H * 0.237;
  const b = mr ?? { x: W - side, y: H - side, width: side, height: side };
  // Build: left of the minimap, bottom-aligned, narrow (no descriptions) and with a clear gap so it never
  // covers the minimap's edge.
  // Timers / pinned ward / rival back on lane: right above the minimap, where the eyes go every few
  // seconds anyway (the top-right corner under the score is never looked at mid-fight).
  // Above the build panel (left of the minimap): right above the minimap it sat on the ally portraits.
  const nw = 300 * s, nh = 200 * s;
  const bw = 250 * s, bh = 340 * s;
  // Pop-ups (objective soon, rival back, buffs): bottom centre right above the coach bar, where the eyes are.
  // Timers / rival back: top right under the gold / CS line (edges only: the centre covered the game).
  place(panels.get('notify')!, { x: W - 700 * s - 14 * s, y: 74 * s + 156 * s, width: 700 * s, height: 64 * s });
  void nw; void nh;
  place(panels.get('build')!, { x: b.x - bw - 14 * s, y: b.y + b.height - bh, width: bw, height: bh });
  // Panels the player dragged somewhere else (the minimap layer stays on the minimap).
  for (const kind of DRAGGABLE) {
    const o = dragging === kind ? null : localLayout[kind] ?? m.layout?.[kind];
    if (!o) continue;
    const el = panels.get(kind)!;
    place(el, { x: o.x * W, y: o.y * H, width: el.offsetWidth, height: el.offsetHeight });
  }
}

// ---- dragging: the app watches the cursor (this window never takes the mouse) ----
const DRAGGABLE: HudWindowKind[] = ['topbar', 'death', 'loading', 'power', 'build', 'notify'];
const localLayout: Partial<Record<HudWindowKind, { x: number; y: number }>> = {};
let dragging: HudWindowKind | null = null;
let dragBase = { x: 0, y: 0 };
interface TauriBus {
  event: {
    emit(name: string, payload: unknown): Promise<void>;
    listen<T>(name: string, cb: (e: { payload: T }) => void): Promise<() => void>;
  };
}
const bus = (window as unknown as { __TAURI__?: TauriBus }).__TAURI__;

/** Visible content box of a panel (cards / pills), in window CSS px. */
function contentBox(el: HTMLElement): DOMRect | null {
  let r: DOMRect | null = null;
  for (const c of el.querySelectorAll<HTMLElement>('.card, .bar, .alert, .chip')) {
    if (c.closest('.hidden')) continue;
    const b = c.getBoundingClientRect();
    if (b.width < 2 || b.height < 2) continue;
    r = r ? new DOMRect(Math.min(r.left, b.left), Math.min(r.top, b.top), Math.max(r.right, b.right) - Math.min(r.left, b.left), Math.max(r.bottom, b.bottom) - Math.min(r.top, b.top)) : b;
  }
  return r;
}

let lastRects = '';
function reportRects(): void {
  const rects: Array<[string, number[]]> = [];
  for (const kind of DRAGGABLE) {
    const el = panels.get(kind)!;
    if (el.style.visibility !== 'visible') continue;
    const b = contentBox(el);
    if (b) rects.push([kind, [Math.round(b.left), Math.round(b.top), Math.round(b.width), Math.round(b.height)]]);
  }
  const key = JSON.stringify(rects);
  if (key !== lastRects) { lastRects = key; void bus?.event.emit('hud-rects', rects); }
}
window.setInterval(reportRects, 250);

const panelOf = (k: unknown) => (DRAGGABLE.includes(k as HudWindowKind) ? (k as HudWindowKind) : null);
void bus?.event.listen<{ kind: string | null }>('hud-hover', (e) => {
  for (const k of DRAGGABLE) panels.get(k)!.classList.toggle('grab', k === e.payload.kind);
});
void bus?.event.listen<{ kind: string; dx: number; dy: number }>('hud-drag', (e) => {
  const kind = panelOf(e.payload.kind);
  if (!kind) return;
  const el = panels.get(kind)!;
  if (dragging !== kind) { dragging = kind; dragBase = { x: el.offsetLeft, y: el.offsetTop }; }
  el.classList.add('grab');
  el.style.left = `${Math.round(dragBase.x + e.payload.dx)}px`;
  el.style.top = `${Math.round(dragBase.y + e.payload.dy)}px`;
});
void bus?.event.listen<{ kind: string }>('hud-drag-end', (e) => {
  const kind = panelOf(e.payload.kind);
  if (!kind) return;
  const el = panels.get(kind)!;
  dragging = null;
  const pos = { x: el.offsetLeft / viewW(), y: el.offsetTop / viewH() };
  localLayout[kind] = pos;
  void bus?.event.emit('hud-layout', { kind, ...pos });
});
void bus?.event.listen<{ kind: string }>('hud-drag-reset', (e) => {
  const kind = panelOf(e.payload.kind);
  if (!kind) return;
  delete localLayout[kind];
  void bus?.event.emit('hud-layout', { kind, reset: true });
});

const hideTimers = new Map<HudWindowKind, number>();

export function dispatch(m: HudMessage): void {
  const scale = Math.min(1.6, Math.max(0.55, viewH() / 1440));
  layout(m, scale);
  for (const kind of KINDS) {
    const want = !!m.wanted[kind] && (kind !== 'minimap' || !!m.minimap);
    const el = panels.get(kind)!;
    // Keep 60 fps a moment after the content goes so ghosts can fade out.
    if (kind === 'minimap') {
      minimapWanted = want;
      if (want) minimapActive = true;
      else window.setTimeout(() => { if (!minimapWanted) minimapActive = false; }, 600);
    }
    // Keep a panel mapped 180 ms after its content goes away so it can fade out.
    if (want) {
      const t = hideTimers.get(kind);
      if (t) { clearTimeout(t); hideTimers.delete(kind); }
      el.style.visibility = 'visible';
    } else if (el.style.visibility !== 'hidden' && !hideTimers.has(kind)) {
      hideTimers.set(kind, window.setTimeout(() => { hideTimers.delete(kind); el.style.visibility = 'hidden'; }, 180));
    }
    const state: HudState = want ? m.state : { ...m.state, visible: false };
    callbacks.get(kind)?.({ state, scale });
  }
}
