// Extract the in-game minimap champion icons (hud/<id>_circle_0) straight from the local League
// install (WAD archives), so the detector never depends on raw.communitydragon.org being up.
// Writes <APPDATA>/Rift Coach/ddragon/icons/cd-<id>-0.png — the cache the app reads first.
//
// usage: node scripts/extract-minimap-icons.mjs [leagueDir="C:/Riot Games/League of Legends"] [champion ...]
import { readFileSync, writeFileSync, readdirSync, existsSync, mkdirSync } from 'node:fs';
import path from 'node:path';
import zlib from 'node:zlib';

const args = process.argv.slice(2);
const leagueDir = args[0] && existsSync(args[0]) ? args.shift() : 'C:/Riot Games/League of Legends';
const champDir = path.join(leagueDir, 'Game', 'DATA', 'FINAL', 'Champions');
const outDir = process.env.OUT || path.join(process.env.APPDATA, 'Rift Coach', 'ddragon', 'icons');
mkdirSync(outDir, { recursive: true });

// ---- xxHash64 (seed 0) of the lower-case asset path: the WAD's entry key
const P1 = 11400714785074694791n, P2 = 14029467366897019727n, P3 = 1609587929392839161n, P4 = 9650029242287828579n, P5 = 2870177450012600261n;
const M = (1n << 64n) - 1n;
const rotl = (x, r) => ((x << BigInt(r)) | (x >> BigInt(64 - r))) & M;
const round = (acc, v) => (rotl((acc + v * P2) & M, 31) * P1) & M;
const merge = (acc, v) => ((((acc ^ round(0n, v)) * P1) & M) + P4) & M;
function xxh64(buf) {
  let i = 0, h;
  const n = buf.length;
  const u64 = (o) => buf.readBigUInt64LE(o);
  if (n >= 32) {
    let v1 = (P1 + P2) & M, v2 = P2, v3 = 0n, v4 = (0n - P1) & M;
    for (; i + 32 <= n; i += 32) { v1 = round(v1, u64(i)); v2 = round(v2, u64(i + 8)); v3 = round(v3, u64(i + 16)); v4 = round(v4, u64(i + 24)); }
    h = (rotl(v1, 1) + rotl(v2, 7) + rotl(v3, 12) + rotl(v4, 18)) & M;
    h = merge(h, v1); h = merge(h, v2); h = merge(h, v3); h = merge(h, v4);
  } else h = P5;
  h = (h + BigInt(n)) & M;
  for (; i + 8 <= n; i += 8) h = ((rotl(h ^ round(0n, u64(i)), 27) * P1) & M) + P4 & M;
  for (; i + 4 <= n; i += 4) h = ((rotl(h ^ ((BigInt(buf.readUInt32LE(i)) * P1) & M), 23) * P2) & M) + P3 & M;
  for (; i < n; i++) h = (rotl(h ^ ((BigInt(buf[i]) * P5) & M), 11) * P1) & M;
  h ^= h >> 33n; h = (h * P2) & M; h ^= h >> 29n; h = (h * P3) & M; h ^= h >> 32n;
  return h;
}

// ---- WAD v3: header 272 bytes, then 32-byte entries
function readWadEntry(file, wantHashes) {
  const b = readFileSync(file);
  if (b.toString('latin1', 0, 2) !== 'RW' || b[2] !== 3) throw new Error(`not a WAD v3: ${file}`);
  const count = b.readUInt32LE(268);
  for (let k = 0; k < count; k++) {
    const o = 272 + k * 32;
    const hash = b.readBigUInt64LE(o);
    if (!wantHashes.has(hash)) continue;
    const off = b.readUInt32LE(o + 8), csize = b.readUInt32LE(o + 12), size = b.readUInt32LE(o + 16), type = b[o + 20] & 0x0f;
    const raw = b.subarray(off, off + csize);
    let data;
    if (type === 0) data = raw;
    else if (type === 1) data = zlib.gunzipSync(raw);
    else if (type === 3 || type === 4) data = zlib.zstdDecompressSync(raw);
    else continue;
    if (data.length !== size && type !== 4) continue;
    return { path: wantHashes.get(hash), data };
  }
  return null;
}

// ---- TEX -> RGBA (BGRA8 / DXT1 / DXT5, largest mip is stored last)
function rgb565(c) { return [((c >> 11) & 31) * 255 / 31 | 0, ((c >> 5) & 63) * 255 / 63 | 0, (c & 31) * 255 / 31 | 0]; }
function decodeDxt(src, w, h, dxt5) {
  const out = Buffer.alloc(w * h * 4);
  const bs = dxt5 ? 16 : 8;
  let p = 0;
  for (let by = 0; by < h; by += 4) for (let bx = 0; bx < w; bx += 4, p += bs) {
    let alpha = null;
    if (dxt5) {
      const a0 = src[p], a1 = src[p + 1];
      const pal = [a0, a1];
      if (a0 > a1) for (let i = 1; i < 7; i++) pal.push(((7 - i) * a0 + i * a1) / 7 | 0);
      else { for (let i = 1; i < 5; i++) pal.push(((5 - i) * a0 + i * a1) / 5 | 0); pal.push(0, 255); }
      let bits = 0n;
      for (let i = 0; i < 6; i++) bits |= BigInt(src[p + 2 + i]) << BigInt(8 * i);
      alpha = [];
      for (let i = 0; i < 16; i++) alpha.push(pal[Number((bits >> BigInt(3 * i)) & 7n)]);
    }
    const q = p + (dxt5 ? 8 : 0);
    const c0 = src.readUInt16LE(q), c1 = src.readUInt16LE(q + 2);
    const a = rgb565(c0), bcol = rgb565(c1);
    const cols = [a, bcol];
    if (c0 > c1 || dxt5) { cols.push(a.map((v, i) => (2 * v + bcol[i]) / 3 | 0), a.map((v, i) => (v + 2 * bcol[i]) / 3 | 0)); }
    else { cols.push(a.map((v, i) => (v + bcol[i]) / 2 | 0), [0, 0, 0]); }
    const idx = src.readUInt32LE(q + 4);
    for (let i = 0; i < 16; i++) {
      const x = bx + (i & 3), y = by + (i >> 2);
      if (x >= w || y >= h) continue;
      const ci = (idx >> (2 * i)) & 3, c = cols[ci], o = (y * w + x) * 4;
      out[o] = c[0]; out[o + 1] = c[1]; out[o + 2] = c[2];
      out[o + 3] = dxt5 ? alpha[i] : (c0 <= c1 && ci === 3 ? 0 : 255);
    }
  }
  return out;
}
function decodeTex(buf) {
  if (buf.toString('latin1', 0, 4) !== 'TEX\0') throw new Error('not TEX');
  const w = buf.readUInt16LE(4), h = buf.readUInt16LE(6), fmt = buf[9];
  const body = buf.subarray(12);
  if (fmt === 20) { // BGRA8
    const n = w * h * 4, src = body.subarray(body.length - n), out = Buffer.alloc(n);
    for (let i = 0; i < n; i += 4) { out[i] = src[i + 2]; out[i + 1] = src[i + 1]; out[i + 2] = src[i]; out[i + 3] = src[i + 3]; }
    return { w, h, rgba: out };
  }
  if (fmt === 10 || fmt === 12) {
    const n = Math.ceil(w / 4) * Math.ceil(h / 4) * (fmt === 12 ? 16 : 8);
    return { w, h, rgba: decodeDxt(body.subarray(body.length - n), w, h, fmt === 12) };
  }
  throw new Error(`TEX format ${fmt} not supported`);
}

// ---- minimal PNG writer
const CRC = new Int32Array(256).map((_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c; });
const crc32 = (b) => { let c = -1; for (const x of b) c = CRC[(c ^ x) & 255] ^ (c >>> 8); return (c ^ -1) >>> 0; };
function png(w, h, rgba) {
  const chunk = (type, data) => { const len = Buffer.alloc(4); len.writeUInt32BE(data.length); const td = Buffer.concat([Buffer.from(type), data]); const c = Buffer.alloc(4); c.writeUInt32BE(crc32(td)); return Buffer.concat([len, td, c]); };
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(w, 0); ihdr.writeUInt32BE(h, 4); ihdr[8] = 8; ihdr[9] = 6;
  const raw = Buffer.alloc((w * 4 + 1) * h);
  for (let y = 0; y < h; y++) rgba.copy(raw, y * (w * 4 + 1) + 1, y * w * 4, (y + 1) * w * 4);
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', ihdr), chunk('IDAT', zlib.deflateSync(raw)), chunk('IEND', Buffer.alloc(0))]);
}

// ---- all champions (or the ones given)
const wads = readdirSync(champDir).filter((f) => /^[A-Za-z]+\.wad\.client$/.test(f));
const want = args.length ? new Set(args.map((a) => a.toLowerCase())) : null;
let ok = 0, miss = [];
for (const f of wads) {
  const id = f.split('.')[0].toLowerCase();
  if (want && !want.has(id)) continue;
  // Some champions keep their old internal names in the game files.
  const ALIAS = { anivia: 'cryophoenix', blitzcrank: 'steamgolem', chogath: 'greenterror', orianna: 'oriana', rammus: 'armordillo', shaco: 'jester', xinzhao: 'xenzhao', zilean: 'chronokeeper' };
  const names = [id, ALIAS[id]].filter(Boolean);
  const candidates = [];
  for (const folder of names) for (const file of names) for (const ext of ['tex', 'dds', 'png']) {
    candidates.push(`assets/characters/${folder}/hud/${file}_circle_0.${ext}`, `assets/characters/${folder}/hud/${file}_circle.${ext}`);
  }
  const hashes = new Map(candidates.map((p) => [xxh64(Buffer.from(p)), p]));
  try {
    const e = readWadEntry(path.join(champDir, f), hashes);
    if (!e) { miss.push(id); continue; }
    let img;
    if (e.path.endsWith('.png')) { writeFileSync(path.join(outDir, `cd-${id}-0.png`), e.data); ok++; continue; }
    if (e.path.endsWith('.tex')) img = decodeTex(e.data);
    else throw new Error('dds not supported yet');
    writeFileSync(path.join(outDir, `cd-${id}-0.png`), png(img.w, img.h, img.rgba));
    ok++;
  } catch (err) {
    miss.push(`${id} (${err.message})`);
  }
}
console.log(`extracted ${ok} minimap icons to ${outDir}`);
if (miss.length) console.log(`not found: ${miss.join(', ')}`);
