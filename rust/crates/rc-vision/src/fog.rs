//! Fog-of-war vision mask from a minimap crop (port of `src/main/vision/fog.ts`, SPEC 5.3).
//!
//! The fog overlay is a near-uniform multiplicative darkening of the map texture (~0.36×
//! luminance, hue preserved). Compared with the clean reference texture every map pixel votes
//! "visible" (frame ≈ reference) or "fogged" (frame ≈ 0.36 × reference); pixels whose colour
//! is not a scaled copy of the reference abstain. Cells without votes are filled from their
//! neighbours, then lightly smoothed.

use crate::image::Rgba;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FogParams {
    /// Luminance factor the fog overlay applies.
    pub fog_factor: f64,
    /// Reference pixels darker than this (luminance 0–255) carry no signal: walls/void.
    pub min_ref_lum: f64,
    /// Max per-channel colour residual (fraction of reference channel sum) for a pixel to vote.
    pub max_residual: f64,
    /// Centre weight of the 3×3 smoothing kernel; 0 disables smoothing.
    pub smooth_center: f64,
}

impl Default for FogParams {
    fn default() -> Self {
        FogParams { fog_factor: 0.36, min_ref_lum: 40.0, max_residual: 0.12, smooth_center: 8.0 }
    }
}

/// `mask[i] >= VISIBLE_THRESHOLD` ⇒ treat the cell as visible.
pub const VISIBLE_THRESHOLD: f32 = 0.52;

#[inline]
fn lum(d: &[u8], o: usize) -> f64 {
    0.299 * d[o] as f64 + 0.587 * d[o + 1] as f64 + 0.114 * d[o + 2] as f64
}

/// Precomputed per-reference data; build once per (reference, grid) and reuse every frame.
pub struct FogReader {
    pub grid: usize,
    width: usize,
    height: usize,
    params: FogParams,
    pix: Vec<u32>,
    cell: Vec<u32>,
    ref_rgb: Vec<[f64; 3]>,
    ref_sum: Vec<f64>,
    /// Cells with fog signal at all (≥ 1/4 non-wall map texture in the reference).
    pub map_cells: Vec<u8>,
}

impl FogReader {
    pub fn new(reference: &Rgba, grid: usize, params: FogParams) -> FogReader {
        let (w, h) = (reference.width, reference.height);
        let d = &reference.data;
        let mut pix = Vec::new();
        let mut cell = Vec::new();
        let mut ref_rgb = Vec::new();
        let mut ref_sum = Vec::new();
        let mut counts = vec![0i32; grid * grid];
        let mut totals = vec![0i32; grid * grid];
        let cell_of = |x: usize, y: usize| (y * grid / h) * grid + (x * grid / w);
        for y in 0..h {
            for x in 0..w {
                totals[cell_of(x, y)] += 1;
            }
        }
        for i in 0..w * h {
            let o = i * 4;
            if lum(d, o) < params.min_ref_lum {
                continue;
            }
            let (x, y) = (i % w, i / w);
            let c = cell_of(x, y);
            pix.push(i as u32);
            cell.push(c as u32);
            counts[c] += 1;
            ref_rgb.push([d[o] as f64, d[o + 1] as f64, d[o + 2] as f64]);
            // refSum is a Float32Array in TS (integer, exact).
            ref_sum.push((d[o] as u32 + d[o + 1] as u32 + d[o + 2] as u32) as f64);
        }
        let map_cells = (0..grid * grid).map(|c| if counts[c] as f64 >= totals[c] as f64 * 0.25 { 1 } else { 0 }).collect();
        FogReader { grid, width: w, height: h, params, pix, cell, ref_rgb, ref_sum, map_cells }
    }

    /// Per grid cell probability that the cell is currently VISIBLE (1) vs fogged (0).
    /// Row-major `grid*grid`, index = gy*grid + gx (y down, blue base bottom-left).
    pub fn read(&self, frame: &Rgba) -> Vec<f32> {
        assert!(
            frame.width == self.width && frame.height == self.height,
            "read_vision_mask: frame {}x{} vs reference {}x{}",
            frame.width,
            frame.height,
            self.width,
            self.height
        );
        let grid = self.grid;
        let nc = grid * grid;
        let d = &frame.data;
        let f = self.params.fog_factor;
        let tol = self.params.max_residual;
        let mut vis = vec![0f32; nc];
        let mut votes = vec![0f32; nc];
        for k in 0..self.pix.len() {
            let o = self.pix[k] as usize * 4;
            let (r, g, b) = (d[o] as f64, d[o + 1] as f64, d[o + 2] as f64);
            let [rr, gg, bb] = self.ref_rgb[k];
            let s_ref = self.ref_sum[k];
            let s = (r + g + b) / s_ref;
            if s > 1.2 {
                continue;
            }
            let res = (r - s * rr).abs() + (g - s * gg).abs() + (b - s * bb).abs();
            if res > tol * s_ref * s.max(0.3) + 6.0 {
                continue;
            }
            let v = ((s - f) / (1.0 - f)).clamp(0.0, 1.0);
            let c = self.cell[k] as usize;
            vis[c] = (vis[c] as f64 + v) as f32;
            votes[c] += 1.0;
        }
        let mut out = vec![0f32; nc];
        let mut known = vec![0u8; nc];
        for c in 0..nc {
            if votes[c] >= 2.0 {
                out[c] = (vis[c] as f64 / votes[c] as f64) as f32;
                known[c] = 1;
            }
        }
        fill_unknown(&mut out, &mut known, grid);
        if self.params.smooth_center > 0.0 {
            smooth(&out, grid, self.params.smooth_center)
        } else {
            out
        }
    }
}

/// Map-cell mask (see [`FogReader::map_cells`]).
pub fn map_cell_mask(reference: &Rgba, grid: usize, params: FogParams) -> Vec<u8> {
    FogReader::new(reference, grid, params).map_cells
}

/// One-shot vision mask (prefer a cached [`FogReader`] for repeated frames).
pub fn read_vision_mask(frame: &Rgba, reference: &Rgba, grid: usize, params: FogParams) -> Vec<f32> {
    FogReader::new(reference, grid, params).read(frame)
}

/// Fill cells without votes with the mean of known 8-neighbours, growing outward.
fn fill_unknown(v: &mut [f32], known: &mut [u8], grid: usize) {
    let g = grid as i32;
    let mut frontier = true;
    let mut pass = 0;
    let mut next = vec![0u8; known.len()];
    while frontier && pass < grid {
        pass += 1;
        frontier = false;
        next.copy_from_slice(known);
        for y in 0..g {
            for x in 0..g {
                let c = (y * g + x) as usize;
                if known[c] != 0 {
                    continue;
                }
                let (mut s, mut n) = (0f64, 0f64);
                for dy in -1..=1 {
                    let yy = y + dy;
                    if yy < 0 || yy >= g {
                        continue;
                    }
                    for dx in -1..=1 {
                        let xx = x + dx;
                        if xx < 0 || xx >= g {
                            continue;
                        }
                        let cc = (yy * g + xx) as usize;
                        if known[cc] != 0 {
                            s += v[cc] as f64;
                            n += 1.0;
                        }
                    }
                }
                if n > 0.0 {
                    v[c] = (s / n) as f32;
                    next[c] = 1;
                    frontier = true;
                }
            }
        }
        known.copy_from_slice(&next);
    }
    for c in 0..v.len() {
        if known[c] == 0 {
            v[c] = 0.0; // nothing known at all: assume fog
        }
    }
}

fn smooth(v: &[f32], grid: usize, center: f64) -> Vec<f32> {
    let g = grid as i32;
    let mut out = vec![0f32; v.len()];
    for y in 0..g {
        for x in 0..g {
            let mut s = v[(y * g + x) as usize] as f64 * center;
            let mut n = center;
            for dy in -1..=1 {
                let yy = y + dy;
                if yy < 0 || yy >= g {
                    continue;
                }
                for dx in -1..=1 {
                    let xx = x + dx;
                    if (dx | dy) == 0 || xx < 0 || xx >= g {
                        continue;
                    }
                    s += v[(yy * g + xx) as usize] as f64;
                    n += 1.0;
                }
            }
            out[(y * g + x) as usize] = (s / n) as f32;
        }
    }
    out
}

/// Ground-truth vision mask from a fogged frame and the fog-free frame at the same game time:
/// per cell median luminance ratio over map pixels; ratio < `fog_ratio` ⇒ fogged (0), else 1.
/// With `reference`, only pixels where the clear frame shows the bare map count.
/// Cells with too few map pixels get 255 (unknown). Used for calibration and tests.
pub fn truth_vision_mask(fogged: &Rgba, clear: &Rgba, grid: usize, fog_ratio: f64, reference: Option<&Rgba>) -> Vec<u8> {
    let (w, h) = (clear.width, clear.height);
    let mut lists: Vec<Vec<f64>> = vec![Vec::new(); grid * grid];
    for y in 0..h {
        for x in 0..w {
            let o = (y * w + x) * 4;
            let lt = lum(&clear.data, o);
            if lt < 40.0 {
                continue;
            }
            if let Some(rf) = reference {
                let rd = &rf.data;
                let cd = &clear.data;
                let diff = (cd[o] as i32 - rd[o] as i32).abs() + (cd[o + 1] as i32 - rd[o + 1] as i32).abs() + (cd[o + 2] as i32 - rd[o + 2] as i32).abs();
                if lum(rd, o) < 40.0 || diff > 60 {
                    continue;
                }
            }
            lists[(y * grid / h) * grid + (x * grid / w)].push(lum(&fogged.data, o) / lt);
        }
    }
    let cell_pix = (w as f64 / grid as f64) * (h as f64 / grid as f64);
    lists
        .into_iter()
        .map(|mut l| {
            if (l.len() as f64) < cell_pix * 0.25 {
                return 255;
            }
            l.sort_by(|a, b| a.partial_cmp(b).unwrap());
            if l[l.len() >> 1] < fog_ratio {
                0
            } else {
                1
            }
        })
        .collect()
}

/// P(visible) at normalized minimap coords from a `grid*grid` mask (nearest cell).
#[inline]
pub fn sample_mask(mask: &[f32], grid: usize, x: f64, y: f64) -> f64 {
    let g = grid as f64;
    let gx = (x * g).floor().clamp(0.0, g - 1.0) as usize;
    let gy = (y * g).floor().clamp(0.0, g - 1.0) as usize;
    mask[gy * grid + gx] as f64
}
