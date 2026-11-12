//! Particle filter predicting where an unseen enemy is (port of `src/main/tracker/predictor.ts`).
//!
//! Particles start at the last sighting (or the fountain), each follows one behaviour (walk to
//! a goal, stay around where it vanished, recall), moving along walkable terrain at the
//! champion's move speed; goals come from a role/phase/objective prior; particles inside areas
//! we currently see are down-weighted. Output: densest area + radius holding ~70% of the mass.

use super::terrain::GridTerrain;
use crate::detect::Team;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::f64::consts::PI;
use std::sync::Arc;

/// Summoner's Rift side length in game units.
pub const MAP_UNITS: f64 = 14870.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum GoalKind {
    #[serde(rename = "camp")]
    Camp,
    #[serde(rename = "lane-top")]
    LaneTop,
    #[serde(rename = "lane-mid")]
    LaneMid,
    #[serde(rename = "lane-bot")]
    LaneBot,
    #[serde(rename = "dragon")]
    Dragon,
    #[serde(rename = "baron")]
    Baron,
    #[serde(rename = "base")]
    Base,
    #[serde(rename = "invade")]
    Invade,
    #[serde(rename = "river")]
    River,
    #[serde(rename = "stay")]
    Stay,
}

impl GoalKind {
    fn is_lane(self) -> bool {
        matches!(self, GoalKind::LaneTop | GoalKind::LaneMid | GoalKind::LaneBot)
    }
}

pub type KindMap = BTreeMap<GoalKind, f64>;

#[derive(Clone, Debug, PartialEq)]
pub struct Goal {
    pub id: String,
    pub kind: GoalKind,
    pub x: f64,
    pub y: f64,
    pub neutral: bool,
}

fn g(id: &str, kind: GoalKind, x: f64, y: f64) -> Goal {
    Goal { id: id.into(), kind, x, y, neutral: false }
}

/// Goals in minimap coords for an enemy on the given team (CHAOS = red, top-right base).
pub fn goals_for(team: Team) -> Vec<Goal> {
    use GoalKind::*;
    let red = vec![
        g("own-blue-buff", Camp, 0.75, 0.53),
        g("own-gromp", Camp, 0.82, 0.49),
        g("own-wolves", Camp, 0.72, 0.43),
        g("own-raptors", Camp, 0.53, 0.36),
        g("own-red-buff", Camp, 0.48, 0.26),
        g("own-krugs", Camp, 0.44, 0.17),
        g("enemy-blue-buff", Invade, 0.25, 0.47),
        g("enemy-raptors", Invade, 0.47, 0.62),
        g("enemy-red-buff", Invade, 0.52, 0.74),
        g("base", Base, 0.95, 0.05),
    ];
    let neutral_points = vec![
        g("top-1", LaneTop, 0.08, 0.45),
        g("top-2", LaneTop, 0.08, 0.25),
        g("top-3", LaneTop, 0.12, 0.10),
        g("top-4", LaneTop, 0.28, 0.07),
        g("top-5", LaneTop, 0.45, 0.07),
        g("mid-1", LaneMid, 0.38, 0.62),
        g("mid-2", LaneMid, 0.50, 0.50),
        g("mid-3", LaneMid, 0.62, 0.38),
        g("bot-1", LaneBot, 0.55, 0.93),
        g("bot-2", LaneBot, 0.72, 0.93),
        g("bot-3", LaneBot, 0.88, 0.90),
        g("bot-4", LaneBot, 0.92, 0.75),
        g("bot-5", LaneBot, 0.92, 0.55),
        g("dragon", Dragon, 0.67, 0.70),
        g("baron", Baron, 0.33, 0.30),
        g("river-top", River, 0.30, 0.38),
        g("river-bot", River, 0.70, 0.62),
    ];
    let side: Vec<Goal> = if team == Team::Chaos { red } else { red.into_iter().map(|g| Goal { x: 1.0 - g.x, y: 1.0 - g.y, ..g }).collect() };
    side.into_iter().chain(neutral_points.into_iter().map(|g| Goal { neutral: true, ..g })).collect()
}

pub fn fountain_of(team: Team) -> (f64, f64) {
    if team == Team::Chaos {
        (0.95, 0.05)
    } else {
        (0.05, 0.95)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Role {
    Top,
    Jungle,
    Middle,
    Bottom,
    Utility,
    Unknown,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Top => "TOP",
            Role::Jungle => "JUNGLE",
            Role::Middle => "MIDDLE",
            Role::Bottom => "BOTTOM",
            Role::Utility => "UTILITY",
            Role::Unknown => "UNKNOWN",
        }
    }
    pub fn parse(s: &str) -> Role {
        match s {
            "TOP" => Role::Top,
            "JUNGLE" => Role::Jungle,
            "MIDDLE" => Role::Middle,
            "BOTTOM" => Role::Bottom,
            "UTILITY" => Role::Utility,
            _ => Role::Unknown,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct PriorContext {
    pub role: Option<Role>,
    pub game_time: f64,
    /// Seconds until dragon / baron (re)spawns; <= 0 = alive now; None unknown.
    pub dragon_in: Option<f64>,
    pub baron_in: Option<f64>,
    pub likely_recall: bool,
    /// Learned / profile multipliers per goal kind (default 1).
    pub learned: KindMap,
    pub start_top_side: Option<f64>,
    /// Just left the fountain: nobody stays or recalls again.
    pub leaving_base: bool,
}

/// Tunable behaviour constants (fitted on harvested replays).
pub mod tuning {
    pub const STAY_WEIGHT: f64 = 2.0;
    pub const RECALL_WEIGHT: f64 = 1.2;
    pub const RECALL_CHANNEL: f64 = 8.0;
    pub const WANDER_FRAC: f64 = 0.35;
    pub const WANDER_RADIUS: f64 = 0.04;
    pub const LEAVE_GOAL_RATE: f64 = 0.06;
    pub const LEAVE_STAY_RATE: f64 = 0.05;
    pub const SPEED_MIN: f64 = 0.55;
}
use tuning::*;

fn table(role: Role, kind: GoalKind) -> Option<f64> {
    use GoalKind::*;
    let v = match role {
        Role::Jungle => match kind {
            Camp => 5.0,
            LaneTop | LaneMid | LaneBot => 1.2,
            Dragon => 1.5,
            Baron => 1.0,
            Base => 0.5,
            Invade => 0.8,
            River => 1.5,
            Stay => return None,
        },
        Role::Top => match kind {
            Camp => 0.3,
            LaneTop => 8.0,
            LaneMid => 0.4,
            LaneBot => 0.1,
            Dragon => 0.2,
            Baron => 0.6,
            Base => RECALL_WEIGHT,
            Invade => 0.2,
            River => 0.6,
            Stay => return None,
        },
        Role::Middle => match kind {
            Camp => 0.5,
            LaneTop => 0.8,
            LaneMid => 8.0,
            LaneBot => 0.8,
            Dragon => 0.8,
            Baron => 0.6,
            Base => RECALL_WEIGHT,
            Invade => 0.3,
            River => 1.2,
            Stay => return None,
        },
        Role::Bottom => match kind {
            Camp => 0.2,
            LaneTop => 0.1,
            LaneMid => 0.4,
            LaneBot => 8.0,
            Dragon => 1.0,
            Baron => 0.2,
            Base => RECALL_WEIGHT,
            Invade => 0.1,
            River => 0.4,
            Stay => return None,
        },
        Role::Utility => match kind {
            Camp => 0.3,
            LaneTop => 0.2,
            LaneMid => 0.6,
            LaneBot => 6.0,
            Dragon => 1.5,
            Baron => 0.4,
            Base => RECALL_WEIGHT,
            Invade => 0.6,
            River => 2.0,
            Stay => return None,
        },
        Role::Unknown => return None,
    };
    Some(v)
}

/// Prior weight of each goal kind for an enemy, before learning.
pub fn goal_prior(kind: GoalKind, c: &PriorContext) -> f64 {
    let early = c.game_time < 14.0 * 60.0;
    if c.leaving_base && (kind == GoalKind::Stay || kind == GoalKind::Base) {
        return 0.01;
    }
    if kind == GoalKind::Stay {
        return STAY_WEIGHT * c.learned.get(&GoalKind::Stay).copied().unwrap_or(1.0);
    }
    let mut w = table(c.role.unwrap_or(Role::Unknown), kind).unwrap_or(1.0);
    if kind == GoalKind::Base && c.game_time < 3.5 * 60.0 {
        w *= 0.2;
    }
    if !early && kind.is_lane() {
        w = (w * 0.6).max(1.5);
    }
    if matches!(c.dragon_in, Some(d) if d < 50.0) && kind == GoalKind::Dragon {
        w *= 4.0;
    }
    if matches!(c.baron_in, Some(d) if d < 50.0) && kind == GoalKind::Baron {
        w *= 4.0;
    }
    if c.likely_recall && kind == GoalKind::Base {
        w *= 5.0;
    }
    w *= c.learned.get(&kind).copied().unwrap_or(1.0);
    w.max(0.01)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Particle {
    pub x: f64,
    pub y: f64,
    pub goal: usize,
    pub w: f64,
    pub arrived: bool,
}

/// One group of the particle cloud: where a fogged enemy may be, with its share of the probability.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Zone {
    pub x: f64,
    pub y: f64,
    pub radius: f64,
    /// Share of the probability in this zone, 0..1.
    pub mass: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Estimate {
    pub x: f64,
    pub y: f64,
    pub radius: f64,
    pub confidence: f64,
    pub spread: f64,
}

/// Deterministic xorshift PRNG, bit-identical to the TS `rng(seed)`.
#[derive(Clone, Debug)]
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Rng {
        Rng(if seed == 0 { 1 } else { seed })
    }
    #[inline]
    pub fn next(&mut self) -> f64 {
        let mut s = self.0;
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        self.0 = s;
        (s % 1_000_000) as f64 / 1_000_000.0
    }
}

#[inline]
fn clamp01(v: f64) -> f64 {
    v.max(0.001).min(0.999)
}

const NB8: [(i32, i32); 8] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)];

pub struct ParticleFilter {
    pub particles: Vec<Particle>,
    terrain: Arc<GridTerrain>,
    goals: Vec<Goal>,
    fields: Vec<Arc<Vec<f32>>>,
    goal_w: Vec<f64>,
    elapsed: f64,
    fountain: (f64, f64),
    n: usize,
}

impl ParticleFilter {
    pub fn new(terrain: Arc<GridTerrain>, base_goals: Vec<Goal>, n: usize) -> ParticleFilter {
        let mut goals = base_goals;
        let fountain = goals.iter().find(|g| g.kind == GoalKind::Base).map(|g| (g.x, g.y)).unwrap_or((0.95, 0.05));
        // Last goal slot is the dynamic "stay" goal (set on reset).
        goals.push(Goal { id: "stay".into(), kind: GoalKind::Stay, x: 0.5, y: 0.5, neutral: false });
        let fields = goals.iter().map(|g| terrain.field_to(g.x, g.y)).collect();
        ParticleFilter { particles: Vec::new(), terrain, goals, fields, goal_w: Vec::new(), elapsed: 0.0, fountain, n }
    }

    pub fn goals(&self) -> &[Goal] {
        &self.goals
    }

    /// Start from a point with a small spread. `heading` favours goals ahead of the last motion.
    pub fn reset(&mut self, at: (f64, f64), ctx: &PriorContext, heading: Option<(f64, f64)>, rng: &mut Rng) {
        let si = self.goals.len() - 1;
        self.goals[si] = Goal { id: "stay".into(), kind: GoalKind::Stay, x: at.0, y: at.1, neutral: false };
        self.fields[si] = self.terrain.field_to(at.0, at.1);
        self.elapsed = 0.0;
        self.set_prior(ctx, heading.map(|h| (at, h)));
        let n = self.n;
        self.particles = (0..n)
            .map(|_| {
                let (x, y) = self.walkable_near((clamp01(at.0 + (rng.next() - 0.5) * 0.01), clamp01(at.1 + (rng.next() - 0.5) * 0.01)), at);
                let goal = self.sample_goal(None, rng);
                Particle { x, y, goal, w: 1.0 / n as f64, arrived: false }
            })
            .collect();
    }

    /// Jittered point if walkable, else the unjittered one, else the nearest walkable cell.
    fn walkable_near(&self, q: (f64, f64), fallback: (f64, f64)) -> (f64, f64) {
        if self.terrain.walkable(q.0, q.1) {
            return q;
        }
        if self.terrain.walkable(fallback.0, fallback.1) {
            return fallback;
        }
        self.nearest_walkable(fallback)
    }

    /// Nearest walkable point by a small ring search (up to ~0.06 map); the point itself if none.
    fn nearest_walkable(&self, q: (f64, f64)) -> (f64, f64) {
        let step = 0.004;
        for ring in 1..=15 {
            let r = ring as f64 * step;
            for k in 0..(8 * ring) {
                let a = k as f64 / (8 * ring) as f64 * std::f64::consts::TAU;
                let (x, y) = (clamp01(q.0 + r * a.cos()), clamp01(q.1 + r * a.sin()));
                if self.terrain.walkable(x, y) {
                    return (x, y);
                }
            }
        }
        q
    }

    /// Goal-kind shares the filter actually samples from (role/time prior, learned multipliers,
    /// heading, the dynamic Stay goal): what the learner must compare credit against.
    pub fn prior_by_kind(&self) -> KindMap {
        let mut out = KindMap::new();
        for (g, w) in self.goals.iter().zip(&self.goal_w) {
            *out.entry(g.kind).or_insert(0.0) += *w;
        }
        out
    }

    /// Update goal weights. `motion` = (position, heading velocity).
    pub fn set_prior(&mut self, ctx: &PriorContext, motion: Option<((f64, f64), (f64, f64))>) {
        let mut count: BTreeMap<GoalKind, f64> = BTreeMap::new();
        for g in &self.goals {
            *count.entry(g.kind).or_insert(0.0) += 1.0;
        }
        self.goal_w = self
            .goals
            .iter()
            .map(|g| {
                let mut w = goal_prior(g.kind, ctx) / count[&g.kind];
                if let Some((at, (vx, vy))) = motion {
                    if g.kind != GoalKind::Stay && g.kind != GoalKind::Base {
                        let sp = vx.hypot(vy);
                        if sp > 1e-4 {
                            let (dx, dy) = (g.x - at.0, g.y - at.1);
                            let mut d = dx.hypot(dy);
                            if d == 0.0 {
                                d = 1.0;
                            }
                            let cos = (dx * vx + dy * vy) / (d * sp);
                            w *= 0.35 + 1.3 * cos.max(0.0);
                        }
                    }
                }
                w
            })
            .collect();
    }

    fn sample_goal(&self, exclude: Option<usize>, rng: &mut Rng) -> usize {
        let mut total = 0.0;
        for (i, w) in self.goal_w.iter().enumerate() {
            if Some(i) != exclude {
                total += w;
            }
        }
        let mut r = rng.next() * total;
        for (i, w) in self.goal_w.iter().enumerate() {
            if Some(i) == exclude {
                continue;
            }
            r -= w;
            if r <= 0.0 {
                return i;
            }
        }
        self.goal_w.len() - 2
    }

    /// Advance by dt seconds at move speed (game units/s).
    pub fn step(&mut self, mut dt: f64, move_speed: f64, rng: &mut Rng) {
        while dt > 1.0 {
            self.step_once(1.0, move_speed, rng);
            dt -= 1.0;
        }
        if dt > 0.0 {
            self.step_once(dt, move_speed, rng);
        }
    }

    fn step_once(&mut self, dt: f64, move_speed: f64, rng: &mut Rng) {
        self.elapsed += dt;
        let step_len = (move_speed / MAP_UNITS) * dt;
        let t = self.terrain.clone();
        let cell = 1.0 / t.size as f64;
        let stay_idx = self.goals.len() - 1;
        for pi in 0..self.particles.len() {
            let mut p = self.particles[pi];
            let goal = &self.goals[p.goal];
            if goal.kind == GoalKind::Base && self.elapsed >= RECALL_CHANNEL + (p.w * 1e6 % 3.0) {
                p.x = clamp01(self.fountain.0 + (rng.next() - 0.5) * 0.02);
                p.y = clamp01(self.fountain.1 + (rng.next() - 0.5) * 0.02);
                p.arrived = true;
                self.particles[pi] = p;
                continue;
            }
            if p.arrived || goal.kind == GoalKind::Stay {
                let leave = if goal.kind == GoalKind::Stay { LEAVE_STAY_RATE } else { LEAVE_GOAL_RATE };
                if rng.next() < dt * leave {
                    p.goal = self.sample_goal(Some(stay_idx), rng);
                    p.arrived = false;
                    self.particles[pi] = p;
                    continue;
                }
                let a = rng.next() * PI * 2.0;
                let l = step_len * WANDER_FRAC * rng.next();
                let nx = clamp01(p.x + a.cos() * l);
                let ny = clamp01(p.y + a.sin() * l);
                if t.walkable(nx, ny) && (nx - goal.x).hypot(ny - goal.y) < WANDER_RADIUS + 0.01 {
                    p.x = nx;
                    p.y = ny;
                }
                self.particles[pi] = p;
                continue;
            }
            let mut remaining = step_len * (SPEED_MIN + (1.0 - SPEED_MIN) * rng.next());
            let field = &self.fields[p.goal];
            let mut guard = 0;
            while remaining > 1e-6 && guard < 12 {
                guard += 1;
                let here = t.sample(field, p.x, p.y);
                if here < WANDER_RADIUS * 0.5 {
                    p.arrived = true;
                    break;
                }
                let (mut bx, mut by, mut best) = (0i32, 0i32, here);
                for &(dx, dy) in &NB8 {
                    // No corner cutting: a diagonal step needs both orthogonal neighbours walkable.
                    if dx != 0 && dy != 0 && (!t.walkable(p.x + dx as f64 * cell, p.y) || !t.walkable(p.x, p.y + dy as f64 * cell)) {
                        continue;
                    }
                    let v = t.sample(field, p.x + dx as f64 * cell, p.y + dy as f64 * cell);
                    if v < best {
                        best = v;
                        bx = dx;
                        by = dy;
                    }
                }
                if bx == 0 && by == 0 {
                    break;
                }
                let h = (bx as f64).hypot(by as f64);
                let len = h * cell;
                let mv = remaining.min(len);
                let (nx, ny) = (clamp01(p.x + (bx as f64 / h) * mv), clamp01(p.y + (by as f64 / h) * mv));
                // A partial step can end inside a wall cell even when the target cell is free.
                if !t.walkable(nx, ny) {
                    break;
                }
                p.x = nx;
                p.y = ny;
                remaining -= mv;
            }
            self.particles[pi] = p;
        }
    }

    /// Negative evidence: `visible(x,y)` = P(point in our vision now). Returns false when every
    /// hypothesis is contradicted (weights left unnormalised, as in TS).
    pub fn observe_absent(&mut self, visible: &dyn Fn(f64, f64) -> f64, rng: &mut Rng) -> bool {
        let before: f64 = self.particles.iter().map(|p| p.w).sum();
        // Mass in cells the fog reader calls visible (>= 0.52, fog.rs): if nearly all of it is there and
        // nobody is seen, the hypothesis is contradicted even when v < 1 (v = 0.8 used to survive).
        let seen: f64 = self.particles.iter().filter(|p| visible(clamp01(p.x), clamp01(p.y)) >= 0.52).map(|p| p.w).sum();
        let mut total = 0.0;
        for p in self.particles.iter_mut() {
            let v = visible(clamp01(p.x), clamp01(p.y));
            let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
            p.w *= 1.0 - 0.95 * v;
            total += p.w;
        }
        // Almost all of the mass sat where we can see and nobody is there: the whole hypothesis is
        // contradicted. (Normalizing would bring the same distribution back — e.g. full map vision
        // left a ghost at ~95% confidence.)
        if !(total > 1e-9) || total < before * 0.12 || seen > before * 0.9 {
            return false;
        }
        for p in self.particles.iter_mut() {
            p.w /= total;
        }
        if self.effective_n() < self.n as f64 * 0.5 {
            self.resample(rng);
        }
        true
    }

    /// Soft evidence ("a camp near q was just cleared"): particles near q gain weight, the rest keep
    /// at least `floor` of theirs. Not a sighting: no state change, nothing to learn from.
    pub fn observe_hint(&mut self, q: (f64, f64), r: f64, floor: f64, rng: &mut Rng) {
        let mut total = 0.0;
        for p in self.particles.iter_mut() {
            let d2 = (p.x - q.0).powi(2) + (p.y - q.1).powi(2);
            p.w *= floor + (1.0 - floor) * (-d2 / (2.0 * r * r)).exp();
            total += p.w;
        }
        if total > 1e-12 {
            for p in self.particles.iter_mut() {
                p.w /= total;
            }
            if self.effective_n() < self.n as f64 * 0.5 {
                self.resample(rng);
            }
        }
    }

    /// Tests: replace the particles with equally weighted points.
    #[doc(hidden)]
    pub fn set_points(&mut self, pts: &[(f64, f64)]) {
        let w = 1.0 / pts.len().max(1) as f64;
        self.particles = pts.iter().map(|&(x, y)| Particle { x, y, goal: 0, w, arrived: false }).collect();
    }

    pub fn effective_n(&self) -> f64 {
        let s: f64 = self.particles.iter().map(|p| p.w * p.w).sum();
        1.0 / s.max(1e-12)
    }

    fn resample(&mut self, rng: &mut Rng) {
        let n = self.n;
        let step = 1.0 / n as f64;
        let mut out = Vec::with_capacity(n);
        let mut c = self.particles[0].w;
        let mut i = 0;
        let r = rng.next() * step;
        for k in 0..n {
            let u = r + k as f64 * step;
            while u > c && i < self.particles.len() - 1 {
                i += 1;
                c += self.particles[i].w;
            }
            let p = self.particles[i];
            let (x, y) = self.walkable_near((clamp01(p.x + (rng.next() - 0.5) * 0.004), clamp01(p.y + (rng.next() - 0.5) * 0.004)), (p.x, p.y));
            out.push(Particle { x, y, goal: p.goal, w: step, arrived: p.arrived });
        }
        self.particles = out;
    }

    /// Densest area (mean-shift from the weighted mean) and the radius containing `mass`.
    pub fn estimate(&self, mass: f64) -> Estimate {
        let (mut mx, mut my) = (0.0, 0.0);
        for p in &self.particles {
            mx += p.x * p.w;
            my += p.y * p.w;
        }
        for _ in 0..4 {
            let (mut sx, mut sy, mut sw) = (0.0, 0.0, 0.0);
            for p in &self.particles {
                let d2 = (p.x - mx).powi(2) + (p.y - my).powi(2);
                let k = (-d2 / (2.0 * 0.06f64.powi(2))).exp() * p.w;
                sx += p.x * k;
                sy += p.y * k;
                sw += k;
            }
            if sw > 1e-9 {
                mx = sx / sw;
                my = sy / sw;
            }
        }
        // The mean of two clusters on both sides of a wall can be inside the wall: then take the densest
        // particle (all particles are on walkable ground), not a point between the clusters.
        // Also when hardly any mass is near the center: the middle between two hidden groups (e.g. on
        // both sides of a strip we can see) is an empty place, not a guess.
        if (!self.terrain.walkable(mx, my) || self.mass_near((mx, my), 0.06) < 0.02) && !self.particles.is_empty() {
            let best = self
                .particles
                .iter()
                .filter(|p| self.terrain.walkable(p.x, p.y))
                .step_by((self.particles.len() / 80).max(1))
                .max_by(|a, b| self.mass_near((a.x, a.y), 0.05).total_cmp(&self.mass_near((b.x, b.y), 0.05)));
            (mx, my) = match best {
                Some(b) => (b.x, b.y),
                None => self.nearest_walkable((mx, my)),
            };
        }
        let mut ds: Vec<(f64, f64)> = self.particles.iter().map(|p| ((p.x - mx).hypot(p.y - my), p.w)).collect();
        ds.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let (mut acc, mut radius) = (0.0, 0.0);
        for (d, w) in ds {
            acc += w;
            radius = d;
            if acc >= mass {
                break;
            }
        }
        let spread: f64 = self.particles.iter().map(|p| p.w * (p.x - mx).hypot(p.y - my)).sum();
        Estimate { x: mx, y: my, radius: radius.max(0.02), confidence: (1.0 - radius / 0.35).clamp(0.0, 1.0), spread }
    }

/// Up to `max` separate groups of the cloud, most likely first (greedy: densest remaining
    /// particle -> mean-shift -> take its group out). Groups under 15% of the mass are dropped.
    pub fn zones(&self, max: usize) -> Vec<Zone> {
        let n = self.particles.len();
        if n == 0 {
            return Vec::new();
        }
        let total: f64 = self.particles.iter().map(|p| p.w).sum::<f64>().max(1e-12);
        let mut taken = vec![false; n];
        let mut out = Vec::new();
        let step = (n / 80).max(1);
        for _ in 0..max {
            let near_free = |q: (f64, f64), r: f64, taken: &[bool]| -> f64 {
                self.particles.iter().zip(taken).filter(|(p, t)| !**t && (p.x - q.0).hypot(p.y - q.1) <= r).map(|(p, _)| p.w).sum()
            };
            let Some(seed) = (0..n).step_by(step).filter(|&i| !taken[i]).max_by(|&a, &b| {
                let (pa, pb) = (self.particles[a], self.particles[b]);
                near_free((pa.x, pa.y), 0.05, &taken).total_cmp(&near_free((pb.x, pb.y), 0.05, &taken))
            }) else {
                break;
            };
            let (mut cx, mut cy) = (self.particles[seed].x, self.particles[seed].y);
            for _ in 0..3 {
                let (mut sx, mut sy, mut sw) = (0.0, 0.0, 0.0);
                for (p, t) in self.particles.iter().zip(&taken) {
                    if *t {
                        continue;
                    }
                    let k = (-((p.x - cx).powi(2) + (p.y - cy).powi(2)) / (2.0 * 0.05f64.powi(2))).exp() * p.w;
                    sx += p.x * k;
                    sy += p.y * k;
                    sw += k;
                }
                if sw > 1e-12 {
                    cx = sx / sw;
                    cy = sy / sw;
                }
            }
            if !self.terrain.walkable(cx, cy) {
                cx = self.particles[seed].x;
                cy = self.particles[seed].y;
            }
            let members: Vec<usize> = (0..n).filter(|&i| !taken[i] && (self.particles[i].x - cx).hypot(self.particles[i].y - cy) <= 0.12).collect();
            let mass: f64 = members.iter().map(|&i| self.particles[i].w).sum::<f64>() / total;
            if mass < 0.15 {
                break;
            }
            let spread = members.iter().map(|&i| self.particles[i].w * (self.particles[i].x - cx).hypot(self.particles[i].y - cy)).sum::<f64>() / (mass * total);
            for i in members {
                taken[i] = true;
            }
            out.push(Zone { x: cx, y: cy, radius: (spread * 1.6).max(0.02), mass });
        }
        out
    }

    /// Probability mass within r of q.
    pub fn mass_near(&self, q: (f64, f64), r: f64) -> f64 {
        self.particles.iter().filter(|p| (p.x - q.0).hypot(p.y - q.1) <= r).map(|p| p.w).sum()
    }

    /// Goal kinds of particles near q, weighted (credit assignment for learning).
    pub fn goal_kinds_near(&self, q: (f64, f64), r: f64) -> KindMap {
        let mut out = KindMap::new();
        for p in &self.particles {
            if (p.x - q.0).hypot(p.y - q.1) <= r {
                *out.entry(self.goals[p.goal].kind).or_insert(0.0) += p.w;
            }
        }
        out
    }
}
