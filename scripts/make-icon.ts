// Generates assets/icon.png (256 px) and assets/icon.ico (PNG-in-ICO, 256/64/48/32/16).
// The mark: a dark rounded square with a minimap-like frame, the mid-lane diagonal,
// and a translucent "fog prediction" circle with a solid dot — what the app does.
import { writeFileSync, mkdirSync } from 'node:fs';
import { encodePng } from '../src/main/vision/pngEncode';
import { resize, type Rgba } from '../src/main/vision/png';

function draw(S: number): Rgba {
  const img: Rgba = { width: S, height: S, data: new Uint8Array(S * S * 4) };
  const put = (x: number, y: number, r: number, g: number, b: number, a: number) => {
    if (x < 0 || y < 0 || x >= S || y >= S) return;
    const i = (y * S + x) * 4, A = img.data[i + 3]! / 255, na = a + A * (1 - a);
    if (na <= 0) return;
    img.data[i] = (r * a + img.data[i]! * A * (1 - a)) / na;
    img.data[i + 1] = (g * a + img.data[i + 1]! * A * (1 - a)) / na;
    img.data[i + 2] = (b * a + img.data[i + 2]! * A * (1 - a)) / na;
    img.data[i + 3] = na * 255;
  };
  const rad = S * 0.2, k = S / 256;
  for (let y = 0; y < S; y++) for (let x = 0; x < S; x++) {
    // Rounded square background, subtle vertical shade.
    const dx = Math.max(rad - x, 0, x - (S - 1 - rad)), dy = Math.max(rad - y, 0, y - (S - 1 - rad));
    const d = Math.hypot(dx, dy);
    if (d > rad) continue;
    const a = Math.min(1, rad - d);
    const shade = 1 - y / S * 0.25;
    put(x, y, 14 * shade, 26 * shade, 32 * shade, a);
    // Minimap frame (inset square outline).
    const m = 44 * k, t = 6 * k;
    const inFrame = x > m && x < S - m && y > m && y < S - m;
    const onFrame = inFrame && (x < m + t || x > S - m - t || y < m + t || y > S - m - t);
    if (onFrame) put(x, y, 60, 207, 180, 0.95);
    // Mid-lane diagonal (bottom-left to top-right).
    if (inFrame && Math.abs((x - m) - (S - m - y)) < 7 * k) put(x, y, 60, 207, 180, 0.35);
    // Fog prediction circle (top-right, translucent red) + outline.
    const cx = 160 * k, cy = 98 * k, R = 40 * k, dc = Math.hypot(x - cx, y - cy);
    if (dc < R) put(x, y, 255, 90, 79, 0.28);
    if (Math.abs(dc - R) < 3 * k) put(x, y, 255, 110, 95, 0.95);
    // Player dot (bottom-left, teal) and enemy dot (circle centre).
    if (Math.hypot(x - 92 * k, y - 166 * k) < 13 * k) put(x, y, 60, 207, 180, 1);
    if (dc < 9 * k) put(x, y, 255, 110, 95, 1);
  }
  return img;
}

mkdirSync('assets', { recursive: true });
const big = draw(256);
writeFileSync('assets/icon.png', encodePng(big));
const sizes = [256, 64, 48, 32, 16];
const pngs = sizes.map(s => encodePng(s === 256 ? big : resize(draw(512), s, s)));
// ICO container with PNG entries.
const header = Buffer.alloc(6 + 16 * sizes.length);
header.writeUInt16LE(0, 0); header.writeUInt16LE(1, 2); header.writeUInt16LE(sizes.length, 4);
let offset = header.length;
sizes.forEach((s, i) => {
  const e = 6 + i * 16;
  header[e] = s === 256 ? 0 : s; header[e + 1] = s === 256 ? 0 : s;
  header.writeUInt16LE(1, e + 4); header.writeUInt16LE(32, e + 6);
  header.writeUInt32LE(pngs[i]!.length, e + 8); header.writeUInt32LE(offset, e + 12);
  offset += pngs[i]!.length;
});
writeFileSync('assets/icon.ico', Buffer.concat([header, ...pngs]));
console.log('icon written');
