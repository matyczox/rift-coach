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
        "dragon" => Pos { x: 0.663, y: 0.703 },
        "baron" | "herald" | "grubs" => Pos { x: 0.337, y: 0.296 },
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
    /// Out of sight long enough to call it a gank threat (4 s; never my lane opponent).
    pub gank_ready: bool,
    /// Probability (all hypotheses) that he is within reach of me; None = unknown.
    pub near_me: Option<f64>,
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
    /// "Dragon", "Elder Dragon", "Baron", "Herald", "Voidgrubs"
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
    if !t.in_fog {
        return 0.0;
    }
    if let Some(m) = t.near_me {
        return (m / 0.6).min(1.0);
    }
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
    // p_me = the local fight estimate (counts allies near me): with the edge on our side, enemies
    // around are less of a threat (>= 0.6: half, 0.5-0.6: three quarters).
    let shield = match p_me { Some(p) if p >= 0.6 => 0.5, Some(p) if p >= 0.5 => 0.75, _ => 1.0 };
    for e in visible_enemies {
        let d = (e.x - me.x).hypot(e.y - me.y);
        if d > 0.15 {
            continue;
        }
        let is_fed = fed.contains(&e.champion);
        // His HP from his health bar (seen in the last 10 s): at 1/12 HP he is barely a threat.
        let hp = intel.players.iter().find(|p| p.champion == e.champion).and_then(|p| match (p.hp_pct, p.hp_seen_at) {
            (Some(h), Some(at)) if intel.game_time - at <= 10.0 => Some(h),
            _ => None,
        });
        let hp_w = hp.map_or(1.0, |h| 0.25 + 0.75 * h.clamp(0.0, 1.0));
        let w = (1.0 - d / 0.15) * if is_fed { 1.6 } else { 1.0 } * 0.45 * shield * hp_w;
        score += w;
        reasons.push((w, format!("{} blisko{}", e.champion, if is_fed { " (fed)" } else { "" })));
    }
    for t in tracks.iter().filter(|t| t.gank_ready) {
        let w = ghost_threat(t, me, 0.12) * if fed.contains(&t.champion) { 1.5 } else { 1.0 } * 0.35 * shield;
        if w > 0.03 {
            score += w;
            reasons.push((w, format!("{} gank", t.champion)));
        }
    }
    let hp = match &intel.my_stats {
        Some(s) if s.max_hp != 0.0 => s.hp / s.max_hp,
        _ => 1.0,
    };
    if hp < 0.5 {
        let w = (0.5 - hp) * 0.8;
        score += w;
        reasons.push((w, format!("Mało HP ({}%) → back", js_round(hp * 100.0) as i64)));
    }
    // Out of mana / energy, or Rumble overheated: fewer spells (or none) when it matters. Counts only
    // when something already threatens me.
    if score > 0.1 {
        if let Some(m) = mi.mana_pct.filter(|m| *m < 0.25) {
            let w = (0.25 - m) * 1.2;
            score += w;
            reasons.push((w, format!("{}% {}", js_round(m * 100.0) as i64, if intel.my_stats.as_ref().is_some_and(|s| s.resource_type == "ENERGY") { "energii" } else { "many" })));
        }
        if intel.my_stats.as_ref().is_some_and(|s| s.overheated) {
            score += 0.3;
            reasons.push((0.3, "przegrzany".into()));
        }
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
        // Counts towards the danger level, but is never shown as the reason (says nothing to act on).
        reasons.push((0.0, String::new()));
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
    Some(DangerState { level, label, reasons: reasons.into_iter().filter(|r| !r.1.is_empty()).take(2).map(|r| r.1).collect() })
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
    let ally_alive = intel.ally.alive_count as f64;
    let enemy_alive = intel.enemy.alive_count as f64;
    let gold_k = intel.gold_diff / 1000.0;
    // Score every candidate and take the best one (ties: the sooner one). Taking the first in the list
    // said "Dragon / skip" with 4 of us at Baron and 3 enemies at Dragon.
    let score_of = |o: &super::game_intel::ObjectiveTimer| -> Option<(f64, usize, Pos)> {
        let pos = obj_pos(o.kind)?;
        let near = |x: f64, y: f64, r: f64| (x - pos.x).hypot(y - pos.y) <= r;
        let allies_near = allies.iter().filter(|a| near(a.x, a.y, 0.2)).count();
        let enemies_seen_near = visible_enemies.iter().filter(|e| near(e.x, e.y, 0.2)).count();
        let enemies_likely_near = tracks.iter().filter(|t| t.in_fog && t.estimate.is_some_and(|e| near(e.x, e.y, 0.2 + e.radius * 0.5))).count();
        let enemy_near = enemies_seen_near as f64 + 0.6 * enemies_likely_near as f64;
        let score = (ally_alive - enemy_alive) * 0.9 + (allies_near as f64 - enemy_near) * 0.6 + gold_k * 0.25 + if o.kind == "baron" { -0.4 } else { 0.0 };
        Some((score, allies_near, pos))
    };
    let (o, (score, allies_near, pos)) = cands
        .iter()
        .filter_map(|o| score_of(o).map(|s| (*o, s)))
        .fold(None::<(&super::game_intel::ObjectiveTimer, (f64, usize, Pos))>, |best, cur| match best {
            Some(b) if b.1 .0 >= cur.1 .0 - 1e-9 => Some(b),
            _ => Some(cur),
        })?;
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
        "dragon" => o.label.clone(),
        "baron" => "Baron".into(),
        "herald" => "Herald".into(),
        "grubs" => "Voidgrubs".into(),
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
            // Really coming to my lane: right next to me, or walking at me — not clearing his camp nearby
            // ("Zaahen obok" while he did Raptors). Not for me as the jungler (I gank, nobody gives me pings).
            let d = (jp.x - me.x).hypot(jp.y - me.y);
            let heading = match (jp.vx, jp.vy) {
                (Some(vx), Some(vy)) => { let sp = vx.hypot(vy); sp > 0.003 && d > 1e-6 && ((me.x - jp.x) * vx + (me.y - jp.y) * vy) / (d * sp) > 0.7 }
                _ => false,
            };
            let ganking = d < 0.08 || (d < 0.18 && heading);
            if mi.role != "JUNGLE" && ganking && enemy_near_me && p_me.unwrap_or(0.5) >= 0.42 && now - self.last_ask > 30_000.0 {
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
                d < 0.25 && d > 0.1 && sp > 0.004 && (dx * vx + dy * vy) / (d * sp) > 0.8
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
        let fog = |c: &str| FogTrack { champion: c.into(), in_fog: true, gank_ready: true, near_me: None, estimate: Some(FogEstimate { x: 0.52, y: 0.5, radius: 0.05 }) };
        let d = danger_at(&i, me, &[fog("A"), fog("B"), fog("C")], &[], None, &[]).unwrap();
        assert_eq!(d.reasons[0], "A gank");
        assert!(d.level > 0.5);
        assert!(danger_at(&i, None, &[], &[], None, &[]).is_none());
    }

    /// Audit V3 F12: two live objectives -> the better one, not the first in the list.
    #[test]
    fn best_objective_not_first() {
        let mut i = intel(vec![player("Ahri", true, true, "MIDDLE", 14)]);
        i.game_time = 1500.0;
        i.objectives = vec![
            ObjectiveTimer { kind: "dragon", label: "Dragon".into(), spawns_at: None, alive: true },
            ObjectiveTimer { kind: "baron", label: "Baron".into(), spawns_at: None, alive: true },
        ];
        let allies = [unit("A", 0.33, 0.3), unit("B", 0.34, 0.31), unit("C", 0.32, 0.29), unit("D", 0.35, 0.3)];
        let enemies = [unit("X", 0.67, 0.7), unit("Y", 0.68, 0.71), unit("Z", 0.66, 0.69)];
        let c = objective_call(&i, &[], &allies, &enemies).unwrap();
        assert_eq!((c.objective.as_str(), c.verdict), ("Baron", "go"));
    }

    #[test]
    fn objective_calls() {
        let mut i = intel(vec![player("Ahri", true, true, "MIDDLE", 9)]);
        i.objectives = vec![
            ObjectiveTimer { kind: "baron", label: "Baron".into(), spawns_at: Some(1200.0), alive: false },
            ObjectiveTimer { kind: "dragon", label: "Dragon".into(), spawns_at: Some(640.0), alive: false },
        ];
        let c = objective_call(&i, &[], &[], &[]).unwrap();
        assert_eq!((c.objective.as_str(), c.verdict, c.in_sec), ("Dragon", "contest", Some(40)));
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
        let vi = SeenUnit { champion: "Vi".into(), x: 0.35, y: 0.5, vx: Some(0.01), vy: Some(0.0) };
        let a = w.update(&i, me, &[vi.clone()], &[unit("Zed", 0.55, 0.5)], Some(0.6), 40_000.0);
        assert_eq!(a.iter().map(|x| x.text.as_str()).collect::<Vec<_>>(), ["Vi obok → pinguj, wchodźcie", "Vi idzie do ciebie"]);
        assert!(w.update(&i, me, &[vi.clone()], &[unit("Zed", 0.55, 0.5)], Some(0.6), 50_000.0).is_empty());
        assert!(w.update(&i, me, &[vi], &[], Some(0.6), 90_000.0).is_empty()); // no enemy near
        // Standing at his camp 0.12 away (not walking at me): no "obok → pinguj".
        let camp = SeenUnit { champion: "Vi".into(), x: 0.38, y: 0.5, vx: Some(0.0), vy: Some(0.0) };
        let mut w2 = TeamWatcher::default();
        assert!(!w2.update(&i, me, &[camp], &[unit("Zed", 0.55, 0.5)], Some(0.6), 40_000.0).iter().any(|a| a.kind == "gank-ask"));
    }
}
