// Death card: title + 3–5 review bullets, live respawn countdown.
import { $, setOn, subscribe } from './common';
import { fmtClock } from './types';

const root = $('root');
const title = $('title');
const respawn = $('respawn');
const points = $('points');

let lastKey = '';
let respawnAt = 0;

const clock = subscribe(({ state }) => {
  const dc = state.visible ? state.deathCard : null;
  setOn(root, !!dc);
  if (!dc) return;
  respawnAt = dc.respawnAt;
  const key = `${dc.title}\n${dc.points.join('\n')}`;
  if (key === lastKey) return;
  lastKey = key;
  // The countdown is rendered live on the right; drop a baked-in "· respawn 0:24".
  title.textContent = dc.title.replace(/\s*[·•|-]\s*respawn\s*\d+:\d{2}\s*$/i, '');
  points.replaceChildren(...dc.points.slice(0, 5).map((p) => {
    const li = document.createElement('li');
    li.textContent = p;
    return li;
  }));
  tick();
});

function tick(): void {
  const left = respawnAt - clock.gameTime();
  respawn.textContent = left > 0 ? `respawn ${fmtClock(left)}` : '';
}
setInterval(tick, 250);
