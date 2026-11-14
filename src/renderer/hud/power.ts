// Right-side laning panel: danger meter row (top) + lane matchup power bar.
// Either part may be absent; the whole panel hides while the death card shows.
import type { DangerState } from '../../shared/intel';
import { $, setOn, subscribe } from './common';
import { isLive, objectiveUrgent, VERDICT } from './types';

const root = $('root');
const pw = $('pw');
const meIcon = $('meIcon') as HTMLImageElement;
const enIcon = $('enIcon') as HTMLImageElement;
const meNear = $('meNear');
const enNear = $('enNear');
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

/** Small icons of the champions around me; fog ones faded with a '?'. */
function setNear(box: HTMLElement, list: Array<{ champion: string; iconUrl: string; level: number; sure: boolean }> | undefined): void {
  const items = (list ?? []).slice(0, 4);
  const key = items.map((n) => `${n.champion}:${n.level}:${n.sure}`).join(',');
  if (box.dataset.k === key) return;
  box.dataset.k = key;
  box.replaceChildren(...items.map((n) => {
    const s = document.createElement('span');
    s.className = 'pw-mini' + (n.sure ? '' : ' fog');
    const img = document.createElement('img');
    img.src = n.iconUrl; img.alt = ''; img.title = `${n.champion} · lvl ${n.level}`;
    s.append(img);
    return s;
  }));
}

function setSrc(img: HTMLImageElement, url: string): void {
  if (img.getAttribute('src') !== url) img.src = url;
}

subscribe(({ state }) => {
  const live = state.visible && !state.deathCard;
  const p = live ? state.power : null;
  const al = live && state.alert && isLive(state.alert.expiresAt, state.gameTime) ? state.alert : null;
  const o = live && state.objective && objectiveUrgent(state.objective) ? state.objective : null;
  const extra = [al?.text, o ? `${o.objective}: ${VERDICT[o.verdict].word}` : null].filter((x): x is string => !!x);
  const d0 = live ? (state.danger ?? (extra.length ? { level: 0.25, label: 'uważaj', reasons: [] } as DangerState : null)) : null;
  // Glanceable: nothing to say when it is safe and nothing happened.
  // Danger / gank warnings moved to the top bar (next to the coach's call); this panel = the fight bar.
  void d0;
  const d = null as DangerState | null;
  setOn(root, !!(p || d));

  danger.classList.toggle('hidden', !d);
  if (d) {
    const lvl = Math.max(0, Math.min(1, d.level));
    danger.style.setProperty('--dc', DANGER_COLOR[d.label] ?? '#e8c25a');
    dgLabel.textContent = d.label.charAt(0).toUpperCase() + d.label.slice(1);
    const filled = lvl <= 0 ? 0 : Math.max(1, Math.ceil(lvl * 4 - 1e-6));
    dgSegs.forEach((s, i) => s.classList.toggle('on', i < filled));
    // One reason only, never a sentence to read.
    const why = [...extra, ...d.reasons][0] ?? '';
    dgWhy.textContent = why;
    dgWhy.classList.toggle('hidden', !why);
  }

  pw.classList.toggle('hidden', !p);
  detail.classList.toggle('hidden', !p?.lane);
  if (!p) return;
  setSrc(meIcon, p.me.iconUrl);
  setSrc(enIcon, p.enemy.iconUrl);
  setNear(meNear, p.allies);
  setNear(enNear, p.enemies);
  meIcon.title = p.me.champion;
  enIcon.title = p.enemy.champion;
  const v = Math.max(0, Math.min(1, p.pMe));
  fill.style.width = `${(v * 100).toFixed(1)}%`;
  fill.classList.toggle('mid', v >= 0.4 && v < 0.6);
  fill.classList.toggle('low', v < 0.4);
  // Percentage label only when the segment is wide enough to hold it.
  pct.textContent = v >= 0.16 ? `${Math.round(v * 100)}%` : '';
  const ln = p.lane;
  if (ln) {
    // Blitz-like benchmark card: three big diffs (gold / CS / level), CS per minute vs my rival, and
    // the item value of both teams as a split bar. Numbers first, tiny labels, nothing to read.
    const k = (v: number) => (Math.abs(v) >= 1000 ? `${(Math.abs(v) / 1000).toFixed(1)}k` : `${Math.abs(v)}`);
    const sign = (v: number) => (v > 0 ? '+' : v < 0 ? '−' : '');
    const cell = (v: number, label: string, fmt: (x: number) => string) => {
      const d = document.createElement('div');
      d.className = 'bm-cell ' + (v > 0 ? 'up' : v < 0 ? 'down' : '');
      const b = document.createElement('b');
      b.textContent = sign(v) + fmt(v);
      const sm = document.createElement('small');
      sm.textContent = label;
      d.append(b, sm);
      return d;
    };
    const top = document.createElement('div');
    top.className = 'bm-top';
    top.append(cell(ln.gold, 'złoto', k), cell(ln.cs, 'cs', (x) => `${Math.abs(x)}`), cell(ln.level, 'lvl', (x) => `${Math.abs(x)}`));
    const rows: HTMLElement[] = [top];
    const target = document.createElement('div');
    target.className = 'bm-row';
    target.textContent = `${p.me.champion} ${p.me.level} vs ${p.enemy.champion} ${p.enemy.level} lvl`;
    rows.push(target);
    if (p.botLevels) {
      const pair = document.createElement('div');
      pair.className = 'bm-row bm-bot';
      const b = p.botLevels;
      const levels = (side: typeof b.ours) => side.map(x => `${x.champion} ${x.level}`).join(' + ');
      pair.textContent = `Bot: ${levels(b.ours)} vs ${levels(b.theirs)} · suma ${sign(b.difference)}${Math.abs(b.difference)} lvl`;
      rows.push(pair);
    }
    if (p.csMin && p.csMin.opp != null) {
      const r = document.createElement('div');
      const diff = p.csMin.me - p.csMin.opp;
      r.className = 'bm-row ' + (diff > 0.05 ? 'up' : diff < -0.05 ? 'down' : '');
      r.innerHTML = `<small>CS/min</small><b>${p.csMin.me.toFixed(1)}</b><i>${diff > 0.05 ? '▲' : diff < -0.05 ? '▼' : '='}</i><span>${p.csMin.opp.toFixed(1)}</span>`;
      rows.push(r);
    }
    if (p.itemValue && p.itemValue.ours + p.itemValue.theirs > 0) {
      const { ours, theirs } = p.itemValue;
      const r = document.createElement('div');
      r.className = 'bm-items';
      const share = (ours / (ours + theirs)) * 100;
      r.innerHTML = `<small>przedmioty</small><b class="us">${k(ours)}</b><div class="bm-split"><i style="width:${share.toFixed(1)}%"></i></div><b class="them">${k(theirs)}</b>`;
      rows.push(r);
    }
    detail.className = 'card bench';
    detail.replaceChildren(...rows);
  } else {
    detail.replaceChildren();
  }
  advice.textContent = p.advice;
});
