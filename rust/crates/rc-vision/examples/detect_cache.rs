//! Re-run the production (Rust) detector over every frame of a harvested replay and write
//! `<session>/detections-<sub>.json` (same format as scripts/dev/detect-cache.ts), using the
//! in-game minimap icons (CommunityDragon circle_0, as the app does) with the Data Dragon square
//! as fallback. Frames are processed in parallel.
//!
//! usage: detect_cache <session dir|id> [sub=frames|red|all ...]

#[path = "util/session.rs"]
mod session;

use rayon::prelude::*;
use rc_vision::detect::{build_template, detect, DetectParams, Team};
use rc_vision::icons::load_champion_icon;
use rc_vision::image::load_png;
use std::collections::BTreeMap;
use std::time::Instant;

fn circle_icon(dd_id: &str) -> Option<rc_vision::Rgba> {
    let id = dd_id.to_lowercase();
    let dir = session::root().join("ddragon").join("icons");
    let file = dir.join(format!("cd-{id}-0.png"));
    if !file.exists() {
        // Fetch like the app does (curl keeps this example free of an HTTP client dependency).
        let url = format!("https://raw.communitydragon.org/latest/game/assets/characters/{id}/hud/{id}_circle_0.png");
        let ok = std::process::Command::new("curl").args(["-sf", "--max-time", "10", "-o"]).arg(&file).arg(&url).status().map(|s| s.success()).unwrap_or(false);
        if !ok {
            let _ = std::fs::remove_file(&file);
            return None;
        }
    }
    load_png(&file).ok()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = session::session_dir(args.first().expect("usage: detect_cache <session> [subs...]"));
    let subs: Vec<String> = if args.len() > 1 { args[1..].to_vec() } else { vec!["frames".into(), "red".into(), "all".into()] };
    let dd = session::ddragon();
    let meta = session::meta(&dir);
    let icons_dir = session::root().join("ddragon").join("icons");
    for sub in subs {
        if !dir.join(&sub).is_dir() {
            continue;
        }
        let times = session::frame_times(&dir, &sub);
        let Some(&first) = times.first() else { continue };
        let width = load_png(dir.join(&sub).join(format!("{first}.png"))).unwrap().width;
        let own = if sub == "red" { Team::Chaos } else { Team::Order };
        let p = DetectParams { own_team: Some(own), red_team: Some(Team::Chaos), ..DetectParams::default() };
        let mut templates = Vec::new();
        for (team, list) in [(Team::Order, &meta.allies), (Team::Chaos, &meta.enemies)] {
            for c in list {
                let id = dd.ids.resolve(c);
                let icon = match circle_icon(&id) {
                    Some(i) => i,
                    // STRICT_ICONS=1: stop instead of detecting with the worse square icon.
                    None if std::env::var("STRICT_ICONS").is_ok() && !icons_dir.join(format!("cd-{}-0.png.missing", id.to_lowercase())).exists() => panic!("no game icon for {c} ({id})"),
                    None => load_champion_icon(&icons_dir, &dd.version, &id).unwrap_or_else(|e| panic!("icon {c} ({id}): {e}")),
                };
                templates.push(build_template(c, team, &icon, width, &p));
            }
        }
        let t0 = Instant::now();
        let out: BTreeMap<u64, Vec<serde_json::Value>> = times
            .par_iter()
            .map(|&ms| {
                let img = load_png(dir.join(&sub).join(format!("{ms}.png"))).unwrap();
                let dets = detect(&img, &templates, &p, &[])
                    .detections
                    .into_iter()
                    .map(|d| {
                        serde_json::json!({
                            "c": d.champion,
                            "t": if d.team == Team::Order { "ORDER" } else { "CHAOS" },
                            "x": (d.x * 1e4).round() / 1e4,
                            "y": (d.y * 1e4).round() / 1e4,
                            "s": (d.score * 1e3).round() / 1e3,
                        })
                    })
                    .collect();
                (ms, dets)
            })
            .collect();
        let as_str: BTreeMap<String, Vec<serde_json::Value>> = out.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        std::fs::write(dir.join(format!("detections-{sub}.json")), serde_json::to_string(&as_str).unwrap()).unwrap();
        println!("{sub}: {} frames in {:.0} s", times.len(), t0.elapsed().as_secs_f64());
    }
}
