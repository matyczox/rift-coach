// Single-window HUD (Tauri): one transparent page over the whole game client area that hosts
// every HUD panel. The panel modules (minimap.ts, topbar.ts, ...) are reused unchanged: each
// looks up its elements by id and then calls window.hud.onState() once, at load time. This
// module runs first and
//   - scopes document.getElementById to the panel whose module is loading (several panels
//     use the same ids, e.g. #root), advancing to the next panel on every onState(),
//   - gives each panel its own state (visible:false when that panel has no content, like a
//     hidden HUD window in the Electron app),
//   - makes window.innerWidth/innerHeight report the minimap panel (minimap.ts sizes its
//     canvas from them).
import type { HudState } from '../../shared/intel';
import type { HudPayload, HudWindowKind } from './types';

/** Import order in all.ts — must match. */
export const KINDS: HudWindowKind[] = ['minimap', 'topbar', 'death', 'loading', 'power', 'build'];

export interface HudMessage {
  state: HudState;
  /** Panels with content (missing = false). */
  wanted: Partial<Record<HudWindowKind, boolean>>;
  /** Minimap square as fractions of the game client area (null = not calibrated). */
  minimap: { x: number; y: number; width: number; height: number } | null;
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
  place(panels.get('topbar')!, { x: (W - tw) / 2, y: H * 0.06, width: tw, height: th });
  const dw = 380 * s, dh = 380 * s;
  place(panels.get('death')!, { x: W - dw - W * 0.02, y: H * 0.42 - dh / 2, width: dw, height: dh });
  const pw = 320 * s, ph = 220 * s;
  place(panels.get('power')!, { x: W - pw - W * 0.02 - 10 * s, y: H * 0.32 - ph / 2, width: pw, height: ph });
  const lw = 720 * s, lh = 480 * s;
  place(panels.get('loading')!, { x: (W - lw) / 2, y: (H - lh) / 2, width: lw, height: lh });
  const mr = m.minimap ? { x: m.minimap.x * W, y: m.minimap.y * H, width: m.minimap.width * W, height: m.minimap.height * H } : null;
  if (mr && place(mm(), mr)) window.dispatchEvent(new Event('resize'));
  // Build panel: directly left of the minimap, bottom-aligned (default minimap without calibration).
  const side = H * 0.237;
  const b = mr ?? { x: W - side, y: H - side, width: side, height: side };
  const bw = 250 * s, bh = 230 * s;
  place(panels.get('build')!, { x: b.x - bw - 4 * s, y: b.y + b.height - bh, width: bw, height: bh });
}

const hideTimers = new Map<HudWindowKind, number>();

export function dispatch(m: HudMessage): void {
  const scale = Math.min(1.6, Math.max(0.55, viewH() / 1440));
  layout(m, scale);
  for (const kind of KINDS) {
    const want = !!m.wanted[kind] && (kind !== 'minimap' || !!m.minimap);
    const el = panels.get(kind)!;
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
