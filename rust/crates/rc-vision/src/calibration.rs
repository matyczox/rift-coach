//! Minimap location in the full game frame (port of `src/main/calibration.ts`).

use crate::image::js_round;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Minimap rectangle measured on a window of the given size (as stored in settings / meta.json).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Calibration {
    pub rect: Rect,
    pub window_width: i32,
    pub window_height: i32,
}

impl Calibration {
    /// The rectangle for a window of `w`×`h` (scaled, anchored bottom-right).
    pub fn rect_for(&self, w: i32, h: i32) -> Rect {
        scale_rect(self.rect, self.window_width, self.window_height, w, h)
    }
}

fn luminance(px: &[u8], width: usize, height: usize) -> Vec<f32> {
    (0..width * height).map(|i| ((px[i * 4] as u32 + px[i * 4 + 1] as u32 + px[i * 4 + 2] as u32) as f64 / 3.0) as f32).collect()
}

#[allow(clippy::too_many_arguments)]
fn edge_fraction(lum: &[f32], width: i32, x0: i32, y0: i32, dx: i32, dy: i32, len: i32, ox: i32, oy: i32) -> f64 {
    let l = |x: i32, y: i32| lum[(y * width + x) as usize] as f64;
    let (mut hits, mut n) = (0, 0);
    let mut k = 0;
    while k < len {
        let (x, y) = (x0 + dx * k, y0 + dy * k);
        let (mut out, mut inn) = (0f64, 0f64);
        for d in 1..=3 {
            out += l(x - ox * d, y - oy * d);
            inn += l(x + ox * (d - 1), y + oy * (d - 1));
        }
        out /= 3.0;
        inn /= 3.0;
        if inn < 55.0 && out - inn > 12.0 {
            hits += 1;
        }
        k += 2;
        n += 1;
    }
    if n > 0 {
        hits as f64 / n as f64
    } else {
        0.0
    }
}

fn square_score(lum: &[f32], width: i32, height: i32, x0: i32, y0: i32, s: i32) -> f64 {
    if x0 < 3 || y0 < 3 || x0 + s + 3 >= width || y0 + s + 3 >= height {
        return 0.0;
    }
    let inset = js_round(s as f64 * 0.1) as i32;
    let len = s - 2 * inset;
    edge_fraction(lum, width, x0, y0 + inset, 0, 1, len, 1, 0)
        .min(edge_fraction(lum, width, x0 + inset, y0, 1, 0, len, 0, 1))
        .min(edge_fraction(lum, width, x0 + s - 1, y0 + inset, 0, 1, len, -1, 0))
        .min(edge_fraction(lum, width, x0 + inset, y0 + s - 1, 1, 0, len, 0, -1))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Found {
    pub rect: Rect,
    pub score: f64,
}

/// Locate the minimap square in a full RGBA game frame (bottom-right anchored).
pub fn find_minimap(px: &[u8], width: usize, height: usize) -> Option<Found> {
    let lum = luminance(px, width, height);
    let (w, h) = (width as i32, height as i32);
    let min_s = js_round(h as f64 * 0.14) as i32;
    let max_s = js_round(h as f64 * 0.42) as i32;
    let max_margin = js_round(h as f64 * 0.04) as i32;
    let search = |sr: (i32, i32), mrr: (i32, i32), mbr: (i32, i32), step: i32| -> Option<Found> {
        let mut best: Option<Found> = None;
        let mut s = sr.0;
        while s <= sr.1 {
            let mut mr = mrr.0;
            while mr <= mrr.1 {
                let mut mb = mbr.0;
                while mb <= mbr.1 {
                    let (x0, y0) = (w - mr - s, h - mb - s);
                    let score = square_score(&lum, w, h, x0, y0, s);
                    let better = match best {
                        None => true,
                        Some(b) => score > b.score + 0.02 || (score >= b.score - 0.02 && s > b.rect.width),
                    };
                    if better {
                        let sc = match best {
                            Some(b) => score.max(b.score),
                            None => score,
                        };
                        best = Some(Found { rect: Rect { x: x0, y: y0, width: s, height: s }, score: sc });
                    }
                    mb += step;
                }
                mr += step;
            }
            s += step;
        }
        best
    };
    let coarse = search((min_s, max_s), (0, max_margin), (0, max_margin), 3)?;
    if coarse.score < 0.5 {
        return None;
    }
    let c = coarse.rect;
    let mr = w - c.x - c.width;
    let mb = h - c.y - c.height;
    let fine = search((c.width - 4, c.width + 4), ((mr - 4).max(0), mr + 4), ((mb - 4).max(0), mb + 4), 1);
    let best = match fine {
        Some(f) if f.score >= coarse.score - 0.02 => f,
        _ => coarse,
    };
    Some(Found { rect: best.rect, score: square_score(&lum, w, h, best.rect.x, best.rect.y, best.rect.width) })
}

/// Score an existing calibration on a new frame (0..1).
pub fn verify_minimap(px: &[u8], width: usize, height: usize, r: Rect) -> f64 {
    square_score(&luminance(px, width, height), width as i32, height as i32, r.x, r.y, r.width)
}

/// Scale a calibration rect measured on one window size to another (anchored bottom-right).
pub fn scale_rect(r: Rect, from_w: i32, from_h: i32, to_w: i32, to_h: i32) -> Rect {
    if from_w == to_w && from_h == to_h {
        return r;
    }
    let k = to_h as f64 / from_h as f64;
    let right = (from_w - (r.x + r.width)) as f64;
    let bottom = (from_h - (r.y + r.height)) as f64;
    let s = js_round(r.width as f64 * k);
    Rect { x: js_round(to_w as f64 - right * k - s) as i32, y: js_round(to_h as f64 - bottom * k - s) as i32, width: s as i32, height: s as i32 }
}
