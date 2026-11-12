// Minimap layer: fog-of-war ghosts, coach hints, objective timers, frame, rec dot.
// Canvas 2D, DPR-aware, 60 fps with ghost positions eased between ~10 Hz pushes.
import type { HudState, MinimapGhost, MinimapHint, ObjectiveCall } from '../../shared/intel';
import { subscribe } from './common';
import { fmtClock, isLive, objectiveKind, secondsUntil, VERDICT } from './types';

const GREEN = '60,207,142';
const RED = '255,90,79';
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
function drawGhost(gh: Ghost, fed: boolean): void {
  const { g } = gh;
  const cx = gh.x * W, cy = gh.y * H;
  const iconD = Math.max(14, 0.075 * W);
  // Long in fog the area covers half the map and says little: cap its size, fade it from 30 s, gone at 60 s.
  const r = Math.max(iconD * 0.5 + 3, Math.min(gh.r, 0.28) * W);
  const conf = Math.max(0, Math.min(1, g.confidence));
  const zf = g.age <= 30 ? 1 : g.age >= 60 ? 0 : 1 - (g.age - 30) / 30;
  if (zf <= 0) return;
  const a = gh.alpha * zf;

  ctx.beginPath();
  ctx.arc(cx, cy, r, 0, Math.PI * 2);
  ctx.fillStyle = `rgba(${RED},${(fed ? 0.28 : 0.11 + 0.07 * conf) * a})`;
  ctx.fill();
  ctx.lineWidth = fed ? 2 : 1.25;
  ctx.strokeStyle = `rgba(${RED},${(fed ? 1 : 0.8) * a})`;
  ctx.stroke();

  // Portrait: ~55% opacity, fades from 30 s to 45 s since last seen.
  const pf = g.age <= 30 ? 1 : g.age >= 45 ? 0 : 1 - (g.age - 30) / 15;
  const img = pf > 0 ? image(g.iconUrl) : null;
  if (img) {
    ctx.save();
    ctx.globalAlpha = 0.55 * pf * a;
    ctx.beginPath();
    ctx.arc(cx, cy, iconD / 2, 0, Math.PI * 2);
    ctx.clip();
    // ddragon square icons have a thin border; crop ~8% to hide it.
    const c = img.naturalWidth * 0.08;
    ctx.drawImage(img, c, c, img.naturalWidth - 2 * c, img.naturalHeight - 2 * c,
      cx - iconD / 2, cy - iconD / 2, iconD, iconD);
    ctx.restore();
  }

  if (fed) {
    // Tiny "FED" tag on the top edge of the area.
    const fs = Math.max(8, Math.round(W * 0.026));
    ctx.font = `800 ${fs}px ${FONT}`;
    const tw = ctx.measureText('FED').width;
    const pw = tw + fs * 0.7, ph = fs * 1.3;
    const tx = Math.max(1, Math.min(W - pw - 1, cx - pw / 2));
    const ty = Math.max(1, cy - r - ph * 0.55);
    ctx.globalAlpha = a;
    ctx.beginPath();
    ctx.roundRect(tx, ty, pw, ph, 3);
    ctx.fillStyle = `rgb(${RED})`;
    ctx.fill();
    ctx.fillStyle = '#1a0806';
    ctx.textBaseline = 'middle';
    ctx.textAlign = 'center';
    ctx.fillText('FED', tx + pw / 2, ty + ph / 2 + 0.5);
    ctx.globalAlpha = 1;
  }
}

function arrow(fx: number, fy: number, tx: number, ty: number, alpha: number): void {
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
  path(); ctx.lineWidth = 2.5; ctx.strokeStyle = `rgba(${GREEN},${alpha})`; ctx.stroke();
  tri(); ctx.fillStyle = `rgba(${GREEN},${alpha})`; ctx.fill();
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
      if (p > 0) ring(x, y, s * 2.1, rgb, alpha * 0.6 * p, 0.12);
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
        pillSegs([base, ...objectiveSegs(obj, true)], t.x * W, t.y * H);
      } else {
        pillSegs([base], t.x * W, t.y * H);
      }
    }
    if (obj && !merged) pillSegs(objectiveSegs(obj, false), obj.x * W, obj.y * H + Math.max(14, 0.07 * W));
    for (const h of st.hints) if (isLive(h.expiresAt, gt)) drawHint(h, gt, now);
  }

  // ghosts: ease toward latest target, fade in/out
  const fedSet = new Set((st?.fed ?? []).map((f) => f.toLowerCase()));
  const k = 1 - Math.exp(-dt * 6);
  for (const [name, gh] of ghosts) {
    gh.x += (gh.g.x - gh.x) * k;
    gh.y += (gh.g.y - gh.y) * k;
    gh.r += (gh.g.radius - gh.r) * k;
    const target = gh.gone || !st?.visible ? 0 : 1;
    gh.alpha += (target - gh.alpha) * (1 - Math.exp(-dt * 10));
    if (gh.gone && gh.alpha < 0.01) { ghosts.delete(name); continue; }
    drawGhost(gh, fedSet.has(name.toLowerCase()));
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
