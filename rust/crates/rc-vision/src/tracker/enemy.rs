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
}

impl EnemyTracker {
    pub fn new(terrain: Arc<GridTerrain>, learner: Learner, particles: usize) -> EnemyTracker {
        EnemyTracker { tracks: Vec::new(), last_t: None, rng: Rng::new(12345), terrain, learner, particles, learning_enabled: true }
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
                Some(Ghost { champion: p.champion.clone(), x: e.x, y: e.y, radius: e.radius, age: t - fs, confidence: e.confidence })
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
                if cur - prev >= 250.0 && tr.pub_.state != TrackState::Visible {
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
                }
            }

            if tr.pub_.state == TrackState::Fog {
                if let Some(pf) = tr.pf.as_mut() {
                    if f.t - tr.last_prior_at > 5.0 {
                        let from_base = tr.baseline_at.is_some_and(|b| (b.0 - fp.0).hypot(b.1 - fp.1) < 0.05) && f.t - tr.pub_.fog_since.unwrap_or(f.t) < 25.0;
                        pf.set_prior(&PriorContext { leaving_base: from_base, ..ctx.clone() }, None);
                        tr.last_prior_at = f.t;
                    }
                    if dt > 0.0 {
                        pf.step(dt, e.move_speed, &mut self.rng);
                    }
                    if let Some(vis) = f.visible {
                        let ok = pf.observe_absent(vis, &mut self.rng);
                        if !ok {
                            if let Some(l) = tr.pub_.last {
                                // Everything contradicted: restart broadly from the last sighting.
                                pf.reset((l.x, l.y), &ctx, None, &mut self.rng);
                                pf.step((f.t - l.t).min(60.0), e.move_speed, &mut self.rng);
                            }
                        }
                    }
                    // Calibrated: the particle spread alone is overconfident; a speed x time floor
                    // brings truth-inside to ~70% for the first 15 s in fog.
                    let est = pf.estimate(0.7);
                    let age = f.t - tr.pub_.fog_since.unwrap_or(f.t);
                    let est = Estimate { radius: est.radius.max((e.move_speed / MAP_UNITS) * age).min(0.45), ..est };
                    tr.pub_.estimate = Some(est);
                    tr.frozen = Some(est);
                }
            }
        }
        self.tracks[idx] = tr;
    }

    /// Offline training: the true position of a fogged champion at time t is known.
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
        let mut prior = KindMap::new();
        for g in goals_for(e.team) {
            *prior.entry(g.kind).or_insert(0.0) += 1.0;
        }
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
