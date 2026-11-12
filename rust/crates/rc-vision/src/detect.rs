//! Champion portrait detector for the LoL minimap (port of `src/main/vision/detect.ts`).
//!
//! 1. Candidates: circle Hough transform on team-coloured ring pixels (red ring / blue-teal
//!    ring) at the known portrait radius. Works with partially covered rings.
//! 2. Identity: masked normalized cross-correlation of the portrait interior against the
//!    Data Dragon icons of the 5 champions of that team, with a small local search.
//! 3. Assignment: each champion at most once, best scores first.
//! 4. Fallback sweep (full minimap or only around given regions) for champions still missing.
//!
//! All sizes are relative to the minimap width so any resolution works.
//!
//! Speed-ups versus the TS version (same maths, same thresholds):
//! - the frame is converted once to packed f32 RGB plus per-row integer prefix sums of
//!   `r+g+b` and `r²+g²+b²`, so the NCC denominator costs O(rows) instead of O(pixels) and the
//!   numerator is a contiguous dot product per disc row (vectorised);
//! - ring colour classes are computed once per frame;
//! - Hough / ring-support offsets are precomputed;
//! - candidate scoring and the per-champion fallback sweeps run in parallel (rayon); the
//!   order-dependent acceptance stays sequential, so results do not depend on thread timing.

use crate::image::{js_round, resize, Rgba};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Team {
    #[serde(rename = "ORDER")]
    Order,
    #[serde(rename = "CHAOS")]
    Chaos,
}

impl Team {
    pub fn other(self) -> Team {
        match self {
            Team::Order => Team::Chaos,
            Team::Chaos => Team::Order,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Team::Order => "ORDER",
            Team::Chaos => "CHAOS",
        }
    }
    pub fn parse(s: &str) -> Option<Team> {
        match s {
            "ORDER" => Some(Team::Order),
            "CHAOS" => Some(Team::Chaos),
            _ => None,
        }
    }
}

/// A circular region in normalized minimap coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Region {
    pub x: f64,
    pub y: f64,
    pub r: f64,
}

/// Where the expensive fallback sweep runs.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub enum Sweep {
    /// Whole minimap (default).
    #[default]
    Full,
    None,
    /// Only around these points (e.g. last known positions).
    Regions(Vec<Region>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DetectParams {
    /// Ring radius / minimap width.
    pub ring_r: f64,
    /// Portrait interior radius used for matching / minimap width.
    pub inner_r: f64,
    /// Zoom applied to the ddragon icon before masking.
    pub zoom: f64,
    /// Min fraction of ring samples that must vote for a centre.
    pub min_votes: f64,
    /// Min NCC to accept an identity.
    pub min_score: f64,
    /// Stricter threshold for the opposing team.
    pub min_score_enemy: f64,
    /// The viewing team; enemies use `min_score_enemy`.
    pub own_team: Option<Team>,
    /// Which team has RED rings (replays: CHAOS; live with relative colours: the enemy team).
    pub red_team: Option<Team>,
    pub sweep: Sweep,
    /// Local refinement radius in pixels.
    pub refine: i32,
    /// Min NCC for a fallback-sweep hit (None = the normal min_score / min_score_enemy). The sweep
    /// can lock onto minion waves / towers at 0.45–0.68, so this is a tuning knob.
    pub fallback_min_score: Option<f64>,
    /// Min ring support for a fallback-sweep hit (TS default 0.4).
    pub fallback_support: f64,
}

impl Default for DetectParams {
    fn default() -> Self {
        DetectParams {
            ring_r: 0.042,
            inner_r: 0.032,
            zoom: 1.2,
            min_votes: 0.28,
            min_score: 0.45,
            min_score_enemy: 0.6,
            own_team: None,
            red_team: None,
            sweep: Sweep::Full,
            refine: 2,
            fallback_min_score: None,
            fallback_support: 0.4,
        }
    }
}

/// One disc row of a template: pixels `x0 + l .. x0 + l + len` of template row `dy`.
#[derive(Clone, Copy, Debug)]
struct Span {
    dy: usize,
    l: usize,
    len: usize,
    /// Offset into `vec` (floats).
    off: usize,
}

#[derive(Clone, Debug)]
pub struct Template {
    pub champion: String,
    pub team: Team,
    pub size: usize,
    /// Zero-mean, unit-norm masked RGB vector (row-major over masked pixels).
    pub vec: Vec<f32>,
    pub mask: Vec<u8>,
    spans: Vec<Span>,
    /// Number of masked values (3 per masked pixel).
    k: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub x: i32,
    pub y: i32,
    pub team: Team,
    pub votes: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Detection {
    pub champion: String,
    pub team: Team,
    /// Normalized minimap coords 0..1.
    pub x: f64,
    pub y: f64,
    pub score: f64,
    /// Margin over the second-best champion of the same team.
    pub margin: f64,
    pub votes: f64,
}

/// Extra centre to try (e.g. last known position of a visible champion).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hint {
    pub x: f64,
    pub y: f64,
    pub team: Team,
}

pub const RED: u8 = 1;
pub const BLUE: u8 = 2;

#[inline]
pub fn ring_class(r: u8, g: u8, b: u8) -> u8 {
    let (r, g, b) = (r as f64, g as f64, b as f64);
    if r > 140.0 && r > g * 1.5 && r > b * 1.4 {
        return RED;
    }
    if b > 110.0 && b > r * 1.3 && g > r * 1.05 {
        return BLUE;
    }
    0
}

/// Per-frame precomputation shared by every step of the detector.
pub struct Prepared<'a> {
    pub img: &'a Rgba,
    w: usize,
    h: usize,
    /// Packed f32 RGB.
    rgb: Vec<f32>,
    /// Row prefix sums (stride w+1) of r+g+b and r²+g²+b² (exact integers).
    psum: Vec<i64>,
    psq: Vec<i64>,
    /// Ring colour class per pixel.
    pub cls: Vec<u8>,
}

impl<'a> Prepared<'a> {
    pub fn new(img: &'a Rgba) -> Prepared<'a> {
        let (w, h) = (img.width, img.height);
        let d = &img.data;
        let mut rgb = Vec::with_capacity(w * h * 3);
        let mut psum = vec![0i64; (w + 1) * h];
        let mut psq = vec![0i64; (w + 1) * h];
        let mut cls = vec![0u8; w * h];
        for y in 0..h {
            let (mut s, mut q) = (0i64, 0i64);
            psum[y * (w + 1)] = 0;
            psq[y * (w + 1)] = 0;
            for x in 0..w {
                let o = (y * w + x) * 4;
                let (r, g, b) = (d[o], d[o + 1], d[o + 2]);
                rgb.extend_from_slice(&[r as f32, g as f32, b as f32]);
                let (ri, gi, bi) = (r as i64, g as i64, b as i64);
                s += ri + gi + bi;
                q += ri * ri + gi * gi + bi * bi;
                psum[y * (w + 1) + x + 1] = s;
                psq[y * (w + 1) + x + 1] = q;
                cls[y * w + x] = ring_class(r, g, b);
            }
        }
        Prepared { img, w, h, rgb, psum, psq, cls }
    }

    /// Masked NCC of template `t` centred at (cx, cy); -1 when out of bounds / flat.
    /// The template vector is zero-mean and unit-norm, so NCC = Σ v·t / sqrt(Σv² − (Σv)²/k).
    #[inline]
    pub fn ncc(&self, cx: i32, cy: i32, t: &Template) -> f64 {
        let r = ((t.size - 1) / 2) as i32;
        let (x0, y0) = (cx - r, cy - r);
        if x0 < 0 || y0 < 0 || x0 as usize + t.size > self.w || y0 as usize + t.size > self.h {
            return -1.0;
        }
        let (x0, y0) = (x0 as usize, y0 as usize);
        let w1 = self.w + 1;
        let mut dot = 0f64;
        let (mut sum, mut sq) = (0i64, 0i64);
        for s in &t.spans {
            let y = y0 + s.dy;
            let xa = x0 + s.l;
            let base = (y * self.w + xa) * 3;
            dot += dot_f32(&self.rgb[base..base + s.len * 3], &t.vec[s.off..s.off + s.len * 3]) as f64;
            let pr = y * w1;
            sum += self.psum[pr + xa + s.len] - self.psum[pr + xa];
            sq += self.psq[pr + xa + s.len] - self.psq[pr + xa];
        }
        let (sum, sq) = (sum as f64, sq as f64);
        let var_term = sq - (sum * sum) / t.k as f64;
        if var_term > 1e-6 {
            dot / var_term.sqrt()
        } else {
            -1.0
        }
    }

    /// Best score of a template around (cx, cy) within ±refine px.
    pub fn match_around(&self, cx: i32, cy: i32, t: &Template, refine: i32) -> Match {
        let mut best = Match { score: -1.0, x: cx, y: cy };
        for dy in -refine..=refine {
            for dx in -refine..=refine {
                let s = self.ncc(cx + dx, cy + dy, t);
                if s > best.score {
                    best = Match { score: s, x: cx + dx, y: cy + dy };
                }
            }
        }
        best
    }

    /// Fraction of directions around (cx,cy) with a ring pixel of class c at radius R±1.5.
    pub fn ring_support(&self, cx: i32, cy: i32, offs: &RingOffsets, c: u8) -> f64 {
        let mut hits = 0;
        for dir in &offs.dirs {
            for &(dx, dy) in dir {
                let (x, y) = (cx + dx, cy + dy);
                if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h && self.cls[y as usize * self.w + x as usize] == c {
                    hits += 1;
                    break;
                }
            }
        }
        hits as f64 / offs.dirs.len() as f64
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Match {
    pub score: f64,
    pub x: i32,
    pub y: i32,
}

/// Dot product with 8 independent f32 lanes (lets LLVM vectorise without fast-math).
#[inline]
fn dot_f32(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    let (a, b) = (&a[..n], &b[..n]);
    let mut acc = [0f32; 8];
    let ca = a.chunks_exact(8);
    let cb = b.chunks_exact(8);
    let (ra, rb) = (ca.remainder(), cb.remainder());
    for (x, y) in ca.zip(cb) {
        for i in 0..8 {
            acc[i] += x[i] * y[i];
        }
    }
    let mut s = (acc[0] + acc[4]) + (acc[1] + acc[5]) + (acc[2] + acc[6]) + (acc[3] + acc[7]);
    for (x, y) in ra.iter().zip(rb) {
        s += x * y;
    }
    s
}

/// Ring-support sample offsets: per direction (48), the 5 radii R-1.5..R+1.5.
pub struct RingOffsets {
    dirs: Vec<Vec<(i32, i32)>>,
}

impl RingOffsets {
    pub fn new(r: f64) -> RingOffsets {
        let n = 48;
        let dirs = (0..n)
            .map(|k| {
                let t = (k as f64 / n as f64) * PI * 2.0;
                let mut v = Vec::with_capacity(5);
                let mut dr = -1.5;
                while dr <= 1.5 {
                    v.push((js_round(t.cos() * (r + dr)) as i32, js_round(t.sin() * (r + dr)) as i32));
                    dr += 0.75;
                }
                v
            })
            .collect();
        RingOffsets { dirs }
    }
}

/// Circle Hough on ring-coloured pixels; returns local maxima per team.
pub fn find_candidates(prep: &Prepared, p: &DetectParams) -> Vec<Candidate> {
    let (w, h) = (prep.w, prep.h);
    let r = p.ring_r * w as f64;
    let angles = 48usize;
    let red_team = p.red_team.unwrap_or(Team::Chaos);
    let blue_team = red_team.other();
    // Vote offsets: 2 radii (R ± 0.6) × 48 angles.
    let mut vote_offs: Vec<(i32, i32)> = Vec::with_capacity(2 * angles);
    for rr in [r - 0.6, r + 0.6] {
        for k in 0..angles {
            let t = (k as f64 / angles as f64) * PI * 2.0;
            vote_offs.push((js_round(t.cos() * rr) as i32, js_round(t.sin() * rr) as i32));
        }
    }
    let ring = RingOffsets::new(r);
    let nms = (js_round(r * 0.9) as i32).max(3);
    let classes: [(Team, u8); 2] = [(red_team, RED), (blue_team, BLUE)];
    let per_team: Vec<Vec<Candidate>> = classes
        .par_iter()
        .map(|&(team, c)| {
            let mut acc = vec![0u32; w * h];
            for y in 0..h {
                for x in 0..w {
                    if prep.cls[y * w + x] != c {
                        continue;
                    }
                    for &(dx, dy) in &vote_offs {
                        let (cx, cy) = (x as i32 + dx, y as i32 + dy);
                        if cx >= 0 && cy >= 0 && (cx as usize) < w && (cy as usize) < h {
                            acc[cy as usize * w + cx as usize] += 1;
                        }
                    }
                }
            }
            let thr = angles as f64 * 0.15;
            let mut peaks: Vec<(i32, i32, u32)> = Vec::new();
            for y in 1..h.saturating_sub(1) {
                for x in 1..w.saturating_sub(1) {
                    let v = acc[y * w + x];
                    if (v as f64) < thr {
                        continue;
                    }
                    peaks.push((x as i32, y as i32, v));
                }
            }
            // Stable sort by votes, descending (same tie order as the TS Array.sort).
            peaks.sort_by(|a, b| b.2.cmp(&a.2));
            let mut kept: Vec<(i32, i32)> = Vec::new();
            let mut out = Vec::new();
            for &(px, py, _) in &peaks {
                if kept.iter().any(|&(kx, ky)| (kx - px).abs() <= nms && (ky - py).abs() <= nms) {
                    continue;
                }
                let votes = prep.ring_support(px, py, &ring, c);
                if votes >= p.min_votes {
                    kept.push((px, py));
                    out.push(Candidate { x: px, y: py, team, votes });
                }
                if kept.len() >= 40 {
                    break;
                }
            }
            out
        })
        .collect();
    per_team.into_iter().flatten().collect()
}

/// Zero-mean, unit-norm RGB vector of masked pixels of a size×size RGBA patch
/// (f32 storage, f64 arithmetic, exactly as the TS Float32Array code).
fn normalized_patch(data: &[u8], stride: usize, size: usize, mask: &[u8]) -> Vec<f32> {
    let n: usize = mask.iter().map(|&m| m as usize).sum();
    let mut v = vec![0f32; n * 3];
    let mut k = 0;
    let mut sum = 0f64;
    for y in 0..size {
        for x in 0..size {
            if mask[y * size + x] == 0 {
                continue;
            }
            let i = (y * stride + x) * 4;
            for c in 0..3 {
                v[k] = data[i + c] as f32;
                sum += v[k] as f64;
                k += 1;
            }
        }
    }
    let mean = sum / v.len() as f64;
    let mut norm = 0f64;
    for e in v.iter_mut() {
        *e = (*e as f64 - mean) as f32;
        norm += (*e as f64) * (*e as f64);
    }
    let mut norm = norm.sqrt();
    if norm == 0.0 {
        norm = 1.0;
    }
    for e in v.iter_mut() {
        *e = (*e as f64 / norm) as f32;
    }
    v
}

/// Build a matching template from a ddragon square icon for a given minimap width.
pub fn build_template(champion: &str, team: Team, icon: &Rgba, minimap_width: usize, p: &DetectParams) -> Template {
    let r = (js_round(p.inner_r * minimap_width as f64) as usize).max(4);
    let size = r * 2 + 1;
    // Crop the centre of the icon according to zoom, then resize to the portrait size.
    let crop = js_round(icon.width as f64 / p.zoom) as usize;
    let off = js_round((icon.width as f64 - crop as f64) / 2.0) as usize;
    let cropped = icon.crop(off, off, crop, crop);
    let small = resize(&cropped, size, size);
    let mut mask = vec![0u8; size * size];
    let rr = (r as f64 - 0.5) * (r as f64 - 0.5);
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f64 - r as f64, y as f64 - r as f64);
            if dx * dx + dy * dy <= rr {
                mask[y * size + x] = 1;
            }
        }
    }
    let vec = normalized_patch(&small.data, size, size, &mask);
    let mut spans = Vec::new();
    let mut off = 0;
    for y in 0..size {
        let row = &mask[y * size..(y + 1) * size];
        if let Some(l) = row.iter().position(|&m| m == 1) {
            let len = row.iter().filter(|&&m| m == 1).count();
            debug_assert!(row[l..l + len].iter().all(|&m| m == 1));
            spans.push(Span { dy: y, l, len, off });
            off += len * 3;
        }
    }
    Template { champion: champion.to_string(), team, size, k: vec.len(), vec, mask, spans }
}

#[derive(Clone, Debug, Default)]
pub struct DetectOutput {
    pub detections: Vec<Detection>,
    pub candidates: Vec<Candidate>,
}

/// Full detection on one minimap frame. `extra` = e.g. tracker positions to also try.
pub fn detect(img: &Rgba, templates: &[Template], p: &DetectParams, extra: &[Hint]) -> DetectOutput {
    let prep = Prepared::new(img);
    detect_prepared(&prep, templates, p, extra)
}

pub fn detect_prepared(prep: &Prepared, templates: &[Template], p: &DetectParams, extra: &[Hint]) -> DetectOutput {
    let w = prep.w;
    let wf = w as f64;
    let candidates = find_candidates(prep, p);
    let mut all = candidates.clone();
    all.extend(extra.iter().map(|c| Candidate { x: js_round(c.x * wf) as i32, y: js_round(c.y * wf) as i32, team: c.team, votes: 0.0 }));

    struct Pair {
        ci: usize,
        ti: usize,
        score: f64,
        x: i32,
        y: i32,
        margin: f64,
    }
    // Score every (candidate, champion of that team) pair.
    let per_cand: Vec<Vec<Pair>> = all
        .par_iter()
        .enumerate()
        .map(|(ci, c)| {
            let mut scored: Vec<(usize, Match)> = templates
                .iter()
                .enumerate()
                .filter(|(_, t)| t.team == c.team)
                .map(|(ti, t)| (ti, prep.match_around(c.x, c.y, t, p.refine)))
                .collect();
            scored.sort_by(|a, b| b.1.score.partial_cmp(&a.1.score).unwrap_or(std::cmp::Ordering::Equal));
            let second = scored.get(1).map(|s| s.1.score).unwrap_or(-1.0);
            scored
                .iter()
                .enumerate()
                .map(|(k, &(ti, m))| Pair { ci, ti, score: m.score, x: m.x, y: m.y, margin: if k == 0 { m.score - second } else { 0.0 } })
                .collect()
        })
        .collect();
    let mut pairs: Vec<Pair> = per_cand.into_iter().flatten().collect();
    pairs.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

    let need_for = |team: Team| match p.own_team {
        Some(own) if team != own => p.min_score_enemy,
        _ => p.min_score,
    };
    let mut used_c = vec![false; all.len()];
    let mut used_t = vec![false; templates.len()];
    let mut detections: Vec<Detection> = Vec::new();
    let ring_px = p.ring_r * wf;
    for pr in &pairs {
        let t = &templates[pr.ti];
        if pr.score < need_for(t.team) || used_c[pr.ci] || used_t[pr.ti] || templates.iter().enumerate().any(|(j, o)| used_t[j] && o.champion == t.champion) {
            continue;
        }
        // Two candidates on the same spot (ring Hough + tracker hint): skip duplicates.
        if detections.iter().any(|d| (d.x * wf - pr.x as f64).hypot(d.y * wf - pr.y as f64) < ring_px * 0.8) {
            used_c[pr.ci] = true;
            continue;
        }
        used_c[pr.ci] = true;
        used_t[pr.ti] = true;
        detections.push(Detection { champion: t.champion.clone(), team: t.team, x: pr.x as f64 / wf, y: pr.y as f64 / wf, score: pr.score, margin: pr.margin, votes: all[pr.ci].votes });
    }

    // Fallback sweep for champions still missing: portraits whose ring is mostly covered by
    // another portrait don't make it through the Hough step. Try every position (stride 2)
    // that has ring colour of the right team within ~R.
    let missing: Vec<usize> = (0..templates.len()).filter(|&i| !used_t[i]).collect();
    if !missing.is_empty() && p.sweep != Sweep::None {
        let near = sweep_mask(prep, ring_px);
        let ring = RingOffsets::new(ring_px);
        let red_team = p.red_team.unwrap_or(Team::Chaos);
        let h = prep.h;
        let regions: Option<&Vec<Region>> = match &p.sweep {
            Sweep::Regions(r) => Some(r),
            _ => None,
        };
        // Positions to try (stride 2, inside the sweep regions), shared by all templates.
        let mut pos_red: Vec<(i32, i32)> = Vec::new();
        let mut pos_blue: Vec<(i32, i32)> = Vec::new();
        for y in (0..h).step_by(2) {
            for x in (0..w).step_by(2) {
                let n = near[y * w + x];
                if n == 0 {
                    continue;
                }
                if let Some(rs) = regions {
                    if !rs.iter().any(|r| (x as f64 / wf - r.x).hypot(y as f64 / wf - r.y) <= r.r) {
                        continue;
                    }
                }
                if n & RED != 0 {
                    pos_red.push((x as i32, y as i32));
                }
                if n & BLUE != 0 {
                    pos_blue.push((x as i32, y as i32));
                }
            }
        }
        let fines: Vec<(usize, Match, f64, u8)> = missing
            .par_iter()
            .map(|&ti| {
                let t = &templates[ti];
                let want = if t.team == red_team { RED } else { BLUE };
                let positions = if want == RED { &pos_red } else { &pos_blue };
                let mut best = Match { score: -1.0, x: 0, y: 0 };
                for &(x, y) in positions {
                    let s = prep.ncc(x, y, t);
                    if s > best.score {
                        best = Match { score: s, x, y };
                    }
                }
                let fine = prep.match_around(best.x, best.y, t, 1);
                // Team-coloured ring around it is the strong cue (structures/pings don't have one).
                let support = prep.ring_support(fine.x, fine.y, &ring, want);
                (ti, fine, support, want)
            })
            .collect();
        for (ti, fine, support, _) in fines {
            let t = &templates[ti];
            let need = p.fallback_min_score.unwrap_or_else(|| need_for(t.team));
            if fine.score < need || support < p.fallback_support {
                continue;
            }
            if detections.iter().any(|d| (d.x * wf - fine.x as f64).hypot(d.y * wf - fine.y as f64) < ring_px * 0.9) {
                continue;
            }
            used_t[ti] = true;
            detections.push(Detection { champion: t.champion.clone(), team: t.team, x: fine.x as f64 / wf, y: fine.y as f64 / wf, score: fine.score, margin: 0.0, votes: 0.0 });
        }
    }
    DetectOutput { detections, candidates }
}

/// Per pixel: which team ring colours occur within ~R (1 red, 2 blue, 3 both, 0 none).
fn sweep_mask(prep: &Prepared, r: f64) -> Vec<u8> {
    let (w, h) = (prep.w as i32, prep.h as i32);
    let mut out = vec![0u8; (w * h) as usize];
    let rr = (r + 1.0).ceil() as i32;
    for y in (0..h).step_by(2) {
        for x in (0..w).step_by(2) {
            let c = prep.cls[(y * w + x) as usize];
            if c == 0 {
                continue;
            }
            let mut dy = -rr;
            while dy <= rr {
                let yy = y + dy;
                if yy >= 0 && yy < h {
                    let mut dx = -rr;
                    while dx <= rr {
                        let xx = x + dx;
                        if xx >= 0 && xx < w {
                            out[(yy * w + xx) as usize] |= c;
                        }
                        dx += 2;
                    }
                }
                dy += 2;
            }
        }
    }
    out
}
