import { inflateSync } from 'node:zlib';

/** Decoded image, always RGBA 8-bit. */
export interface Rgba { width: number; height: number; data: Uint8Array }

/**
 * Minimal PNG decoder (non-interlaced, 8-bit gray/RGB/RGBA/gray+alpha/palette).
 * Enough for canvas-encoded minimap crops and Data Dragon icons; pure JS so
 * vision code and tests run in plain Node.
 */
export function decodePng(buf: Uint8Array): Rgba {
  const dv = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
  if (dv.getUint32(0) !== 0x89504e47) throw new Error('not a PNG');
  let pos = 8, width = 0, height = 0, depth = 0, colorType = 0, interlace = 0;
  let palette: Uint8Array | null = null, trns: Uint8Array | null = null;
  const idat: Uint8Array[] = [];
  while (pos < buf.length) {
    const len = dv.getUint32(pos), type = String.fromCharCode(buf[pos + 4]!, buf[pos + 5]!, buf[pos + 6]!, buf[pos + 7]!);
    const data = buf.subarray(pos + 8, pos + 8 + len);
    if (type === 'IHDR') {
      width = dv.getUint32(pos + 8); height = dv.getUint32(pos + 12);
      depth = data[8]!; colorType = data[9]!; interlace = data[12]!;
    } else if (type === 'PLTE') palette = data;
    else if (type === 'tRNS') trns = data;
    else if (type === 'IDAT') idat.push(data);
    else if (type === 'IEND') break;
    pos += 12 + len;
  }
  if (depth !== 8 || interlace) throw new Error(`unsupported PNG (depth ${depth}, interlace ${interlace})`);
  const channels = ({ 0: 1, 2: 3, 3: 1, 4: 2, 6: 4 } as Record<number, number>)[colorType];
  if (!channels) throw new Error(`unsupported color type ${colorType}`);
  const raw = inflateSync(Buffer.concat(idat));
  const stride = width * channels;
  const pix = new Uint8Array(stride * height);
  let prev = new Uint8Array(stride);
  for (let y = 0; y < height; y++) {
    const filter = raw[y * (stride + 1)]!;
    const line = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    const out = pix.subarray(y * stride, (y + 1) * stride);
    for (let i = 0; i < stride; i++) {
      const a = i >= channels ? out[i - channels]! : 0, b = prev[i]!, c = i >= channels ? prev[i - channels]! : 0;
      let v = line[i]!;
      switch (filter) {
        case 1: v += a; break;
        case 2: v += b; break;
        case 3: v += (a + b) >> 1; break;
        case 4: { const p = a + b - c, pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c); v += pa <= pb && pa <= pc ? a : pb <= pc ? b : c; break; }
      }
      out[i] = v & 255;
    }
    prev = out;
  }
  const rgba = new Uint8Array(width * height * 4);
  for (let i = 0, j = 0; i < width * height; i++, j += channels) {
    const o = i * 4;
    switch (colorType) {
      case 0: rgba[o] = rgba[o + 1] = rgba[o + 2] = pix[j]!; rgba[o + 3] = 255; break;
      case 2: rgba[o] = pix[j]!; rgba[o + 1] = pix[j + 1]!; rgba[o + 2] = pix[j + 2]!; rgba[o + 3] = 255; break;
      case 3: { const k = pix[j]!; rgba[o] = palette![k * 3]!; rgba[o + 1] = palette![k * 3 + 1]!; rgba[o + 2] = palette![k * 3 + 2]!; rgba[o + 3] = trns && k < trns.length ? trns[k]! : 255; break; }
      case 4: rgba[o] = rgba[o + 1] = rgba[o + 2] = pix[j]!; rgba[o + 3] = pix[j + 1]!; break;
      case 6: rgba[o] = pix[j]!; rgba[o + 1] = pix[j + 1]!; rgba[o + 2] = pix[j + 2]!; rgba[o + 3] = pix[j + 3]!; break;
    }
  }
  return { width, height, data: rgba };
}

/** Bilinear resize of an RGBA image. */
export function resize(img: Rgba, w: number, h: number): Rgba {
  const out = new Uint8Array(w * h * 4);
  const sx = img.width / w, sy = img.height / h;
  for (let y = 0; y < h; y++) {
    const fy = Math.max(0, (y + 0.5) * sy - 0.5), y0 = Math.floor(fy), y1 = Math.min(img.height - 1, y0 + 1), ty = fy - y0;
    for (let x = 0; x < w; x++) {
      const fx = Math.max(0, (x + 0.5) * sx - 0.5), x0 = Math.floor(fx), x1 = Math.min(img.width - 1, x0 + 1), tx = fx - x0;
      for (let c = 0; c < 4; c++) {
        const p00 = img.data[(y0 * img.width + x0) * 4 + c]!, p01 = img.data[(y0 * img.width + x1) * 4 + c]!;
        const p10 = img.data[(y1 * img.width + x0) * 4 + c]!, p11 = img.data[(y1 * img.width + x1) * 4 + c]!;
        out[(y * w + x) * 4 + c] = Math.round((p00 * (1 - tx) + p01 * tx) * (1 - ty) + (p10 * (1 - tx) + p11 * tx) * ty);
      }
    }
  }
  return { width: w, height: h, data: out };
}
