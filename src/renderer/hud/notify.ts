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
/** Spawn chips: bundled icons (Dragon / Atakhan tinted in CSS). */
const OBJ_ICON: Record<string, string> = { baron: 'icons/baron.png', elder: 'icons/dragon_elder.png', dragon: 'icons/dragon_elder.png', herald: 'icons/riftherald.png', grubs: 'icons/riftherald.png', atakhan: 'icons/baron.png' };
const trinket = document.querySelector<HTMLElement>('#trinket-glow');
const firstSeen = new Map<string, number>();
let trinketWas = false;
let trinketAt = 0;
/** Held Eye duration is not exact (it may be used earlier): show it as an estimate. */
const APPROX = new Set(['herald']);

const guide = document.querySelector<HTMLElement>('#guide');
const guideLabel = document.querySelector<HTMLElement>('#guide-label');

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
      guide.style.transform = `rotate(${deg.toFixed(1)}deg) translateX(9rem)`;
      guide.classList.remove('hidden');
      // What the arrow is for, right under my champion (a bare arrow said nothing).
      if (guideLabel) {
        guideLabel.textContent = g.kind === 'ward' ? `Postaw ward: ${g.label ?? ''}` : (g.label ?? 'Idź tutaj');
        guideLabel.classList.toggle('ward', g.kind === 'ward');
        guideLabel.classList.remove('hidden');
      }
    } else {
      guide.classList.add('hidden');
      guideLabel?.classList.add('hidden');
    }
  }
  // Chips: objectives spawning soon (icon + countdown), buffs, rival back on lane. One entry list.
  type Entry = { key: string; cls: string; icon: string; iconCls?: string; name: string; left: number; approx?: boolean; ret?: boolean };
  const entries: Entry[] = on ? [
    ...(state.objTimers ?? []).map((o): Entry => ({ key: 'o:' + o.kind, cls: 'chip obj ' + o.kind, icon: OBJ_ICON[o.kind] ?? '', iconCls: 'obj-' + o.kind, name: o.label, left: o.secondsLeft })),
    // Eye of the Herald: no way to know when it is used, so the chip only hung there. Baron / Elder only.
    ...(state.buffs ?? []).filter((b) => b.kind !== 'herald').map((b): Entry => ({ key: 'b:' + b.kind + b.team, cls: `chip ${b.kind} ${b.team}`, icon: BUFF_ICON[b.kind] ?? '', name: BUFF_NAME[b.kind] ?? b.kind, left: b.secondsLeft, approx: APPROX.has(b.kind) })),
    ...(state.returns ?? []).map((r): Entry => ({ key: 'r:' + r.champion, cls: 'chip return enemy', icon: r.iconUrl, iconCls: 'champ', name: 'na linii za', left: r.secondsLeft, ret: true })),
  ] : [];
  // Pop up, don't pin: an objective at 60 s, 30 s and the last 10 s; a buff while it runs; the rival's
  // return for 5 s when it appears and the last 5 s. Everything else stays hidden.
  const nowMs = performance.now();
  for (const e of entries) if (!firstSeen.has(e.key)) firstSeen.set(e.key, nowMs);
  for (const k of [...firstSeen.keys()]) if (!entries.some((e) => e.key === k)) firstSeen.delete(k);
  const dt0 = clock.gameTime() - state.gameTime;
  const visibleNow = entries.filter((e) => {
    const left = e.left - dt0;
    if (e.key.startsWith('o:')) return (left > 55 && left <= 60) || (left > 25 && left <= 30) || left <= 10;
    if (e.ret) return true; // the rival's return stays until he is back (it vanished after 5 s)
    return true;
  });
  entries.length = 0;
  entries.push(...visibleNow);
  const key = entries.map((e) => e.key).join(',');
  if (chips.dataset.k !== key) {
    chips.dataset.k = key;
    chips.replaceChildren(...entries.map((e) => {
      const c = document.createElement('div');
      c.className = e.cls;
      const ic = document.createElement('img');
      ic.src = e.icon; ic.alt = '';
      if (e.iconCls) ic.className = e.iconCls;
      const nm = document.createElement('span');
      nm.textContent = e.name;
      c.append(ic, nm, document.createElement('b'));
      return c;
    }));
  }
  const dt = clock.gameTime() - state.gameTime;
  [...chips.children].forEach((c, i) => {
    const e = entries[i];
    if (!e) return;
    const shown = Math.max(0, e.left - dt);
    (c.lastChild as HTMLElement).textContent = (e.approx ? '~' : '') + (e.ret && shown <= 0 ? 'już' : e.ret ? fmtClock(shown) : `${Math.ceil(shown)}s`);
  });
  // Trinket ready: glow on the HUD slot (like Blitz's trinket reminder).
  // Flash 5 s when the ward comes back, then off (a ring sitting there all game was noise).
  const ready = !!(on && state.trinketReady);
  if (ready && !trinketWas) trinketAt = nowMs;
  trinketWas = ready;
  trinket?.classList.toggle('hidden', !(ready && nowMs - trinketAt < 5000));

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
