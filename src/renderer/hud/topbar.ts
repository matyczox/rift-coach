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
const edge = document.querySelector<HTMLElement>('#danger-edge');
const banner = document.querySelector<HTMLElement>('#obj-banner');
const obj = $('obj');
const objVerdict = $('objVerdict');
const objTxt = $('objTxt');
let lastObjKey = '';
const mini = $('miniWrap');
const miTip = $('miTip');
const miMe = $('miMe') as HTMLImageElement;
const miEn = $('miEn') as HTMLImageElement;
const miFill = $('miFill');
const miPct = $('miPct');
const miAl = $('miAl');
const miEnN = $('miEnN');

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
    case 'invade': case 'gank': case 'fed': case 'steal': return 'red';
    case 'ally': case 'gank-ask': case 'ace': return 'green';
    default: return 'amber';
  }
}

subscribe(({ state }) => {
  // Mini fight bar right above the call (the full panel stays on the right).
  const p = state.visible && !state.deathCard ? state.power : null;
  if (!p) {
    mini.classList.add('hidden');
  } else {
    const v = Math.max(0, Math.min(1, p.pMe));
    if (miMe.src !== p.me.iconUrl) miMe.src = p.me.iconUrl;
    if (miEn.src !== p.enemy.iconUrl) miEn.src = p.enemy.iconUrl;
    miFill.style.width = `${(v * 100).toFixed(1)}%`;
    miFill.className = v < 0.4 ? 'low' : v < 0.6 ? 'mid' : '';
    miPct.textContent = `${Math.round(v * 100)}%`;
    const al = (p.allies ?? []).length, en = (p.enemies ?? []).length;
    miAl.textContent = al ? `+${al}` : '';
    miEnN.textContent = en ? `+${en}` : '';
    $('mini').classList.toggle('far', !p.local);
    mini.classList.remove('hidden');
    // Tip: danger reason first when it is high, else the bar's own advice.
    const dg = state.danger;
    const hot = dg && (dg.label === 'niebezpiecznie' || dg.label === 'uciekaj') && dg.reasons[0];
    // Tip only for a fight here (far away it was "Masz 10, on 9 → trade teraz" all game long), never the
    // danger reason (the alert pill shows it) and not when the coach's call already says back / retreat.
    void hot;
    const callNow = (state.topBar?.call ?? '').toLowerCase();
    const sameAsCall = /back|cofnij|wycofaj|uciekaj/.test(callNow) && /back|hp|wycofaj|odejdź/i.test(p.advice);
    const showTip = p.local && !sameAsCall;
    miTip.textContent = showTip ? p.advice : '';
    miTip.className = 'mini-tip' + (showTip ? '' : ' hidden');
  }
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

  // Warning under the coach's call: gank / invade / missing alerts, else a high danger reason.
  // (Moved here from the power panel: a warning must be seen at a glance, next to the advice.)
  const live = state.visible && state.alert && isLive(state.alert.expiresAt, state.gameTime) ? state.alert : null;
  const dg = state.visible && !state.deathCard ? state.danger : null;
  let al: AlertState | null = live && ['gank', 'invade', 'missing', 'objective', 'steal', 'ace', 'spike'].includes(live.kind)
    ? live
    : null; // no danger reason as a red alert ('Brand blisko' = my lane rival next to me, nothing new)
  void dg;
  // A % only means something for a gank / invade guess; on a danger reason ("Mało HP (2%)") it confused.
  const showConf = !!live && al === live && ['gank', 'invade', 'steal', 'objective'].includes(al.kind);
  // Objective warnings ('Mogą robić Dragon', 'X może ukraść Dragon'): a big banner at the top centre,
  // not one more small pill at the bottom.
  const objAl = !!al && al === live && (al.kind === 'objective' || al.kind === 'steal');
  if (banner) {
    banner.classList.toggle('hidden', !objAl);
    banner.classList.toggle('steal', !!al && al.kind === 'steal');
    if (objAl && al) {
      // Like the game's objective banner: the dragon (tinted by its element), the text, the dragon
      // score and SOUL POINT when a team sits on 3.
      const dr = state.dragons;
      const isDragon = /dragon/i.test(al.text) && !/elder/i.test(al.text);
      const soulPoint = dr && isDragon ? (dr.theirs >= 3 ? 'their' : dr.ours >= 3 ? 'our' : null) : null;
      const k = [al.text, dr?.ours, dr?.theirs, dr?.soul, soulPoint].join('|');
      if (banner.dataset.k !== k) {
        banner.dataset.k = k;
        const icon = document.createElement('img');
        icon.alt = '';
        icon.src = /baron/i.test(al.text) ? 'icons/baron.png' : /herald|grubs/i.test(al.text) ? 'icons/riftherald.png' : 'icons/dragon_elder.png';
        icon.className = 'ob-icon' + (isDragon ? ' el-' + (dr?.soul ?? 'none').toLowerCase() : '');
        const txt = document.createElement('span');
        txt.className = 'ob-text';
        txt.textContent = al.text;
        const parts: HTMLElement[] = [icon, txt];
        if (isDragon && dr) {
          const sc = document.createElement('span');
          sc.className = 'ob-score';
          sc.innerHTML = `<b class="us">${dr.ours}</b>:<b class="them">${dr.theirs}</b>`;
          parts.push(sc);
        }
        if (soulPoint) {
          const sp = document.createElement('span');
          sp.className = 'ob-soul ' + soulPoint;
          sp.textContent = soulPoint === 'their' ? 'ICH SOUL POINT' : 'NASZ SOUL POINT';
          parts.push(sp);
        }
        banner.replaceChildren(...parts);
      }
    }
  }
  if (objAl) al = null;
  // A real gank / invade / steal (not a danger reason): red pulsing screen edge and a solid red pill.
  const threat = !!al && al === live && ['gank', 'invade', 'steal'].includes(al.kind);
  edge?.classList.toggle('hidden', !threat);
  alert.classList.toggle('live', threat);
  if (!al) {
    alert.classList.add('hidden');
    lastAlertKey = '';
  } else {
    const key = `${al.kind}|${al.text}`;
    alertTxt.textContent = al.text;
    alertConf.textContent = showConf ? `${Math.round(Math.max(0, Math.min(1, al.confidence)) * 100)}%` : '';
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
