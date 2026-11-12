//! Clean reference minimap texture (no icons, no fog, no camera box) — port of
//! `src/main/vision/background.ts`. The shipped asset (`assets/minimap-reference.png`) is
//! embedded in the crate; `build_reference` rebuilds it from fog-free frames.

use crate::image::{decode_png, js_round, resize, ImageError, Rgba};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

/// Native size the reference asset is stored at.
pub const REFERENCE_SIZE: usize = 341;
pub const REFERENCE_FILE: &str = "minimap-reference.png";

/// The repo asset, embedded at compile time.
pub static REFERENCE_PNG: &[u8] = include_bytes!("../data/minimap-reference.png");

fn is_transient(r: u8, g: u8, b: u8) -> bool {
    let mn = r.min(g).min(b);
    if mn > 190 {
        return true; // camera box / white text
    }
    if r > 150 && r as i32 - (g.max(b) as i32) > 70 {
        return true; // red team icons, pings
    }
    false
}

/// Per-pixel robust median of `frames` (all the same size), ignoring transient pixels when a
/// pixel has enough clean samples; ranking by luminance, averaging the middle ~20%.
pub fn build_reference(frames: &[Rgba]) -> Rgba {
    let f0 = frames.first().expect("build_reference: no frames");
    let (w, h) = (f0.width, f0.height);
    let n = frames.len();
    let mut out = vec![0u8; w * h * 4];
    let lum_of = |d: &[u8], o: usize| 0.299 * d[o] as f64 + 0.587 * d[o + 1] as f64 + 0.114 * d[o + 2] as f64;
    let mut idx: Vec<(usize, f32)> = Vec::with_capacity(n);
    for p in 0..w * h {
        let o = p * 4;
        idx.clear();
        for (i, f) in frames.iter().enumerate() {
            let d = &f.data;
            if !is_transient(d[o], d[o + 1], d[o + 2]) {
                idx.push((i, lum_of(d, o) as f32));
            }
        }
        if (idx.len() as f64) < n as f64 * 0.2 {
            idx.clear();
            for (i, f) in frames.iter().enumerate() {
                idx.push((i, lum_of(&f.data, o) as f32));
            }
        }
        let k = idx.len();
        idx.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        let lo = (k as f64 * 0.4).floor() as usize;
        let hi = (lo + 1).max((k as f64 * 0.6).ceil() as usize);
        let (mut r, mut g, mut b) = (0f64, 0f64, 0f64);
        for &(i, _) in &idx[lo..hi.min(k)] {
            let d = &frames[i].data;
            r += d[o] as f64;
            g += d[o + 1] as f64;
            b += d[o + 2] as f64;
        }
        let c = (hi - lo) as f64;
        out[o] = js_round(r / c) as u8;
        out[o + 1] = js_round(g / c) as u8;
        out[o + 2] = js_round(b / c) as u8;
        out[o + 3] = 255;
    }
    let mut img = Rgba { width: w, height: h, data: out };
    inpaint_white(&mut img);
    img
}

/// Replace near-white pixels (camera rectangle) by the mean of non-white neighbours, growing inward.
fn inpaint_white(img: &mut Rgba) {
    let (w, h) = (img.width as i32, img.height as i32);
    let n = (w * h) as usize;
    let data = &mut img.data;
    let mut bad = vec![0u8; n];
    for i in 0..n {
        if data[i * 4].min(data[i * 4 + 1]).min(data[i * 4 + 2]) > 175 {
            bad[i] = 1;
        }
    }
    let mut grown = bad.clone();
    for y in 0..h {
        for x in 0..w {
            if bad[(y * w + x) as usize] == 0 {
                continue;
            }
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (xx, yy) = (x + dx, y + dy);
                    if xx >= 0 && yy >= 0 && xx < w && yy < h {
                        grown[(yy * w + xx) as usize] = 1;
                    }
                }
            }
        }
    }
    bad = grown;
    for _pass in 0..20 {
        let mut fixed: Vec<(usize, f64, f64, f64)> = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) as usize;
                if bad[i] == 0 {
                    continue;
                }
                let (mut r, mut g, mut b, mut c) = (0f64, 0f64, 0f64, 0f64);
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let (xx, yy) = (x + dx, y + dy);
                        if xx < 0 || yy < 0 || xx >= w || yy >= h {
                            continue;
                        }
                        let j = (yy * w + xx) as usize;
                        if bad[j] != 0 {
                            continue;
                        }
                        r += data[j * 4] as f64;
                        g += data[j * 4 + 1] as f64;
                        b += data[j * 4 + 2] as f64;
                        c += 1.0;
                    }
                }
                if c > 0.0 {
                    fixed.push((i, r / c, g / c, b / c));
                }
            }
        }
        if fixed.is_empty() {
            break;
        }
        for (i, r, g, b) in fixed {
            data[i * 4] = js_round(r) as u8;
            data[i * 4 + 1] = js_round(g) as u8;
            data[i * 4 + 2] = js_round(b) as u8;
            bad[i] = 0;
        }
    }
}

fn cache() -> &'static Mutex<HashMap<usize, Arc<Rgba>>> {
    static C: OnceLock<Mutex<HashMap<usize, Arc<Rgba>>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Embedded reference minimap texture resized to `size`×`size` (cached).
pub fn load_reference(size: usize) -> Arc<Rgba> {
    let mut c = cache().lock().unwrap();
    if let Some(hit) = c.get(&size) {
        return hit.clone();
    }
    let base = decode_png(REFERENCE_PNG).expect("embedded reference png");
    let img = if base.width == size && base.height == size { base } else { resize(&base, size, size) };
    let arc = Arc::new(img);
    c.insert(size, arc.clone());
    arc
}

/// Reference texture from a custom assets folder (not cached).
pub fn load_reference_from(assets_dir: &Path, size: usize) -> Result<Rgba, ImageError> {
    let base = crate::image::load_png(assets_dir.join(REFERENCE_FILE))?;
    Ok(if base.width == size && base.height == size { base } else { resize(&base, size, size) })
}
