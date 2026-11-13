//! Enemy state machine + fog predictions (port of `src/main/tracker/tracker.ts`).
//!
//!   visible: portrait detected in >= 2 of the last 3 frames
//!   fog:     not detected for >= 3 frames (and >= 0.35 s) — particle filter running
//!   dead:    from the Live API; on respawn the filter restarts at their fountain
//! A detection always wins: the moment a fogged enemy is seen again its ghost disappears,
//! and the reappearance trains the model. An item purchase while unseen is a proven recall.

use super::learning::{Learner, Observation};
use super::predictor::{fountain_of, goals_for, Estimate, KindMap, ParticleFilter, PriorContext, Rng, Role, MAP_UNITS};
use super::terrain::GridTerrain;
use crate::detect::{Detection, Team};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EnemyInput {
    pub champion: String,
    pub team: Team,
    pub role: Role,
    pub is_dead: bool,
    /// Move speed in game units/s (estimated).
    pub move_speed: f64,
    pub likely_recall: bool,
    pub start_top_side: Option<f64>,
    /// Shop value of current items (Live API — visible even in fog).
    pub item_gold: Option<f64>,
    pub has_flash: bool,
}

impl EnemyInput {
    pub fn new(champion: &str, team: Team, role: Role, move_speed: f64) -> EnemyInput {
        EnemyInput { champion: champion.into(), team, role, is_dead: false, move_speed, likely_recall: false, start_top_side: None, item_gold: None, has_flash: false }
    }
}

pub struct FrameInput<'a> {
    /// Game time in seconds.
    pub t: f64,
    pub detections: &'a [Detection],
    /// P(point currently in our vision), from the fog mask; None = no negative evidence.
    pub visible: Option<&'a dyn Fn(f64, f64) -> f64>,
    pub enemies: &'a [EnemyInput],
    pub dragon_in: Option<f64>,
    pub baron_in: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackState {
    Unknown,
    Visible,
    Fog,
    Dead,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sighting {
    pub x: f64,
    pub y: f64,
    pub t: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnemyTrack {
    pub champion: String,
    pub team: Team,
    pub role: Role,
    pub state: TrackState,
    /// Last confirmed position (normalized).
    pub last: Option<Sighting>,
    pub vel: (f64, f64),
    pub fog_since: Option<f64>,
    pub estimate: Option<Estimate>,
    /// Game time a recall was proven by an item purchase while unseen.
    pub recalled_at: Option<f64>,
    /// Game time of a probable Flash.
    pub flash_suspect_at: Option<f64>,
}

/// What the overlay draws for a fogged enemy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ghost {
    pub champion: String,
    pub x: f64,
    pub y: f64,
    pub radius: f64,
    pub age: f64,
    pub confidence: f64,
    /// Other places the enemy may well be (each >= 20% of the probability), besides the main one.
    pub zones: Vec<super::predictor::Zone>,
}

struct Internal {
    pub_: EnemyTrack,
    hits: Vec<u8>,
    pf: Option<ParticleFilter>,
    baseline_at: Option<(f64, f64)>,
    last_prior_at: f64,
    pending: Option<Detection>,
    last_item_gold: Option<f64>,
    /// Estimate frozen at the last fog frame (scored when the enemy reappears).
    frozen: Option<Estimate>,
}

pub struct EnemyTracker {
    tracks: Vec<Internal>,
    last_t: Option<f64>,
    rng: Rng,
    terrain: Arc<GridTerrain>,
    pub learner: Learner,
    particles: usize,
    /// Learn from reappearances only in real Summoner's Rift games.
    pub learning_enabled: bool,
    /// Treat a jump in an unseen enemy's item gold as a proven recall (reset to his fountain). Off: the
    /// game shows enemy items like the scoreboard, as last seen, so the jump arrives when he walks out of
    /// the fog far from base ("Sion in base" while he was top).
    pub item_recall: bool,
}

impl EnemyTracker {
    pub fn new(terrain: Arc<GridTerrain>, learner: Learner, particles: usize) -> EnemyTracker {
        EnemyTracker { tracks: Vec::new(), last_t: None, rng: Rng::new(12345), terrain, learner, particles, learning_enabled: true, item_recall: false }
    }

    pub fn reset(&mut self) {
        self.tracks.clear();
        self.last_t = None;
    }

    pub fn all(&self) -> Vec<EnemyTrack> {
        self.tracks.iter().map(|t| t.pub_.clone()).collect()
    }

    /// Fogged enemies with an estimate, at most 120 s old (what the overlay shows).
    pub fn ghosts(&self, t: f64) -> Vec<Ghost> {
        self.tracks
            .iter()
            .filter_map(|tr| {
                let p = &tr.pub_;
                let (TrackState::Fog, Some(e), Some(fs)) = (p.state, p.estimate, p.fog_since) else { return None };
                if t - fs >= 120.0 {
                    return None;
                }
                // Two or three real options (e.g. Dragon or top): the main ghost sits on the most likely
                // one and the others are shown as weaker zones instead of one circle in between.
                let mut z = tr.pf.as_ref().map(|pf| pf.zones(3)).unwrap_or_default();
                let (mut x, mut y) = (e.x, e.y);
                if z.len() >= 2 && z[1].mass >= 0.2 {
                    if (x - z[0].x).hypot(y - z[0].y) > z[0].radius {
                        x = z[0].x;
                        y = z[0].y;
                    }
                    z.remove(0);
                    z.retain(|zz| zz.mass >= 0.2);
                } else {
                    z.clear();
                }
                Some(Ghost { champion: p.champion.clone(), x, y, radius: e.radius, age: t - fs, confidence: e.confidence, zones: z })
            })
            .collect()
    }

    fn ctx_for(&self, e: &EnemyInput, f: &FrameInput) -> PriorContext {
        PriorContext {
            role: Some(e.role),
            game_time: f.t,
            dragon_in: f.dragon_in,
            baron_in: f.baron_in,
            likely_recall: e.likely_recall,
            learned: self.learner.multipliers(e.role, f.t),
            start_top_side: e.start_top_side,
            leaving_base: false,
        }
    }

    fn new_filter(&self, e: &EnemyInput) -> ParticleFilter {
        ParticleFilter::new(self.terrain.clone(), goals_for(e.team), self.particles)
    }

    pub fn update(&mut self, f: &FrameInput) -> Vec<EnemyTrack> {
        let dt = match self.last_t {
            None => 0.0,
            Some(l) => (f.t - l).clamp(0.0, 5.0),
        };
        self.last_t = Some(f.t);
        for e in f.enemies {
            let idx = match self.tracks.iter().position(|t| t.pub_.champion == e.champion) {
                Some(i) => i,
                None => {
                    let mut tr = Internal {
                        pub_: EnemyTrack { champion: e.champion.clone(), team: e.team, role: e.role, state: TrackState::Unknown, last: None, vel: (0.0, 0.0), fog_since: None, estimate: None, recalled_at: None, flash_suspect_at: None },
                        hits: Vec::new(),
                        pf: None,
                        baseline_at: None,
                        last_prior_at: -99.0,
                        pending: None,
                        last_item_gold: e.item_gold,
                        frozen: None,
                    };
                    // Game start: everyone walks out of their fountain.
                    if f.t < 100.0 {
                        let fp = fountain_of(e.team);
                        tr.pub_.state = TrackState::Fog;
                        tr.pub_.fog_since = Some(0.0);
                        tr.pub_.last = Some(Sighting { x: fp.0, y: fp.1, t: 0.0 });
                        tr.baseline_at = Some(fp);
                        let mut pf = self.new_filter(e);
                        let ctx = self.ctx_for(e, f);
                        pf.reset(fp, &ctx, None, &mut self.rng);
                        tr.pf = Some(pf);
                    }
                    self.tracks.push(tr);
                    self.tracks.len() - 1
                }
            };
            self.update_one(idx, e, f, dt);
        }
        self.all()
    }

    /// Seconds since this enemy left the fountain after a respawn or a proven recall (Homeguard), else None.
    fn homeguard_since(tr: &Internal, fp: (f64, f64), t: f64) -> Option<f64> {
        let fs = tr.pub_.fog_since?;
        let b = tr.baseline_at?;
        // Game start (fog_since 0) has no Homeguard.
        if fs <= 0.0 || (b.0 - fp.0).hypot(b.1 - fp.1) >= 0.05 { return None; }
        Some(t - fs)
    }

    fn update_one(&mut self, idx: usize, e: &EnemyInput, f: &FrameInput, dt: f64) {
        let ctx = self.ctx_for(e, f);
        let mut tr = std::mem::replace(&mut self.tracks[idx], placeholder());
        tr.pub_.role = e.role;
        let prev_item_gold = tr.last_item_gold;
        if e.item_gold.is_some() {
            tr.last_item_gold = e.item_gold;
        }
        let fp = fountain_of(e.team);

        'body: {
            // Dead: nothing to show; restart at the fountain on respawn.
            if e.is_dead {
                tr.pub_.state = TrackState::Dead;
                tr.pf = None;
                tr.pub_.estimate = None;
                tr.hits.clear();
                break 'body;
            }
            if tr.pub_.state == TrackState::Dead {
                tr.pub_.state = TrackState::Fog;
                tr.pub_.fog_since = Some(f.t);
                tr.pub_.last = Some(Sighting { x: fp.0, y: fp.1, t: f.t });
                tr.baseline_at = Some(fp);
                let mut pf = self.new_filter(e);
                pf.reset(fp, &PriorContext { leaving_base: true, ..ctx.clone() }, None, &mut self.rng);
                tr.pf = Some(pf);
            }

            // Bought something while unseen -> was in the fountain just now (proven recall).
            if let (Some(prev), Some(cur)) = (prev_item_gold, e.item_gold) {
                // A recall channel is 8 s: someone lost from sight a moment ago cannot have bought in base.
                let unseen_for = tr.pub_.fog_since.map_or(f64::INFINITY, |fs| f.t - fs);
                if self.item_recall && cur - prev >= 250.0 && tr.pub_.state != TrackState::Visible && unseen_for >= 6.0 {
                    tr.pub_.state = TrackState::Fog;
                    tr.pub_.fog_since = Some(f.t);
                    tr.pub_.last = Some(Sighting { x: fp.0, y: fp.1, t: f.t });
                    tr.baseline_at = Some(fp);
                    tr.pub_.recalled_at = Some(f.t);
                    tr.frozen = None;
                    let mut pf = self.new_filter(e);
                    pf.reset(fp, &PriorContext { leaving_base: true, ..ctx.clone() }, None, &mut self.rng);
                    tr.pf = Some(pf);
                }
            }

            let det = f.detections.iter().find(|d| d.champion == e.champion && d.team == e.team);
            tr.hits.push(det.is_some() as u8);
            if tr.hits.len() > 3 {
                tr.hits.remove(0);
            }

            if let Some(det) = det {
                let jump = match tr.pub_.last {
                    Some(l) if tr.pub_.state == TrackState::Visible => (det.x - l.x).hypot(det.y - l.y),
                    _ => 0.0,
                };
                let last_t = tr.pub_.last.map(|l| l.t).unwrap_or(f.t);
                let max_move = ((e.move_speed * 1.6) / MAP_UNITS) * (f.t - last_t).max(0.25) + 0.03;
                let unconfirmed = match &tr.pending {
                    None => true,
                    Some(p) => (p.x - det.x).hypot(p.y - det.y) > 0.03,
                };
                if jump > max_move && unconfirmed {
                    tr.pending = Some(det.clone()); // wait for confirmation
                } else {
                    tr.pending = None;
                    if e.has_flash {
                        if let Some(l) = tr.pub_.last {
                            if tr.pub_.state == TrackState::Visible && f.t - l.t <= 0.6 {
                                let walk = (e.move_speed / MAP_UNITS) * (f.t - l.t);
                                let excess = jump - walk;
                                if (0.014..=0.045).contains(&excess) && tr.pub_.flash_suspect_at.is_none_or(|fs| f.t - fs > 60.0) {
                                    tr.pub_.flash_suspect_at = Some(f.t);
                                }
                            }
                        }
                    }
                    if tr.pub_.state == TrackState::Fog && tr.pub_.fog_since.is_some() {
                        self.learn_reappearance(&tr, e, (det.x, det.y), f.t);
                    }
                    if let Some(l) = tr.pub_.last {
                        if tr.pub_.state == TrackState::Visible && f.t > l.t {
                            let (k, dtv) = (0.5, f.t - l.t);
                            tr.pub_.vel = (tr.pub_.vel.0 * (1.0 - k) + ((det.x - l.x) / dtv) * k, tr.pub_.vel.1 * (1.0 - k) + ((det.y - l.y) / dtv) * k);
                        }
                    }
                    tr.pub_.last = Some(Sighting { x: det.x, y: det.y, t: f.t });
                    let hits: u32 = tr.hits.iter().map(|&h| h as u32).sum();
                    if tr.pub_.state != TrackState::Visible && (hits >= 2 || tr.pub_.state == TrackState::Fog || tr.pub_.state == TrackState::Unknown) {
                        tr.pub_.state = TrackState::Visible;
                    }
                    tr.pf = None;
                    tr.pub_.estimate = None;
                    tr.pub_.fog_since = None;
                    tr.frozen = None;
                    break 'body;
                }
            }

            // Not seen this frame.
            let mut caught_up = false;
            if let Some(l) = tr.pub_.last {
                if tr.pub_.state == TrackState::Visible && tr.hits.iter().all(|&h| h == 0) && f.t - l.t >= 0.35 {
                    tr.pub_.state = TrackState::Fog;
                    tr.pub_.fog_since = Some(l.t);
                    tr.baseline_at = Some((l.x, l.y));
                    let mut pf = self.new_filter(e);
                    pf.reset((l.x, l.y), &ctx, Some(tr.pub_.vel), &mut self.rng);
                    tr.last_prior_at = f.t;
                    // Catch up the time between the last sighting and now.
                    pf.step(f.t - l.t, e.move_speed, &mut self.rng);
                    tr.pf = Some(pf);
                    caught_up = true;
                }
            }

            if tr.pub_.state == TrackState::Fog {
                let hg = Self::homeguard_since(&tr, fp, f.t);
                if let Some(pf) = tr.pf.as_mut() {
                    if f.t - tr.last_prior_at > 5.0 {
                        let from_base = tr.baseline_at.is_some_and(|b| (b.0 - fp.0).hypot(b.1 - fp.1) < 0.05) && f.t - tr.pub_.fog_since.unwrap_or(f.t) < 25.0;
                        pf.set_prior(&PriorContext { leaving_base: from_base, ..ctx.clone() }, None);
                        tr.last_prior_at = f.t;
                    }
                    // The catch-up step above already moved it up to now.
                    if dt > 0.0 && !caught_up {
                        let mult = hg.map_or(1.0, |a| homeguard_mult(a - dt * 0.5, f.t));
                        pf.step(dt, e.move_speed * mult, &mut self.rng);
                    }
                    let mut contradicted = false;
                    if let Some(vis) = f.visible {
                        let ok = pf.observe_absent(vis, &mut self.rng);
                        if !ok {
                            if let Some(l) = tr.pub_.last {
                                // Everything contradicted: restart broadly from the last sighting.
                                pf.reset((l.x, l.y), &ctx, None, &mut self.rng);
                                pf.step((f.t - l.t).min(60.0), e.move_speed, &mut self.rng);
                            }
                            // Still all in sight (e.g. full vision): no honest guess -> no ghost.
                            contradicted = !pf.observe_absent(vis, &mut self.rng);
                        }
                    }
                    // Calibrated: the particle spread alone is overconfident; a speed x time floor
                    // brings truth-inside to ~70% for the first 15 s in fog.
                    if contradicted {
                        tr.pub_.estimate = None;
                        tr.frozen = None;
                    } else {
                        let est = pf.estimate(0.7);
                        let age = f.t - tr.pub_.fog_since.unwrap_or(f.t);
                        let extra = hg.map_or(0.0, |a| homeguard_extra_secs(a, f.t));
                        let est = Estimate { radius: est.radius.max((e.move_speed / MAP_UNITS) * (age + extra)).min(0.45), ..est };
                        tr.pub_.estimate = Some(est);
                        tr.frozen = Some(est);
                    }
                }
            }
        }
        self.tracks[idx] = tr;
    }

    /// Offline training: the true position of a fogged champion at time t is known.
    /// Soft evidence for a champion in fog (e.g. a camp near q was cleared): reweights the fog
    /// estimate only. Never a sighting: the track stays in Fog and nothing is learned from it.
    pub fn mass_near(&self, champion: &str, q: (f64, f64), r: f64) -> Option<f64> {
        let tr = self.tracks.iter().find(|t| t.pub_.champion == champion && t.pub_.state == TrackState::Fog)?;
        let pf = tr.pf.as_ref()?;
        let total: f64 = pf.particles.iter().map(|p| p.w).sum();
        (total > 1e-12).then(|| pf.mass_near(q, r) / total)
    }

    pub fn soft_hint(&mut self, champion: &str, q: (f64, f64)) {
        if let Some(tr) = self.tracks.iter_mut().find(|t| t.pub_.champion == champion && t.pub_.state == TrackState::Fog) {
            if let Some(pf) = tr.pf.as_mut() {
                pf.observe_hint(q, 0.06, 0.35, &mut self.rng);
            }
        }
    }

    pub fn train_at(&mut self, champion: &str, q: (f64, f64), t: f64) {
        let Some(i) = self.tracks.iter().position(|t| t.pub_.champion == champion) else { return };
        if self.tracks[i].pub_.state != TrackState::Fog {
            return;
        }
        let tr = std::mem::replace(&mut self.tracks[i], placeholder());
        let e = EnemyInput::new(champion, tr.pub_.team, tr.pub_.role, 345.0);
        self.learn_reappearance(&tr, &e, q, t);
        self.tracks[i] = tr;
    }

    fn learn_reappearance(&mut self, tr: &Internal, e: &EnemyInput, q: (f64, f64), t: f64) {
        if !self.learning_enabled {
            return;
        }
        let gap = t - tr.pub_.fog_since.unwrap_or(t);
        let (Some(frozen), Some(base), Some(pf)) = (tr.frozen, tr.baseline_at, tr.pf.as_ref()) else { return };
        if !(3.0..=90.0).contains(&gap) {
            return;
        }
        // Baseline: Gaussian around the last sighting, sigma growing at half the move speed.
        let sigma = (((e.move_speed * 0.5) / MAP_UNITS) * gap).max(0.02);
        let (r, d) = (0.05, (q.0 - base.0).hypot(q.1 - base.1));
        let mass_baseline = ((r * r) / (2.0 * sigma * sigma) * (-(d * d) / (2.0 * sigma * sigma)).exp()).min(1.0);
        // The distribution the filter really sampled from (not a count of goals per kind): with the
        // wrong prior, observations that matched the model still moved the weights.
        let prior = pf.prior_by_kind();
        self.learner.observe(Observation {
            champion: &e.champion,
            role: e.role,
            game_time: t,
            gap_sec: gap,
            credit: pf.goal_kinds_near(q, 0.06),
            prior,
            mass_model: pf.mass_near(q, 0.05),
            mass_baseline,
            error_pred: (q.0 - frozen.x).hypot(q.1 - frozen.y),
            error_base: d,
        });
    }
}

fn placeholder() -> Internal {
    Internal {
        pub_: EnemyTrack { champion: String::new(), team: Team::Order, role: Role::Unknown, state: TrackState::Unknown, last: None, vel: (0.0, 0.0), fog_since: None, estimate: None, recalled_at: None, flash_suspect_at: None },
        hits: Vec::new(),
        pf: None,
        baseline_at: None,
        last_prior_at: 0.0,
        pending: None,
        last_item_gold: None,
        frozen: None,
    }
}

/// Homeguard: leaving the fountain after a respawn or recall gives a big move speed bonus that
/// decays once outside (about +80% before 14:00, +150% after). Modelled as full bonus for the
/// first 3 s (crossing the platform), then a linear decay to nothing over the next 4 s.
const HG_FULL: f64 = 3.0;
const HG_DECAY: f64 = 4.0;

fn homeguard_bonus(game_t: f64) -> f64 {
    if game_t < 14.0 * 60.0 { 0.8 } else { 1.5 }
}

/// Speed multiplier `since` seconds after leaving the fountain.
pub fn homeguard_mult(since: f64, game_t: f64) -> f64 {
    let b = homeguard_bonus(game_t);
    if since < 0.0 { 1.0 } else if since <= HG_FULL { 1.0 + b } else if since >= HG_FULL + HG_DECAY { 1.0 } else { 1.0 + b * (1.0 - (since - HG_FULL) / HG_DECAY) }
}

/// Extra distance from Homeguard, in seconds of normal walking, after `since` seconds.
pub fn homeguard_extra_secs(since: f64, game_t: f64) -> f64 {
    let b = homeguard_bonus(game_t);
    let a = since.max(0.0);
    let full = a.min(HG_FULL);
    let d = (a - HG_FULL).clamp(0.0, HG_DECAY);
    b * (full + d - d * d / (2.0 * HG_DECAY))
}

#[cfg(test)]
mod homeguard_tests {
    use super::*;
    #[test]
    fn homeguard_decays_and_doubles_late() {
        assert_eq!(homeguard_mult(1.0, 300.0), 1.8);
        assert_eq!(homeguard_mult(1.0, 1000.0), 2.5);
        assert!((homeguard_mult(5.0, 300.0) - 1.4).abs() < 1e-9);
        assert_eq!(homeguard_mult(8.0, 300.0), 1.0);
        // 3 s full + 4 s triangle = 0.8 * (3 + 2) = 4 s of extra walking
        assert!((homeguard_extra_secs(30.0, 300.0) - 4.0).abs() < 1e-9);
    }
}
