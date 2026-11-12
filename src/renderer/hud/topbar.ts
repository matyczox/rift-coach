// Top bar: one coach call (tone-colored left edge, optional tag) + alert pill.
import type { AlertState } from '../../shared/intel';
import { $, subscribe } from './common';
import { fmtClock, isLive, objectiveUrgent, VERDICT } from './types';

const bar = $('bar');
const call = $('call');
const tag = $('tag');
const alert = $('alert');
const alertTxt = $('alertTxt');
const alertConf = $('alertConf');
const obj = $('obj');
const objVerdict = $('objVerdict');
const objTxt = $('objTxt');
let lastObjKey = '';

let lastCall = '';
let lastAlertKey = '';

/** Re-run the 150 ms fade-in on an element. */
function enter(el: HTMLElement, extra?: string): void {
  el.classList.remove('enter', 'blink');
  void el.offsetWidth; // restart animation
  el.classList.add('enter');
  if (extra) el.classList.add(extra);
}

function alertColor(a: AlertState): 'red' | 'amber' | 'green' {
  switch (a.kind) {
    case 'invade': case 'gank': case 'fed': return 'red';
    case 'ally': case 'gank-ask': return 'green';
    default: return 'amber';
  }
}

subscribe(({ state }) => {
  const tb = state.visible && state.topBar && isLive(state.topBar.expiresAt, state.gameTime) ? state.topBar : null;
  if (!tb) {
    bar.classList.add('hidden');
    lastCall = '';
  } else {
    bar.className = `bar tone-${tb.tone}`;
    tag.textContent = tb.tag ?? '';
    tag.classList.toggle('hidden', !tb.tag);
    if (tb.call !== lastCall) {
      call.textContent = tb.call;
      lastCall = tb.call;
      enter(bar);
    }
  }

  // Only the coach's call lives up here; alerts and the objective verdict are shown in the danger panel.
  const al = null as AlertState | null;
  if (!al) {
    alert.classList.add('hidden');
    lastAlertKey = '';
  } else {
    const key = `${al.kind}|${al.text}`;
    alertTxt.textContent = al.text;
    alertConf.textContent = `· ${Math.round(Math.max(0, Math.min(1, al.confidence)) * 100)}%`;
    alert.classList.remove('hidden', 'red', 'amber', 'green');
    alert.classList.add(alertColor(al));
    if (key !== lastAlertKey) {
      lastAlertKey = key;
      enter(alert, 'blink'); // blink only during the first second of a new alert
    }
  }

  // Objective call echo: only when the objective is alive or spawns within 30 s.
  const o = null as typeof state.objective;
  if (!o) {
    obj.classList.add('hidden');
    lastObjKey = '';
  } else {
    const v = VERDICT[o.verdict];
    obj.style.setProperty('--c', v.color);
    objVerdict.textContent = v.word;
    const when = o.inSec && o.inSec > 0 ? ` za ${fmtClock(o.inSec)}` : '';
    objTxt.textContent = `${o.objective}${when}${o.reason ? ` — ${o.reason}` : ''}`;
    obj.classList.remove('hidden');
    const key = `${o.objective}|${o.verdict}`;
    if (key !== lastObjKey) { lastObjKey = key; enter(obj); }
  }
});
