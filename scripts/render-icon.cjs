// Renders assets/icon.svg with Electron (Chromium rasteriser: anti-aliasing, gradients) into
// assets/icon.png (256), assets/icon-512.png and assets/icon.ico (PNG-in-ICO: 256/128/64/48/32/24/16).
// usage: npx electron scripts/render-icon.cjs
const { app, BrowserWindow, nativeImage } = require('electron');
const { readFileSync, writeFileSync } = require('node:fs');
const path = require('node:path');

const ROOT = path.join(__dirname, '..');
const SVG = readFileSync(path.join(ROOT, 'assets', 'icon.svg'), 'utf8');

async function render(size) {
  const win = new BrowserWindow({ width: size, height: size, show: false, frame: false, transparent: true, useContentSize: true, webPreferences: { offscreen: true } });
  const html = `<html><body style="margin:0;background:transparent">${SVG.replace('width="256" height="256"', `width="${size}" height="${size}"`)}</body></html>`;
  await win.loadURL('data:text/html;charset=utf-8,' + encodeURIComponent(html));
  await new Promise(r => setTimeout(r, 300));
  const img = await win.webContents.capturePage({ x: 0, y: 0, width: size, height: size });
  win.destroy();
  return img.getSize().width === size ? img : img.resize({ width: size, height: size, quality: 'best' });
}

function ico(pngs) {
  const n = pngs.length, head = Buffer.alloc(6 + 16 * n);
  head.writeUInt16LE(0, 0); head.writeUInt16LE(1, 2); head.writeUInt16LE(n, 4);
  let off = head.length;
  pngs.forEach(([s, b], i) => {
    const e = 6 + 16 * i;
    head.writeUInt8(s >= 256 ? 0 : s, e); head.writeUInt8(s >= 256 ? 0 : s, e + 1);
    head.writeUInt16LE(1, e + 4); head.writeUInt16LE(32, e + 6);
    head.writeUInt32LE(b.length, e + 8); head.writeUInt32LE(off, e + 12);
    off += b.length;
  });
  return Buffer.concat([head, ...pngs.map(p => p[1])]);
}

app.disableHardwareAcceleration();
app.whenReady().then(async () => {
  const big = await render(1024);
  const at = s => nativeImage.createFromBuffer(big.toPNG()).resize({ width: s, height: s, quality: 'best' }).toPNG();
  writeFileSync(path.join(ROOT, 'assets', 'icon-512.png'), at(512));
  writeFileSync(path.join(ROOT, 'assets', 'icon.png'), at(256));
  writeFileSync(path.join(ROOT, 'assets', 'icon.ico'), ico([256, 128, 64, 48, 32, 24, 16].map(s => [s, at(s)])));
  console.log('icon written');
  app.quit();
});
