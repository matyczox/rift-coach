//! What the predictor learns across games (port of `src/main/tracker/learning.ts`): per
//! (role, game phase) a multiplier per goal kind, updated whenever a fogged enemy reappears,
//! plus a running score of model vs the naive "still where we last saw it" baseline.
//! The JSON file format is the TS one, so a model learned by the Electron app loads here.

use super::predictor::{KindMap, Role};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Score {
    pub n: u64,
    pub log_model: f64,
    pub log_baseline: f64,
    pub hits: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentSample {
    pub at: f64,
    pub champion: String,
    pub gap_sec: f64,
    pub model: f64,
    pub baseline: f64,
    pub error_pred: f64,
    pub error_base: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LearnedModel {
    pub version: u32,
    pub multipliers: BTreeMap<String, KindMap>,
    pub score: Score,
    pub recent: Vec<RecentSample>,
}

impl Default for LearnedModel {
    fn default() -> Self {
        LearnedModel { version: 1, multipliers: BTreeMap::new(), score: Score::default(), recent: Vec::new() }
    }
}

pub fn phase_of(game_time: f64) -> &'static str {
    if game_time < 14.0 * 60.0 {
        "early"
    } else if game_time < 25.0 * 60.0 {
        "mid"
    } else {
        "late"
    }
}

/// One reappearance of a fogged enemy.
#[derive(Clone, Debug)]
pub struct Observation<'a> {
    pub champion: &'a str,
    pub role: Role,
    pub game_time: f64,
    pub gap_sec: f64,
    /// Goal-kind mass of particles near the reappearance point.
    pub credit: KindMap,
    /// Goal-kind shares the filter used.
    pub prior: KindMap,
    pub mass_model: f64,
    pub mass_baseline: f64,
    pub error_pred: f64,
    pub error_base: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub samples: u64,
    pub avg_log_model: f64,
    pub avg_log_baseline: f64,
    pub better_than_baseline: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Learner {
    pub model: LearnedModel,
    file: Option<PathBuf>,
    dirty: bool,
    /// While a copy trains elsewhere (post-game timeline), live observations are also kept here so
    /// they can be replayed onto the trained copy instead of being lost.
    journal: Option<Vec<OwnedObservation>>,
}

/// Owned copy of an Observation (for the journal).
#[derive(Clone, Debug)]
pub struct OwnedObservation {
    pub champion: String,
    pub role: Role,
    pub game_time: f64,
    pub gap_sec: f64,
    pub credit: KindMap,
    pub prior: KindMap,
    pub mass_model: f64,
    pub mass_baseline: f64,
    pub error_pred: f64,
    pub error_base: f64,
}

fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

impl Learner {
    /// In-memory learner, or one persisted to `file` (loaded if it exists and is valid).
    pub fn new(file: Option<PathBuf>) -> Learner {
        let mut model = LearnedModel::default();
        if let Some(f) = &file {
            if let Ok(bytes) = std::fs::read(f) {
                if let Ok(m) = serde_json::from_slice::<LearnedModel>(&bytes) {
                    if m.version == 1 {
                        model = m;
                    }
                }
            }
        }
        Learner { model, file, dirty: false, journal: None }
    }

    pub fn multipliers(&self, role: Role, game_time: f64) -> KindMap {
        self.model.multipliers.get(&format!("{}|{}", role.as_str(), phase_of(game_time))).cloned().unwrap_or_default()
    }

    /// Exponentiated-gradient step towards the goal kinds that explained the reappearance.
    /// Copy to train on elsewhere; from now on live observations are journaled (see take_journal).
    pub fn fork_for_training(&mut self) -> Learner {
        let mut copy = self.clone();
        copy.journal = None;
        self.journal = Some(Vec::new());
        copy
    }

    /// Live observations since fork_for_training (and stop journaling).
    pub fn take_journal(&mut self) -> Vec<OwnedObservation> {
        self.journal.take().unwrap_or_default()
    }

    /// Apply journaled live observations (merge after training on a fork).
    pub fn replay(&mut self, js: &[OwnedObservation]) {
        for o in js {
            self.observe(Observation { champion: &o.champion, role: o.role, game_time: o.game_time, gap_sec: o.gap_sec, credit: o.credit.clone(), prior: o.prior.clone(), mass_model: o.mass_model, mass_baseline: o.mass_baseline, error_pred: o.error_pred, error_base: o.error_base });
        }
    }

    pub fn observe(&mut self, e: Observation) {
        if let Some(j) = self.journal.as_mut() {
            j.push(OwnedObservation { champion: e.champion.to_string(), role: e.role, game_time: e.game_time, gap_sec: e.gap_sec, credit: e.credit.clone(), prior: e.prior.clone(), mass_model: e.mass_model, mass_baseline: e.mass_baseline, error_pred: e.error_pred, error_base: e.error_base });
        }
        let eps = 1e-3;
        let s = &mut self.model.score;
        s.n += 1;
        s.log_model += (e.mass_model + eps).ln();
        s.log_baseline += (e.mass_baseline + eps).ln();
        if e.error_pred < e.error_base {
            s.hits += 1;
        }
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as f64).unwrap_or(0.0);
        self.model.recent.insert(
            0,
            RecentSample { at: now, champion: e.champion.to_string(), gap_sec: e.gap_sec.round(), model: r3(e.mass_model), baseline: r3(e.mass_baseline), error_pred: r3(e.error_pred), error_base: r3(e.error_base) },
        );
        self.model.recent.truncate(200);

        let credit_total: f64 = e.credit.values().sum();
        let prior_total: f64 = e.prior.values().sum();
        if credit_total > 0.02 && prior_total > 0.0 {
            let key = format!("{}|{}", e.role.as_str(), phase_of(e.game_time));
            let m = self.model.multipliers.entry(key).or_default();
            // Conservative updates and shrinkage prevent correlated labels from
            // a single fog episode from overpowering the role/phase prior.
            let lr = 0.08;
            let kinds: std::collections::BTreeSet<_> = e.credit.keys().chain(e.prior.keys()).copied().collect();
            for k in kinds {
                let c = e.credit.get(&k).copied().unwrap_or(0.0) / credit_total;
                let p = e.prior.get(&k).copied().unwrap_or(0.0) / prior_total;
                let cur = m.get(&k).copied().unwrap_or(1.0);
                m.insert(k, (cur.ln() * 0.995 + lr * (c - p)).exp().clamp(0.2, 5.0));
            }
        }
        self.dirty = true;
    }

    pub fn summary(&self) -> Summary {
        let s = &self.model.score;
        let n = s.n as f64;
        if s.n == 0 {
            return Summary::default();
        }
        Summary { samples: s.n, avg_log_model: s.log_model / n, avg_log_baseline: s.log_baseline / n, better_than_baseline: s.hits as f64 / n }
    }

    pub fn save(&mut self) -> std::io::Result<()> {
        let Some(f) = &self.file else { return Ok(()) };
        if !self.dirty {
            return Ok(());
        }
        if let Some(dir) = f.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(f, serde_json::to_vec(&self.model).map_err(std::io::Error::other)?)?;
        self.dirty = false;
        Ok(())
    }
}
