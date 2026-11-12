// Port of tests/camps.test.ts.
mod common;
use common::load;
use rc_vision::camps::*;
use rc_vision::image::resize;
use CampIconState::*;

type Exp = &'static [(&'static str, Option<CampIconState>)];
const EXPECTED: &[(&str, Exp)] = &[
    (
        "frames-99935.png",
        &[
            ("blue_wolves", Some(Up)), ("blue_raptors", Some(Up)), ("blue_red", Some(Up)), ("blue_krugs", Some(Up)), ("red_gromp", Some(Up)), ("red_blue", Some(Empty)),
            ("red_wolves", Some(Up)), ("red_raptors", Some(Up)), ("red_red", Some(Up)), ("red_krugs", Some(Up)), ("scuttle_bot", Some(Empty)),
        ],
    ),
    (
        "frames-399947.png",
        &[
            ("blue_gromp", Some(Empty)), ("blue_blue", Some(Up)), ("blue_wolves", Some(Empty)), ("blue_raptors", Some(Empty)), ("blue_red", Some(Empty)), ("blue_krugs", Some(Empty)),
            ("red_gromp", Some(RespawnSoon)), ("red_blue", Some(Up)), ("red_wolves", Some(Respawning)), ("red_raptors", Some(Empty)), ("red_red", Some(Empty)), ("red_krugs", Some(Empty)),
        ],
    ),
    (
        "red-399883.png",
        &[
            ("blue_gromp", Some(Empty)), ("blue_blue", Some(Up)), ("blue_wolves", Some(Empty)), ("blue_raptors", Some(Empty)), ("blue_red", Some(Empty)), ("blue_krugs", Some(Empty)),
            ("red_gromp", Some(RespawnSoon)), ("red_blue", Some(Up)), ("red_wolves", Some(Respawning)), ("red_raptors", Some(Empty)), ("red_red", Some(Empty)), ("red_krugs", Some(Empty)),
        ],
    ),
    (
        "truth-40073.png",
        &[
            ("blue_gromp", Some(Respawning)), ("blue_wolves", Some(Respawning)), ("blue_raptors", Some(Respawning)), ("blue_red", Some(Respawning)), ("blue_krugs", Some(Respawning)),
            ("red_gromp", Some(Respawning)), ("red_blue", Some(Respawning)), ("red_wolves", Some(Respawning)), ("red_raptors", Some(Respawning)), ("red_red", Some(Respawning)), ("red_krugs", Some(Respawning)),
        ],
    ),
    (
        "frames-1283210.png",
        &[
            ("blue_gromp", Some(Up)), ("blue_blue", Some(Up)), ("blue_raptors", Some(Empty)), ("red_gromp", Some(Empty)), ("red_blue", Some(Empty)), ("red_raptors", Some(Empty)),
            ("red_red", Some(Empty)), ("red_krugs", Some(RespawnSoon)), ("scuttle_top", Some(Respawning)), ("scuttle_bot", Some(Empty)),
        ],
    ),
    (
        "red-1699894.png",
        &[
            ("blue_gromp", Some(Respawning)), ("blue_blue", Some(Up)), ("blue_raptors", Some(Empty)), ("blue_red", Some(Respawning)), ("red_gromp", Some(RespawnSoon)),
            ("red_wolves", Some(Up)), ("red_raptors", Some(Empty)), ("red_krugs", Some(Empty)), ("scuttle_top", Some(Empty)),
        ],
    ),
];

fn camps(f: &str) -> Vec<CampReading> {
    read_camps(&load("camps", f), &ReadCampsParams::default())
}

#[test]
fn reads_camp_icons() {
    for (file, exp) in EXPECTED {
        let got = camps(file);
        assert_eq!(got.len(), CAMPS.len());
        for (id, icon) in exp.iter() {
            let r = got.iter().find(|r| r.id == *id).unwrap();
            assert_eq!((file, id, r.icon), (file, id, *icon));
            assert_eq!(r.present, icon.map(|i| i == Up));
        }
    }
}

#[test]
fn same_confident_readings_on_other_crop_sizes() {
    for size in [256usize, 420] {
        for file in ["frames-99935.png", "frames-399947.png", "truth-40073.png"] {
            let img = load("camps", file);
            let native = read_camps(&img, &ReadCampsParams::default());
            let scaled = read_camps(&resize(&img, size, size), &ReadCampsParams::default());
            for (r, s) in native.iter().zip(&scaled) {
                if r.icon.is_some() && r.confidence >= 0.5 {
                    assert_eq!((size, file, &r.id, s.icon), (size, file, &r.id, r.icon));
                }
                if s.icon.is_some() {
                    assert_eq!((size, file, &r.id, s.icon), (size, file, &r.id, r.icon.or(s.icon)));
                }
            }
        }
    }
}

// ---- tracker ----

fn all(icon: Option<CampIconState>, over: &[(&str, Option<CampIconState>)]) -> Vec<CampReading> {
    CAMPS
        .iter()
        .map(|c| {
            let i = over.iter().find(|(id, _)| *id == c.id).map(|o| o.1).unwrap_or(icon);
            CampReading { id: c.id.into(), side: c.side, present: i.map(|i| i == Up), icon: i, confidence: if i.is_some() { 1.0 } else { 0.0 } }
        })
        .collect()
}
fn st(tr: &CampTracker, id: &str) -> CampStatus {
    tr.status().into_iter().find(|s| s.id == id).unwrap()
}
fn kinds(tr: &CampTracker) -> Vec<CampEventKind> {
    tr.events.iter().map(|e| e.kind).collect()
}

#[test]
fn vanishing_icon_is_a_clear_followed_to_respawn() {
    let mut tr = CampTracker::default();
    let mut t = 100.0;
    for _ in 0..4 {
        tr.update(t, &all(Some(Up), &[]), None);
        t += 1.0;
    }
    assert_eq!(st(&tr, "red_raptors").state, CampState::Up);
    tr.update(t, &all(Some(Up), &[("red_raptors", None)]), None);
    t += 1.0;
    for _ in 0..4 {
        tr.update(t, &all(Some(Up), &[("red_raptors", Some(Empty))]), None);
        t += 1.0;
    }
    let ev: Vec<&CampEvent> = tr.events.iter().filter(|e| e.kind == CampEventKind::Cleared).collect();
    assert_eq!(ev.len(), 1);
    assert_eq!(*ev[0], CampEvent { id: "red_raptors".into(), side: CampSide::Chaos, at: 105.0, kind: CampEventKind::Cleared, respawn_at: Some(240.0), earliest: Some(103.0) });
    let s = st(&tr, "red_raptors");
    assert_eq!((s.state, s.down_since, s.respawn_at), (CampState::Down, Some(105.0), Some(240.0)));
    for t in 209..213 {
        tr.update(t as f64, &all(Some(Up), &[("red_raptors", Some(Respawning))]), None);
    }
    let s = st(&tr, "red_raptors");
    assert_eq!((s.state, s.respawn_at, s.down_since), (CampState::Down, Some(239.0), Some(104.0)));
    for t in 229..233 {
        tr.update(t as f64, &all(Some(Up), &[("red_raptors", Some(RespawnSoon))]), None);
    }
    assert_eq!(st(&tr, "red_raptors").respawn_at, Some(239.0));
    for t in 239..243 {
        tr.update(t as f64, &all(Some(Up), &[]), None);
    }
    let s = st(&tr, "red_raptors");
    assert_eq!((s.state, s.down_since, s.respawn_at), (CampState::Up, None, None));
    assert_eq!(kinds(&tr), vec![CampEventKind::Cleared, CampEventKind::Respawned]);
}

#[test]
fn ignores_short_flickers() {
    let mut tr = CampTracker::default();
    for t in 0..4 {
        tr.update(t as f64, &all(Some(Up), &[]), None);
    }
    tr.update(4.0, &all(Some(Up), &[("blue_gromp", Some(Empty))]), None);
    tr.update(5.0, &all(Some(Up), &[("blue_gromp", Some(Empty))]), None);
    for t in 6..10 {
        tr.update(t as f64, &all(Some(Up), &[]), None);
    }
    assert!(tr.events.is_empty());
    assert_eq!(st(&tr, "blue_gromp").state, CampState::Up);
}

#[test]
fn learns_respawn_from_first_seen_hourglass() {
    let mut tr = CampTracker::default();
    for t in 300..304 {
        tr.update(t as f64, &all(None, &[("red_blue", Some(Respawning))]), None);
    }
    let s = st(&tr, "red_blue");
    assert_eq!((s.state, s.respawn_at, s.down_since), (CampState::Down, Some(330.0), Some(30.0)));
    assert_eq!(st(&tr, "red_red").state, CampState::Unknown);
    assert!(tr.events.is_empty());
}

#[test]
fn retracts_clear_without_hourglass() {
    let mut tr = CampTracker::default();
    for t in 0..4 {
        tr.update(t as f64, &all(Some(Up), &[]), None);
    }
    for t in 4..200 {
        tr.update(t as f64, &all(Some(Up), &[("blue_krugs", Some(Empty))]), None);
    }
    assert_eq!(kinds(&tr), vec![CampEventKind::Cleared, CampEventKind::Retracted]);
    assert_eq!(st(&tr, "blue_krugs").state, CampState::Unknown);
}

#[test]
fn scuttle_leaving_vision_is_not_a_kill() {
    let mut tr = CampTracker::default();
    for t in 0..4 {
        tr.update(t as f64, &all(Some(Up), &[]), None);
    }
    for t in 4..8 {
        tr.update(t as f64, &all(Some(Up), &[("scuttle_top", Some(Empty))]), None);
    }
    assert!(tr.events.is_empty());
    assert_eq!(st(&tr, "scuttle_top").state, CampState::Unknown);
    let mut tr2 = CampTracker::default();
    let vis = |_: f64, _: f64| 1.0;
    for t in 0..4 {
        tr2.update(t as f64, &all(Some(Up), &[]), Some(&vis));
    }
    for t in 4..8 {
        tr2.update(t as f64, &all(Some(Up), &[("scuttle_top", Some(Empty))]), Some(&vis));
    }
    let ev: Vec<(String, CampEventKind)> = tr2.events.iter().map(|e| (e.id.clone(), e.kind)).collect();
    assert_eq!(ev, vec![("scuttle_top".to_string(), CampEventKind::Cleared)]);
}

#[test]
fn trusts_vanishing_icons_in_fog_only_once_seen_drawn_in_fog() {
    let fog_red = |x: f64, y: f64| if x + (1.0 - y) > 1.05 { 0.0 } else { 1.0 };
    let one = |_: f64, _: f64| 1.0;
    let mut tr = CampTracker::default();
    for t in 0..4 {
        tr.update(t as f64, &all(Some(Up), &[("red_gromp", Some(Up))]), Some(&one));
    }
    for t in 4..9 {
        tr.update(t as f64, &all(Some(Up), &[("red_gromp", Some(Empty))]), Some(&fog_red));
    }
    assert!(tr.events.is_empty());
    assert_eq!(st(&tr, "red_gromp").state, CampState::Unknown);
    let mut tr2 = CampTracker::default();
    for t in 0..5 {
        tr2.update(t as f64, &all(Some(Up), &[]), Some(&fog_red));
    }
    for t in 5..10 {
        tr2.update(t as f64, &all(Some(Up), &[("red_gromp", Some(Empty))]), Some(&fog_red));
    }
    let ev: Vec<(String, CampEventKind)> = tr2.events.iter().map(|e| (e.id.clone(), e.kind)).collect();
    assert_eq!(ev, vec![("red_gromp".to_string(), CampEventKind::Cleared)]);
}

#[test]
fn tracks_a_clear_from_real_crops() {
    let a = camps("frames-99935.png");
    let b = camps("frames-399947.png");
    let mut tr = CampTracker::default();
    for t in 100..104 {
        tr.update(t as f64, &a, None);
    }
    for t in 104..108 {
        tr.update(t as f64, &b, None);
    }
    let mut cleared: Vec<String> = tr.events.iter().filter(|e| e.kind == CampEventKind::Cleared).map(|e| e.id.clone()).collect();
    cleared.sort();
    assert_eq!(cleared, vec!["blue_krugs", "blue_raptors", "blue_red", "blue_wolves", "red_krugs", "red_raptors", "red_red"]);
    let s = st(&tr, "red_wolves");
    assert_eq!((s.state, s.respawn_at), (CampState::Down, Some(134.0)));
    assert_eq!(st(&tr, "red_blue").state, CampState::Up);
}
