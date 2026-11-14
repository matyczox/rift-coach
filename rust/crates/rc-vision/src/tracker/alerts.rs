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
pub fn same_lane_roles(a: Role, b: Role) -> bool {
    a != Role::Unknown && b != Role::Unknown
        && (a == b || matches!((a,b), (Role::Bottom,Role::Utility) | (Role::Utility,Role::Bottom)))
}

pub fn gank_ready(t: &EnemyTrack, my_role: Option<Role>, game_time: f64) -> bool {
    let age = game_time - t.fog_since.unwrap_or(game_time);
    let lane_opp = my_role.is_some_and(|r| same_lane_roles(r,t.role));
    !lane_opp && age >= if t.role == Role::Jungle { 1.5 } else { 4.0 }
}

/// Early amber risk, distinct from an enemy inferred to be next to the player.
pub fn forecast_warning(champion:&str, current:f64, forecast:(f64,u32), now:f64) -> Option<AlertState> {
    let (mass,sec)=forecast;
    if (1..=6).contains(&sec) && current<0.6 && mass>=0.4 && mass-current>=0.15 {
        Some(AlertState {kind:AlertKind::Missing,text:format!("{champion}: możliwy gank w ciągu 6 s"),confidence:mass.min(0.9),expires_at:now+1800.0})
    } else {None}
}

/// `near_me(champion)` = probability (all hypotheses) that the fogged enemy is within reach of me; when
/// given it decides gank warnings, so the warning agrees with the zones drawn on the minimap.
pub fn tracker_alerts(tracks: &[EnemyTrack], me: Option<(f64, f64)>, my_team: Team, my_role: Option<Role>, game_time: f64, now: f64, near_me: &dyn Fn(&str) -> Option<f64>) -> Vec<AlertState> {
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
        let overlap = ((r + e.radius - d) / (2.0 * r.min(e.radius) + 1e-6)).clamp(0.0, 1.0);
        // A huge uncertain circle (radius 0.45 at game start) touching me is not "he is next to me": scale by
        // the share of his circle my reach covers (it said "Kha'Zix invade" at 75% while he was in base).
        if e.radius > r { overlap * (r / e.radius).powi(2) } else { overlap }
    };
    if let Some(me) = me {
        let in_my_jungle = if my_team == Team::Order { me.0 + me.1 > 0.98 && me.0 < 0.62 && me.1 > 0.38 } else { me.0 + me.1 < 1.02 && me.0 > 0.38 && me.1 < 0.62 };
        if let Some(j) = fog.iter().find(|t| t.role == Role::Jungle) {
            if in_my_jungle {
                // Same source as the zones drawn on the minimap (all hypotheses), like the gank warning.
                let p = near_me(&j.champion).unwrap_or_else(|| near(j, me, 0.14));
                if p > 0.6 {
                    out.push(AlertState { kind: AlertKind::Invade, text: format!("{} invade", j.champion), confidence: p.min(0.9), expires_at: now + 4000.0 });
                }
            }
        }
        // Short on purpose ("Briar gank"): read at a glance, mid-fight.
        // Probability he is near me from all hypotheses ("Nami gank" while the drawn ghost sat in base:
        // one centre and the zones disagreed). Fallback: the centre/radius overlap.
        let best = fog.iter().map(|t| (near_me(&t.champion).unwrap_or_else(|| near(t, me, 0.11)), t)).max_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((p, t)) = best.filter(|b| out.is_empty() && b.0 > 0.6) {
            out.push(AlertState { kind: AlertKind::Gank, text: format!("{} gank", t.champion), confidence: p.min(0.9), expires_at: now + 4000.0 });
        }
    }
    let long_missing = fog.iter().filter(|t| game_time - t.fog_since.unwrap_or(game_time) > 20.0).count();
    // "3 we mgle" told the player nothing to do: off (the gank / danger warnings cover real threats).
    if false && game_time > 12.0 * 60.0 && long_missing >= 3 && out.is_empty() {
        out.push(AlertState { kind: AlertKind::Missing, text: format!("{long_missing} we mgle"), confidence: (long_missing as f64 / 5.0).min(1.0), expires_at: now + 4000.0 });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracker::predictor::Estimate;
    fn track(role: Role) -> EnemyTrack {
        EnemyTrack { champion: "Enemy".into(), team: Team::Chaos, role, state: TrackState::Fog,
            last: None, vel:(0.0,0.0), fog_since:Some(100.0),
            estimate:Some(Estimate { x:0.8,y:0.8,radius:0.03,confidence:0.9,spread:0.0 }),
            recalled_at:None, flash_suspect_at:None }
    }
    #[test]
    fn early_risk_precedes_close_gank_but_not_diffuse_uncertainty() {
        let w=forecast_warning("Warwick",0.05,(0.65,4),1000.0).unwrap();
        assert_eq!(w.kind,AlertKind::Missing);
        assert!(w.text.contains("możliwy gank"));
        assert!(forecast_warning("Warwick",0.05,(0.2,4),1000.0).is_none());
        assert!(forecast_warning("Warwick",0.65,(0.8,2),1000.0).is_none());
        assert!(forecast_warning("Warwick",0.4,(0.45,4),1000.0).is_none());
    }

    #[test]
    fn bot_duo_are_lane_opponents_not_gankers() {
        assert!(!gank_ready(&track(Role::Bottom),Some(Role::Utility),110.0));
        assert!(!gank_ready(&track(Role::Utility),Some(Role::Bottom),110.0));
        assert!(gank_ready(&track(Role::Jungle),Some(Role::Utility),110.0));
        assert!(gank_ready(&track(Role::Middle),Some(Role::Utility),110.0));
        assert!(!same_lane_roles(Role::Unknown,Role::Unknown));
    }
    #[test]
    fn probability_is_not_inflated_into_an_alarm() {
        let tracks = [track(Role::Jungle)];
        let ask = |mass| tracker_alerts(&tracks,Some((0.8,0.8)),Team::Order,Some(Role::Utility),110.0,1000.0,&|_|Some(mass));
        assert!(ask(0.4).is_empty());
        let a = ask(0.7);
        assert_eq!(a.len(),1);
        assert_eq!(a[0].confidence,0.7);
    }
    #[test]
    fn visible_or_dead_is_not_a_fog_gank() {
        for state in [TrackState::Visible, TrackState::Dead] {
            let mut t = track(Role::Jungle); t.state=state;
            assert!(tracker_alerts(&[t],Some((0.8,0.8)),Team::Order,Some(Role::Utility),110.0,1000.0,&|_|Some(0.9)).is_empty());
        }
    }
}
