// Port of tests/tracker.test.ts.
use rc_vision::detect::{Detection, Team};
use rc_vision::tracker::predictor::goals_for;
use rc_vision::tracker::*;
use std::sync::Arc;

fn ww() -> EnemyInput {
    EnemyInput::new("Warwick", Team::Chaos, Role::Jungle, 345.0)
}
fn det(x: f64, y: f64) -> Detection {
    Detection { champion: "Warwick".into(), team: Team::Chaos, x, y, score: 0.9, margin: 0.5, votes: 1.0 }
}
fn open() -> Arc<GridTerrain> {
    Arc::new(GridTerrain::open(64))
}
fn upd(tr: &mut EnemyTracker, t: f64, dets: &[Detection], enemies: &[EnemyInput]) -> Vec<EnemyTrack> {
    tr.update(&FrameInput { t, detections: dets, visible: None, enemies, dragon_in: None, baron_in: None })
}

#[test]
fn visible_to_fog_ghost_near_last_sighting_gone_on_reappearance() {
    let mut tr = EnemyTracker::new(open(), Learner::new(None), 200);
    let e = [ww()];
    let mut t = 200.0;
    for _ in 0..5 {
        upd(&mut tr, t, &[det(0.6, 0.4)], &e);
        t += 0.1;
    }
    assert_eq!(tr.all()[0].state, TrackState::Visible);
    assert!(tr.all()[0].estimate.is_none());
    for _ in 0..4 {
        upd(&mut tr, t, &[], &e);
        t += 0.1;
    }
    let w = &tr.all()[0];
    assert_eq!(w.state, TrackState::Fog);
    let est = w.estimate.unwrap();
    assert!((est.x - 0.6).hypot(est.y - 0.4) < 0.05);
    for _ in 0..80 {
        upd(&mut tr, t, &[], &e);
        t += 0.1;
    }
    let max_reach = (345.0 / MAP_UNITS) * 9.0;
    let est = tr.all()[0].estimate.unwrap();
    assert!((est.x - 0.6).hypot(est.y - 0.4) < max_reach);
    upd(&mut tr, t, &[det(0.62, 0.45)], &e);
    assert_eq!(tr.all()[0].state, TrackState::Visible);
    assert!(tr.all()[0].estimate.is_none());
    assert_eq!(tr.learner.summary().samples, 1);
}

#[test]
fn dead_enemies_restart_at_fountain() {
    let mut tr = EnemyTracker::new(open(), Learner::new(None), 100);
    upd(&mut tr, 300.0, &[det(0.5, 0.5)], &[ww()]);
    upd(&mut tr, 301.0, &[], &[EnemyInput { is_dead: true, ..ww() }]);
    assert_eq!(tr.all()[0].state, TrackState::Dead);
    assert!(tr.all()[0].estimate.is_none());
    upd(&mut tr, 330.0, &[], &[ww()]);
    let w = &tr.all()[0];
    assert_eq!(w.state, TrackState::Fog);
    assert!(w.estimate.unwrap().x > 0.85);
    assert!(w.estimate.unwrap().y < 0.15);
}

#[test]
fn teleport_like_detection_waits_for_confirmation() {
    let mut tr = EnemyTracker::new(open(), Learner::new(None), 100);
    let e = [ww()];
    let mut t = 400.0;
    for _ in 0..3 {
        upd(&mut tr, t, &[det(0.2, 0.2)], &e);
        t += 0.1;
    }
    upd(&mut tr, t, &[det(0.8, 0.8)], &e);
    t += 0.1;
    assert!((tr.all()[0].last.unwrap().x - 0.2).abs() < 0.005);
    upd(&mut tr, t, &[det(0.8, 0.8)], &e);
    assert!((tr.all()[0].last.unwrap().x - 0.8).abs() < 0.005);
}

fn ctx(role: Role, game_time: f64, dragon_in: Option<f64>) -> PriorContext {
    PriorContext { role: Some(role), game_time, dragon_in, ..Default::default() }
}

#[test]
fn particle_filter_removes_hypotheses_we_can_see() {
    let mut rng = Rng::new(7);
    let mut pf = ParticleFilter::new(open(), goals_for(Team::Chaos), 300);
    pf.reset((0.5, 0.5), &ctx(Role::Jungle, 600.0, None), None, &mut rng);
    pf.step(10.0, 345.0, &mut rng);
    pf.observe_absent(&|x, _| if x < 0.5 { 1.0 } else { 0.0 }, &mut rng);
    let e = pf.estimate(0.7);
    assert!(e.x > 0.5);
    assert!(pf.mass_near((0.4, 0.5), 0.08) < 0.05);
}

#[test]
fn objective_timer_pulls_junglers_to_dragon() {
    let near = |dragon_in: Option<f64>| {
        let mut rng = Rng::new(3);
        let mut pf = ParticleFilter::new(open(), goals_for(Team::Chaos), 400);
        pf.reset((0.6, 0.5), &ctx(Role::Jungle, 900.0, dragon_in), None, &mut rng);
        for _ in 0..20 {
            pf.step(1.0, 345.0, &mut rng);
        }
        pf.mass_near((0.67, 0.70), 0.08)
    };
    assert!(near(Some(20.0)) > near(None));
}

#[test]
fn item_purchase_in_fog_puts_enemy_in_fountain() {
    let mut tr = EnemyTracker::new(open(), Learner::new(None), 100);
    let e = |gold: f64| [EnemyInput { item_gold: Some(gold), ..ww() }];
    let mut t = 500.0;
    for _ in 0..3 {
        upd(&mut tr, t, &[det(0.3, 0.6)], &e(2000.0));
        t += 0.1;
    }
    for _ in 0..40 {
        upd(&mut tr, t, &[], &e(2000.0));
        t += 0.1;
    }
    upd(&mut tr, t, &[], &e(3300.0));
    let w = &tr.all()[0];
    assert!((w.recalled_at.unwrap() - t).abs() < 1e-3);
    assert!(w.estimate.unwrap().x > 0.85);
    assert!(w.estimate.unwrap().y < 0.15);
}

#[test]
fn flash_range_jump_is_flagged() {
    let mut tr = EnemyTracker::new(open(), Learner::new(None), 50);
    let e = [EnemyInput { has_flash: true, ..ww() }];
    let mut t = 600.0;
    for _ in 0..3 {
        upd(&mut tr, t, &[det(0.5, 0.5)], &e);
        t += 0.1;
    }
    upd(&mut tr, t, &[det(0.525, 0.5)], &e);
    assert!((tr.all()[0].flash_suspect_at.unwrap() - t).abs() < 1e-3);
}

#[test]
fn rng_matches_ts_xorshift() {
    // First values of the TS rng(12345) (computed with node from predictor.ts).
    let mut r = Rng::new(12345);
    let v: Vec<f64> = (0..4).map(|_| r.next()).collect();
    assert_eq!(v, vec![0.92633, 0.253807, 0.511904, 0.480042]);
}
