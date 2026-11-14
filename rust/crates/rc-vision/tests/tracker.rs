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
fn standing_then_entering_fog_does_not_prove_a_recall() {
    let mut tr = EnemyTracker::new(open(), Learner::new(None), 200);
    for i in 0..90 { upd(&mut tr, 300.0+i as f64*0.1, &[det(0.55,0.5)], &[ww()]); }
    for i in 0..12 { upd(&mut tr, 309.0+i as f64*0.1, &[], &[ww()]); }
    let w = &tr.all()[0];
    assert_eq!(w.state, TrackState::Fog);
    assert!(w.recalled_at.is_none());
    assert!((w.last.unwrap().x-0.55).abs() < 0.001);
    let estimate=w.estimate.unwrap();
    assert!((estimate.x-0.55).hypot(estimate.y-0.5)<0.1);
}

#[test]
fn low_confidence_or_impossible_reappearance_does_not_train_routes() {
    for (score, point) in [(0.65,(0.52,0.5)),(0.95,(0.1,0.9))] {
        let mut tr = EnemyTracker::new(open(), Learner::new(None), 200);
        upd(&mut tr, 300.0, &[det(0.5,0.5)], &[ww()]);
        for i in 1..=60 { upd(&mut tr, 300.0+i as f64*0.1, &[], &[ww()]); }
        let mut d=det(point.0,point.1);d.score=score;
        upd(&mut tr,306.1,&[d],&[ww()]);
        assert_eq!(tr.learner.summary().samples,0);
    }
}

#[test]
fn absence_evidence_has_the_same_strength_at_different_frame_rates() {
    let make=|| {
        let mut f=ParticleFilter::new(open(),goals_for(Team::Chaos),2);
        f.reset((0.5,0.5),&PriorContext::default(),None,&mut Rng::new(1));
        f.set_points(&[(0.3,0.3),(0.7,0.7)]);
        f
    };
    let (mut slow,mut fast)=(make(),make());
    let visible=|x:f64,_y:f64|if x<0.5 {0.1}else{0.2};
    assert!(slow.observe_absent_for(&visible,1.0,&mut Rng::new(2)));
    for _ in 0..10 {assert!(fast.observe_absent_for(&visible,0.1,&mut Rng::new(2)));}
    for (a,b) in slow.particles.iter().zip(&fast.particles) {assert!((a.w-b.w).abs()<1e-12);}
}

#[test]
fn uninformative_truth_does_not_learn_visibility_as_a_route_preference() {
    let mut pf=ParticleFilter::new(open(),goals_for(Team::Chaos),2);
    pf.reset((0.5,0.5),&PriorContext::default(),None,&mut Rng::new(1));
    // Negative evidence can leave only one goal kind alive. A label covering
    // that whole surviving cloud says nothing new about the behaviour prior.
    pf.set_points(&[(0.5,0.5),(0.51,0.5)]);
    let conditional=pf.posterior_by_kind();
    let mut learner=Learner::new(None);
    learner.observe(rc_vision::tracker::learning::Observation{
        champion:"Warwick",role:Role::Jungle,game_time:300.0,gap_sec:8.0,
        credit:pf.goal_kinds_near((0.5,0.5),0.06),prior:conditional,
        mass_model:1.0,mass_baseline:1.0,error_pred:0.0,error_base:0.0});
    assert!(learner.multipliers(Role::Jungle,300.0).values().all(|v|(*v-1.0).abs()<1e-12));
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
    tr.item_recall = true;
    let e = |gold: f64| [EnemyInput { item_gold: Some(gold), ..ww() }];
    let mut t = 500.0;
    for _ in 0..3 {
        upd(&mut tr, t, &[det(0.3, 0.6)], &e(2000.0));
        t += 0.1;
    }
    // Out of sight for 2 s only: a recall (8 s channel) is impossible, the purchase is not proof.
    for _ in 0..20 {
        upd(&mut tr, t, &[], &e(2000.0));
        t += 0.1;
    }
    upd(&mut tr, t, &[], &e(2600.0));
    assert!(tr.all()[0].recalled_at.is_none());
    // Out of sight for 8 s: proven recall.
    for _ in 0..60 {
        upd(&mut tr, t, &[], &e(2600.0));
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

// ---- Audit regressions (2026-11-12) ----

/// #3: full map vision, portrait missing for a while -> no ghost (was ~95% confidence).
#[test]
fn full_vision_leaves_no_ghost() {
    let mut tr = EnemyTracker::new(open(), Learner::new(None), 200);
    let e = [ww()];
    let all_visible = |_x: f64, _y: f64| 1.0;
    let mut t = 200.0;
    for _ in 0..5 {
        tr.update(&FrameInput { t, detections: &[det(0.6, 0.4)], visible: Some(&all_visible), enemies: &e, dragon_in: None, baron_in: None });
        t += 0.1;
    }
    for _ in 0..10 {
        tr.update(&FrameInput { t, detections: &[], visible: Some(&all_visible), enemies: &e, dragon_in: None, baron_in: None });
        t += 0.1;
    }
    assert!(tr.all()[0].estimate.is_none(), "ghost under full vision: {:?}", tr.all()[0].estimate);
}

/// #2: a camp hint reweights the fog estimate but is not a sighting and teaches nothing.
#[test]
fn camp_hint_is_not_a_sighting() {
    let mut tr = EnemyTracker::new(open(), Learner::new(None), 200);
    let e = [ww()];
    let mut t = 200.0;
    for _ in 0..5 {
        upd(&mut tr, t, &[det(0.3, 0.3)], &e);
        t += 0.1;
    }
    for _ in 0..40 {
        upd(&mut tr, t, &[], &e);
        t += 0.1;
    }
    assert_eq!(tr.all()[0].state, TrackState::Fog);
    tr.soft_hint("Warwick", (0.35, 0.3));
    upd(&mut tr, t, &[], &e);
    assert_eq!(tr.all()[0].state, TrackState::Fog);
    assert_eq!(tr.learner.summary().samples, 0);
}

/// #7: two clusters on both sides of a wall -> the estimate is on walkable ground.
#[test]
fn estimate_not_inside_a_wall() {
    let size = 64;
    let mut grid = vec![1u8; size * size];
    for y in 0..size {
        grid[y * size + 32] = 0;
    }
    let terrain = Arc::new(GridTerrain::new(size, grid));
    let mut pf = ParticleFilter::new(terrain.clone(), goals_for(Team::Chaos), 200);
    // The audit case: one particle each side of the wall column (x = 32/64).
    pf.set_points(&[(0.45, 0.5), (0.55, 0.5)]);
    let est = pf.estimate(0.7);
    assert!(terrain.walkable(est.x, est.y), "center in a wall: {:?}", (est.x, est.y));
}

/// Audit V3 F04: partial visibility (v = 0.8, above the fog reader's 0.52 "visible") also leaves no ghost.
#[test]
fn partial_vision_leaves_no_ghost() {
    let mut tr = EnemyTracker::new(open(), Learner::new(None), 200);
    let e = [ww()];
    let mostly_visible = |_x: f64, _y: f64| 0.8;
    let mut t = 200.0;
    for _ in 0..5 {
        tr.update(&FrameInput { t, detections: &[det(0.6, 0.4)], visible: Some(&mostly_visible), enemies: &e, dragon_in: None, baron_in: None });
        t += 0.1;
    }
    for _ in 0..10 {
        tr.update(&FrameInput { t, detections: &[], visible: Some(&mostly_visible), enemies: &e, dragon_in: None, baron_in: None });
        t += 0.1;
    }
    assert!(tr.all()[0].estimate.is_none(), "ghost under 80% vision: {:?}", tr.all()[0].estimate);
}

/// Audit V4 D: two hidden groups with an empty (visible) gap between them -> the estimate is in one
/// group, not in the empty middle.
#[test]
fn estimate_not_in_empty_middle() {
    let terrain = Arc::new(GridTerrain::open(64));
    let mut pf = ParticleFilter::new(terrain, goals_for(Team::Chaos), 200);
    pf.set_points(&[(0.42, 0.5), (0.42, 0.51), (0.58, 0.5), (0.58, 0.51)]);
    let est = pf.estimate(0.7);
    assert!((est.x - 0.42).abs() < 0.02 || (est.x - 0.58).abs() < 0.02, "center in the empty middle: {:?}", (est.x, est.y));
}

/// Audit V4 B: particles never end inside a wall while walking (no corner cutting, no partial step
/// into a blocked cell).
#[test]
fn particles_never_walk_into_walls() {
    let size = 64;
    let mut grid = vec![1u8; size * size];
    // A diagonal wall of single cells: corner cutting would slip through.
    for k in 10..54 {
        grid[k * size + k] = 0;
    }
    let terrain = Arc::new(GridTerrain::new(size, grid));
    let mut pf = ParticleFilter::new(terrain.clone(), goals_for(Team::Chaos), 300);
    let mut rng = Rng::new(3);
    pf.reset((0.3, 0.6), &PriorContext::default(), None, &mut rng);
    for _ in 0..60 {
        pf.step(1.0, 380.0, &mut rng);
        assert!(pf.particles.iter().all(|p| terrain.walkable(p.x, p.y)), "particle inside a wall");
    }
}

/// Several real options: zones split the cloud into its groups (most likely first).
#[test]
fn zones_split_two_options() {
    let terrain = Arc::new(GridTerrain::open(64));
    let mut pf = ParticleFilter::new(terrain, goals_for(Team::Chaos), 200);
    let mut pts = Vec::new();
    for k in 0..30 {
        pts.push((0.30 + (k % 5) as f64 * 0.004, 0.30 + (k / 5) as f64 * 0.004));
    }
    for k in 0..20 {
        pts.push((0.70 + (k % 5) as f64 * 0.004, 0.70 + (k / 5) as f64 * 0.004));
    }
    pf.set_points(&pts);
    let z = pf.zones(3);
    assert_eq!(z.len(), 2, "{z:?}");
    assert!((z[0].x - 0.31).abs() < 0.03 && (z[0].mass - 0.6).abs() < 0.05, "{:?}", z[0]);
    assert!((z[1].x - 0.71).abs() < 0.03 && (z[1].mass - 0.4).abs() < 0.05, "{:?}", z[1]);
    // One group: one zone.
    pf.set_points(&pts[..30]);
    assert_eq!(pf.zones(3).len(), 1);
}
