// Right-side laning panel: danger meter row (top) + lane matchup power bar.
// Either part may be absent; the whole panel hides while the death card shows.
import type { DangerState } from '../../shared/intel';
import { $, setOn, subscribe } from './common';

const root = $('root');
const pw = $('pw');
const meIcon = $('meIcon') as HTMLImageElement;
const enIcon = $('enIcon') as HTMLImageElement;
const fill = $('fill');
const pct = $('pct');
const detail = $('detail');
const advice = $('advice');
const danger = $('danger');
const dgLabel = $('dgLabel');
const dgWhy = $('dgWhy');
const dgSegs = [...$('dgMeter').children] as HTMLElement[];

const DANGER_COLOR: Record<DangerState['label'], string> = {
  bezpiecznie: '#3ccf8e',
  'uważaj': '#e8c25a',
  niebezpiecznie: '#ff8a3d',
  uciekaj: '#ff5a4f',
};

function setSrc(img: HTMLImageElement, url: string): void {
  if (img.getAttribute('src') !== url) img.src = url;
}

subscribe(({ state }) => {
  const live = state.visible && !state.deathCard;
  const p = live ? state.power : null;
  const d = live ? state.danger : null;
  setOn(root, !!(p || d));

  danger.classList.toggle('hidden', !d);
  if (d) {
    const lvl = Math.max(0, Math.min(1, d.level));
    danger.style.setProperty('--dc', DANGER_COLOR[d.label] ?? '#e8c25a');
    dgLabel.textContent = d.label.charAt(0).toUpperCase() + d.label.slice(1);
    const filled = lvl <= 0 ? 0 : Math.max(1, Math.ceil(lvl * 4 - 1e-6));
    dgSegs.forEach((s, i) => s.classList.toggle('on', i < filled));
    dgWhy.textContent = d.reasons.slice(0, 2).join(' · ');
    dgWhy.classList.toggle('hidden', d.reasons.length === 0);
  }

  pw.classList.toggle('hidden', !p);
  if (!p) return;
  setSrc(meIcon, p.me.iconUrl);
  setSrc(enIcon, p.enemy.iconUrl);
  meIcon.title = p.me.champion;
  enIcon.title = p.enemy.champion;
  const v = Math.max(0, Math.min(1, p.pMe));
  fill.style.width = `${(v * 100).toFixed(1)}%`;
  fill.classList.toggle('mid', v >= 0.4 && v < 0.6);
  fill.classList.toggle('low', v < 0.4);
  // Percentage label only when the segment is wide enough to hold it.
  pct.textContent = v >= 0.16 ? `${Math.round(v * 100)}%` : '';
  detail.textContent = p.metaWinRate != null
    ? `${p.detail} · meta ${Math.round(p.metaWinRate * 100)}%`
    : p.detail;
  advice.textContent = p.advice;
});
