// Top-right corner: Baron / Elder buff timers (flare up when they appear, count down while the
// buff lasts) and short toasts that slide in and fly away (enemy back).
import { $, subscribe } from './common';
import { fmtClock } from './types';

const chips = $('chips');
const toasts = $('toasts');
const live = new Map<number, HTMLElement>();

const BUFF_NAME: Record<string, string> = { baron: 'Baron', elder: 'Starszy smok' };

subscribe(({ state }, clock) => {
  const on = state.visible;
  // Chips: one per active buff, counting down between pushes.
  const buffs = on ? state.buffs ?? [] : [];
  const key = buffs.map((b) => `${b.kind}:${b.team}`).join(',');
  if (chips.dataset.k !== key) {
    chips.dataset.k = key;
    chips.replaceChildren(...buffs.map((b) => {
      const c = document.createElement('div');
      c.className = `chip ${b.kind} ${b.team}`;
      const n = document.createElement('span');
      n.textContent = BUFF_NAME[b.kind] ?? b.kind;
      const t = document.createElement('b');
      c.append(n, t);
      return c;
    }));
  }
  [...chips.children].forEach((c, i) => {
    const b = buffs[i];
    if (b) (c.lastChild as HTMLElement).textContent = fmtClock(b.secondsLeft - (clock.gameTime() - state.gameTime));
  });

  // Toasts: add new ones, fly out the ones that left the state.
  const want = on ? state.toasts ?? [] : [];
  const ids = new Set(want.map((t) => t.id));
  for (const t of want) {
    if (live.has(t.id)) continue;
    const el = document.createElement('div');
    el.className = `toast ${t.kind} ${t.team}`;
    if (t.iconUrl) {
      const img = document.createElement('img');
      img.src = t.iconUrl; img.alt = '';
      el.append(img);
    }
    const s = document.createElement('span');
    s.textContent = t.text;
    el.append(s);
    toasts.append(el);
    live.set(t.id, el);
    requestAnimationFrame(() => el.classList.add('in'));
  }
  for (const [id, el] of live) {
    if (ids.has(id)) continue;
    live.delete(id);
    el.classList.remove('in');
    el.classList.add('out');
    window.setTimeout(() => el.remove(), 450);
  }
});
