// Minion waves on real recorded minimap crops (341 px). Expected counts were checked by eye on
// 3x zooms of the frames; tolerances allow one dot hidden under a portrait edge.
mod common;
use common::load;
use rc_vision::waves::{read_waves, WaveParams, WaveState};

#[test]
fn replay_blue_view_ally_wave_bot() {
    // 07:00, blue view: our 7-minion wave on the bottom edge, walking up (no enemy minions seen).
    let img = load("waves", "frames-420013.png");
    let portraits = [(0.8827, 0.7009), (0.088, 0.2405), (0.8944, 0.7918), (0.5572, 0.8299), (0.0499, 0.7742), (0.4721, 0.5513), (0.4311, 0.5689), (0.1525, 0.6745)];
    let r = read_waves(&img, &portraits, false, true, None, &WaveParams::default());
    let bot = r.lane("bot").unwrap();
    assert!((6..=8).contains(&bot.ally) && bot.enemy == 0, "{bot:?}");
    assert!(bot.ally_pos.unwrap() > 0.35 && bot.ally_pos.unwrap() < 0.6, "{bot:?}");
    for l in ["top", "mid"] {
        let w = r.lane(l).unwrap();
        assert!(w.ally + w.enemy <= 1, "{w:?}");
    }
}

#[test]
fn replay_no_fog_enemy_wave_top() {
    // 05:00, fog-free view: a red wave of 7 coming down the top lane corner (one dot under a
    // portrait edge).
    let img = load("waves", "all-300049.png");
    let portraits = [(0.4663, 0.6422), (0.4839, 0.4927), (0.6774, 0.39), (0.1026, 0.2053), (0.8622, 0.871), (0.8827, 0.8006), (0.7625, 0.2727), (0.0557, 0.4604)];
    let r = read_waves(&img, &portraits, false, true, None, &WaveParams::default());
    let top = r.lane("top").unwrap();
    assert!((5..=7).contains(&top.enemy) && top.ally == 0, "{top:?}");
    assert!((top.enemy_pos.unwrap() - 0.5).abs() < 0.1, "{top:?}");
    for l in ["mid", "bot"] {
        let w = r.lane(l).unwrap();
        assert!(w.ally + w.enemy <= 1, "{w:?}");
    }
}

#[test]
fn replay_red_view_mirrored() {
    // 19:11, red team's view (absolute colours: our minions are red). Our top wave is deep on the
    // left edge = at the blue base: crashing after the mirror.
    let img = load("waves", "red-1151471.png");
    let portraits = [(0.434, 0.8328), (0.4223, 0.7654), (0.2786, 0.7566), (0.3109, 0.651), (0.3196, 0.6979), (0.129, 0.7155), (0.9619, 0.0323)];
    let r = read_waves(&img, &portraits, true, false, None, &WaveParams::default());
    let top = r.lane("top").unwrap();
    assert!(top.ally >= 8 && top.enemy == 0, "{top:?}");
    assert_eq!(top.state, WaveState::Crashing, "{top:?}");
    // Fresh wave leaving our (red) base through mid: low positions.
    let mid = r.lane("mid").unwrap();
    assert!(mid.ally >= 2 && mid.ally_pos.unwrap() < 0.2, "{mid:?}");
    let bot = r.lane("bot").unwrap();
    assert!(bot.enemy >= 1, "{bot:?}");
}

#[test]
fn live_capture_without_portrait_hints() {
    // Live game (relative colours, blue side), no detections passed: shape filtering alone.
    let img = load("waves", "live-841016.png");
    let r = read_waves(&img, &[], false, true, None, &WaveParams::default());
    let top = r.lane("top").unwrap();
    assert!(top.ally >= 4 && top.enemy >= 2, "{top:?}");
    assert_eq!(top.state, WaveState::UnderOurTower, "{top:?}");
    let bot = r.lane("bot").unwrap();
    assert!(bot.ally >= 3 && bot.enemy >= 3, "{bot:?}");
    assert_eq!(r.lane("mid").unwrap().enemy, 0);
}
