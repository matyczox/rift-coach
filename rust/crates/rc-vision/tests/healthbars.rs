// Port of tests/healthbars.test.ts.
mod common;
use common::load;
use rc_vision::healthbars::*;
use rc_vision::image::{resize, Rgba};
use std::time::Instant;

const H: usize = 1440;
fn opts(c: BarColor, h: Option<usize>) -> ReadOptions {
    ReadOptions { enemy_color: c, min_confidence: 0.5, screen_height: h }
}

#[test]
fn reads_a_red_bar() {
    let bars = read_health_bars(&load("hp", "red-l6-75.png"), &opts(BarColor::Red, Some(H)));
    assert_eq!(bars.len(), 1);
    let b = &bars[0];
    assert!((b.pct - 0.75).abs() < 0.02, "{}", b.pct);
    assert_eq!(b.level, Some(6));
    assert_eq!(b.shield_pct, 0.0);
    assert!(b.confidence > 0.8);
    assert_eq!(b.x, 40);
    assert_eq!(b.width, 125);
}

#[test]
fn reads_a_blue_bar_and_ignores_it_for_red() {
    let img = load("hp", "blue-l5-98.png");
    let blue = read_health_bars(&img, &opts(BarColor::Blue, Some(H)));
    assert_eq!(blue.len(), 1);
    assert!((blue[0].pct - 0.98).abs() < 0.02);
    assert_eq!(blue[0].level, Some(5));
    assert!(read_health_bars(&img, &opts(BarColor::Red, Some(H))).is_empty());
}

#[test]
fn reads_a_near_empty_bar() {
    let bars = read_health_bars(&load("hp", "red-l9-low.png"), &opts(BarColor::Red, Some(H)));
    assert_eq!(bars.len(), 1);
    assert!((bars[0].pct - 0.055).abs() < 0.02);
    assert_eq!(bars[0].level, Some(9));
}

#[test]
fn flags_a_shield_filling_the_bar() {
    let bars = read_health_bars(&load("hp", "red-l15-shield.png"), &opts(BarColor::Red, Some(H)));
    assert_eq!(bars.len(), 1);
    let b = &bars[0];
    assert_eq!(b.level, Some(15));
    assert!(b.shield_pct > 0.05);
    assert!(b.shield_compressed);
    assert!(b.pct + b.shield_pct > 0.97);
}

#[test]
fn reads_two_stacked_bars() {
    let bars = read_health_bars(&load("hp", "red-l18-pair.png"), &opts(BarColor::Red, Some(H)));
    assert_eq!(bars.len(), 2);
    assert_eq!(bars.iter().map(|b| b.level).collect::<Vec<_>>(), vec![Some(18), Some(18)]);
    assert!((bars[0].pct - 0.96).abs() < 0.03);
    assert!((bars[1].pct - 0.47).abs() < 0.02);
    assert!(bars[0].y < bars[1].y);
}

#[test]
fn scales_to_1080p() {
    for (f, pct, level) in [("red-l6-75.png", 0.75, 6), ("red-l15-shield.png", 0.91, 15), ("blue-l5-98.png", 0.98, 5)] {
        let img = load("hp", f);
        let small = resize(&img, (img.width as f64 * 0.75).round() as usize, (img.height as f64 * 0.75).round() as usize);
        let c = if f.starts_with("red") { BarColor::Red } else { BarColor::Blue };
        let bars = read_health_bars(&small, &opts(c, Some(1080)));
        assert_eq!(bars.len(), 1, "{f}");
        assert!((bars[0].pct - pct).abs() < 0.025, "{f}: {}", bars[0].pct);
        assert_eq!(bars[0].level, Some(level), "{f}");
    }
}

#[test]
fn nothing_in_noise_and_fast_on_1440p() {
    let w = 2560;
    let mut img = Rgba::new(w, H);
    let mut seed: i64 = 7;
    let mut rnd = || {
        seed = (seed.wrapping_mul(1103515245).wrapping_add(12345)) & 0x7fffffff;
        seed as f64 / 0x7fffffff as f64
    };
    for i in 0..w * H {
        let k = rnd();
        let c = if k < 0.2 { [180, 50, 40] } else if k < 0.4 { [50, 160, 240] } else if k < 0.6 { [90, 90, 90] } else { [20, 20, 20] };
        img.data[i * 4..i * 4 + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
    }
    read_health_bars(&img, &ReadOptions::new(BarColor::Red));
    let t0 = Instant::now();
    let mut bars = read_health_bars(&img, &ReadOptions::new(BarColor::Red));
    bars.extend(read_health_bars(&img, &ReadOptions::new(BarColor::Blue)));
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    println!("worst-case 1440p both colours: {ms:.1} ms");
    assert!(bars.is_empty());
    assert!(ms < 200.0);
}

struct B {
    x: f64,
    y: f64,
    level: Option<u32>,
}
impl BarLike for B {
    fn bx(&self) -> f64 {
        self.x
    }
    fn by(&self) -> f64 {
        self.y
    }
    fn bwidth(&self) -> f64 {
        125.0
    }
    fn blevel(&self) -> Option<u32> {
        self.level
    }
}
fn bar(x: f64, y: f64, level: Option<u32>) -> B {
    B { x, y, level }
}
fn cand(c: &str, level: u32, screen: Option<(f64, f64)>) -> ChampionCandidate {
    ChampionCandidate { champion: c.into(), level, screen }
}
fn names(r: Vec<Option<String>>) -> Vec<Option<&'static str>> {
    r.into_iter().map(|o| o.map(|s| &*Box::leak(s.into_boxed_str()))).collect()
}

#[test]
fn assigns_by_unique_level() {
    let r = match_bars_to_champions(&[bar(100.0, 100.0, Some(6)), bar(900.0, 500.0, Some(9))], &[cand("Yone", 6, None), cand("Hecarim", 9, None), cand("Xayah", 7, None)], &MatchOptions::default());
    assert_eq!(names(r), vec![Some("Yone"), Some("Hecarim")]);
}

#[test]
fn equal_level_without_positions_unassigned() {
    let r = match_bars_to_champions(&[bar(100.0, 100.0, Some(18))], &[cand("Yone", 18, None), cand("Hecarim", 18, None)], &MatchOptions::default());
    assert_eq!(r, vec![None]);
}

#[test]
fn level_ties_broken_by_proximity() {
    let r = match_bars_to_champions(
        &[bar(1000.0, 400.0, Some(18)), bar(300.0, 800.0, Some(18))],
        &[cand("Yone", 18, Some((1060.0, 600.0))), cand("Hecarim", 18, Some((370.0, 1000.0)))],
        &MatchOptions::default(),
    );
    assert_eq!(names(r), vec![Some("Yone"), Some("Hecarim")]);
}

#[test]
fn ambiguous_stays_unassigned() {
    let r = match_bars_to_champions(&[bar(1000.0, 400.0, None)], &[cand("Yone", 18, Some((1000.0, 600.0))), cand("Hecarim", 17, Some((1120.0, 600.0)))], &MatchOptions::default());
    assert_eq!(r, vec![None]);
}

#[test]
fn unread_level_uses_proximity_never_reuses() {
    let r = match_bars_to_champions(&[bar(1000.0, 400.0, Some(11)), bar(1010.0, 420.0, None)], &[cand("Yone", 11, Some((1060.0, 600.0))), cand("Xayah", 10, Some((1700.0, 900.0)))], &MatchOptions::default());
    assert_eq!(names(r), vec![Some("Yone"), None]);
}
