//! After the game (index.ts `finishGame` / `selfTrain`): post-game review by the review
//! model (saved to `<userData>/reviews/<id>.json`, pushed to the UI as `review`), and
//! self-training of the fog model from this game's observations + the Riot timeline.

use crate::state::AppState;
use rc_core::intel::game_intel::GameIntel;
use rc_core::llm::caller::CliCaller;
use rc_core::llm::profiles::{pick, Task};
use rc_core::patch_brief::brief_for_game;
use rc_core::review::{build_review, MatchLog, ReviewOptions};
use rc_core::riot::CallOptions;
use rc_core::self_train::{load_obs, obs_dir, timeline_truth, train_from_observations, ObsEnemy, ObsTracker, TrackView};
use rc_vision::tracker::{Role, TrackState};
use rc_vision::{Detection, EnemyInput, EnemyTracker, FrameInput, GridTerrain, Learner, Team};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tauri::Emitter;

/// rc-core's training loop on top of the rc-vision fog tracker (250 particles, learning on,
/// detections with score 0.9 / margin 0.5 / votes 1 — the values the TS code used).
pub struct TrainTracker(pub EnemyTracker);

impl TrainTracker {
    pub fn new(learner: Learner) -> TrainTracker {
        let mut t = EnemyTracker::new(Arc::new(GridTerrain::from_asset(128)), learner, 250);
        t.learning_enabled = true;
        TrainTracker(t)
    }
}

impl ObsTracker for TrainTracker {
    fn update(&mut self, t: f64, detections: &[(String, f64, f64)], visible: &dyn Fn(f64, f64) -> f64, enemies: &[ObsEnemy], enemy_team: &str, dragon_in: Option<f64>, baron_in: Option<f64>) -> Vec<TrackView> {
        let team = Team::parse(enemy_team).unwrap_or(Team::Chaos);
        let dets: Vec<Detection> = detections.iter().map(|(c, x, y)| Detection { champion: c.clone(), team, x: *x, y: *y, score: 0.9, margin: 0.5, votes: 1.0 }).collect();
        let en: Vec<EnemyInput> = enemies
            .iter()
            .map(|e| EnemyInput { champion: e.champion.clone(), team, role: Role::parse(&e.role), is_dead: e.is_dead, move_speed: e.move_speed, likely_recall: false, start_top_side: None, item_gold: Some(e.item_gold), has_flash: false })
            .collect();
        self.0
            .update(&FrameInput { t, detections: &dets, visible: Some(visible), enemies: &en, dragon_in, baron_in })
            .into_iter()
            .map(|tr| TrackView { champion: tr.champion, in_fog: tr.state == TrackState::Fog, estimate: tr.estimate.map(|e| (e.x, e.y, e.radius)), last: tr.last.map(|l| (l.x, l.y)) })
            .collect()
    }

    fn train_at(&mut self, champion: &str, q: (f64, f64), t: f64) {
        self.0.train_at(champion, q, t);
    }
}

const REVIEW_MODES: [&str; 3] = ["CLASSIC", "SWIFTPLAY", "PRACTICETOOL"];

/// Post-game review + self-training, once per game (>= 5 min, not a replay).
pub async fn finish_game(st: Arc<AppState>) {
    let key = {
        let mut g = st.game.lock().unwrap();
        let key = g.match_log.game_key.clone();
        let long = g.match_log.last.as_ref().is_some_and(|l| l.game_time >= 300.0);
        // Reviews only for Summoner's Rift modes (no ARAM / Arena / ...).
        let rift = REVIEW_MODES.contains(&g.match_log.game_mode.as_str());
        if key.is_empty() || key == g.review_done_for || !long || !rift || st.replay_open() || g.coach.status.state == "not-logged-in" {
            return;
        }
        g.review_done_for = key.clone();
        key
    };
    let (mut win, mut game_id) = (None, None);
    if let Ok(eog) = st.lcu_client.get::<Value>("/lol-end-of-game/v1/eog-stats-block").await {
        let my_team = eog.pointer("/localPlayer/teamId").and_then(Value::as_i64);
        if let Some(t) = eog.get("teams").and_then(Value::as_array).and_then(|ts| ts.iter().find(|t| t.get("teamId").and_then(Value::as_i64) == my_team && my_team.is_some())) {
            win = t.get("isWinningTeam").and_then(Value::as_bool);
        }
        game_id = eog.get("gameId").and_then(Value::as_i64);
    }
    st.rt.spawn(self_train(st.clone(), game_id));
    // New game in the history + a new review: rebuild the player profile once Riot has the match.
    crate::self_profile::after_game(&st);

    let (log, opts) = {
        let mut g = st.game.lock().unwrap();
        let log = std::mem::replace(&mut g.match_log, MatchLog::new());
        let review_model = st.settings.lock().unwrap().value().coach.review_model.clone();
        let p = pick(&review_model, Task::Review, g.backends);
        let brief = match (&g.patch_brief, &log.last) {
            (Some(b), Some(last)) => Some(brief_for_game(b, &champions(last))),
            _ => None,
        };
        (log, ReviewOptions { model: p.model, effort: Some(p.effort), win, patch_brief: brief })
    };
    match build_review(&CliCaller, &log, &opts, &st.reviews_dir(), rc_core::jsutil::now_ms()).await {
        Ok(Some(r)) => {
            log::info!("review saved {}", r.id);
            let _ = st.app.emit("review", &r);
        }
        Ok(None) => {}
        Err(e) => {
            log::warn!("review {e}");
            // Allow a retry; keep the log if no new game started meanwhile.
            let mut g = st.game.lock().unwrap();
            if g.review_done_for == key {
                g.review_done_for.clear();
            }
            if g.match_log.game_key.is_empty() {
                g.match_log = log;
            }
        }
    }
}

fn champions(i: &GameIntel) -> Vec<String> {
    i.players.iter().map(|p| p.champion.clone()).collect()
}

/// Save this game's observations and learn from the Riot timeline (true positions every minute).
pub async fn self_train(st: Arc<AppState>, game_id: Option<i64>) {
    let file = {
        let mut g = st.game.lock().unwrap();
        let f = g.obs.save(&obs_dir(&st.data_dir), game_id);
        g.obs.log = None;
        g.obs_game_key.clear();
        f
    };
    let (Some(file), Some(gid)) = (file, game_id) else { return };
    log::info!("observations saved {}", file.display());
    let Some(client) = crate::knowledge::riot_client(&st) else { return };
    let match_id = format!("{}_{gid}", crate::knowledge::platform(&st).to_uppercase());
    // The match appears in the API a minute or two after the game.
    let (mut m, mut tl) = (None, None);
    for _ in 0..6 {
        if m.is_some() && tl.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_secs(30)).await;
        m = client.get_match(&match_id, &CallOptions::default()).await.ok().flatten();
        tl = client.get_timeline(&match_id, &CallOptions::default()).await.ok().flatten();
    }
    let (Some(m), Some(tl)) = (m, tl) else { return };
    match train_file(&st, &file, &m, &tl) {
        Ok(r) => log::info!("self-train {r}"),
        Err(e) => log::warn!("self-train {e}"),
    }
}

/// Train the shared fog model on one observation file (blocking; takes the live learner out
/// of the tracker for the duration and puts it back saved).
pub fn train_file(st: &AppState, file: &std::path::Path, m: &rc_core::riot::types::MatchDto, tl: &rc_core::riot::types::TimelineDto) -> anyhow::Result<String> {
    let log = load_obs(file)?;
    let (learner, ids) = {
        let mut g = st.game.lock().unwrap();
        let ids = g.static_data.as_ref().map(|sd| sd.champion_ids.clone()).unwrap_or_default();
        let model_file = st.data_dir.join("model").join("fog-model.json");
        let l = match g.field.tracker.as_mut() {
            // Train a copy; the live tracker keeps learning (journaled) and both are merged after.
            Some(t) => t.learner.fork_for_training(),
            None => Learner::new(Some(model_file)),
        };
        (l, ids)
    };
    // match-v5 names are Data Dragon ids; the tracker uses display names.
    let name_fix = |n: &str| ids.iter().find(|(_, id)| id.as_str() == n).map(|(name, _)| name.clone()).unwrap_or_else(|| n.to_string());
    let truth = |champion: &str, minute: usize| timeline_truth(m, tl, champion, minute, &name_fix);
    let mut tt = TrainTracker::new(learner);
    let res = train_from_observations(&log, &mut tt, &truth);
    let mut learner = tt.0.learner;
    let mut g = st.game.lock().unwrap();
    let saved = match g.field.tracker.as_mut() {
        Some(t) => {
            let live = t.learner.take_journal();
            learner.replay(&live);
            t.learner = learner;
            t.learner.save()
        }
        None => learner.save(),
    };
    saved?;
    Ok(serde_json::to_string(&res)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rc_core::self_train::{pack_mask, ObsFrame, ObsLog};

    /// One enemy seen in mid lane for 30 s, then in fog: at the 1-minute mark the training
    /// loop scores the rc-vision tracker's fog estimate against the timeline truth.
    #[test]
    fn trains_through_rc_vision_tracker() {
        let dark = pack_mask(&vec![0.0; 64 * 64], 64);
        let frames: Vec<ObsFrame> = (0..90)
            .map(|t| ObsFrame {
                t: t as f64,
                d: if t < 30 { vec![("Ahri".into(), 0.5, 0.5)] } else { vec![] },
                m: dark.clone(),
                e: vec![("Ahri".into(), "MIDDLE".into(), 0, 330.0, 1000.0)],
                dr: None,
                br: None,
            })
            .collect();
        let log = ObsLog { game_id: Some(1), team: "ORDER".into(), enemy_team: "CHAOS".into(), frames };
        let mut tt = TrainTracker::new(Learner::new(None));
        let truth = |c: &str, minute: usize| (c == "Ahri" && minute == 1).then_some((0.55, 0.45));
        let r = train_from_observations(&log, &mut tt, &truth);
        assert_eq!(r.samples, 1, "{r:?}");
        assert!(r.err_model.is_finite() && r.err_baseline > 0.0, "{r:?}");
    }
}
