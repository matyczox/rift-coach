//! End-to-end evaluation of fog predictions on a two-pass replay harvest
//! (port of scripts/dev/eval-tracker.ts).
//!
//! Input = what a BLUE player sees (cached blue-view detections + fog mask computed here from
//! the blue frames). Truth = red-team view detections. For every red champion in fog: error of
//! the ghost centre vs truth, whether truth is inside the ghost radius, compared with the
//! naive baseline "still where last seen".
//!
//! usage: eval_tracker <session dir|id> [noFog]     env: TRUTH=red|all, NO_ITEMS=1, CAMPS=1
//!   MODEL=<fog-model.json>  start from this learned model (default: empty)
//!   TRAIN=1                 learn (reappearances + truth positions every 10 s) and save to MODEL
//!                           (without TRAIN learning is off: a pure evaluation of the model)
//!
//! The game-intel inputs the TS script takes from IntelTracker (role, isDead, move speed,
//! item gold, dragon/baron timers) are recomputed here from api.jsonl.gz with the same rules
//! (gameIntel.ts / ddragon.ts), restricted to what the tracker uses.

#[path = "util/session.rs"]
mod session;

use flate2::read::MultiGzDecoder;
use rc_vision::background::load_reference;
use rc_vision::camps::{camp_by_id, read_camps, CampEventKind, CampSide, CampTracker, ReadCampsParams};
use rc_vision::detect::{Detection, Team};
use rc_vision::fog::{sample_mask, FogParams, FogReader};
use rc_vision::image::load_png;
use rc_vision::tracker::{EnemyInput, EnemyTracker, FrameInput, GridTerrain, Learner, Role, TrackState, MAP_UNITS};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::sync::Arc;
use std::time::Instant;

struct ItemData {
    total: f64,
    flat_ms: f64,
    pct_ms: f64,
    purchasable: bool,
}

fn parse_items(items: &Value) -> HashMap<i64, ItemData> {
    let mut out = HashMap::new();
    for (id, it) in items["data"].as_object().unwrap() {
        let idn: i64 = id.parse().unwrap_or(0);
        if idn >= 100000 || it["maps"]["11"] == Value::Bool(false) || it["inStore"] == Value::Bool(false) || !it["requiredAlly"].is_null() {
            continue;
        }
        let st = &it["stats"];
        out.insert(idn, ItemData { total: it["gold"]["total"].as_f64().unwrap_or(0.0), flat_ms: st["FlatMovementSpeedMod"].as_f64().unwrap_or(0.0), pct_ms: st["PercentMovementSpeedMod"].as_f64().unwrap_or(0.0), purchasable: it["gold"]["purchasable"].as_bool().unwrap_or(false) });
    }
    out
}

fn soft_cap(raw: f64) -> f64 {
    if raw > 490.0 {
        raw * 0.5 + 230.0
    } else if raw > 415.0 {
        raw * 0.8 + 83.0
    } else if raw < 220.0 {
        raw * 0.5 + 110.0
    } else {
        raw
    }
}

struct Player {
    champion: String,
    team: Team,
    role: Role,
    is_dead: bool,
    move_speed: f64,
    item_gold: f64,
}

struct Intel {
    players: Vec<Player>,
    /// (alive, spawnsAt)
    dragon: (bool, Option<f64>),
    baron: (bool, Option<f64>),
}

fn name_of(p: &Value) -> String {
    if let Some(s) = p["riotIdGameName"].as_str() {
        return s.into();
    }
    if let Some(s) = p["riotId"].as_str() {
        return s.split('#').next().unwrap_or("").into();
    }
    p["summonerName"].as_str().unwrap_or("").into()
}

fn intel(d: &Value, t: f64, items: &HashMap<i64, ItemData>, champ_ms: &HashMap<String, f64>) -> Intel {
    let all = d["allPlayers"].as_array().unwrap();
    let players = all
        .iter()
        .map(|p| {
            let its = p["items"].as_array().cloned().unwrap_or_default();
            // Shop-only items (recall proof, as in the app): evolving pets / support quest items excluded.
            let item_gold: f64 = its.iter().filter_map(|i| items.get(&i["itemID"].as_i64().unwrap_or(0)).filter(|x| x.purchasable).map(|x| x.total)).sum();
            let (flat, pct) = its.iter().fold((0.0, 0.0), |(f, pc), i| match items.get(&i["itemID"].as_i64().unwrap_or(0)) {
                Some(x) => (f + x.flat_ms, pc + x.pct_ms),
                None => (f, pc),
            });
            let champion = p["championName"].as_str().unwrap_or("").to_string();
            let base = champ_ms.get(&champion).copied().unwrap_or(340.0);
            let pos = p["position"].as_str().unwrap_or("");
            let mut role = Role::parse(pos);
            if role == Role::Unknown {
                let smite = p["summonerSpells"].as_object().map(|o| o.values().any(|s| {
                    let n = s["displayName"].as_str().unwrap_or("").to_lowercase();
                    n.contains("smite") || n.contains("porażenie")
                }));
                if smite == Some(true) {
                    role = Role::Jungle;
                }
            }
            Player {
                champion,
                team: Team::parse(p["team"].as_str().unwrap_or("ORDER")).unwrap_or(Team::Order),
                role,
                is_dead: p["isDead"].as_bool().unwrap_or(false),
                move_speed: soft_cap((base + flat) * (1.0 + pct)).round(),
                item_gold,
            }
        })
        .collect::<Vec<_>>();

    // Objectives (gameIntel.ts events()): only dragon / baron are used by the tracker.
    let name_team: HashMap<String, Team> = all.iter().map(|p| (name_of(p), Team::parse(p["team"].as_str().unwrap_or("ORDER")).unwrap_or(Team::Order))).collect();
    let team_of = |n: &Value| -> Option<Team> {
        let s = n.as_str().map(String::from).unwrap_or_else(|| if n.is_null() { String::new() } else { n.to_string() });
        if let Some(t) = name_team.get(&s) {
            return Some(*t);
        }
        if let Some(b) = all.iter().find(|p| format!("Bot {}", p["championName"].as_str().unwrap_or("")) == s) {
            return Team::parse(b["team"].as_str().unwrap_or(""));
        }
        let l = s.to_lowercase();
        if l.contains("torder") || l.contains("t100") || s.contains("_T1_") {
            return Some(Team::Order);
        }
        if l.contains("tchaos") || l.contains("t200") || s.contains("_T2_") {
            return Some(Team::Chaos);
        }
        None
    };
    let mut seen = HashSet::new();
    let mut evs: Vec<&Value> = d["events"]["Events"]
        .as_array()
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    evs.retain(|e| {
        let et = e["EventTime"].as_f64().unwrap_or(0.0);
        if et > t + 1.0 {
            return false;
        }
        let k = format!("{}|{}|{}|{}", e["EventName"].as_str().unwrap_or(""), et.round(), e["KillerName"].as_str().unwrap_or(""), e["VictimName"].as_str().unwrap_or(""));
        seen.insert(k)
    });
    evs.sort_by(|a, b| a["EventTime"].as_f64().partial_cmp(&b["EventTime"].as_f64()).unwrap());
    let (mut last_dragon, mut last_baron, mut elder) = (None::<f64>, None::<f64>, false);
    let mut drag_count: HashMap<Team, usize> = HashMap::new();
    for e in evs {
        let et = e["EventTime"].as_f64().unwrap_or(0.0);
        match e["EventName"].as_str().unwrap_or("") {
            "DragonKill" => {
                if e["DragonType"].as_str() == Some("Elder") {
                    last_dragon = Some(et);
                } else {
                    if let Some(team) = team_of(&e["KillerName"]) {
                        let c = drag_count.entry(team).or_insert(0);
                        *c += 1;
                        if *c >= 4 {
                            elder = true;
                        }
                    }
                    last_dragon = Some(et);
                }
            }
            "BaronKill" => last_baron = Some(et),
            _ => {}
        }
    }
    let dragon_at = last_dragon.map_or(300.0, |l| l + if elder { 360.0 } else { 300.0 });
    let baron_at = last_baron.map_or(1200.0, |l| l + 360.0);
    Intel {
        players,
        dragon: (dragon_at <= t, if dragon_at > t { Some(dragon_at) } else { None }),
        baron: (baron_at <= t, if baron_at > t { Some(baron_at) } else { None }),
    }
}

fn nearest(arr: &[u64], t: u64) -> u64 {
    let (mut lo, mut hi) = (0usize, arr.len() - 1);
    while lo < hi {
        let m = (lo + hi) >> 1;
        if arr[m] < t {
            lo = m + 1;
        } else {
            hi = m;
        }
    }
    let a = arr[lo];
    if lo > 0 {
        let b = arr[lo - 1];
        if (b as i64 - t as i64).abs() < (a as i64 - t as i64).abs() {
            return b;
        }
    }
    a
}

// Harvester offsets come from js_round(f64), so serde writes e.g. 12000.0.
// as_u64 alone rejects those and used to turn the entire API timeline into time zero.
fn api_time_ms(v: &Value) -> Option<u64> {
    v.as_u64().or_else(|| v.as_f64().filter(|t| t.is_finite() && *t >= 0.0).map(|t| t.round() as u64))
}

#[test]
fn api_offsets_accept_harvester_floats_and_legacy_integers() {
    assert_eq!(api_time_ms(&serde_json::json!(12000.0)), Some(12000));
    assert_eq!(api_time_ms(&serde_json::json!(12000)), Some(12000));
    assert_eq!(api_time_ms(&serde_json::json!(-1.0)), None);
    assert_eq!(api_time_ms(&Value::Null), None);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = session::session_dir(args.first().expect("usage: eval_tracker <session> [noFog]"));
    let use_fog = args.get(1).map(String::as_str) != Some("noFog");
    let truth_sub = std::env::var("TRUTH").unwrap_or_else(|_| "red".into());
    let no_items = std::env::var("NO_ITEMS").is_ok();
    let use_camps = std::env::var("CAMPS").is_ok();

    let dd = session::ddragon();
    let items = parse_items(&dd.raw["items"]);
    let champ_ms: HashMap<String, f64> = dd.raw["champions"]["data"].as_object().unwrap().values().map(|c| (c["name"].as_str().unwrap().to_string(), c["stats"]["movespeed"].as_f64().unwrap())).collect();
    let blue = session::cached(&dir, "frames");
    let red = session::cached(&dir, &truth_sub);
    let mut gz = Vec::new();
    // The recorder's gzip stream may be unterminated (TS reads it with Z_SYNC_FLUSH): keep what decodes.
    let _ = MultiGzDecoder::new(&std::fs::read(dir.join("api.jsonl.gz")).unwrap()[..]).read_to_end(&mut gz);
    let text = String::from_utf8_lossy(&gz);
    let api: Vec<(u64, Value)> = text
        .trim()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|mut v| Some((api_time_ms(&v["o"])?, v["d"].take())))
        .collect();
    assert!(!api.is_empty(), "no valid API timeline in api.jsonl.gz");
    let red_times: Vec<u64> = red.keys().copied().collect();
    let blue_times: Vec<u64> = blue.keys().copied().collect();

    let terrain = Arc::new(GridTerrain::from_asset(128));
    let model_path = std::env::var("MODEL").ok().map(std::path::PathBuf::from);
    let train = std::env::var("TRAIN").is_ok();
    let learner = match &model_path {
        Some(p) if train => Learner::new(Some(p.clone())),
        Some(p) => {
            // Evaluation only: load, never write back.
            let mut l = Learner::new(None);
            if let Ok(bytes) = std::fs::read(p) {
                if let Ok(m) = serde_json::from_slice(&bytes) {
                    l.model = m;
                }
            }
            l
        }
        None => Learner::new(None),
    };
    let mut tracker = EnemyTracker::new(terrain, learner, 300);
    tracker.learning_enabled = train;
    let mut last_train_t = 0.0;
    let mut reader: Option<FogReader> = None;
    let mut camp_t = CampTracker::default();

    let buckets: [(f64, f64); 5] = [(0.0, 5.0), (5.0, 15.0), (15.0, 30.0), (30.0, 60.0), (60.0, 120.0)];
    #[derive(Default, Clone)]
    struct S {
        n: usize,
        err_pred: f64,
        err_base: f64,
        inside: usize,
        better: usize,
        radius: f64,
    }
    let mut stats = vec![S::default(); buckets.len()];
    let ks = [0.4, 0.6, 0.8, 1.0];
    let mut cal = vec![vec![(0usize, 0f64); ks.len()]; buckets.len()];
    let mut speed_of: HashMap<String, f64> = HashMap::new();
    let mut api_idx = 0;
    let t0 = Instant::now();
    let mut ms_fog = 0.0;
    let mut ms_track = 0.0;

    for &ms in &blue_times {
        if ms < 30000 {
            continue;
        }
        while api_idx < api.len() - 1 && api[api_idx + 1].0 <= ms {
            api_idx += 1;
        }
        let t = ms as f64 / 1000.0;
        let it = intel(&api[api_idx].1, t, &items, &champ_ms);
        let enemies: Vec<EnemyInput> = it
            .players
            .iter()
            .filter(|p| p.team == Team::Chaos)
            .map(|p| EnemyInput { item_gold: if no_items { None } else { Some(p.item_gold) }, has_flash: true, is_dead: p.is_dead, ..EnemyInput::new(&p.champion, Team::Chaos, p.role, p.move_speed) })
            .collect();
        for e in &enemies {
            speed_of.insert(e.champion.clone(), e.move_speed);
        }
        let mut mask: Option<Vec<f32>> = None;
        let mut img_opt = None;
        if use_fog {
            let tf = Instant::now();
            let img = load_png(dir.join("frames").join(format!("{ms}.png"))).unwrap();
            let r = reader.get_or_insert_with(|| FogReader::new(&load_reference(img.width), 64, FogParams::default()));
            mask = Some(r.read(&img));
            img_opt = Some(img);
            ms_fog += tf.elapsed().as_secs_f64() * 1e3;
        }
        let visible_fn = mask.as_ref().map(|m| move |x: f64, y: f64| sample_mask(m, 64, x, y));
        let visible: Option<&dyn Fn(f64, f64) -> f64> = visible_fn.as_ref().map(|f| f as &dyn Fn(f64, f64) -> f64);
        let mut dets: Vec<Detection> = blue[&ms]
            .iter()
            .filter(|d| d.t == "CHAOS")
            .map(|d| Detection { champion: d.c.clone(), team: Team::Chaos, x: d.x, y: d.y, score: d.s, margin: 0.0, votes: 0.0 })
            .collect();
        if use_camps {
            if let Some(img) = &img_opt {
                let n0 = camp_t.events.len();
                camp_t.update(t, &read_camps(img, &ReadCampsParams::default()), visible);
                let jg = it.players.iter().find(|p| p.team == Team::Chaos && p.role == Role::Jungle);
                for ev in camp_t.events[n0..].to_vec() {
                    let Some(camp) = camp_by_id(&ev.id) else { continue };
                    if let Some(jg) = jg {
                        if ev.kind == CampEventKind::Cleared && camp.side == CampSide::Chaos && ev.earliest.is_none_or(|e| ev.at - e <= 10.0) && !dets.iter().any(|d| d.champion == jg.champion) {
                            tracker.soft_hint(&jg.champion, (camp.x, camp.y));
                        }
                    }
                }
            }
        }
        let tt = Instant::now();
        let tracks = tracker.update(&FrameInput {
            t,
            detections: &dets,
            visible,
            enemies: &enemies,
            dragon_in: Some(if it.dragon.0 { 0.0 } else { it.dragon.1.unwrap_or(t) - t }),
            baron_in: Some(if it.baron.0 { 0.0 } else { it.baron.1.unwrap_or(t) - t }),
        });
        ms_track += tt.elapsed().as_secs_f64() * 1e3;

        let rt = nearest(&red_times, ms);
        if (rt as i64 - ms as i64).abs() > 300 {
            continue;
        }
        if train && t - last_train_t >= 10.0 {
            last_train_t = t;
            for tr in &tracks {
                if tr.state != TrackState::Fog {
                    continue;
                }
                if let Some(truth) = red[&rt].iter().find(|d| d.c == tr.champion && d.t == "CHAOS") {
                    tracker.train_at(&tr.champion, (truth.x, truth.y), t);
                }
            }
        }
        for tr in &tracks {
            let (TrackState::Fog, Some(est), Some(last), Some(fs)) = (tr.state, tr.estimate, tr.last, tr.fog_since) else { continue };
            let Some(truth) = red[&rt].iter().find(|d| d.c == tr.champion && d.t == "CHAOS") else { continue };
            let age = t - fs;
            let Some(b) = buckets.iter().position(|&(lo, hi)| age >= lo && age < hi) else { continue };
            let s = &mut stats[b];
            let ep = (truth.x - est.x).hypot(truth.y - est.y);
            let eb = (truth.x - last.x).hypot(truth.y - last.y);
            s.n += 1;
            s.err_pred += ep;
            s.err_base += eb;
            s.radius += est.radius;
            if ep <= est.radius {
                s.inside += 1;
            }
            if ep < eb {
                s.better += 1;
            }
            for (ki, k) in ks.iter().enumerate() {
                let r = est.radius.max(k * (speed_of.get(&tr.champion).copied().unwrap_or(345.0) / MAP_UNITS) * age).min(0.45);
                cal[b][ki].1 += r;
                if ep <= r {
                    cal[b][ki].0 += 1;
                }
            }
        }
    }

    println!(
        "fog mask: {}; {} frames in {:.1} s (fog {:.2} ms/frame incl. PNG decode, tracker {:.2} ms/frame)",
        if use_fog { "on" } else { "off" },
        blue_times.len(),
        t0.elapsed().as_secs_f64(),
        ms_fog / blue_times.len() as f64,
        ms_track / blue_times.len() as f64
    );
    println!("fog age   samples  err model  err baseline  model better  truth inside circle  avg radius");
    for (i, &(lo, hi)) in buckets.iter().enumerate() {
        let s = &stats[i];
        if s.n == 0 {
            continue;
        }
        let n = s.n as f64;
        println!(
            "{:>3}-{:<4}s {:>7}   {:>6.1}%      {:>6.1}%       {:>4.0}%          {:>4.0}%          {:.1}%",
            lo,
            hi,
            s.n,
            s.err_pred / n * 100.0,
            s.err_base / n * 100.0,
            100.0 * s.better as f64 / n,
            100.0 * s.inside as f64 / n,
            s.radius / n * 100.0
        );
    }
    println!("calibrated radius r = max(pf radius, K * speed * age): inside% / avg radius");
    for (i, &(lo, hi)) in buckets.iter().enumerate() {
        let s = &stats[i];
        if s.n == 0 {
            continue;
        }
        let parts: Vec<String> = ks.iter().enumerate().map(|(ki, k)| format!("K={k}: {:.0}% r={:.0}%", 100.0 * cal[i][ki].0 as f64 / s.n as f64, 100.0 * cal[i][ki].1 / s.n as f64)).collect();
        println!("{lo}-{hi}s  {}", parts.join("  "));
    }
    println!("learner {}", serde_json::to_string(&tracker.learner.summary()).unwrap());
    // One machine-readable line for comparing variants.
    let rows: Vec<serde_json::Value> = buckets
        .iter()
        .enumerate()
        .filter(|(i, _)| stats[*i].n > 0)
        .map(|(i, &(lo, hi))| {
            let s = &stats[i];
            let n = s.n as f64;
            serde_json::json!({ "lo": lo, "hi": hi, "n": s.n, "errModel": s.err_pred / n, "errBase": s.err_base / n, "better": s.better as f64 / n, "inside": s.inside as f64 / n })
        })
        .collect();
    println!("SUMMARY {}", serde_json::json!({ "buckets": rows }));
    if train {
        tracker.learner.save().expect("save model");
    }
}
