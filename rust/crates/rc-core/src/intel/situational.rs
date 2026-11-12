//! Situational calls (`src/main/intel/situational.ts`): who is fed, danger at my
//! position, go/contest/skip for the next objective, teamplay prompts.
//!
//! The fog tracker lives in rc-vision; these functions take [`FogTrack`], a thin
//! copy of the fields they need (the app maps `rc_vision::tracker::EnemyTrack`:
//! `in_fog = state == Fog`, `estimate = (x, y, radius)`). Positions are
//! normalized minimap coordinates (0..1, y down).

use super::game_intel::{GameIntel, PlayerIntel};
use super::js_round;
use serde::Serialize;

/// Minimap positions of objectives (normalized).
fn obj_pos(kind: &str) -> Option<Pos> {
    Some(match kind {
        "dragon" => Pos { x: 0.67, y: 0.70 },
        "baron" | "herald" | "grubs" => Pos { x: 0.33, y: 0.30 },
        _ => return None,
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pos {
    pub x: f64,
    pub y: f64,
}

/// A champion seen on the minimap now (velocity smoothed by the app).
#[derive(Clone, Debug, PartialEq)]
pub struct SeenUnit {
    pub champion: String,
    pub x: f64,
    pub y: f64,
    pub vx: Option<f64>,
    pub vy: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FogEstimate {
    pub x: f64,
    pub y: f64,
    pub radius: f64,
}

/// What these calls need from an enemy fog track.
#[derive(Clone, Debug, PartialEq)]
pub struct FogTrack {
    pub champion: String,
    /// `state === 'fog'`
    pub in_fog: bool,
    pub estimate: Option<FogEstimate>,
}

/// `AlertState` from shared/intel.ts.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertState {
    /// invade | gank | missing | spike | fed | ally | gank-ask
    pub kind: &'static str,
    pub text: String,
    pub confidence: f64,
    pub expires_at: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DangerState {
    /// 0..1
    pub level: f64,
    /// bezpiecznie | uważaj | niebezpiecznie | uciekaj
    pub label: &'static str,
    /// Top 1–2 reasons.
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectiveCall {
    /// "Smok", "Baron", "Herold", "Larwy"
    pub objective: String,
    /// go | contest | skip
    pub verdict: &'static str,
    pub reason: String,
    pub in_sec: Option<i64>,
    pub x: f64,
    pub y: f64,
}

fn avg_gold(intel: &GameIntel) -> f64 {
    intel.players.iter().map(|p| p.est_gold).sum::<f64>() / (intel.players.len().max(1) as f64)
}

/// Fed = clearly ahead of the lobby: big K-D or gold lead over the game average.
pub fn fed_champions(intel: &GameIntel) -> Vec<&PlayerIntel> {
    let avg = avg_gold(intel);
    intel
        .players
        .iter()
        .filter(|p| {
            let kd = p.kills - p.deaths;
            let lead = p.est_gold - avg;
            (kd >= 4 && p.kills >= 5) || lead > 1500.0 + intel.game_time * 0.8 || (kd >= 3 && lead > 1000.0)
        })
        .collect()
}

/// Alert when an enemy becomes fed (call once per update; keeps its own memory).
#[derive(Default)]
pub struct FedWatcher {
    known: Vec<String>,
}

impl FedWatcher {
    pub fn update(&mut self, intel: &GameIntel, now: f64) -> Vec<AlertState> {
        let mut out = Vec::new();
        let fed: Vec<&PlayerIntel> = fed_champions(intel).into_iter().filter(|p| !p.is_ally).collect();
        for p in &fed {
            if self.known.contains(&p.champion) {
                continue;
            }
            self.known.push(p.champion.clone());
            let avg = intel.players.iter().map(|x| x.est_gold).sum::<f64>() / intel.players.len() as f64;
            let k = js_round((p.est_gold - avg) / 100.0) / 10.0;
            out.push(AlertState { kind: "fed", text: format!("{} nafeedowany: {}/{}, +{:.1}k", p.champion, p.kills, p.deaths, k), confidence: 1.0, expires_at: now + 9000.0 });
        }
        self.known.retain(|c| fed.iter().any(|p| &p.champion == c));
        out
    }
}

fn ghost_threat(t: &FogTrack, me: Pos, r: f64) -> f64 {
    let Some(e) = t.estimate.filter(|_| t.in_fog) else { return 0.0 };
    let d = (e.x - me.x).hypot(e.y - me.y);
    if d >= r + e.radius {
        return 0.0;
    }
    let overlap = ((r + e.radius - d) / (2.0 * r.min(e.radius) + 1e-6)).min(1.0);
    // A huge circle (long in fog) says little.
    overlap * (1.0 - e.radius / 0.45).max(0.25)
}

/// Danger at my position: nearby visible enemies, fog circles reaching me, my HP,
/// matchup, fed enemies.
pub fn danger_at(intel: &GameIntel, me: Option<Pos>, tracks: &[FogTrack], visible_enemies: &[SeenUnit], p_me: Option<f64>, fed: &[String]) -> Option<DangerState> {
    let mi = intel.me.as_ref()?;
    let me = me?;
    if mi.is_dead {
        return None;
    }
    let mut reasons: Vec<(f64, String)> = Vec::new();
    let mut score = 0.0;
    for e in visible_enemies {
        let d = (e.x - me.x).hypot(e.y - me.y);
        if d > 0.14 {
            continue;
        }
        let is_fed = fed.contains(&e.champion);
        let w = (1.0 - d / 0.14) * if is_fed { 1.6 } else { 1.0 } * 0.45;
        score += w;
        reasons.push((w, format!("{} blisko{}", e.champion, if is_fed { " (fed)" } else { "" })));
    }
    for t in tracks {
        let w = ghost_threat(t, me, 0.12) * if fed.contains(&t.champion) { 1.5 } else { 1.0 } * 0.35;
        if w > 0.03 {
            score += w;
            reasons.push((w, format!("{} we mgle blisko", t.champion)));
        }
    }
    let hp = match &intel.my_stats {
        Some(s) if s.max_hp != 0.0 => s.hp / s.max_hp,
        _ => 1.0,
    };
    if hp < 0.5 {
        let w = (0.5 - hp) * 0.8;
        score += w;
        reasons.push((w, format!("{}% HP", js_round(hp * 100.0) as i64)));
    }
    if let Some(p) = p_me {
        if p < 0.4 && score > 0.1 {
            let w = (0.4 - p) * 0.6;
            score += w;
            reasons.push((w, "słabszy w matchupie".into()));
        }
    }
    let missing = tracks.iter().filter(|t| t.in_fog).count();
    if missing >= 3 {
        let w = 0.05 * missing as f64;
        score += w;
        reasons.push((w, format!("{missing} wrogów niewidocznych")));
    }
    let level = score.clamp(0.0, 1.0);
    let label = if level < 0.25 {
        "bezpiecznie"
    } else if level < 0.5 {
        "uważaj"
    } else if level < 0.75 {
        "niebezpiecznie"
    } else {
        "uciekaj"
    };
    reasons.sort_by(|a, b| b.0.total_cmp(&a.0));
    Some(DangerState { level, label, reasons: reasons.into_iter().take(2).map(|r| r.1).collect() })
}

/// Go / contest / skip for the next objective within 60 s (or alive).
pub fn objective_call(intel: &GameIntel, tracks: &[FogTrack], allies: &[SeenUnit], visible_enemies: &[SeenUnit]) -> Option<ObjectiveCall> {
    let gt = intel.game_time;
    let wait = |o: &super::game_intel::ObjectiveTimer| if o.alive { 0.0 } else { o.spawns_at.unwrap_or(f64::NAN) - gt };
    let mut cands: Vec<_> = intel
        .objectives
        .iter()
        .filter(|x| matches!(x.kind, "dragon" | "baron" | "herald" | "grubs") && (x.alive || x.spawns_at.is_some_and(|s| s - gt <= 60.0)))
        .collect();
    cands.sort_by(|a, b| wait(a).total_cmp(&wait(b)));
    let o = *cands.first()?;
    let pos = obj_pos(o.kind)?;
    let near = |x: f64, y: f64, r: f64| (x - pos.x).hypot(y - pos.y) <= r;
    let ally_alive = intel.ally.alive_count as f64;
    let enemy_alive = intel.enemy.alive_count as f64;
    let allies_near = allies.iter().filter(|a| near(a.x, a.y, 0.2)).count();
    let enemies_seen_near = visible_enemies.iter().filter(|e| near(e.x, e.y, 0.2)).count();
    let enemies_likely_near = tracks.iter().filter(|t| t.in_fog && t.estimate.is_some_and(|e| near(e.x, e.y, 0.2 + e.radius * 0.5))).count();
    let enemy_near = enemies_seen_near as f64 + 0.6 * enemies_likely_near as f64;
    let gold_k = intel.gold_diff / 1000.0;
    let score = (ally_alive - enemy_alive) * 0.9 + (allies_near as f64 - enemy_near) * 0.6 + gold_k * 0.25 + if o.kind == "baron" { -0.4 } else { 0.0 };
    let dead_enemies = 5 - intel.enemy.alive_count as i64;
    let (verdict, reason) = if score >= 1.2 {
        (
            "go",
            if dead_enemies >= 2 {
                format!("{dead_enemies} wrogów martwych")
            } else if allies_near >= 3 {
                format!("{allies_near} waszych blisko")
            } else {
                "przewaga".into()
            },
        )
    } else if score <= -0.8 {
        ("skip", if ally_alive < enemy_alive { format!("{}v{}, oddaj", intel.enemy.alive_count, intel.ally.alive_count) } else { "wrogowie bliżej, oddaj".into() })
    } else {
        ("contest", "wyrównane, tylko z wizją".into())
    };
    let label = match o.kind {
        "dragon" => "Smok".to_string(),
        "baron" => "Baron".into(),
        "herald" => "Herold".into(),
        "grubs" => "Larwy".into(),
        _ => o.label.clone(),
    };
    Some(ObjectiveCall { objective: label, verdict, reason, in_sec: Some(if o.alive { 0 } else { js_round(o.spawns_at.unwrap_or(gt) - gt) as i64 }), x: pos.x, y: pos.y })
}

/// Teamplay prompts: my jungler next to my lane while I'm stronger → ask for the
/// gank; allies walking towards me → get ready to fight.
#[derive(Default)]
pub struct TeamWatcher {
    last_ask: f64,
    last_ally: f64,
}

impl TeamWatcher {
    /// `now` in ms.
    pub fn update(&mut self, intel: &GameIntel, me: Option<Pos>, allies: &[SeenUnit], visible_enemies: &[SeenUnit], p_me: Option<f64>, now: f64) -> Vec<AlertState> {
        let mut out = Vec::new();
        let (Some(mi), Some(me)) = (intel.me.as_ref(), me) else { return out };
        if mi.is_dead {
            return out;
        }
        let jungler = intel.players.iter().find(|p| p.is_ally && !p.is_me && p.role == "JUNGLE");
        let j_pos = jungler.and_then(|j| allies.iter().find(|a| a.champion == j.champion));
        let enemy_near_me = visible_enemies.iter().any(|e| (e.x - me.x).hypot(e.y - me.y) < 0.12);
        if let (Some(j), Some(jp)) = (jungler, j_pos) {
            if (jp.x - me.x).hypot(jp.y - me.y) < 0.18 && enemy_near_me && p_me.unwrap_or(0.5) >= 0.42 && now - self.last_ask > 30_000.0 {
                self.last_ask = now;
                out.push(AlertState { kind: "gank-ask", text: format!("{} obok → pinguj, wchodźcie", j.champion), confidence: 0.8, expires_at: now + 6000.0 });
            }
        }
        // Allies heading my way: within 0.25 and closing in.
        let coming: Vec<&SeenUnit> = allies
            .iter()
            .filter(|a| {
                let (Some(vx), Some(vy)) = (a.vx, a.vy) else { return false };
                if a.champion == mi.champion {
                    return false;
                }
                let (dx, dy) = (me.x - a.x, me.y - a.y);
                let d = dx.hypot(dy);
                let sp = vx.hypot(vy);
                d < 0.25 && d > 0.05 && sp > 0.004 && (dx * vx + dy * vy) / (d * sp) > 0.8
            })
            .collect();
        if !coming.is_empty() && enemy_near_me && now - self.last_ally > 20_000.0 {
            self.last_ally = now;
            let names: Vec<&str> = coming.iter().take(2).map(|c| c.champion.as_str()).collect();
            out.push(AlertState { kind: "ally", text: format!("{} {} do ciebie", names.join(" i "), if coming.len() > 1 { "idą" } else { "idzie" }), confidence: 0.75, expires_at: now + 6000.0 });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intel::game_intel::ObjectiveTimer;
    use crate::intel::matchup::tests::{intel, player};

    fn unit(c: &str, x: f64, y: f64) -> SeenUnit {
        SeenUnit { champion: c.into(), x, y, vx: None, vy: None }
    }

    #[test]
    fn fed_and_watcher() {
        let mut zed = player("Zed", false, false, "MIDDLE", 10);
        zed.kills = 6;
        zed.deaths = 1;
        zed.est_gold = 6000.0;
        let mut ahri = player("Ahri", true, true, "MIDDLE", 9);
        ahri.est_gold = 4000.0;
        let mut i = intel(vec![ahri, zed]);
        assert_eq!(fed_champions(&i).iter().map(|p| p.champion.as_str()).collect::<Vec<_>>(), ["Zed"]);
        let mut w = FedWatcher::default();
        let a = w.update(&i, 100.0);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].text, "Zed nafeedowany: 6/1, +1.0k");
        assert_eq!(a[0].expires_at, 9100.0);
        assert!(w.update(&i, 200.0).is_empty());
        i.players[1].kills = 1;
        i.players[1].est_gold = 4000.0;
        assert!(w.update(&i, 300.0).is_empty());
        i.players[1].kills = 6;
        i.players[1].est_gold = 6000.0;
        assert_eq!(w.update(&i, 400.0).len(), 1); // forgotten when no longer fed → alerts again
    }

    #[test]
    fn danger_levels() {
        let i = intel(vec![player("Ahri", true, true, "MIDDLE", 9), player("Zed", false, false, "MIDDLE", 10)]);
        let me = Some(Pos { x: 0.5, y: 0.5 });
        let d = danger_at(&i, me, &[], &[], None, &[]).unwrap();
        assert_eq!((d.level, d.label), (0.0, "bezpiecznie"));
        let d = danger_at(&i, me, &[], &[unit("Zed", 0.5, 0.5)], Some(0.2), &["Zed".into()]).unwrap();
        // 0.45·1.6 = 0.72 + matchup (0.4-0.2)·0.6 = 0.12 → 0.84
        assert!((d.level - 0.84).abs() < 1e-9);
        assert_eq!(d.label, "uciekaj");
        assert_eq!(d.reasons, ["Zed blisko (fed)", "słabszy w matchupie"]);
        let fog = |c: &str| FogTrack { champion: c.into(), in_fog: true, estimate: Some(FogEstimate { x: 0.52, y: 0.5, radius: 0.05 }) };
        let d = danger_at(&i, me, &[fog("A"), fog("B"), fog("C")], &[], None, &[]).unwrap();
        assert_eq!(d.reasons[0], "A we mgle blisko");
        assert!(d.level > 0.5);
        assert!(danger_at(&i, None, &[], &[], None, &[]).is_none());
    }

    #[test]
    fn objective_calls() {
        let mut i = intel(vec![player("Ahri", true, true, "MIDDLE", 9)]);
        i.objectives = vec![
            ObjectiveTimer { kind: "baron", label: "Baron".into(), spawns_at: Some(1200.0), alive: false },
            ObjectiveTimer { kind: "dragon", label: "Smok".into(), spawns_at: Some(640.0), alive: false },
        ];
        let c = objective_call(&i, &[], &[], &[]).unwrap();
        assert_eq!((c.objective.as_str(), c.verdict, c.in_sec), ("Smok", "contest", Some(40)));
        i.enemy.alive_count = 3;
        let c = objective_call(&i, &[], &[], &[]).unwrap();
        assert_eq!((c.verdict, c.reason.as_str()), ("go", "2 wrogów martwych"));
        i.enemy.alive_count = 5;
        i.ally.alive_count = 4;
        let c = objective_call(&i, &[], &[], &[unit("Zed", 0.67, 0.7)]).unwrap();
        assert_eq!((c.verdict, c.reason.as_str()), ("skip", "5v4, oddaj"));
        i.objectives[1].spawns_at = Some(700.0);
        assert!(objective_call(&i, &[], &[], &[]).is_none());
        let v = serde_json::to_value(objective_call(&{
            let mut j = i.clone();
            j.objectives[1].alive = true;
            j
        }, &[], &[], &[]).unwrap()).unwrap();
        assert_eq!(v["inSec"], 0);
    }

    #[test]
    fn team_watcher() {
        let i = intel(vec![player("Ahri", true, true, "MIDDLE", 9), player("Vi", true, false, "JUNGLE", 9), player("Zed", false, false, "MIDDLE", 10)]);
        let me = Some(Pos { x: 0.5, y: 0.5 });
        let mut w = TeamWatcher::default();
        let vi = SeenUnit { champion: "Vi".into(), x: 0.4, y: 0.5, vx: Some(0.01), vy: Some(0.0) };
        let a = w.update(&i, me, &[vi.clone()], &[unit("Zed", 0.55, 0.5)], Some(0.6), 40_000.0);
        assert_eq!(a.iter().map(|x| x.text.as_str()).collect::<Vec<_>>(), ["Vi obok → pinguj, wchodźcie", "Vi idzie do ciebie"]);
        assert!(w.update(&i, me, &[vi.clone()], &[unit("Zed", 0.55, 0.5)], Some(0.6), 50_000.0).is_empty());
        assert!(w.update(&i, me, &[vi], &[], Some(0.6), 90_000.0).is_empty()); // no enemy near
    }
}
