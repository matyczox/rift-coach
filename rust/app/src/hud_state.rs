//! HUD state composition (port of `src/main/hudState.ts`): what the in-game overlay shows.

use crate::vision::{Alert, MinimapGhost};
use rc_core::coach::Coach;
use rc_core::intel::game_intel::GameIntel;
use rc_core::wards::{ward_point, WARD_SPOTS};
use serde_json::{json, Value};

/// Fixed points on the minimap, normalized 0..1 (blue base bottom-left; never flipped).
/// map_point for targets relative to the player's side ("base", "my-jungle", "enemy-jungle"):
/// mirrored for the red side; objectives and lanes stay absolute.
pub fn map_point_for(name: &str, red_side: bool) -> Option<(f64, f64)> {
    let p = map_point(name)?;
    Some(if red_side && matches!(name, "base" | "my-jungle" | "enemy-jungle") { (1.0 - p.0, 1.0 - p.1) } else { p })
}

pub fn map_point(name: &str) -> Option<(f64, f64)> {
    Some(match name {
        "dragon" => (0.67, 0.70),
        "baron" | "herald" | "grubs" => (0.33, 0.30),
        "atakhan" => (0.5, 0.5),
        "top" => (0.10, 0.10),
        "mid" => (0.50, 0.50),
        "bot" => (0.90, 0.90),
        "base" => (0.06, 0.94),
        "my-jungle" => (0.27, 0.62),
        "enemy-jungle" => (0.73, 0.38),
        _ => return None,
    })
}

fn fmt_t(s: f64) -> String {
    format!("{}:{:02}", (s / 60.0).floor() as i64, (s % 60.0).floor() as i64)
}

/// Quick local alerts that don't need the vision system.
pub fn local_alerts(intel: &GameIntel, now: f64) -> Vec<Alert> {
    let mut out = Vec::new();
    let Some(me) = intel.me.as_ref() else { return out };
    // Lane opponent just spiked and I haven't matched it.
    for p in &intel.players {
        if p.is_ally || p.role != me.role || p.role == "UNKNOWN" {
            continue;
        }
        let Some(s) = p.spikes.first() else { continue };
        if intel.game_time - s.at > 45.0 {
            continue;
        }
        let behind = if s.kind == "level" { me.level < p.level } else { me.completed_items < p.completed_items };
        if behind {
            out.push(Alert::new("spike", format!("{}: {}", p.champion, s.label), 1.0, now + 8000.0));
        }
    }
    // Baron / Elder windows: shown as a countdown chip (top right), not as an alert.
    if let Some(ace) = &intel.ace {
        out.push(if ace.team == "ally" { Alert::new("ally", "ACE — push / Baron teraz", 1.0, now + 3000.0) } else { Alert::new("missing", "Wróg zrobił ACE — cofnij się", 1.0, now + 3000.0) });
    }
    for p in &intel.players {
        let s = intel.streaks.get(&p.champion).copied().unwrap_or(0);
        if !p.is_ally && !p.is_dead && s >= 4 {
            out.push(Alert::new("fed", format!("{}: {s} killi z rzędu — duży shutdown", p.champion), 0.9, now + 3000.0));
        }
    }
    out
}

// ---------- ward choice (smartWardSpot from shared/wards.ts) ----------

/// What each spot watches, its kind and when it exists (the parts of wards.ts rc-core lacks).
fn spot_meta(id: &str) -> (&'static [&'static str], &'static str, &'static str) {
    match id {
        "faelight-river-top" => (&["baron", "herald", "gank-top", "enemy-jungler"], "faelight", "start"),
        "faelight-river-bot" => (&["dragon", "gank-bot", "enemy-jungler"], "faelight", "start"),
        "faelight-my-base" => (&["defend-jungle", "gank-mid"], "faelight", "start"),
        "faelight-enemy-base" => (&["invade", "enemy-jungler"], "faelight", "start"),
        "faelight-mid-top" => (&["gank-mid", "baron"], "faelight", "rift-transform"),
        "faelight-mid-bot" => (&["gank-mid", "dragon"], "faelight", "rift-transform"),
        "dragon-pit-entrance" => (&["dragon"], "ward", ""),
        "river-bush-bot" => (&["dragon", "gank-bot", "enemy-jungler"], "ward", ""),
        "baron-pit-entrance" => (&["baron", "herald"], "ward", ""),
        "river-bush-top" => (&["baron", "herald", "gank-top", "enemy-jungler"], "ward", ""),
        "mid-river-bot" => (&["gank-mid", "dragon"], "ward", ""),
        "mid-river-top" => (&["gank-mid", "baron"], "ward", ""),
        "my-tri-bush-bot" => (&["gank-bot", "defend-jungle"], "ward", ""),
        "my-tri-bush-top" => (&["gank-top", "defend-jungle"], "ward", ""),
        "my-raptors" => (&["defend-jungle", "gank-mid"], "ward", ""),
        "my-blue-buff" | "my-red-buff" => (&["defend-jungle"], "ward", ""),
        "enemy-tri-bush-bot" | "enemy-tri-bush-top" | "enemy-blue-buff" | "enemy-red-buff" => (&["enemy-jungler", "invade"], "ward", ""),
        "enemy-raptors" => (&["enemy-jungler", "invade", "gank-mid"], "ward", ""),
        "top-lane-bush" => (&["gank-top"], "ward", ""),
        "bot-lane-bush" => (&["gank-bot"], "ward", ""),
        "plant-dragon" => (&["dragon", "enemy-jungler"], "plant", ""),
        "plant-baron" => (&["baron", "herald", "enemy-jungler"], "plant", ""),
        "plant-my-jungle-bot" => (&["defend-jungle", "dragon"], "plant", ""),
        "plant-my-jungle-top" => (&["defend-jungle", "gank-top"], "plant", ""),
        _ => (&[], "ward", ""),
    }
}

fn ward_label(id: &str) -> &'static str {
    match spot_meta(id).1 {
        "plant" => "roślina",
        "faelight" => "faelight",
        _ => "ward",
    }
}

/// Vision-aware ward choice: value × relevance, minus current coverage, plus expected enemy
/// presence (fog circles), minus walking distance. None when every relevant spot is watched.
fn smart_ward_spot(purposes: &[&str], visible: Option<&dyn Fn(f64, f64) -> f64>, ghosts: &[MinimapGhost], me: Option<(f64, f64)>, red: bool, transformed: bool) -> Option<&'static str> {
    let mut best: Option<(&'static str, f64)> = None;
    for s in WARD_SPOTS {
        let (watches, kind, spawn) = spot_meta(s.id);
        let relevance = purposes.iter().filter(|p| watches.contains(p)).count() as f64;
        if relevance == 0.0 || (spawn == "rift-transform" && !transformed) {
            continue;
        }
        let (px, py) = ward_point(s.id, red)?;
        let mut covered = 0.0;
        if let Some(vis) = visible {
            let mut n = 0.0;
            for k in 0..12 {
                for r in [0.0, 0.03, 0.055] {
                    let a = k as f64 / 12.0 * std::f64::consts::PI * 2.0;
                    covered += vis((px + a.cos() * r).clamp(0.0, 0.999), (py + a.sin() * r).clamp(0.0, 0.999));
                    n += 1.0;
                }
            }
            covered /= n;
        }
        if covered > 0.6 {
            continue; // already watched (ally/ward there)
        }
        let info: f64 = ghosts
            .iter()
            .map(|g| {
                let d = (g.x - px).hypot(g.y - py);
                if d < g.radius + 0.06 {
                    1.0 - d / (g.radius + 0.06)
                } else {
                    0.0
                }
            })
            .sum();
        let dist = me.map(|m| (px - m.0).hypot(py - m.1)).unwrap_or(0.0);
        let score = s.value as f64 * (0.7 + 0.3 * relevance) * (1.0 - covered) + if kind == "faelight" { 0.5 } else { 0.0 } + 1.5 * info - 3.0 * dist;
        if best.is_none_or(|b| score > b.1) {
            best = Some((s.id, score));
        }
    }
    best.map(|b| b.0)
}

pub struct HudInputs<'a> {
    pub visible: bool,
    pub intel: Option<&'a GameIntel>,
    pub advice: Option<&'a Value>,
    pub alerts: &'a [Alert],
    pub ghosts: &'a [MinimapGhost],
    /// Opponent profiles (Riot API) — none until that feature is wired.
    pub profiles: &'a [Value],
    pub champion_icon: &'a dyn Fn(&str) -> String,
    pub show_frame: bool,
    pub recording: bool,
    pub power: Option<Value>,
    pub build: Option<Value>,
    pub danger: Option<Value>,
    pub objective: Option<Value>,
    pub fed: Vec<String>,
    /// P(point is in our vision now), from the fog mask (includes existing wards).
    pub vision_at: Option<&'a dyn Fn(f64, f64) -> f64>,
    pub me: Option<(f64, f64)>,
    pub now: f64,
}

/// `composeHud`: the HudState JSON the overlay renders.
pub fn compose_hud(i: &HudInputs) -> Value {
    let now = i.now;
    let intel = i.intel;
    let advice = match (i.advice, intel) {
        (Some(a), Some(intel)) if Coach::is_fresh(a, intel) => Some(a),
        _ => None,
    };
    let game_time = intel.map(|x| x.game_time).unwrap_or(0.0);

    let next_obj = intel.and_then(|x| {
        x.objectives
            .iter()
            .filter(|o| o.spawns_at.is_some_and(|s| s > 0.0 && s - game_time <= 90.0) && o.kind != "inhibitor")
            .min_by(|a, b| a.spawns_at.partial_cmp(&b.spawns_at).unwrap_or(std::cmp::Ordering::Equal))
    });

    let mut hints: Vec<Value> = Vec::new();
    if let Some(mh) = advice.and_then(|a| a.get("mapHint")).filter(|v| v.is_object()) {
        let target = mh.get("target").and_then(Value::as_str).unwrap_or("");
        let red_side = intel.and_then(|x| x.me.as_ref()).is_some_and(|m| m.team == "CHAOS");
        let p = map_point_for(target, red_side).or_else(|| Some((mh.get("x")?.as_f64()?, mh.get("y")?.as_f64()?)));
        if let Some((x, y)) = p {
            let kind = match mh.get("type").and_then(Value::as_str) {
                Some("avoid") => "avoid",
                Some("objective") => "objective",
                _ => "move",
            };
            hints.push(json!({ "kind": kind, "x": x, "y": y, "expiresAt": now + 15000.0 }));
        }
    }

    // Wards: coach suggestion, else a local rule 60–90 s before dragon/baron.
    let red = intel.and_then(|x| x.me.as_ref()).is_some_and(|m| m.team == "CHAOS");
    let advice_ward = advice.and_then(|a| a.get("ward")).and_then(|w| w.get("spot")).and_then(Value::as_str);
    if let Some(spot) = advice_ward {
        if let Some((x, y)) = ward_point(spot, red) {
            hints.push(json!({ "kind": "ward", "x": x, "y": y, "label": ward_label(spot), "expiresAt": now + 15000.0 }));
        }
    } else if let Some(intel) = intel.filter(|x| !x.me.as_ref().is_some_and(|m| m.is_dead)) {
        // Vision-aware: skip spots our team already sees, prefer spots where an enemy probably is.
        let transformed = intel.ally.dragons.len() + intel.enemy.dragons.len() >= 2;
        let obj = intel.objectives.iter().find(|o| (o.kind == "dragon" || o.kind == "baron") && o.spawns_at.is_some_and(|s| s > 0.0 && s - game_time <= 90.0 && s - game_time >= 30.0));
        let enemy_jg = i.ghosts.iter().find(|g| g.age > 20.0 && intel.players.iter().any(|p| !p.is_ally && p.role == "JUNGLE" && p.champion == g.champion));
        let purposes: Option<Vec<&str>> = if let Some(o) = obj {
            Some(vec![if o.kind == "dragon" { "dragon" } else { "baron" }])
        } else if let (Some(_), Some(me)) = (enemy_jg, i.me) {
            let lane = if me.0 + me.1 < 0.75 {
                "gank-top"
            } else if me.0 + me.1 > 1.25 {
                "gank-bot"
            } else {
                "gank-mid"
            };
            Some(vec!["enemy-jungler", lane])
        } else {
            None
        };
        if let Some(purposes) = purposes {
            if let Some(id) = smart_ward_spot(&purposes, i.vision_at, i.ghosts, i.me, red, transformed) {
                if let Some((x, y)) = ward_point(id, red) {
                    hints.push(json!({ "kind": "ward", "x": x, "y": y, "label": ward_label(id), "expiresAt": now + 3000.0 }));
                }
            }
        }
    }

    let me = intel.and_then(|x| x.me.as_ref());
    let dead = me.is_some_and(|m| m.is_dead);
    let loading_card = match intel {
        Some(_) if game_time < 25.0 && !i.profiles.is_empty() => Some(json!({
            "enemies": i.profiles.iter().map(|p| {
                let champ = p.get("champion").and_then(Value::as_str).unwrap_or("");
                json!({ "champion": champ, "role": p.get("role"), "iconUrl": (i.champion_icon)(champ), "summary": p.get("summary"), "rank": p.get("rank") })
            }).collect::<Vec<_>>()
        })),
        _ => None,
    };
    let top_bar = match advice {
        Some(a) if !dead => {
            let tag = next_obj.map(|o| format!("{} {}", o.label.to_lowercase(), fmt_t(o.spawns_at.unwrap_or(game_time) - game_time)));
            let _ = tag; // objective timers live in the HUD objective panel, not the coach bar
            json!({ "call": a.get("call"), "tone": a.get("tone"), "tag": Value::Null, "expiresAt": now + 3000.0 })
        }
        _ => Value::Null,
    };
    // A threat to me (gank / invade) beats "they may be doing Dragon".
    // Kinds the top bar does not show (ally prompts...) must not hide one it does.
    let rank = |a: &crate::vision::Alert| a.confidence - match a.kind.as_str() { "gank" | "invade" | "missing" => 0.0, "objective" => 0.5, _ => 1.0 };
    let alert = i.alerts.iter().filter(|a| a.expires_at > now).max_by(|a, b| rank(a).partial_cmp(&rank(b)).unwrap_or(std::cmp::Ordering::Equal));
    let death_card = match me {
        Some(m) if dead => json!({
            "title": format!("Nie żyjesz · respawn {}", fmt_t(m.respawn_in)),
            "points": advice.and_then(|a| a.get("deathReview")).filter(|v| v.is_array()).cloned().unwrap_or_else(|| json!(["Analizuję…"])),
            "respawnAt": game_time + m.respawn_in,
        }),
        _ => Value::Null,
    };
    let timers: Vec<Value> = intel
        .map(|x| {
            x.objectives
                .iter()
                .filter(|o| o.spawns_at.is_some_and(|s| s > 0.0 && s - game_time <= 60.0))
                .filter_map(|o| {
                    let (px, py) = map_point(o.kind)?;
                    Some(json!({ "kind": o.kind, "label": o.label.to_lowercase(), "x": px, "y": py, "secondsLeft": (o.spawns_at.unwrap() - game_time).round() }))
                })
                .collect()
        })
        .unwrap_or_default();
    let has_loading = loading_card.is_some();
    json!({
        "visible": i.visible && intel.is_some(),
        "gameTime": game_time,
        "topBar": top_bar,
        "alert": alert,
        "deathCard": death_card,
        "loadingCard": loading_card,
        "power": if !dead && !has_loading { i.power.clone() } else { None },
        "build": i.build,
        "danger": if !dead { i.danger.clone() } else { None },
        "objective": i.objective,
        "fed": i.fed,
        "ghosts": i.ghosts,
        "hints": hints,
        "timers": timers,
        "showFrame": i.show_frame,
        "recording": i.recording,
    })
}
