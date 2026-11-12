//! RGBA image type, PNG decode/encode (via the `png` crate) and the bilinear resize used
//! everywhere in the vision code (port of `src/main/vision/png.ts` / `pngEncode.ts`).

use std::path::Path;

/// Decoded image, always RGBA 8-bit, row-major.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rgba {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

#[derive(Debug)]
pub enum ImageError {
    Io(std::io::Error),
    Png(String),
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImageError::Io(e) => write!(f, "io: {e}"),
            ImageError::Png(e) => write!(f, "png: {e}"),
        }
    }
}
impl std::error::Error for ImageError {}
impl From<std::io::Error> for ImageError {
    fn from(e: std::io::Error) -> Self {
        ImageError::Io(e)
    }
}

impl Rgba {
    pub fn new(width: usize, height: usize) -> Rgba {
        Rgba { width, height, data: vec![0; width * height * 4] }
    }

    pub fn from_raw(width: usize, height: usize, data: Vec<u8>) -> Rgba {
        assert_eq!(data.len(), width * height * 4, "Rgba::from_raw: wrong buffer size");
        Rgba { width, height, data }
    }

    /// Copy of the sub-rectangle (clamped to the image).
    pub fn crop(&self, x: usize, y: usize, w: usize, h: usize) -> Rgba {
        let x = x.min(self.width);
        let y = y.min(self.height);
        let w = w.min(self.width - x);
        let h = h.min(self.height - y);
        let mut out = Vec::with_capacity(w * h * 4);
        for yy in y..y + h {
            let o = (yy * self.width + x) * 4;
            out.extend_from_slice(&self.data[o..o + w * 4]);
        }
        Rgba { width: w, height: h, data: out }
    }

    #[inline]
    pub fn px(&self, x: usize, y: usize) -> [u8; 4] {
        let o = (y * self.width + x) * 4;
        [self.data[o], self.data[o + 1], self.data[o + 2], self.data[o + 3]]
    }
}

/// Decode a PNG (any bit depth / colour type the `png` crate supports) into RGBA8.
pub fn decode_png(buf: &[u8]) -> Result<Rgba, ImageError> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(buf));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().map_err(|e| ImageError::Png(e.to_string()))?;
    let mut raw = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut raw).map_err(|e| ImageError::Png(e.to_string()))?;
    let (w, h) = (info.width as usize, info.height as usize);
    raw.truncate(info.buffer_size());
    let n = w * h;
    let data = match info.color_type {
        png::ColorType::Rgba => raw,
        png::ColorType::Rgb => {
            let mut d = Vec::with_capacity(n * 4);
            for p in raw.chunks_exact(3) {
                d.extend_from_slice(&[p[0], p[1], p[2], 255]);
            }
            d
        }
        png::ColorType::Grayscale => {
            let mut d = Vec::with_capacity(n * 4);
            for &g in &raw {
                d.extend_from_slice(&[g, g, g, 255]);
            }
            d
        }
        png::ColorType::GrayscaleAlpha => {
            let mut d = Vec::with_capacity(n * 4);
            for p in raw.chunks_exact(2) {
                d.extend_from_slice(&[p[0], p[0], p[0], p[1]]);
            }
            d
        }
        png::ColorType::Indexed => return Err(ImageError::Png("indexed PNG not expanded".into())),
    };
    if data.len() != n * 4 {
        return Err(ImageError::Png(format!("unexpected buffer size {} for {w}x{h}", data.len())));
    }
    Ok(Rgba { width: w, height: h, data })
}

pub fn load_png(path: impl AsRef<Path>) -> Result<Rgba, ImageError> {
    decode_png(&std::fs::read(path)?)
}

/// Encode RGBA as PNG (debug renders and test fixtures).
pub fn encode_png(img: &Rgba) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, img.width as u32, img.height as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().expect("png header");
        w.write_image_data(&img.data).expect("png data");
    }
    out
}

/// JS `Math.round` for the value ranges used here (half rounds toward +infinity).
#[inline]
pub(crate) fn js_round(v: f64) -> f64 {
    (v + 0.5).floor()
}

/// Bilinear resize of an RGBA image (same pixel-centre convention and rounding as png.ts).
pub fn resize(img: &Rgba, w: usize, h: usize) -> Rgba {
    let mut out = vec![0u8; w * h * 4];
    let sx = img.width as f64 / w as f64;
    let sy = img.height as f64 / h as f64;
    let iw = img.width;
    // Per-column source indices and weights are the same for every row.
    let cols: Vec<(usize, usize, f64)> = (0..w)
        .map(|x| {
            let fx = (((x as f64) + 0.5) * sx - 0.5).max(0.0);
            let x0 = fx.floor() as usize;
            let x1 = (x0 + 1).min(iw - 1);
            (x0, x1, fx - x0 as f64)
        })
        .collect();
    let d = &img.data;
    for y in 0..h {
        let fy = (((y as f64) + 0.5) * sy - 0.5).max(0.0);
        let y0 = fy.floor() as usize;
        let y1 = (y0 + 1).min(img.height - 1);
        let ty = fy - y0 as f64;
        for (x, &(x0, x1, tx)) in cols.iter().enumerate() {
            for c in 0..4 {
                let p00 = d[(y0 * iw + x0) * 4 + c] as f64;
                let p01 = d[(y0 * iw + x1) * 4 + c] as f64;
                let p10 = d[(y1 * iw + x0) * 4 + c] as f64;
                let p11 = d[(y1 * iw + x1) * 4 + c] as f64;
                let v = (p00 * (1.0 - tx) + p01 * tx) * (1.0 - ty) + (p10 * (1.0 - tx) + p11 * tx) * ty;
                out[(y * w + x) * 4 + c] = js_round(v) as u8;
            }
        }
    }
    Rgba { width: w, height: h, data: out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_roundtrip_and_resize() {
        let mut img = Rgba::new(5, 3);
        for (i, v) in img.data.iter_mut().enumerate() {
            *v = (i * 7 % 256) as u8;
        }
        let back = decode_png(&encode_png(&img)).unwrap();
        assert_eq!(back, img);
        let same = resize(&img, 5, 3);
        assert_eq!(same, img);
        let big = resize(&img, 10, 6);
        assert_eq!((big.width, big.height), (10, 6));
    }
}
