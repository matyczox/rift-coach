//! Invade / gank / missing alerts from fog predictions (port of `src/main/tracker/alerts.ts`).

use super::enemy::{EnemyTrack, TrackState};
use super::predictor::Role;
use crate::detect::Team;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlertKind {
    Invade,
    Gank,
    Missing,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertState {
    pub kind: AlertKind,
    pub text: String,
    pub confidence: f64,
    /// Wall-clock ms.
    pub expires_at: f64,
}

/// `me` = my champion's minimap position (from the detector); `now` = wall-clock ms.
/// A gank warning: someone from elsewhere (jungler, roamer) out of sight for at least 4 s. Never my lane
/// opponent: him in a bush or behind his tower is lane play (his threat is in the fight bar), and
/// "Sion gank" about my own laner only confused.
pub fn gank_ready(t: &EnemyTrack, my_role: Option<Role>, game_time: f64) -> bool {
    let age = game_time - t.fog_since.unwrap_or(game_time);
    let lane_opp = my_role.is_some_and(|r| r != Role::Unknown && r == t.role);
    !lane_opp && age >= 4.0
}

pub fn tracker_alerts(tracks: &[EnemyTrack], me: Option<(f64, f64)>, my_team: Team, my_role: Option<Role>, game_time: f64, now: f64) -> Vec<AlertState> {
    let mut out = Vec::new();
    let fog: Vec<&EnemyTrack> = tracks.iter().filter(|t| t.state == TrackState::Fog && t.estimate.is_some() && t.fog_since.is_some() && gank_ready(t, my_role, game_time)).collect();
    let near = |t: &EnemyTrack, p: (f64, f64), r: f64| {
        let e = t.estimate.unwrap();
        let d = (e.x - p.0).hypot(e.y - p.1);
        if d <= (r - e.radius).max(0.0001) {
            return 1.0;
        }
        if d >= r + e.radius {
            return 0.0;
        }
        ((r + e.radius - d) / (2.0 * r.min(e.radius) + 1e-6)).clamp(0.0, 1.0)
    };
    if let Some(me) = me {
        let in_my_jungle = if my_team == Team::Order { me.0 + me.1 > 0.98 && me.0 < 0.62 && me.1 > 0.38 } else { me.0 + me.1 < 1.02 && me.0 > 0.38 && me.1 < 0.62 };
        if let Some(j) = fog.iter().find(|t| t.role == Role::Jungle) {
            if in_my_jungle {
                let p = near(j, me, 0.14);
                if p > 0.45 {
                    out.push(AlertState { kind: AlertKind::Invade, text: format!("{} invade", j.champion), confidence: p, expires_at: now + 4000.0 });
                }
            }
        }
        // Short on purpose ("Briar gank"): read at a glance, mid-fight.
        let best = fog.iter().map(|t| (near(t, me, 0.11), t)).max_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((p, t)) = best.filter(|b| out.is_empty() && b.0 > 0.5) {
            out.push(AlertState { kind: AlertKind::Gank, text: format!("{} gank", t.champion), confidence: p, expires_at: now + 4000.0 });
        }
    }
    let long_missing = fog.iter().filter(|t| game_time - t.fog_since.unwrap_or(game_time) > 20.0).count();
    if game_time > 12.0 * 60.0 && long_missing >= 3 && out.is_empty() {
        out.push(AlertState { kind: AlertKind::Missing, text: format!("{long_missing} we mgle"), confidence: (long_missing as f64 / 5.0).min(1.0), expires_at: now + 4000.0 });
    }
    out
}
