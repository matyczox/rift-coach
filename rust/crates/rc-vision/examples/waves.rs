//! Minion wave state per lane over a recorded session (minimap crops).
//!
//! usage: waves <session dir|id> [sub=frames|red|all] [from_ms=0] [to_ms=inf] [step_ms=10000]
//!              [--side=blue|red] [--abs] [--debug=<dir>] [--blobs]
//!
//! Replays use absolute ring colours (blue = ORDER) and `frames`/`all` are the blue view, `red`
//! the red view; a live recording (no `--abs`) assumes relative colours (allies blue).
//! Portraits are masked with `detections-<sub>.json` when the session has one.
//! `--debug=<dir>` writes 3x upscaled crops with the detected dots boxed (cyan = blue dot,
//! yellow = red dot) for eyeballing; `--blobs` prints every colour component with its stats.
//! Output per lane: `<ours>v<theirs> f@<where the waves meet, 0 = our inhib turret> <state>`;
//! the timing covers `read_waves` only (the fog mask is read separately, as in the app).

#[path = "util/session.rs"]
mod session;

use rc_vision::background::load_reference;
use rc_vision::fog::{FogParams, FogReader};
use rc_vision::image::{encode_png, load_png, Rgba};
use rc_vision::waves::{blob_dots, blobs, on_structure_icon, read_waves, WaveParams};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let pos: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let flag = |k: &str| args.iter().find_map(|a| a.strip_prefix(&format!("--{k}=")).map(String::from));
    let has = |k: &str| args.iter().any(|a| a == &format!("--{k}"));
    let dir = session::session_dir(pos.first().expect("usage: waves <session> [sub] [from] [to] [step]"));
    let sub = pos.get(1).map(|s| s.as_str()).unwrap_or("frames");
    let from: u64 = pos.get(2).map(|s| s.parse().unwrap()).unwrap_or(0);
    let to: u64 = pos.get(3).map(|s| s.parse().unwrap()).unwrap_or(u64::MAX);
    let step: u64 = pos.get(4).map(|s| s.parse().unwrap()).unwrap_or(10_000);
    let red_side = flag("side").map(|s| s == "red").unwrap_or(sub == "red");
    let abs = has("abs") || dir.join("detections-frames.json").exists();
    // Absolute colours: blue = ORDER. Relative: allies blue.
    let blue_is_ally = if abs { !red_side } else { true };
    let debug = flag("debug");
    if let Some(d) = &debug {
        std::fs::create_dir_all(d).unwrap();
    }
    let cached = if dir.join(format!("detections-{sub}.json")).exists() { Some(session::cached(&dir, sub)) } else { None };
    let p = WaveParams::default();
    let times = session::frame_times(&dir, sub);
    let mut next = from;
    // Fog mask like the app (lanes without dots: no_wave in vision, unknown in fog).
    let mut fog: Option<(usize, FogReader)> = None;
    let mut total_ms = 0.0;
    let mut n = 0;
    println!("{} / {sub}: side={} colours={} portraits={}", dir.display(), if red_side { "red" } else { "blue" }, if abs { "absolute" } else { "relative" }, cached.is_some());
    for &t in times.iter().filter(|&&t| t >= from && t <= to) {
        if t < next {
            continue;
        }
        next = t + step;
        let img = load_png(dir.join(sub).join(format!("{t}.png"))).unwrap();
        let portraits: Vec<(f64, f64)> = cached.as_ref().and_then(|c| c.get(&t)).map(|v| v.iter().map(|d| (d.x, d.y)).collect()).unwrap_or_default();
        if fog.as_ref().is_none_or(|(w, _)| *w != img.width) && img.width == img.height {
            fog = Some((img.width, FogReader::new(&load_reference(img.width), 64, FogParams::default())));
        }
        let mask = fog.as_ref().filter(|(w, _)| *w == img.width && sub != "all").map(|(_, f)| f.read(&img));
        let t0 = Instant::now();
        let r = read_waves(&img, &portraits, red_side, blue_is_ally, mask.as_deref().map(|m| (m, 64)), &p);
        let ms = t0.elapsed().as_secs_f64() * 1e3;
        total_ms += ms;
        n += 1;
        let lanes: Vec<String> = r
            .lanes
            .iter()
            .map(|l| {
                let f = |v: Option<f64>| v.map(|v| format!("{v:.2}")).unwrap_or("-".into());
                format!("{}: {}v{} f@{} {}", l.lane, l.ally, l.enemy, f(l.front), l.state.as_str())
            })
            .collect();
        let gt = t / 1000;
        println!("{t:>8} ({:02}:{:02})  {}  [{:.2} ms]", gt / 60, gt % 60, lanes.join(" | "), ms);
        if has("blobs") {
            for b in blobs(&img) {
                if b.area >= 3 {
                    println!("    {} a={:>3} at ({:>5.1},{:>5.1}) maj={:.1} min={:.1} dark={:.2} peak={} -> {}", if b.blue { "B" } else { "R" }, b.area, b.cx, b.cy, b.major, b.minor, b.dark_border, b.peak, if on_structure_icon(b.cx / img.width as f64, b.cy / img.height as f64) { 0 } else { blob_dots(&b, img.width, &p) });
                }
            }
        }
        if let Some(d) = &debug {
            let k = 3;
            let mut big = Rgba::new(img.width * k, img.height * k);
            for y in 0..big.height {
                for x in 0..big.width {
                    let s = img.px(x / k, y / k);
                    let o = (y * big.width + x) * 4;
                    big.data[o..o + 4].copy_from_slice(&s);
                }
            }
            for dot in &r.dots {
                let (cx, cy) = ((dot.x * big.width as f64) as i64, (dot.y * big.height as f64) as i64);
                let c: [u8; 4] = if dot.blue { [0, 255, 255, 255] } else { [255, 255, 0, 255] };
                let rad = 8 + 3 * (dot.n as i64 - 1);
                for dd in -rad..=rad {
                    for (x, y) in [(cx + dd, cy - rad), (cx + dd, cy + rad), (cx - rad, cy + dd), (cx + rad, cy + dd)] {
                        if x >= 0 && y >= 0 && (x as usize) < big.width && (y as usize) < big.height {
                            let o = (y as usize * big.width + x as usize) * 4;
                            big.data[o..o + 4].copy_from_slice(&c);
                        }
                    }
                }
            }
            std::fs::write(format!("{d}/{t}.png"), encode_png(&big)).unwrap();
        }
    }
    if n > 0 {
        println!("{n} frames, detector {:.3} ms per frame (mean)", total_ms / n as f64);
    }
}
