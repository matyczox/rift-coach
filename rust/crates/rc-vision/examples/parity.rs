//! Parity + speed of the Rust portrait detector against the cached TS detector output.
//!
//! usage: parity <session dir|id> [sub=frames|red|all] [every=1] [tol=0.01]
//!
//! Runs `detect` (full sweep, own team as in scripts/dev/detect-cache.ts) on every `every`-th
//! frame of <session>/<sub>/ and compares with <session>/detections-<sub>.json: a TS detection
//! agrees when the Rust detector reports the same champion (and team) within `tol`
//! (normalized minimap units; 0.01 ≈ 3.4 px on a 341 px minimap). Also times the regional
//! sweep (regions = this frame's no-sweep detections, r = 0.08, as scripts/dev/bench-detector.ts).
//! Read-only on the session folder.

#[path = "util/session.rs"]
mod session;

use rc_vision::detect::{detect_prepared, DetectParams, Prepared, Region, Sweep, Team};
use rc_vision::image::load_png;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = session::session_dir(args.first().expect("usage: parity <session> [sub] [every] [tol]"));
    let sub = args.get(1).map(String::as_str).unwrap_or("frames");
    let every: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
    let tol: f64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0.01);
    let own = if sub == "red" { Team::Chaos } else { Team::Order };

    let dd = session::ddragon();
    let meta = session::meta(&dir);
    let cached = session::cached(&dir, sub);
    let times: Vec<u64> = session::frame_times(&dir, sub).into_iter().enumerate().filter(|(i, _)| i % every == 0).map(|(_, t)| t).collect();
    let params = DetectParams { own_team: Some(own), ..DetectParams::default() };

    let mut templates = None;
    let (mut ts_total, mut rs_total, mut matched, mut frames, mut exact) = (0usize, 0usize, 0usize, 0usize, 0usize);
    let (mut reg_ts_total, mut reg_rs_total, mut reg_matched) = (0usize, 0usize, 0usize);
    let (mut ms_full, mut ms_regional, mut ms_none, mut ms_decode) = (0f64, 0f64, 0f64, 0f64);
    let mut score_diffs: Vec<f64> = Vec::new();
    let mut misses: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
    let t_all = Instant::now();
    for &ms in &times {
        let Some(ts) = cached.get(&ms) else { continue };
        let t0 = Instant::now();
        let img = load_png(dir.join(sub).join(format!("{ms}.png"))).unwrap();
        ms_decode += t0.elapsed().as_secs_f64() * 1e3;
        let tpl = templates.get_or_insert_with(|| session::templates(&dd, &meta, img.width, &params));

        // Full sweep (what the TS cache used).
        let t0 = Instant::now();
        let prep = Prepared::new(&img);
        let out = detect_prepared(&prep, tpl, &params, &[]);
        ms_full += t0.elapsed().as_secs_f64() * 1e3;

        // No sweep (also gives the regions for the regional timing).
        let t0 = Instant::now();
        let prep = Prepared::new(&img);
        let none = detect_prepared(&prep, tpl, &DetectParams { sweep: Sweep::None, ..params.clone() }, &[]);
        ms_none += t0.elapsed().as_secs_f64() * 1e3;
        let regions = none.detections.iter().map(|d| Region { x: d.x, y: d.y, r: 0.08 }).collect();
        let t0 = Instant::now();
        let prep = Prepared::new(&img);
        let reg = detect_prepared(&prep, tpl, &DetectParams { sweep: Sweep::Regions(regions), ..params.clone() }, &[]);
        ms_regional += t0.elapsed().as_secs_f64() * 1e3;

        frames += 1;
        let mut frame_ok = ts.len() == out.detections.len();
        for d in ts {
            let hit = out.detections.iter().find(|r| r.champion == d.c && r.team.as_str() == d.t && (r.x - d.x).hypot(r.y - d.y) <= tol);
            let e = misses.entry(d.c.clone()).or_default();
            e.0 += 1;
            match hit {
                Some(r) => {
                    matched += 1;
                    score_diffs.push((r.score - d.s).abs());
                }
                None => {
                    frame_ok = false;
                    e.1 += 1;
                }
            }
            if reg.detections.iter().any(|r| r.champion == d.c && r.team.as_str() == d.t && (r.x - d.x).hypot(r.y - d.y) <= tol) {
                reg_matched += 1;
            }
        }
        if frame_ok {
            exact += 1;
        }
        ts_total += ts.len();
        rs_total += out.detections.len();
        reg_ts_total += ts.len();
        reg_rs_total += reg.detections.len();
    }
    let n = frames as f64;
    score_diffs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pct = |a: usize, b: usize| if b == 0 { 0.0 } else { 100.0 * a as f64 / b as f64 };
    println!("session {} / {sub}: {frames} frames (every {every}), tol {tol}, threads {}", dir.file_name().unwrap().to_string_lossy(), rayon::current_num_threads());
    println!("FULL sweep   : TS dets {ts_total}, Rust dets {rs_total}, matched {matched}");
    println!("  agreement (matched / union) {:.2}%   recall vs TS {:.2}%   precision vs TS {:.2}%   identical frames {:.2}%",
        pct(matched, ts_total + rs_total - matched), pct(matched, ts_total), pct(matched, rs_total), pct(exact, frames));
    if !score_diffs.is_empty() {
        println!("  |score diff| on matches: median {:.2e}, p99 {:.2e}, max {:.2e}", score_diffs[score_diffs.len() / 2], score_diffs[(score_diffs.len() * 99) / 100], score_diffs[score_diffs.len() - 1]);
    }
    println!("REGIONAL     : vs TS full-sweep cache: recall {:.2}%  precision {:.2}%", pct(reg_matched, reg_ts_total), pct(reg_matched, reg_rs_total));
    println!("speed (ms/frame, detect only): full {:.2}   regional {:.2}   no sweep {:.2}   | PNG decode {:.2}", ms_full / n, ms_regional / n, ms_none / n, ms_decode / n);
    println!("wall time {:.1} s", t_all.elapsed().as_secs_f64());
    let worst: Vec<String> = misses.iter().filter(|(_, v)| v.1 > 0).map(|(k, v)| format!("{k} {}/{}", v.1, v.0)).collect();
    println!("TS detections not reproduced, per champion: {}", if worst.is_empty() { "none".into() } else { worst.join(", ") });
}
