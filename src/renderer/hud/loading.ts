// Loading card: one row per enemy (icon, champion + role, summary, rank).
import type { LoadingCardState, Role } from '../../shared/intel';
import { $, setOn, subscribe } from './common';

const ROLE_PL: Record<Role, string> = {
  TOP: 'Góra', JUNGLE: 'Dżungla', MIDDLE: 'Środek', BOTTOM: 'Dół', UTILITY: 'Wsparcie', UNKNOWN: '',
};
const ROLE_ORDER: Role[] = ['TOP', 'JUNGLE', 'MIDDLE', 'BOTTOM', 'UTILITY', 'UNKNOWN'];

const root = $('root');
const rows = $('rows');
let lastKey = '';

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls: string, text?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

function render(card: LoadingCardState): void {
  const enemies = [...card.enemies]
    .sort((a, b) => ROLE_ORDER.indexOf(a.role) - ROLE_ORDER.indexOf(b.role))
    .slice(0, 5);
  rows.replaceChildren(...enemies.map((e) => {
    const row = el('div', 'row');
    const img = el('img', '');
    img.alt = '';
    img.src = e.iconUrl;
    img.onerror = () => { img.style.visibility = 'hidden'; };
    const main = el('div', 'main');
    const who = el('div', 'who', e.champion);
    if (ROLE_PL[e.role]) who.append(el('span', 'role', ROLE_PL[e.role]));
    main.append(who, el('div', 'sum', e.summary || '—'));
    row.append(img, main, el('div', 'rank', e.rank ?? 'brak rangi'));
    return row;
  }));
}

subscribe(({ state }) => {
  const lc = state.visible ? state.loadingCard : null;
  setOn(root, !!lc);
  if (!lc) return;
  const key = JSON.stringify(lc.enemies);
  if (key !== lastKey) { lastKey = key; render(lc); }
});
