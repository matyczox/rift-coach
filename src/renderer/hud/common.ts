// Small helpers shared by the HUD window renderers.
import type { HudPayload } from './types';

export interface Clock {
  /** Game time extrapolated from the last push (seconds). */
  gameTime(): number;
}

/** Subscribe to HUD pushes; sets root font-size (16px @ 1440p) and keeps a game clock. */
export function subscribe(cb: (p: HudPayload, clock: Clock) => void): Clock {
  let base = 0;
  let at = performance.now();
  let lastScale = -1;
  const clock: Clock = { gameTime: () => base + (performance.now() - at) / 1000 };
  window.hud?.onState((p) => {
    base = p.state.gameTime;
    at = performance.now();
    if (p.scale !== lastScale) {
      lastScale = p.scale;
      document.documentElement.style.fontSize = `${(16 * p.scale).toFixed(2)}px`;
    }
    cb(p, clock);
  });
  return clock;
}

export function $(id: string): HTMLElement {
  const el = document.getElementById(id);
  if (!el) throw new Error(`missing #${id}`);
  return el;
}

/** Show/hide a .fade element with the 150 ms fade. */
export function setOn(el: HTMLElement, on: boolean): void {
  el.classList.toggle('on', on);
}
