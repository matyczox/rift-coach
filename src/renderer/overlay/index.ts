// Transparent, click-through layer placed exactly over the game's minimap.
// M0: only an alignment outline and a small recording dot.
import type { OverlayState } from '../../shared/types';

const canvas = document.getElementById('c') as HTMLCanvasElement;
const ctx = canvas.getContext('2d')!;
let state: OverlayState = { visible: false, showFrame: false, recording: false };

function draw() {
  const dpr = window.devicePixelRatio || 1;
  const w = window.innerWidth, h = window.innerHeight;
  if (canvas.width !== Math.round(w * dpr)) { canvas.width = Math.round(w * dpr); canvas.height = Math.round(h * dpr); }
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  if (!state.visible) return;
  if (state.showFrame) {
    ctx.strokeStyle = 'rgba(60, 207, 180, 0.9)';
    ctx.lineWidth = 2;
    const c = 18;
    for (const [x, y, dx, dy] of [[1, 1, 1, 1], [w - 1, 1, -1, 1], [1, h - 1, 1, -1], [w - 1, h - 1, -1, -1]] as const) {
      ctx.beginPath(); ctx.moveTo(x + dx * c, y); ctx.lineTo(x, y); ctx.lineTo(x, y + dy * c); ctx.stroke();
    }
  }
  if (state.recording) {
    ctx.fillStyle = 'rgba(226, 75, 74, 0.85)';
    ctx.beginPath(); ctx.arc(w - 7, 7, 3.5, 0, Math.PI * 2); ctx.fill();
  }
}

window.rc.onOverlay(s => { state = s; draw(); });
window.addEventListener('resize', draw);
draw();
