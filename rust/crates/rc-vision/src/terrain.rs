//! Walkable terrain mask + geodesic distance field on the minimap grid (port of
//! `src/main/vision/terrain.ts`). Walls and the void are (near-)black on the minimap.

use crate::image::{decode_png, Rgba};
use std::collections::{BinaryHeap, HashMap};
use std::sync::{Arc, Mutex, OnceLock};

pub const WALKABLE_FILE: &str = "walkable-128.png";
pub const WALKABLE_GRID: usize = 128;
pub static WALKABLE_PNG: &[u8] = include_bytes!("../data/walkable-128.png");

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainParams {
    pub floor_lum: f64,
    pub cell_frac: f64,
    pub max_hole: usize,
}

impl Default for TerrainParams {
    fn default() -> Self {
        TerrainParams { floor_lum: 40.0, cell_frac: 0.4, max_hole: 6 }
    }
}

#[inline]
fn lum(d: &[u8], o: usize) -> f64 {
    0.299 * d[o] as f64 + 0.587 * d[o + 1] as f64 + 0.114 * d[o + 2] as f64
}

/// Walkable mask (row-major `grid*grid`, y down; 1 = walkable) from the reference texture.
pub fn walkable_grid(reference: &Rgba, grid: usize, params: TerrainParams) -> Vec<u8> {
    let (w, h) = (reference.width, reference.height);
    let mut floor = vec![0f32; grid * grid];
    let mut total = vec![0f32; grid * grid];
    for y in 0..h {
        for x in 0..w {
            let c = (y * grid / h) * grid + (x * grid / w);
            total[c] += 1.0;
            if lum(&reference.data, (y * w + x) * 4) >= params.floor_lum {
                floor[c] += 1.0;
            }
        }
    }
    let mut m: Vec<u8> = (0..grid * grid).map(|c| if total[c] > 0.0 && (floor[c] / total[c]) as f64 >= params.cell_frac { 1 } else { 0 }).collect();
    for comp in components(&m, grid, 0, false) {
        if comp.cells.len() <= params.max_hole && !comp.touches_edge {
            for &c in &comp.cells {
                m[c] = 1;
            }
        }
    }
    m = prune_spurs(&m, grid);
    let mut comps = components(&m, grid, 1, true);
    comps.sort_by(|a, b| b.cells.len().cmp(&a.cells.len()));
    let mut out = vec![0u8; grid * grid];
    if let Some(main) = comps.first() {
        for &c in &main.cells {
            out[c] = 1;
        }
    }
    out
}

struct Comp {
    cells: Vec<usize>,
    touches_edge: bool,
}

fn components(m: &[u8], grid: usize, value: u8, diag: bool) -> Vec<Comp> {
    let g = grid as i32;
    let mut seen = vec![false; m.len()];
    let mut out = Vec::new();
    let mut stack = Vec::new();
    for s in 0..m.len() {
        if seen[s] || m[s] != value {
            continue;
        }
        let mut comp = Comp { cells: Vec::new(), touches_edge: false };
        seen[s] = true;
        stack.push(s);
        while let Some(c) = stack.pop() {
            comp.cells.push(c);
            let (x, y) = ((c % grid) as i32, (c / grid) as i32);
            if x == 0 || y == 0 || x == g - 1 || y == g - 1 {
                comp.touches_edge = true;
            }
            for dy in -1..=1 {
                for dx in -1..=1 {
                    if (dx | dy) == 0 || (!diag && dx != 0 && dy != 0) {
                        continue;
                    }
                    let (xx, yy) = (x + dx, y + dy);
                    if xx < 0 || yy < 0 || xx >= g || yy >= g {
                        continue;
                    }
                    let n = (yy * g + xx) as usize;
                    if seen[n] || m[n] != value {
                        continue;
                    }
                    seen[n] = true;
                    stack.push(n);
                }
            }
        }
        out.push(comp);
    }
    out
}

fn prune_spurs(m: &[u8], grid: usize) -> Vec<u8> {
    let mut out = m.to_vec();
    for y in 0..grid {
        for x in 0..grid {
            let c = y * grid + x;
            if m[c] == 0 {
                continue;
            }
            let mut n = 0;
            if x > 0 && m[c - 1] != 0 {
                n += 1;
            }
            if x < grid - 1 && m[c + 1] != 0 {
                n += 1;
            }
            if y > 0 && m[c - grid] != 0 {
                n += 1;
            }
            if y < grid - 1 && m[c + grid] != 0 {
                n += 1;
            }
            if n <= 1 {
                out[c] = 0;
            }
        }
    }
    out
}

/// Walkable grid → grayscale RGBA (255 = walkable).
pub fn walkable_to_image(walkable: &[u8], grid: usize) -> Rgba {
    let mut data = vec![0u8; grid * grid * 4];
    for c in 0..grid * grid {
        let v = if walkable[c] != 0 { 255 } else { 0 };
        data[c * 4..c * 4 + 4].copy_from_slice(&[v, v, v, 255]);
    }
    Rgba { width: grid, height: grid, data }
}

/// Nearest-neighbour resample of a square mask to another grid size.
pub fn resample_grid(mask: &[u8], from: usize, to: usize) -> Vec<u8> {
    if from == to {
        return mask.to_vec();
    }
    let mut out = vec![0u8; to * to];
    for y in 0..to {
        for x in 0..to {
            let sy = (((y as f64 + 0.5) * from as f64 / to as f64).floor() as usize).min(from - 1);
            let sx = (((x as f64 + 0.5) * from as f64 / to as f64).floor() as usize).min(from - 1);
            out[y * to + x] = mask[sy * from + sx];
        }
    }
    out
}

/// The embedded `assets/walkable-128.png` as a `grid*grid` mask (cached).
pub fn load_walkable(grid: usize) -> Arc<Vec<u8>> {
    static C: OnceLock<Mutex<HashMap<usize, Arc<Vec<u8>>>>> = OnceLock::new();
    let mut c = C.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap();
    if let Some(hit) = c.get(&grid) {
        return hit.clone();
    }
    let img = decode_png(WALKABLE_PNG).expect("embedded walkable png");
    let base: Vec<u8> = (0..img.width * img.height).map(|i| if img.data[i * 4] >= 128 { 1 } else { 0 }).collect();
    let m = Arc::new(resample_grid(&base, img.width, grid));
    c.insert(grid, m.clone());
    m
}

/// Nearest walkable cell to (gx, gy) by ring search, or None if the mask is empty.
pub fn nearest_walkable(walkable: &[u8], grid: usize, gx: i32, gy: i32) -> Option<usize> {
    let g = grid as i32;
    let x0 = gx.clamp(0, g - 1);
    let y0 = gy.clamp(0, g - 1);
    if walkable[(y0 * g + x0) as usize] != 0 {
        return Some((y0 * g + x0) as usize);
    }
    let mut best: Option<usize> = None;
    let mut best_d = i32::MAX;
    let mut r = 1;
    while r < g && best.is_none() {
        for y in y0 - r..=y0 + r {
            for x in x0 - r..=x0 + r {
                if (x - x0).abs().max((y - y0).abs()) != r || x < 0 || y < 0 || x >= g || y >= g {
                    continue;
                }
                let c = (y * g + x) as usize;
                if walkable[c] == 0 {
                    continue;
                }
                let d = (x - x0).pow(2) + (y - y0).pow(2);
                if d < best_d {
                    best_d = d;
                    best = Some(c);
                }
            }
        }
        r += 1;
    }
    best
}

/// Geodesic distance (normalized map units) from `from` to every cell, 8-connected through
/// walkable cells, diagonal = √2 cells, no corner-cutting between two walls. Unreachable /
/// wall cells are +inf. A start inside a wall snaps to the nearest walkable cell.
pub fn distance_field(walkable: &[u8], from: (f64, f64), grid: usize) -> Vec<f32> {
    let n = grid * grid;
    let g = grid as i32;
    let mut dist = vec![f32::INFINITY; n];
    let Some(start) = nearest_walkable(walkable, grid, (from.0 * grid as f64).floor() as i32, (from.1 * grid as f64).floor() as i32) else {
        return dist;
    };
    let step = 1.0 / grid as f64;
    let dstep = std::f64::consts::SQRT_2 / grid as f64;
    let mut done = vec![false; n];
    dist[start] = 0.0;
    // Heap keys are doubles (as in TS); `dist` is f32, so staleness is checked via `done`.
    let mut heap2: BinaryHeap<std::cmp::Reverse<(OrdF64, usize)>> = BinaryHeap::new();
    heap2.push(std::cmp::Reverse((OrdF64(0.0), start)));
    while let Some(std::cmp::Reverse((OrdF64(d0), c))) = heap2.pop() {
        if done[c] {
            continue;
        }
        done[c] = true;
        let (x, y) = ((c % grid) as i32, (c / grid) as i32);
        for dy in -1..=1 {
            for dx in -1..=1 {
                if (dx | dy) == 0 {
                    continue;
                }
                let (xx, yy) = (x + dx, y + dy);
                if xx < 0 || yy < 0 || xx >= g || yy >= g {
                    continue;
                }
                let nc = (yy * g + xx) as usize;
                if walkable[nc] == 0 || done[nc] {
                    continue;
                }
                if dx != 0 && dy != 0 && walkable[(y * g + xx) as usize] == 0 && walkable[(yy * g + x) as usize] == 0 {
                    continue;
                }
                let nd = d0 + if dx != 0 && dy != 0 { dstep } else { step };
                if nd < dist[nc] as f64 {
                    dist[nc] = nd as f32;
                    heap2.push(std::cmp::Reverse((OrdF64(nd), nc)));
                }
            }
        }
    }
    dist
}

#[derive(Clone, Copy, PartialEq, PartialOrd)]
struct OrdF64(f64);
impl Eq for OrdF64 {}
impl Ord for OrdF64 {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        self.0.partial_cmp(&o.0).unwrap_or(std::cmp::Ordering::Equal)
    }
}
