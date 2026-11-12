//! Terrain used by the predictor (port of `src/main/tracker/terrain.ts`): walkable grid plus
//! cached geodesic distance fields to goals.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Max cached fields; the "stay" goal creates a new field for every fog entry, so the TS
/// unbounded cache is capped here (cleared when full; goal fields are recomputed on demand).
const MAX_CACHED_FIELDS: usize = 2048;

pub struct GridTerrain {
    pub size: usize,
    grid: Vec<u8>,
    cache: Mutex<HashMap<(i64, i64), Arc<Vec<f32>>>>,
}

impl GridTerrain {
    pub fn new(size: usize, grid: Vec<u8>) -> GridTerrain {
        assert_eq!(grid.len(), size * size);
        GridTerrain { size, grid, cache: Mutex::new(HashMap::new()) }
    }

    /// Everything inside the unit square walkable (tests / fallback).
    pub fn open(size: usize) -> GridTerrain {
        GridTerrain::new(size, vec![1; size * size])
    }

    /// From the embedded walkable asset (`assets/walkable-128.png`) resampled to `size`.
    pub fn from_asset(size: usize) -> GridTerrain {
        GridTerrain::new(size, crate::terrain::load_walkable(size).to_vec())
    }

    pub fn walkable(&self, x: f64, y: f64) -> bool {
        let n = self.size as f64;
        let gx = (x * n).floor();
        let gy = (y * n).floor();
        if gx < 0.0 || gy < 0.0 || gx >= n || gy >= n {
            return false;
        }
        self.grid[gy as usize * self.size + gx as usize] == 1
    }

    /// Geodesic distance field (normalized map units) to a goal; +inf where unreachable.
    pub fn field_to(&self, gx_: f64, gy_: f64) -> Arc<Vec<f32>> {
        // Same key as the TS `toFixed(3)` string.
        let key = ((gx_ * 1000.0).round() as i64, (gy_ * 1000.0).round() as i64);
        if let Some(hit) = self.cache.lock().unwrap().get(&key) {
            return hit.clone();
        }
        let field = Arc::new(self.compute_field(gx_, gy_));
        let mut c = self.cache.lock().unwrap();
        if c.len() >= MAX_CACHED_FIELDS {
            c.clear();
        }
        c.insert(key, field.clone());
        field
    }

    fn compute_field(&self, goal_x: f64, goal_y: f64) -> Vec<f32> {
        let n = self.size;
        let ni = n as i64;
        let mut dist = vec![f32::INFINITY; n * n];
        let clampi = |v: f64| (v.floor() as i64).clamp(0, ni - 1);
        let (mut sx, mut sy) = (clampi(goal_x * n as f64), clampi(goal_y * n as f64));
        if self.grid[(sy * ni + sx) as usize] != 1 {
            let (ox, oy) = (sx, sy);
            let mut best = i64::MAX;
            for y in 0..ni {
                for x in 0..ni {
                    if self.grid[(y * ni + x) as usize] != 1 {
                        continue;
                    }
                    let d = (x - ox).pow(2) + (y - oy).pow(2);
                    if d < best {
                        best = d;
                        sx = x;
                        sy = y;
                    }
                }
            }
        }
        // "Dijkstra-lite" exactly as in TS: FIFO queue, re-push on improvement.
        let step = 1.0 / n as f64;
        let s2 = std::f64::consts::SQRT_2;
        let nb: [(i64, i64, f64); 8] = [(1, 0, 1.0), (-1, 0, 1.0), (0, 1, 1.0), (0, -1, 1.0), (1, 1, s2), (1, -1, s2), (-1, 1, s2), (-1, -1, s2)];
        let start = (sy * ni + sx) as usize;
        dist[start] = 0.0;
        let mut queue: Vec<usize> = vec![start];
        let mut qi = 0;
        while qi < queue.len() {
            let c = queue[qi];
            qi += 1;
            let (cx, cy) = ((c % n) as i64, (c / n) as i64);
            let cd = dist[c] as f64;
            for &(dx, dy, w) in &nb {
                let (x, y) = (cx + dx, cy + dy);
                if x < 0 || y < 0 || x >= ni || y >= ni {
                    continue;
                }
                let j = (y * ni + x) as usize;
                if self.grid[j] != 1 {
                    continue;
                }
                let nd = cd + w * step;
                if nd < dist[j] as f64 - 1e-9 {
                    dist[j] = nd as f32;
                    queue.push(j);
                }
            }
        }
        dist
    }

    /// Sample a distance field at a normalized point (clamped to the grid).
    #[inline]
    pub fn sample(&self, field: &[f32], x: f64, y: f64) -> f64 {
        let n = self.size as f64;
        let gx = (x * n).floor().clamp(0.0, n - 1.0) as usize;
        let gy = (y * n).floor().clamp(0.0, n - 1.0) as usize;
        field[gy * self.size + gx] as f64
    }
}
