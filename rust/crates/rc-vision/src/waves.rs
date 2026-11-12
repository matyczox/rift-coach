//! Minion waves from the minimap: ally / enemy minion dots grouped per lane into a coarse
//! "wave state" (where the waves meet along each lane) for the coach.
//!
//! Minions are small filled dots (core ~4 px at a 341 px minimap) with a dark outline: blue for
//! the blue-ring team, red for the red-ring team (with "relative team colours" allies are blue).
//! Detection is a single pass over the crop:
//! 1. classify pixels as saturated minion-blue / minion-red core colour (the river and the fog
//!    are darker / less saturated, the camera rectangle is white);
//! 2. 4-connected components of core pixels; touching dots of a wave stay separate most of the
//!    time thanks to their outline, merged ones are counted by area;
//! 3. keep compact components with a mostly dark border (minion outline); drop the ones inside
//!    champion portraits (ring colour = minion colour), long arcs / outlines (portrait rings
//!    the detector missed, turret / inhibitor icons, pings) and the bases;
//! 4. project dots onto the three lane centre lines; position 0 = our inhibitor turret,
//!    1 = the enemy inhibitor turret (mirrored across the main diagonal for the red side).
//!
//! Enemy minions are only drawn where we have vision, ally minions always (they give vision):
//! a lane with no dots and no vision is "unknown", not "empty".

use crate::image::Rgba;
use serde::Serialize;

/// Reference minimap width the pixel constants are tuned for.
pub const REF_W: f64 = 341.0;

#[derive(Clone, Debug)]
pub struct WaveParams {
    /// Portrait exclusion radius / minimap width (portrait ring at 0.042).
    pub portrait_r: f64,
    /// Max distance (normalized) from a lane centre line for a dot to count for that lane.
    pub corridor: f64,
    /// Core area (px) of one minion dot at [`REF_W`].
    pub dot_area: f64,
    /// Min fraction of a component's border that must be dark (the minion outline).
    pub min_dark_border: f64,
    /// Fog mask threshold for "visible".
    pub visible_threshold: f32,
}

impl Default for WaveParams {
    fn default() -> Self {
        WaveParams { portrait_r: 0.049, corridor: 0.05, dot_area: 12.5, min_dark_border: 0.6, visible_threshold: crate::fog::VISIBLE_THRESHOLD }
    }
}

/// One minion dot (or a few merged dots, `n`), normalized minimap coords (screen, not mirrored).
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Dot {
    pub x: f64,
    pub y: f64,
    /// Blue dot (else red).
    pub blue: bool,
    /// Estimated number of minions in this blob.
    pub n: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WaveState {
    /// Our wave at / into the enemy tower (meeting point >= 0.62).
    Crashing,
    /// Waves meet on the enemy half (our wave pushing).
    Pushing,
    /// Waves meet around the middle of the lane.
    Even,
    /// Waves meet on our half, not yet at our tower.
    OurSide,
    /// Enemy wave at our tower (meeting point / enemy front <= 0.39).
    UnderOurTower,
    /// Lane in vision, no minions seen (between waves).
    NoWave,
    /// No minions seen and (mostly) no vision on the lane.
    Unknown,
}

impl WaveState {
    pub fn as_str(self) -> &'static str {
        match self {
            WaveState::Crashing => "crashing",
            WaveState::Pushing => "pushing",
            WaveState::Even => "even",
            WaveState::OurSide => "our_side",
            WaveState::UnderOurTower => "under_our_tower",
            WaveState::NoWave => "no_wave",
            WaveState::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaneWave {
    /// "top" | "mid" | "bot".
    pub lane: &'static str,
    pub ally: u32,
    pub enemy: u32,
    /// Mean position of our minions along the lane (0 = our inhibitor turret, 1 = theirs).
    pub ally_pos: Option<f64>,
    pub enemy_pos: Option<f64>,
    /// Where the waves meet (front-most ally / enemy minion), or the lone wave's front.
    pub front: Option<f64>,
    pub state: WaveState,
    /// Fraction of the lane in vision (None without a fog mask).
    pub vision: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WaveReading {
    pub lanes: Vec<LaneWave>,
    #[serde(skip)]
    pub dots: Vec<Dot>,
}

impl WaveReading {
    pub fn lane(&self, name: &str) -> Option<&LaneWave> {
        self.lanes.iter().find(|l| l.lane == name)
    }
}

// ---------- lane geometry (blue side view: our base bottom-left) ----------

pub const LANES: [&str; 3] = ["top", "mid", "bot"];

/// Lane centre lines from our inhibitor turret to the enemy one (normalized, blue side). Each
/// is symmetric under (x,y)->(y,x) (reversed), so the same lines serve the red side mirrored.
const TOP: &[(f64, f64)] = &[(0.085, 0.71), (0.085, 0.20), (0.13, 0.13), (0.20, 0.085), (0.71, 0.085)];
const MID: &[(f64, f64)] = &[(0.245, 0.755), (0.755, 0.245)];
const BOT: &[(f64, f64)] = &[(0.29, 0.912), (0.80, 0.912), (0.87, 0.87), (0.912, 0.80), (0.912, 0.29)];

fn lane_line(i: usize) -> &'static [(f64, f64)] {
    [TOP, MID, BOT][i]
}

/// (distance to the line, position 0..1 along it) of a point.
pub fn project(line: &[(f64, f64)], x: f64, y: f64) -> (f64, f64) {
    let total: f64 = line.windows(2).map(|s| (s[1].0 - s[0].0).hypot(s[1].1 - s[0].1)).sum();
    let mut best = (f64::INFINITY, 0.0);
    let mut acc = 0.0;
    for s in line.windows(2) {
        let (ax, ay, bx, by) = (s[0].0, s[0].1, s[1].0, s[1].1);
        let (dx, dy) = (bx - ax, by - ay);
        let len2 = dx * dx + dy * dy;
        let u = (((x - ax) * dx + (y - ay) * dy) / len2).clamp(0.0, 1.0);
        let (px, py) = (ax + u * dx, ay + u * dy);
        let d = (x - px).hypot(y - py);
        if d < best.0 {
            best = (d, (acc + u * len2.sqrt()) / total);
        }
        acc += len2.sqrt();
    }
    best
}

/// Point on a lane at position t (blue side coords).
fn point_at(line: &[(f64, f64)], t: f64) -> (f64, f64) {
    let total: f64 = line.windows(2).map(|s| (s[1].0 - s[0].0).hypot(s[1].1 - s[0].1)).sum();
    let mut left = t.clamp(0.0, 1.0) * total;
    for s in line.windows(2) {
        let len = (s[1].0 - s[0].0).hypot(s[1].1 - s[0].1);
        if left <= len {
            let u = left / len;
            return (s[0].0 + u * (s[1].0 - s[0].0), s[0].1 + u * (s[1].1 - s[0].1));
        }
        left -= len;
    }
    *line.last().unwrap()
}

/// Lane index and position of a point in screen coords; `flip` = we are the red side.
pub fn lane_of(x: f64, y: f64, flip: bool, corridor: f64) -> Option<(usize, f64)> {
    let (x, y) = if flip { (y, x) } else { (x, y) };
    (0..3).map(|i| (i, project(lane_line(i), x, y))).filter(|(_, (d, _))| *d <= corridor).min_by(|a, b| a.1 .0.total_cmp(&b.1 .0)).map(|(i, (_, t))| (i, t))
}

// ---------- dot detection ----------

const BLUE: u8 = 1;
const RED: u8 = 2;

/// Minion core colour class. Blue minions ~ (5,142,204), red ~ (205,62,61); the river is
/// (16,124,156), fogged map is dark, portrait rings share the minion colours (masked).
#[inline]
fn core_class(r: u8, g: u8, b: u8) -> u8 {
    let (r, g, b) = (r as i32, g as i32, b as i32);
    if b >= 170 && r <= 90 && (95..=190).contains(&g) && b >= g + 25 {
        BLUE
    } else if r >= 165 && (38..=100).contains(&g) && (32..=100).contains(&b) && r >= g + 80 && r >= b + 80 {
        // (Pure red (226,10,9) is the skull icon of a kill / destroyed structure, not a minion.)
        RED
    } else {
        0
    }
}

#[inline]
fn is_dark(r: u8, g: u8, b: u8) -> bool {
    (r as u32 * 299 + g as u32 * 587 + b as u32 * 114) < 95_000
}

/// Component statistics (debug / tuning).
#[derive(Clone, Debug)]
pub struct Blob {
    pub blue: bool,
    pub area: u32,
    pub cx: f64,
    pub cy: f64,
    /// Principal axis lengths (px, ~ extent of a uniform bar).
    pub major: f64,
    pub minor: f64,
    pub dark_border: f64,
    pub w: u32,
    pub h: u32,
    /// Brightest value of the team channel (blue: B, red: R).
    pub peak: u8,
}

/// All core-colour components of a minimap crop (no filtering).
pub fn blobs(img: &Rgba) -> Vec<Blob> {
    let (w, h) = (img.width, img.height);
    let d = &img.data;
    let mut cls = vec![0u8; w * h];
    for (i, c) in cls.iter_mut().enumerate() {
        let o = i * 4;
        *c = core_class(d[o], d[o + 1], d[o + 2]);
    }
    let mut seen = vec![false; w * h];
    let mut stack: Vec<usize> = Vec::new();
    let mut out = Vec::new();
    for start in 0..w * h {
        let c = cls[start];
        if c == 0 || seen[start] {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let (mut n, mut sx, mut sy, mut sxx, mut syy, mut sxy) = (0u32, 0f64, 0f64, 0f64, 0f64, 0f64);
        let (mut x0, mut x1, mut y0, mut y1) = (usize::MAX, 0, usize::MAX, 0);
        let (mut border, mut dark) = (0u32, 0u32);
        let mut peak = 0u8;
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            n += 1;
            peak = peak.max(if c == BLUE { d[i * 4 + 2] } else { d[i * 4] });
            let (fx, fy) = (x as f64, y as f64);
            sx += fx;
            sy += fy;
            sxx += fx * fx;
            syy += fy * fy;
            sxy += fx * fy;
            x0 = x0.min(x);
            x1 = x1.max(x);
            y0 = y0.min(y);
            y1 = y1.max(y);
            let nb = [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (y + 1 < h).then(|| i + w)];
            for j in nb {
                let Some(j) = j else {
                    border += 1;
                    continue;
                };
                if cls[j] == c {
                    if !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                } else {
                    border += 1;
                    // The outline is dark; a neighbouring dot of the other colour (enemy minion
                    // touching ours) also counts as "not map".
                    let o = j * 4;
                    if cls[j] != 0 || is_dark(d[o], d[o + 1], d[o + 2]) {
                        dark += 1;
                    } else {
                        // Anti-aliased outline: one pixel further out.
                        let (jx, jy) = ((j % w) as i64 * 2 - x as i64, (j / w) as i64 * 2 - y as i64);
                        if jx >= 0 && jy >= 0 && (jx as usize) < w && (jy as usize) < h {
                            let k = (jy as usize * w + jx as usize) * 4;
                            if is_dark(d[k], d[k + 1], d[k + 2]) {
                                dark += 1;
                            }
                        }
                    }
                }
            }
        }
        let nf = n as f64;
        let (mx, my) = (sx / nf, sy / nf);
        let (vxx, vyy, vxy) = (sxx / nf - mx * mx, syy / nf - my * my, sxy / nf - mx * my);
        let tr = vxx + vyy;
        let disc = ((vxx - vyy).powi(2) / 4.0 + vxy * vxy).sqrt();
        let (l1, l2) = (tr / 2.0 + disc, (tr / 2.0 - disc).max(0.0));
        out.push(Blob {
            blue: c == BLUE,
            area: n,
            cx: mx + 0.5,
            cy: my + 0.5,
            major: (12.0 * l1 + 1.0).sqrt(),
            minor: (12.0 * l2 + 1.0).sqrt(),
            dark_border: dark as f64 / border.max(1) as f64,
            w: (x1 - x0 + 1) as u32,
            h: (y1 - y0 + 1) as u32,
            peak,
        });
    }
    out
}

/// Number of minion dots a blob stands for (0 = not a minion).
///
/// Measured on 341 px replays: a single minion core is 11–15 px, ~3.5–4.3 × 3.1–3.7 px
/// (principal axes) with an all-dark border; two merged dots ~23 px, 8 × 3.6. Portrait ring
/// fragments, turret digits and icon outlines are thin strokes (minor axis 1–2 px).
pub fn blob_dots(b: &Blob, width: usize, p: &WaveParams) -> u32 {
    let s = width as f64 / REF_W;
    let a1 = p.dot_area * s * s;
    let area = b.area as f64;
    // Minion cores peak at ~190-215 in the team channel; brighter = pings / UI highlights.
    if area < a1 * 0.55 || b.dark_border < p.min_dark_border || b.peak > 225 {
        return 0;
    }
    if b.minor < 2.6 * s || b.minor > 5.5 * s || b.major > 26.0 * s {
        return 0;
    }
    let n = (area / a1).round().max(1.0);
    // A chain of n dots is ~4.5 px per dot long: much longer = an arc / outline.
    if b.major > (n + 1.0) * 4.8 * s {
        return 0;
    }
    n as u32
}

/// Turret and inhibitor positions (game units, both teams). Their icons carry digits / dots in
/// the team colours that look like minions; the icon centre is masked.
const TURRETS: [(f64, f64); 18] = [
    (981.0, 10441.0),
    (1512.0, 6699.0),
    (1169.0, 4287.0),
    (5846.0, 6396.0),
    (5048.0, 4812.0),
    (3651.0, 3696.0),
    (10504.0, 1029.0),
    (6919.0, 1483.0),
    (4281.0, 1253.0),
    (4318.0, 13875.0),
    (7943.0, 13411.0),
    (10481.0, 13650.0),
    (8955.0, 8510.0),
    (9767.0, 10113.0),
    (11134.0, 11207.0),
    (13866.0, 4505.0),
    (13327.0, 8226.0),
    (13624.0, 10572.0),
];
const INHIBS: [(f64, f64); 6] = [(1171.0, 3571.0), (3203.0, 3208.0), (3452.0, 1236.0), (11261.0, 13676.0), (11598.0, 11667.0), (13604.0, 11316.0)];
const MAP_UNITS: f64 = 14870.0;

/// Masked icon spots (normalized centre, radius).
fn structure_spots() -> impl Iterator<Item = (f64, f64, f64)> {
    // The turret digit sits slightly below-right of the turret's map position.
    let t = TURRETS.iter().map(|&(x, y)| (x / MAP_UNITS + 0.004, 1.0 - y / MAP_UNITS + 0.007, 0.018));
    let i = INHIBS.iter().map(|&(x, y)| (x / MAP_UNITS, 1.0 - y / MAP_UNITS, 0.022));
    t.chain(i)
}

/// Normalized point on a turret digit / inhibitor icon.
pub fn on_structure_icon(x: f64, y: f64) -> bool {
    structure_spots().any(|(sx, sy, r)| (sx - x).hypot(sy - y) < r)
}

/// Minion dots of a minimap crop. `portraits` = champion detections (normalized centres):
/// their rings have the minion colours and are masked out.
pub fn find_dots(img: &Rgba, portraits: &[(f64, f64)], p: &WaveParams) -> Vec<Dot> {
    let (w, h) = (img.width as f64, img.height as f64);
    if img.width < 32 || img.height < 32 {
        return Vec::new();
    }
    blobs(img)
        .into_iter()
        .filter_map(|b| {
            let (x, y) = (b.cx / w, b.cy / h);
            if portraits.iter().any(|&(px, py)| (px - x).hypot(py - y) < p.portrait_r) || on_structure_icon(x, y) {
                return None;
            }
            let n = blob_dots(&b, img.width, p);
            (n > 0).then_some(Dot { x, y, blue: b.blue, n })
        })
        .collect()
}

/// Group dots per lane. `flip` = we are the red side (CHAOS); `blue_is_ally` = allies have
/// blue rings / minions (relative colours, or blue side without them).
pub fn lane_waves(dots: &[Dot], flip: bool, blue_is_ally: bool, fog: Option<(&[f32], usize)>, p: &WaveParams) -> Vec<LaneWave> {
    // Per lane: (position, minions) of our / their dots.
    let mut acc: [(Vec<(f64, u32)>, Vec<(f64, u32)>); 3] = Default::default();
    for d in dots {
        let Some((i, t)) = lane_of(d.x, d.y, flip, p.corridor) else { continue };
        if d.blue == blue_is_ally {
            acc[i].0.push((t, d.n));
        } else {
            acc[i].1.push((t, d.n));
        }
    }
    (0..3)
        .map(|i| {
            let (al, el) = &acc[i];
            let count = |l: &[(f64, u32)]| l.iter().map(|d| d.1).sum::<u32>();
            let mean = |l: &[(f64, u32)]| l.iter().map(|d| d.0 * d.1 as f64).sum::<f64>() / count(l).max(1) as f64;
            let (na, ne) = (count(al), count(el));
            let (sa, se) = (mean(al) * na as f64, mean(el) * ne as f64);
            let lf = lane_front(al, el);
            let vision = fog.map(|(m, g)| {
                let n = 21;
                let vis = (0..n)
                    .filter(|k| {
                        let (bx, by) = point_at(lane_line(i), *k as f64 / (n - 1) as f64);
                        let (x, y) = if flip { (by, bx) } else { (bx, by) };
                        crate::fog::sample_mask(m, g, x, y) >= p.visible_threshold as f64
                    })
                    .count();
                (vis as f64 / n as f64 * 100.0).round() / 100.0
            });
            let r2 = |v: f64| (v * 100.0).round() / 100.0;
            let ally_pos = (na > 0).then(|| r2(sa / na as f64));
            let enemy_pos = (ne > 0).then(|| r2(se / ne as f64));
            let front = lf.map(|(f, _, _)| r2(f));
            let state = match lf {
                None => {
                    if vision.is_some_and(|v| v >= 0.6) {
                        WaveState::NoWave
                    } else {
                        WaveState::Unknown
                    }
                }
                Some((f, wa, we)) => {
                    // A lone enemy wave deep on our half is at our tower; a lone ally wave on
                    // our half is just walking up (no enemy minions in its vision yet).
                    if we && (f <= 0.39 || (!wa && f <= 0.42)) {
                        WaveState::UnderOurTower
                    } else if wa && f >= 0.62 {
                        WaveState::Crashing
                    } else if f >= 0.56 {
                        WaveState::Pushing
                    } else if f <= 0.45 && we {
                        WaveState::OurSide
                    } else {
                        WaveState::Even
                    }
                }
            };
            LaneWave { lane: LANES[i], ally: na, enemy: ne, ally_pos, enemy_pos, front, state, vision }
        })
        .collect()
}

/// Where the waves of a lane are: (position, our wave there, their wave there).
///
/// Dots are grouped along the lane (gap > 0.07 starts a new group; a lane can hold several
/// waves). The biggest group with both colours (>= 3 minions) is the clash: its mean
/// position. Without a clash: the front of our biggest wave (most advanced dot) and/or of
/// theirs (>= 2 minions), their midpoint when both exist (waves walking towards each other).
fn lane_front(al: &[(f64, u32)], el: &[(f64, u32)]) -> Option<(f64, bool, bool)> {
    let mut all: Vec<(f64, u32, bool)> = al.iter().map(|&(t, n)| (t, n, true)).chain(el.iter().map(|&(t, n)| (t, n, false))).collect();
    if all.is_empty() {
        return None;
    }
    all.sort_by(|a, b| a.0.total_cmp(&b.0));
    // One group per wave: minions per side, sum of t*n, our most / their least advanced dot.
    struct G {
        na: u32,
        ne: u32,
        st: f64,
        a_max: f64,
        e_min: f64,
    }
    let mut groups: Vec<G> = Vec::new();
    let mut last = f64::NEG_INFINITY;
    for &(t, n, ours) in &all {
        if t - last > 0.07 || groups.is_empty() {
            groups.push(G { na: 0, ne: 0, st: 0.0, a_max: f64::NEG_INFINITY, e_min: f64::INFINITY });
        }
        last = t;
        let g = groups.last_mut().unwrap();
        g.st += t * n as f64;
        if ours {
            g.na += n;
            g.a_max = g.a_max.max(t);
        } else {
            g.ne += n;
            g.e_min = g.e_min.min(t);
        }
    }
    if let Some(g) = groups.iter().filter(|g| g.na >= 1 && g.ne >= 1 && g.na + g.ne >= 3).max_by_key(|g| g.na + g.ne) {
        return Some((g.st / (g.na + g.ne) as f64, true, true));
    }
    // Outside a clash a lone dot is a straggler or a false positive: not a wave.
    let min_n = 2;
    // Biggest wave of a side; ties -> the most advanced one.
    let ours = groups.iter().filter(|g| g.na >= min_n).max_by(|a, b| a.na.cmp(&b.na).then(a.a_max.total_cmp(&b.a_max))).map(|g| g.a_max);
    let theirs = groups.iter().filter(|g| g.ne >= min_n).max_by(|a, b| a.ne.cmp(&b.ne).then(b.e_min.total_cmp(&a.e_min))).map(|g| g.e_min);
    match (ours, theirs) {
        (Some(a), Some(e)) => Some(((a + e) / 2.0, true, true)),
        (Some(a), None) => Some((a, true, false)),
        (None, Some(e)) => Some((e, false, true)),
        (None, None) => None,
    }
}

/// Full reading of one minimap crop.
pub fn read_waves(img: &Rgba, portraits: &[(f64, f64)], flip: bool, blue_is_ally: bool, fog: Option<(&[f32], usize)>, p: &WaveParams) -> WaveReading {
    let dots = find_dots(img, portraits, p);
    let lanes = lane_waves(&dots, flip, blue_is_ally, fog, p);
    WaveReading { lanes, dots }
}

/// Short temporal vote over consecutive readings: a portrait walking over a wave, a ping or a
/// kill skull flips one frame, not the state. Per lane: the most frequent state of the window
/// (ties: the newest), median counts, positions of the newest frame with that state.
pub struct WaveSmoother {
    window_ms: u64,
    hist: std::collections::VecDeque<(u64, WaveReading)>,
}

impl WaveSmoother {
    pub fn new(window_ms: u64) -> WaveSmoother {
        WaveSmoother { window_ms, hist: std::collections::VecDeque::new() }
    }

    pub fn reset(&mut self) {
        self.hist.clear();
    }

    pub fn push(&mut self, at: u64, r: WaveReading) -> WaveReading {
        if self.hist.back().is_some_and(|(t, _)| *t > at) {
            self.hist.clear();
        }
        self.hist.push_back((at, r));
        while self.hist.front().is_some_and(|(t, _)| at - *t > self.window_ms) {
            self.hist.pop_front();
        }
        let newest = &self.hist.back().unwrap().1;
        let lanes = newest
            .lanes
            .iter()
            .enumerate()
            .map(|(i, cur)| {
                let ls: Vec<&LaneWave> = self.hist.iter().filter_map(|(_, r)| r.lanes.get(i)).collect();
                let mut best = (cur.state, 0usize);
                // Newest first: on a tie the newer state wins.
                for l in ls.iter().rev() {
                    let c = ls.iter().filter(|x| x.state == l.state).count();
                    if c > best.1 {
                        best = (l.state, c);
                    }
                }
                let src = ls.iter().rev().find(|l| l.state == best.0).copied().unwrap_or(cur);
                let median = |f: &dyn Fn(&LaneWave) -> u32| {
                    let mut v: Vec<u32> = ls.iter().map(|l| f(l)).collect();
                    v.sort_unstable();
                    v[v.len() / 2]
                };
                LaneWave { ally: median(&|l| l.ally), enemy: median(&|l| l.enemy), vision: cur.vision, ..src.clone() }
            })
            .collect();
        WaveReading { lanes, dots: newest.dots.clone() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Beige map background with a minion dot drawn like the game does (dark outline, core).
    fn map(w: usize) -> Rgba {
        let mut img = Rgba::new(w, w);
        for p in img.data.chunks_exact_mut(4) {
            p.copy_from_slice(&[0x91, 0x9b, 0x6e, 255]);
        }
        img
    }

    fn disc(img: &mut Rgba, cx: f64, cy: f64, r: f64, c: [u8; 3]) {
        let w = img.width;
        for y in 0..img.height {
            for x in 0..w {
                if (x as f64 + 0.5 - cx).hypot(y as f64 + 0.5 - cy) <= r {
                    let o = (y * w + x) * 4;
                    img.data[o..o + 3].copy_from_slice(&c);
                }
            }
        }
    }

    const BLUE_C: [u8; 3] = [5, 142, 204];
    const RED_C: [u8; 3] = [205, 62, 61];

    fn minion(img: &mut Rgba, x: f64, y: f64, c: [u8; 3]) {
        let s = img.width as f64 / REF_W;
        disc(img, x * img.width as f64, y * img.height as f64, 3.0 * s, [0x10, 0x30, 0x30]);
        disc(img, x * img.width as f64, y * img.height as f64, 2.0 * s, c);
    }

    fn ring(img: &mut Rgba, x: f64, y: f64, c: [u8; 3]) {
        let w = img.width as f64;
        let (cx, cy, r) = (x * w, y * w, 0.042 * w);
        for yy in 0..img.height {
            for xx in 0..img.width {
                let d = (xx as f64 + 0.5 - cx).hypot(yy as f64 + 0.5 - cy);
                let o = (yy * img.width + xx) * 4;
                if (d - r).abs() <= 1.3 {
                    img.data[o..o + 3].copy_from_slice(&c);
                } else if d < r - 1.3 {
                    img.data[o..o + 3].copy_from_slice(&[120, 90, 80]);
                }
            }
        }
    }

    #[test]
    fn lane_projection_and_mirror() {
        // Blue side: a point on the left edge near our top inhib turret is top lane, low t.
        let (i, t) = lane_of(0.085, 0.65, false, 0.05).unwrap();
        assert_eq!((LANES[i], t < 0.1), ("top", true));
        // Same screen point for the red side is (mirrored) near THEIR top inhib: top, high t.
        let (i, t) = lane_of(0.65, 0.085, true, 0.05).unwrap();
        assert_eq!((LANES[i], t < 0.1), ("top", true));
        let (i, t) = lane_of(0.085, 0.65, true, 0.05).unwrap();
        assert_eq!((LANES[i], t > 0.9), ("top", true));
        // Centre of the map = middle of mid lane; bottom-right corner = middle of bot.
        let (i, t) = lane_of(0.5, 0.5, false, 0.05).unwrap();
        assert_eq!(LANES[i], "mid");
        assert!((t - 0.5).abs() < 0.02);
        let (i, t) = lane_of(0.9, 0.9, true, 0.05).unwrap();
        assert_eq!(LANES[i], "bot");
        assert!((t - 0.5).abs() < 0.03);
        // Jungle: no lane.
        assert!(lane_of(0.3, 0.4, false, 0.05).is_none());
    }

    #[test]
    fn synthetic_frame_counts_and_states() {
        for w in [341usize, 260, 420] {
            let mut img = map(w);
            // Top: our 4 minions meet 3 enemy ones just past the corner (enemy half, before their tower).
            for k in 0..4 {
                minion(&mut img, 0.13 + k as f64 * 0.025, 0.085, BLUE_C);
            }
            for k in 0..3 {
                minion(&mut img, 0.225 + k as f64 * 0.02, 0.085, RED_C);
            }
            // Bot: an enemy wave of 5 at our bot inner turret (x ~0.42), touching dots.
            for k in 0..5 {
                minion(&mut img, 0.36 + k as f64 * 5.0 / REF_W, 0.912, RED_C);
            }
            // Mid: nothing; a champion portrait (blue ring) in mid must not count.
            ring(&mut img, 0.5, 0.5, BLUE_C);
            // Jungle dot: ignored by lane grouping.
            minion(&mut img, 0.3, 0.42, RED_C);
            let fog_all = vec![1.0f32; 64 * 64];
            let r = read_waves(&img, &[(0.5, 0.5)], false, true, Some((&fog_all, 64)), &WaveParams::default());
            let top = r.lane("top").unwrap();
            assert_eq!((top.ally, top.enemy), (4, 3), "w={w} {top:?}");
            assert_eq!(top.state, WaveState::Pushing, "w={w} {top:?}");
            let bot = r.lane("bot").unwrap();
            assert_eq!((bot.ally, bot.enemy), (0, 5), "w={w} {bot:?} {:?}", r.dots);
            assert_eq!(bot.state, WaveState::UnderOurTower, "w={w}");
            let mid = r.lane("mid").unwrap();
            assert_eq!((mid.ally, mid.enemy, mid.state), (0, 0, WaveState::NoWave), "w={w} {mid:?}");
            // Without the portrait hint the ring is rejected by shape alone.
            let r2 = read_waves(&img, &[], false, true, Some((&fog_all, 64)), &WaveParams::default());
            assert_eq!(r2.lane("mid").unwrap().ally, 0, "w={w}");
            // No vision anywhere: mid is unknown, not empty.
            let fog_none = vec![0.0f32; 64 * 64];
            let r3 = read_waves(&img, &[(0.5, 0.5)], false, true, Some((&fog_none, 64)), &WaveParams::default());
            assert_eq!(r3.lane("mid").unwrap().state, WaveState::Unknown);
        }
    }

    #[test]
    fn red_side_mirrors_positions() {
        // Red side, relative colours: allies blue. Our wave pushing bot towards the BLUE base
        // (= the enemy base) sits on the bottom edge, left part of the map.
        let mut img = map(341);
        for k in 0..3 {
            minion(&mut img, 0.34 + k as f64 * 0.025, 0.912, BLUE_C);
        }
        let r = read_waves(&img, &[], true, true, None, &WaveParams::default());
        let bot = r.lane("bot").unwrap();
        assert_eq!(bot.ally, 3);
        assert!(bot.ally_pos.unwrap() > 0.85, "{bot:?}");
        assert_eq!(bot.state, WaveState::Crashing);
        // Blue side: the same dots are our wave at home (no enemy seen): walking, "even".
        let r = read_waves(&img, &[], false, true, None, &WaveParams::default());
        let bot = r.lane("bot").unwrap();
        assert!(bot.ally_pos.unwrap() < 0.15);
        assert_eq!(bot.state, WaveState::Even);
        // No fog mask and no dots: unknown.
        assert_eq!(r.lane("top").unwrap().state, WaveState::Unknown);
    }

    #[test]
    fn rejects_non_minion_shapes() {
        let mut img = map(341);
        // Camera rectangle (white) and a big red blob (pinged objective) are not minions.
        for x in 100..200 {
            let o = (150 * 341 + x) * 4;
            img.data[o..o + 3].copy_from_slice(&[255, 255, 255]);
        }
        disc(&mut img, 30.0, 300.0, 9.0, RED_C);
        // A long thin red line (outline of an icon) on the bot lane.
        for x in 150..200 {
            let o = (311 * 341 + x) * 4;
            img.data[o..o + 3].copy_from_slice(&RED_C);
            let o = (312 * 341 + x) * 4;
            img.data[o..o + 3].copy_from_slice(&[0, 0, 0]);
        }
        assert!(find_dots(&img, &[], &WaveParams::default()).is_empty(), "{:?}", find_dots(&img, &[], &WaveParams::default()));
    }

    #[test]
    fn smoother_votes_out_single_frame_flips() {
        let lane = |state: WaveState, ally: u32| LaneWave { lane: "mid", ally, enemy: 3, ally_pos: Some(0.5), enemy_pos: Some(0.5), front: Some(0.5), state, vision: Some(1.0) };
        let reading = |state, ally| WaveReading { lanes: vec![lane(state, ally)], dots: Vec::new() };
        let mut s = WaveSmoother::new(2000);
        s.push(0, reading(WaveState::Even, 4));
        s.push(200, reading(WaveState::Even, 5));
        let out = s.push(400, reading(WaveState::UnderOurTower, 0));
        assert_eq!(out.lanes[0].state, WaveState::Even);
        assert_eq!(out.lanes[0].ally, 4);
        // The new state holds: it takes over once it has the majority.
        s.push(600, reading(WaveState::UnderOurTower, 0));
        let out = s.push(800, reading(WaveState::UnderOurTower, 1));
        assert_eq!(out.lanes[0].state, WaveState::UnderOurTower);
        // Old frames leave the window; time going backwards (new game) resets.
        let out = s.push(100, reading(WaveState::Pushing, 6));
        assert_eq!((out.lanes[0].state, out.lanes[0].ally), (WaveState::Pushing, 6));
    }
}
