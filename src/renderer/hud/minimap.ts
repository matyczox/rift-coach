// Minimap layer: fog-of-war ghosts, coach hints, objective timers, frame, rec dot.
// Canvas 2D, DPR-aware, 60 fps with ghost positions eased between ~10 Hz pushes.
import type { HudState, MinimapGhost, MinimapHint, ObjectiveCall } from '../../shared/intel';
import { subscribe } from './common';
import { fmtClock, isLive, objectiveKind, secondsUntil, VERDICT } from './types';

const GREEN = '60,207,142';
const RED = '255,90,79';
/** Ghost areas: muted rose, so they inform without shouting over the minimap. */
const GHOST = '214,150,142';
const OUTLINE = 'rgba(0,0,0,0.55)';

const canvas = document.getElementById('c') as HTMLCanvasElement;
const ctx = canvas.getContext('2d')!;

let state: HudState | null = null;

// ---- ghosts (eased) ----
interface Ghost {
  g: MinimapGhost;        // latest data
  x: number; y: number; r: number; // displayed (eased) values, normalized
  alpha: number;          // presence fade 0..1
  gone: boolean;
  area?: boolean;         // draws its area (only the 3 most recent ghosts do)
}
const ghosts = new Map<string, Ghost>();

// ---- images ----
const images = new Map<string, HTMLImageElement>();
function image(url: string): HTMLImageElement | null {
  if (!url) return null;
  let img = images.get(url);
  if (!img) {
    img = new Image();
    img.src = url;
    images.set(url, img);
  }
  return img.complete && img.naturalWidth > 0 ? img : null;
}

// ---- hint first-seen times (invade pulse) ----
const firstSeen = new Map<string, number>();
const hintKey = (h: MinimapHint) => `${h.kind}:${h.x.toFixed(3)}:${h.y.toFixed(3)}`;

const clock = subscribe(({ state: st }) => {
  state = st;
  const seen = new Set<string>();
  if (st.visible) {
    for (const g of st.ghosts) {
      seen.add(g.champion);
      const cur = ghosts.get(g.champion);
      if (cur) { cur.g = g; cur.gone = false; }
      else ghosts.set(g.champion, { g, x: g.x, y: g.y, r: g.radius, alpha: 0, gone: false });
      image(g.iconUrl);
    }
  }
  for (const [k, v] of ghosts) if (!seen.has(k)) v.gone = true;

  const now = performance.now();
  const live = new Set<string>();
  for (const h of st.hints) {
    const k = hintKey(h);
    live.add(k);
    if (!firstSeen.has(k)) firstSeen.set(k, now);
  }
  for (const k of [...firstSeen.keys()]) if (!live.has(k)) firstSeen.delete(k);
});

// ---- sizing ----
let W = 0, H = 0, dpr = 1;
function resize(): void {
  dpr = window.devicePixelRatio || 1;
  W = window.innerWidth; H = window.innerHeight;
  canvas.width = Math.max(1, Math.round(W * dpr));
  canvas.height = Math.max(1, Math.round(H * dpr));
}
window.addEventListener('resize', resize);
resize();

// ---- drawing ----
function drawGhost(gh: Ghost): void {
  const { g } = gh;
  const cx = gh.x * W, cy = gh.y * H;
  const iconD = Math.max(14, 0.075 * W);
  // Readability (a real game had 2.6 ghosts at once, overlapping 43% of the time, median radius 0.41):
  // small capped area, fading from 15 s and gone at 40 s; the portrait (who) stays a bit longer.
  // Just-lost enemies (< 2 s) get no area yet: they are still where you saw them.
  if (g.age < 2) return;
  const r = Math.max(iconD * 0.5 + 3, Math.min(gh.r, 0.16) * W);
  const conf = Math.max(0, Math.min(1, g.confidence));
  const zf = !gh.area ? 0 : g.age <= 15 ? 1 : g.age >= 40 ? 0 : 1 - (g.age - 15) / 25;
  const a = gh.alpha;

  if (zf > 0) {
    ctx.beginPath();
    ctx.arc(cx, cy, r, 0, Math.PI * 2);
    ctx.fillStyle = `rgba(${GHOST},${(0.05 + 0.05 * conf) * a * zf})`;
    ctx.fill();
    ctx.setLineDash([Math.max(3, W * 0.012), Math.max(3, W * 0.012)]);
    ctx.lineWidth = 1.1;
    ctx.strokeStyle = `rgba(${GHOST},${0.55 * a * zf})`;
    ctx.stroke();
    ctx.setLineDash([]);
    // Other real options (e.g. Dragon or top): weaker zones with a small portrait, so one circle in the
    // middle does not pretend to know.
    for (const z of g.zones ?? []) {
      const zx = z.x * W, zy = z.y * H;
      const zr = Math.max(iconD * 0.35 + 3, Math.min(z.radius, 0.12) * W);
      const za = a * zf * Math.min(1, z.mass / 0.5);
      ctx.beginPath();
      ctx.arc(zx, zy, zr, 0, Math.PI * 2);
      ctx.fillStyle = `rgba(${GHOST},${0.05 * za})`;
      ctx.fill();
      ctx.setLineDash([Math.max(2, W * 0.008), Math.max(3, W * 0.014)]);
      ctx.lineWidth = 1;
      ctx.strokeStyle = `rgba(${GHOST},${0.45 * za})`;
      ctx.stroke();
      ctx.setLineDash([]);
      const zimg = image(g.iconUrl);
      if (zimg) {
        const d = iconD * 0.6;
        ctx.save();
        ctx.globalAlpha = 0.5 * za;
        ctx.beginPath();
        ctx.arc(zx, zy, d / 2, 0, Math.PI * 2);
        ctx.clip();
        const c = zimg.naturalWidth * 0.08;
        ctx.drawImage(zimg, c, c, zimg.naturalWidth - 2 * c, zimg.naturalHeight - 2 * c, zx - d / 2, zy - d / 2, d, d);
        ctx.restore();
      }
    }
  }

  // Portrait: who it is, always readable; fades out from 35 s to 45 s since last seen.
  const pf = g.age <= 35 ? 1 : g.age >= 45 ? 0 : 1 - (g.age - 35) / 10;
  const img = pf > 0 ? image(g.iconUrl) : null;
  if (img) {
    ctx.save();
    ctx.globalAlpha = 0.8 * pf * a;
    ctx.beginPath();
    ctx.arc(cx, cy, iconD / 2, 0, Math.PI * 2);
    ctx.clip();
    // ddragon square icons have a thin border; crop ~8% to hide it.
    const c = img.naturalWidth * 0.08;
    ctx.drawImage(img, c, c, img.naturalWidth - 2 * c, img.naturalHeight - 2 * c,
      cx - iconD / 2, cy - iconD / 2, iconD, iconD);
    ctx.restore();
  }
}

function arrow(fx: number, fy: number, tx: number, ty: number, alpha: number, rgb: string = GREEN): void {
  const dx = tx - fx, dy = ty - fy;
  const len = Math.hypot(dx, dy);
  if (len < 1) return;
  const ux = dx / len, uy = dy / len;
  const maxLen = 0.2 * W;
  const L = Math.min(len, maxLen);
  const sx = tx - ux * L, sy = ty - uy * L;
  const head = Math.max(6, 0.03 * W);
  const bx = tx - ux * head, by = ty - uy * head;
  const px = -uy, py = ux;

  const path = () => {
    ctx.beginPath();
    ctx.moveTo(sx, sy);
    ctx.lineTo(bx, by);
  };
  const tri = () => {
    ctx.beginPath();
    ctx.moveTo(tx, ty);
    ctx.lineTo(bx + px * head * 0.55, by + py * head * 0.55);
    ctx.lineTo(bx - px * head * 0.55, by - py * head * 0.55);
    ctx.closePath();
  };
  ctx.lineCap = 'round';
  ctx.lineJoin = 'round';
  // thin dark outline for contrast on the green minimap
  path(); ctx.lineWidth = 4.5; ctx.strokeStyle = `rgba(0,0,0,${0.45 * alpha})`; ctx.stroke();
  tri(); ctx.lineWidth = 2; ctx.strokeStyle = `rgba(0,0,0,${0.45 * alpha})`; ctx.stroke();
  path(); ctx.lineWidth = 2.5; ctx.strokeStyle = `rgba(${rgb},${alpha})`; ctx.stroke();
  tri(); ctx.fillStyle = `rgba(${rgb},${alpha})`; ctx.fill();
}

function ring(x: number, y: number, r: number, rgb: string, alpha: number, fill: number): void {
  ctx.beginPath();
  ctx.arc(x, y, r, 0, Math.PI * 2);
  if (fill > 0) { ctx.fillStyle = `rgba(${rgb},${fill * alpha})`; ctx.fill(); }
  ctx.lineWidth = 1.5;
  ctx.strokeStyle = `rgba(${rgb},${0.9 * alpha})`;
  ctx.stroke();
}

interface Seg { text: string; color?: string; weight?: number; small?: boolean }
const FONT = '"Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif';

function pill(text: string, x: number, y: number, color = '#eef3f6', anchor: 'center' | 'left' = 'center'): void {
  pillSegs([{ text, color }], x, y, anchor);
}

/** Pill made of differently styled text runs, clamped inside the minimap. */
function pillSegs(segs: Seg[], x: number, y: number, anchor: 'center' | 'left' = 'center'): void {
  // Keep the minimap readable: only short timers (e.g. '0:42') get a label there; verdicts, reasons and
  // hint words ('ward', 'roślina') are already in the top bar / coach and just covered the map.
  // Short hint words ("Fala", "Pomóż Teemo", a champion) are allowed; long reasons stay off the map.
  segs = segs.flatMap((sg): Seg[] => { const m = /\d+:\d{2}/.exec(sg.text); if (m) return [{ ...sg, text: m[0], small: true }]; const t = sg.text.trim(); return t.length > 0 && t.length <= 16 ? [{ ...sg, text: t, small: true }] : []; }).slice(0, 1);
  if (!segs.length) return;
  const fs = Math.max(10, Math.round(W * 0.033));
  const fsSmall = Math.max(9, Math.round(fs * 0.88));
  const fontOf = (sg: Seg) => `${sg.weight ?? 600} ${sg.small ? fsSmall : fs}px ${FONT}`;
  const widths = segs.map((sg) => { ctx.font = fontOf(sg); return ctx.measureText(sg.text).width; });
  const tw = widths.reduce((a, b) => a + b, 0);
  const padX = fs * 0.45, h = fs * 1.45;
  let bx = anchor === 'left' ? x : x - tw / 2 - padX;
  let by = y - h / 2;
  bx = Math.max(2, Math.min(W - tw - 2 * padX - 2, bx));
  by = Math.max(2, Math.min(H - h - 2, by));
  ctx.beginPath();
  ctx.roundRect(bx, by, tw + 2 * padX, h, h / 2);
  ctx.fillStyle = 'rgba(8,14,20,0.78)';
  ctx.fill();
  ctx.textBaseline = 'middle';
  ctx.textAlign = 'left';
  let cx = bx + padX;
  segs.forEach((sg, i) => {
    ctx.font = fontOf(sg);
    ctx.fillStyle = sg.color ?? '#eef3f6';
    ctx.fillText(sg.text, cx, by + h / 2 + 0.5);
    cx += widths[i] ?? 0;
  });
}

function objectiveSegs(o: ObjectiveCall, afterTimer: boolean): Seg[] {
  const v = VERDICT[o.verdict];
  const segs: Seg[] = [];
  if (afterTimer) segs.push({ text: ' · ', color: '#9fb0bb' });
  segs.push({ text: v.word, color: v.color, weight: 700 });
  if (o.reason) segs.push({ text: ` — ${o.reason}`, color: '#c9d4db', weight: 500, small: true });
  return segs;
}

function drawHint(h: MinimapHint, gt: number, now: number): void {
  const left = secondsUntil(h.expiresAt, gt);
  if (left <= 0) return;
  const alpha = Math.min(1, left / 1.5); // fade out over the last 1.5 s
  const x = h.x * W, y = h.y * H;
  switch (h.kind) {
    case 'move':
    case 'objective': {
      if (h.from) arrow(h.from.x * W, h.from.y * H, x, y, alpha);
      if (h.kind === 'objective' || !h.from) {
        ring(x, y, Math.max(6, 0.035 * W), GREEN, alpha, 0.14);
      }
      if (h.label) pill(h.label, x, y + Math.max(6, 0.035 * W) + W * 0.035, `rgb(${GREEN})`);
      break;
    }
    case 'invade': {
      const t = (now - (firstSeen.get(hintKey(h)) ?? now)) / 1000;
      const base = Math.max(10, 0.09 * W);
      let fill = 0.16, rr = base, a = alpha;
      if (t < 2) {
        const p = 0.5 - 0.5 * Math.cos(t * Math.PI * 2 * 1.5); // 3 soft pulses in 2 s
        fill = 0.12 + 0.22 * p;
        rr = base * (1 + 0.08 * p);
        a = alpha * (0.75 + 0.25 * p);
      }
      // Dashed outline keeps the zone distinct from (solid) ghost areas.
      ctx.setLineDash([5, 4]);
      ring(x, y, rr, RED, a, fill);
      ctx.setLineDash([]);
      if (h.label) pill(h.label, x, y - rr - W * 0.035, `rgb(${RED})`);
      break;
    }
    case 'ward': {
      // Ward (yellow) or Scryer's Bloom vision plant (green-teal) diamond.
      const plant = (h.label ?? '').toLowerCase().startsWith('roślin');
      const rgb = plant ? '95,211,176' : '242,209,75';
      const t = (now - (firstSeen.get(hintKey(h)) ?? now)) / 1000;
      const p = t < 2 ? 0.5 - 0.5 * Math.cos(t * Math.PI * 2) : 0; // 1 s pulses for 2 s
      const s = Math.max(5, 0.035 * W) / 2 * (1 + 0.15 * p);
      const diamond = () => {
        ctx.beginPath();
        ctx.moveTo(x, y - s * 1.25); ctx.lineTo(x + s, y);
        ctx.lineTo(x, y + s * 1.25); ctx.lineTo(x - s, y);
        ctx.closePath();
      };
      // Way there (yellow arrow from me) and a steady ring around the spot while it is pinned.
      if (h.from) arrow(h.from.x * W, h.from.y * H, x, y, alpha, rgb);
      ring(x, y, s * 2.1, rgb, alpha * (0.45 + 0.4 * p), 0.1);
      diamond();
      ctx.lineJoin = 'round';
      ctx.lineWidth = 2.5; ctx.strokeStyle = `rgba(0,0,0,${0.5 * alpha})`; ctx.stroke();
      ctx.fillStyle = `rgba(${rgb},${alpha})`; ctx.fill();
      if (h.label) pill(h.label, x + s + 4, y, `rgb(${rgb})`, 'left');
      break;
    }
    case 'avoid': {
      const r = Math.max(8, 0.06 * W);
      ring(x, y, r, RED, alpha * 0.7, 0.12);
      const k = r * 0.42;
      ctx.lineCap = 'round';
      for (const [lw, col] of [[4.5, `rgba(0,0,0,${0.45 * alpha})`], [2.25, `rgba(${RED},${alpha})`]] as const) {
        ctx.beginPath();
        ctx.moveTo(x - k, y - k); ctx.lineTo(x + k, y + k);
        ctx.moveTo(x + k, y - k); ctx.lineTo(x - k, y + k);
        ctx.lineWidth = lw; ctx.strokeStyle = col; ctx.stroke();
      }
      if (h.label) pill(h.label, x, y + r + W * 0.035, `rgb(${RED})`);
      break;
    }
  }
}

function drawFrame(): void {
  const L = Math.max(10, 0.08 * W), i = 1.5;
  ctx.lineWidth = 2;
  ctx.lineCap = 'butt';
  ctx.strokeStyle = 'rgba(230,240,245,0.6)';
  ctx.beginPath();
  ctx.moveTo(i, i + L); ctx.lineTo(i, i); ctx.lineTo(i + L, i);
  ctx.moveTo(W - i - L, i); ctx.lineTo(W - i, i); ctx.lineTo(W - i, i + L);
  ctx.moveTo(W - i, H - i - L); ctx.lineTo(W - i, H - i); ctx.lineTo(W - i - L, H - i);
  ctx.moveTo(i + L, H - i); ctx.lineTo(i, H - i); ctx.lineTo(i, H - i - L);
  ctx.stroke();
}

let lastFrame = performance.now();
function frame(now: number): void {
  const dt = Math.min(0.1, (now - lastFrame) / 1000);
  lastFrame = now;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, W, H);

  const st = state;
  if (st && st.visible) {
    const gt = clock.gameTime();

    // timers (under everything else)
    const obj = st.objective;
    const objKind = obj ? objectiveKind(obj.objective) : null;
    let merged = false;
    for (const t of st.timers) {
      if (t.secondsLeft > 60 || t.secondsLeft <= 0) continue;
      const base: Seg = { text: `${t.label} ${fmtClock(t.secondsLeft)}` };
      if (obj && !merged && objKind === t.kind) {
        merged = true;
        pillSegs([base], t.x * W, t.y * H);
      } else {
        pillSegs([base], t.x * W, t.y * H);
      }
    }
    // Camp / inhibitor respawn countdowns (like Blitz's minimap timers), counting down between pushes.
    const mdt = gt - st.gameTime;
    for (const t of st.mapTimers ?? []) {
      const left = t.secondsLeft - mdt;
      if (left <= 0) continue;
      const color = t.kind === 'inhib' ? (t.ours ? '#ff8f88' : '#3ccf8e') : '#eef3f6';
      pillSegs([{ text: fmtClock(left), color }], t.x * W, t.y * H);
    }
    // An alive objective's standing verdict ('WALCZ' at Herald while the fight was at Dragon) was noise:
    // only a 'go' and only when no fight / help arrow points elsewhere.
    const fightElsewhere = st.hints.some((h) => h.kind === 'move' && isLive(h.expiresAt, gt));
    // No verdict word at all ('IDŹ' / 'WALCZ' said nothing the arrow does not).
    void fightElsewhere;
    for (const h of st.hints) if (isLive(h.expiresAt, gt)) drawHint(h, gt, now);
  }

  // ghosts: ease toward latest target, fade in/out
  const k = 1 - Math.exp(-dt * 6);
  const withArea = new Set([...ghosts.values()].filter(gh => !gh.gone && gh.g.age >= 2).sort((p, q) => p.g.age - q.g.age).slice(0, 3).map(gh => gh.g.champion));
  for (const [name, gh] of ghosts) {
    gh.area = withArea.has(gh.g.champion);
    gh.x += (gh.g.x - gh.x) * k;
    gh.y += (gh.g.y - gh.y) * k;
    gh.r += (gh.g.radius - gh.r) * k;
    const target = gh.gone || !st?.visible ? 0 : 1;
    gh.alpha += (target - gh.alpha) * (1 - Math.exp(-dt * 10));
    if (gh.gone && gh.alpha < 0.01) { ghosts.delete(name); continue; }
    drawGhost(gh);
  }

  if (st && st.visible) {
    if (st.showFrame) drawFrame();
    if (st.recording) {
      const r = Math.max(3, 0.013 * W);
      ctx.beginPath();
      ctx.arc(W - r - 6, r + 6, r, 0, Math.PI * 2);
      ctx.fillStyle = `rgb(${RED})`;
      ctx.fill();
      ctx.lineWidth = 1;
      ctx.strokeStyle = OUTLINE;
      ctx.stroke();
    }
  }
  requestAnimationFrame(frame);
}
requestAnimationFrame(frame);
