// Port of tests/fog.test.ts.
mod common;
use common::{fixtures, load};
use rc_vision::background::{load_reference, REFERENCE_SIZE};
use rc_vision::fog::{map_cell_mask, truth_vision_mask, FogParams, FogReader, VISIBLE_THRESHOLD};
use rc_vision::terrain::{distance_field, load_walkable, walkable_grid, TerrainParams, WALKABLE_GRID};
use std::time::Instant;

fn pairs() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(fixtures().join("fog"))
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|f| f.strip_suffix("-fog.png").map(String::from))
        .collect();
    v.sort();
    assert!(!v.is_empty());
    v
}

#[test]
fn reference_loads_at_native_size_and_resizes() {
    let r = load_reference(REFERENCE_SIZE);
    assert_eq!(r.width, 341);
    let small = load_reference(256);
    assert_eq!((small.width, small.height), (256, 256));
}

#[test]
fn vision_mask_matches_fog_free_truth() {
    let reference = load_reference(341);
    let map = map_cell_mask(&reference, 64, FogParams::default());
    let reader = FogReader::new(&reference, 64, FogParams::default());
    for t in pairs() {
        let fog = load("fog", &format!("{t}-fog.png"));
        let clear = load("fog", &format!("{t}-clear.png"));
        let pred = reader.read(&fog);
        let truth = truth_vision_mask(&fog, &clear, 64, 0.75, Some(&reference));
        let (mut ok, mut n, mut fogged) = (0, 0, 0);
        for c in 0..64 * 64 {
            if truth[c] == 255 || map[c] == 0 {
                continue;
            }
            n += 1;
            if (pred[c] >= VISIBLE_THRESHOLD) as u8 == truth[c] {
                ok += 1;
            }
            if truth[c] == 0 {
                fogged += 1;
            }
        }
        assert!(n > 800, "{t}: n={n}");
        assert!(fogged > 50, "{t}: fogged={fogged}");
        let acc = ok as f64 / n as f64;
        println!("{t}: accuracy {acc:.3} on {n} cells");
        assert!(acc >= 0.9, "{t}: accuracy {acc}");
        assert!(pred.iter().all(|&v| (0.0..=1.0).contains(&v)));
    }
}

#[test]
fn fog_free_frame_is_fully_visible() {
    let reference = load_reference(341);
    let map = map_cell_mask(&reference, 64, FogParams::default());
    let clear = load("fog", &format!("{}-clear.png", pairs()[0]));
    let pred = FogReader::new(&reference, 64, FogParams::default()).read(&clear);
    let (mut vis, mut n) = (0, 0);
    for c in 0..64 * 64 {
        if map[c] != 0 {
            n += 1;
            if pred[c] >= VISIBLE_THRESHOLD {
                vis += 1;
            }
        }
    }
    assert!(vis as f64 / n as f64 > 0.97);
}

#[test]
fn vision_mask_is_fast() {
    let reference = load_reference(341);
    let reader = FogReader::new(&reference, 64, FogParams::default());
    let fog = load("fog", &format!("{}-fog.png", pairs()[0]));
    reader.read(&fog);
    let n = 20;
    let t0 = Instant::now();
    for _ in 0..n {
        std::hint::black_box(reader.read(&fog));
    }
    let ms = t0.elapsed().as_secs_f64() * 1000.0 / n as f64;
    println!("readVisionMask {ms:.3} ms");
    assert!(ms < 5.0);
}

const G: usize = WALKABLE_GRID;
fn cell(x: f64, y: f64) -> usize {
    (y * G as f64).floor() as usize * G + (x * G as f64).floor() as usize
}

#[test]
fn terrain_asset_matches_walkable_grid() {
    let fresh = walkable_grid(&load_reference(341), G, TerrainParams::default());
    let walk = load_walkable(G);
    let diff = fresh.iter().zip(walk.iter()).filter(|(a, b)| a != b).count();
    assert_eq!(diff, 0);
}

#[test]
fn blue_fountain_reaches_red_fountain() {
    let walk = load_walkable(G);
    let df = distance_field(&walk, (0.06, 0.94), G);
    let d = df[cell(0.94, 0.06)] as f64;
    assert!(d.is_finite());
    assert!(d >= (0.88f64).hypot(0.88) - 2.0 / G as f64);
    assert!(d < 2.0);
    for (x, y) in [(0.08, 0.5), (0.5, 0.08), (0.92, 0.5), (0.5, 0.92), (0.1, 0.1), (0.9, 0.9)] {
        assert!(df[cell(x, y)].is_finite(), "{x},{y}");
    }
}

#[test]
fn known_walls_and_void_are_unwalkable() {
    let walk = load_walkable(G);
    assert_eq!(walk[cell(0.137, 0.61)], 0);
    assert_eq!(walk[cell(0.5, 0.995)], 0);
    assert_eq!(walk[cell(0.5, 0.5)], 1);
    let df = distance_field(&walk, (0.06, 0.94), G);
    assert_eq!(df[cell(0.137, 0.61)], f32::INFINITY);
}

#[test]
fn distance_field_is_a_consistent_geodesic() {
    let walk = load_walkable(G);
    let df = distance_field(&walk, (0.5, 0.5), G);
    let s = cell(0.5, 0.5);
    assert_eq!(df[s], 0.0);
    let mut checked = 0;
    for c in 0..G * G {
        let d = df[c];
        if !d.is_finite() || c == s {
            continue;
        }
        let (x, y) = ((c % G) as i64, (c / G) as i64);
        let (sx, sy) = ((s % G) as i64, (s / G) as i64);
        assert!(d as f64 >= ((x - sx) as f64).hypot((y - sy) as f64) / G as f64 - 1e-4);
        let mut best = f32::INFINITY;
        for dy in -1..=1i64 {
            for dx in -1..=1i64 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let (xx, yy) = (x + dx, y + dy);
                if xx < 0 || yy < 0 || xx >= G as i64 || yy >= G as i64 {
                    continue;
                }
                best = best.min(df[(yy * G as i64 + xx) as usize]);
            }
        }
        assert!(best < d);
        checked += 1;
    }
    assert!(checked > 5000);
}
