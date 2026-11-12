// Build panel left of the minimap: just the next item to buy (the coach adapts it during the game).
// Stays visible while dead (that's when you shop); hides only when null.
import type { BuildPanelState } from '../../shared/intel';
import { $, setOn, subscribe } from './common';

const root = $('root');
const phase = $('phase');
const rows = $('rows');
let lastKey = '';

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls: string, text?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

function render(b: BuildPanelState): void {
  phase.classList.add('hidden');
  rows.replaceChildren(...b.items.slice(0, 1).map((it) => {
    const row = el('div', 'bd-row');
    const img = el('img', '');
    img.alt = '';
    img.src = it.iconUrl;
    img.onerror = () => { img.style.visibility = 'hidden'; };
    const main = el('div', 'bd-main');
    main.append(el('div', 'bd-name', it.name));
    row.append(img, main, el('div', 'bd-cost', it.cost != null ? String(it.cost) : ''));
    return row;
  }));
}

subscribe(({ state }) => {
  const b = state.visible ? state.build : null;
  setOn(root, !!b && b.items.length > 0);
  if (!b) return;
  const key = JSON.stringify(b.items[0] ?? null);
  if (key !== lastKey) { lastKey = key; render(b); }
});
