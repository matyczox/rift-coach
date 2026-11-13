// Top-right corner: Baron / Elder buff timers (flare up when they appear, count down while the
// buff lasts) and short toasts that slide in and fly away (enemy back).
import { $, subscribe } from './common';
import { fmtClock, secondsUntil } from './types';

const chips = $('chips');
// Alerts live in their own full-window layer (outside the panel, which clips).
const toasts = document.querySelector<HTMLElement>('#toasts-layer') ?? $('toasts');
const live = new Map<number, HTMLElement>();

const BUFF_NAME: Record<string, string> = { baron: 'Baron', elder: 'Elder Dragon', herald: 'Eye of the Herald' };
/** In-game minimap icons (CommunityDragon), bundled with the app. */
const BUFF_ICON: Record<string, string> = { baron: 'icons/baron.png', elder: 'icons/dragon_elder.png', herald: 'icons/riftherald.png' };
/** Held Eye duration is not exact (it may be used earlier): show it as an estimate. */
const APPROX = new Set(['herald']);

const guide = document.querySelector<HTMLElement>('#guide');

subscribe(({ state }, clock) => {
  const on = state.visible;
  // Pale arrow on my champion towards a "go there" hint (help an ally / coach move): the angle on the
  // minimap, with the camera's tilt (screen y shorter than map y).
  if (guide) {
    const live = on ? (state.hints ?? []).filter((x) => x.from && secondsUntil(x.expiresAt, clock.gameTime()) > 0) : [];
    const g = live.find((x) => x.kind === 'move') ?? live.find((x) => x.kind === 'ward');
    guide.classList.toggle('ward', g?.kind === 'ward');
    if (g && g.from && Math.hypot(g.x - g.from.x, g.y - g.from.y) > 0.04) {
      const deg = (Math.atan2((g.y - g.from.y) * 0.8, g.x - g.from.x) * 180) / Math.PI;
      guide.style.transform = `rotate(${deg.toFixed(1)}deg) translateX(5.5rem)`;
      guide.classList.remove('hidden');
    } else {
      guide.classList.add('hidden');
    }
  }
  // Chips: one per active buff, counting down between pushes.
  const buffs = on ? state.buffs ?? [] : [];
  const rets = on ? state.returns ?? [] : [];
  const pin = on ? state.pin ?? null : null;
  const key = buffs.map((b) => `${b.kind}:${b.team}`).concat(rets.map((r) => `r:${r.champion}`)).join(',') + `|${pin ?? ''}`;
  if (chips.dataset.k !== key) {
    chips.dataset.k = key;
    const pinChip = pin ? [(() => { const c = document.createElement('div'); c.className = 'chip pin ally'; const n = document.createElement('span'); n.textContent = pin; c.append(n); return c; })()] : [];
    chips.replaceChildren(...pinChip, ...buffs.map((b) => {
      const c = document.createElement('div');
      c.className = `chip ${b.kind} ${b.team}`;
      const ic = document.createElement('img');
      ic.src = BUFF_ICON[b.kind] ?? ''; ic.alt = '';
      const n = document.createElement('span');
      n.textContent = BUFF_NAME[b.kind] ?? b.kind;
      const t = document.createElement('b');
      c.append(ic, n, t);
      return c;
    }), ...rets.map((r) => {
      const c = document.createElement('div');
      c.className = 'chip return enemy';
      const ic = document.createElement('img');
      ic.src = r.iconUrl; ic.alt = ''; ic.className = 'champ';
      const n = document.createElement('span');
      n.textContent = 'na linii za';
      c.append(ic, n, document.createElement('b'));
      return c;
    }));
  }
  const dt = clock.gameTime() - state.gameTime;
  const off = pin ? 1 : 0;
  [...chips.children].forEach((c0, i0) => {
    const i = i0 - off;
    const c = c0;
    if (i < 0) return;
    const b = buffs[i];
    const r = rets[i - buffs.length];
    const left = b ? b.secondsLeft : r ? r.secondsLeft : null;
    if (left == null) return;
    const shown = Math.max(0, left - dt);
    (c.lastChild as HTMLElement).textContent = (b && APPROX.has(b.kind) ? '~' : '') + (r && shown <= 0 ? 'już' : fmtClock(shown));
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
