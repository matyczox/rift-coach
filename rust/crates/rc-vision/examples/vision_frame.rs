//! Run the full `Vision` pipeline on one full game-window frame (PNG).
//!
//! usage: vision_frame <frame.png> <x> <y> <w> <h> <winW> <winH> <allies,comma,separated> <enemies,...> [relative=1]
//! Allies are ORDER (own team), enemies CHAOS. Icons from the Data Dragon cache.

#[path = "util/session.rs"]
mod session;

use rc_vision::icons::load_champion_icon;
use rc_vision::image::load_png;
use rc_vision::{Calibration, Rect, Roster, RosterEntry, Team, Vision, VisionParams};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    assert!(a.len() >= 9, "usage: vision_frame <frame.png> x y w h winW winH allies enemies [relative]");
    let frame = load_png(&a[0]).unwrap();
    let v: Vec<i32> = a[1..7].iter().map(|s| s.parse().unwrap()).collect();
    let cal = Calibration { rect: Rect { x: v[0], y: v[1], width: v[2], height: v[3] }, window_width: v[4], window_height: v[5] };
    let dd = session::ddragon();
    let icons = session::root().join("ddragon").join("icons");
    let mut champions = Vec::new();
    for (team, list) in [(Team::Order, &a[7]), (Team::Chaos, &a[8])] {
        for c in list.split(',') {
            let icon = load_champion_icon(&icons, &dd.version, &dd.ids.resolve(c)).unwrap();
            champions.push(RosterEntry { champion: c.into(), team, icon });
        }
    }
    let relative = a.get(9).map(|s| s != "0").unwrap_or(true);
    let mut vision = Vision::new(Roster { champions, own_team: Team::Order, relative_colors: relative }, Some(cal), VisionParams::default());
    let r = vision.process(&frame, 0);
    println!("minimap {} px, {:.1} ms (first frame: includes template + fog-reader setup)", r.width, r.ms);
    for d in &r.detections {
        println!("  {:<12} {:?} ({:.3}, {:.3}) score {:.3}", d.champion, d.team, d.x, d.y, d.score);
    }
    let mask = r.fog_mask.as_ref().unwrap();
    println!("fog: {:.0}% of cells visible (>= 0.52)", 100.0 * mask.iter().filter(|&&m| m >= 0.52).count() as f64 / mask.len() as f64);
    let camps: Vec<String> = r.camps.iter().map(|c| format!("{}={}", c.id, c.icon.map(|i| format!("{i:?}")).unwrap_or("?".into()))).collect();
    println!("camps: {}", camps.join(" "));
    println!("hp bars: {:?}", r.hp.as_ref().map(|h| h.iter().map(|b| (b.pct * 100.0).round() as i32).collect::<Vec<_>>()));
    let n = 20;
    let t = std::time::Instant::now();
    for i in 0..n {
        vision.process(&frame, 1000 + i * 200);
    }
    println!("steady state: {:.1} ms per process() (full sweep every 5th frame, HP every 350 ms)", t.elapsed().as_secs_f64() * 1e3 / n as f64);
}
