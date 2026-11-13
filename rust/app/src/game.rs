//! In-game loop: Live API snapshot -> intel -> coach (onGameData in index.ts).

use crate::state::AppState;
use crate::vision::Alert;
use rc_core::build_advisor::BuildTrigger;
use rc_core::coach::{validate_item, Coach, CoachContext, TickAction};
use rc_core::intel::ddragon::StaticData;
use rc_core::intel::game_intel::{GameIntel, IntelTracker};
use rc_core::intel::matchup::matchup_for_coach;
use rc_core::intel::situational::{FedWatcher, TeamWatcher};
use rc_core::llm::profiles::{pick, Backends, Task};
use rc_core::review::MatchLog;
use rc_core::self_train::ObsRecorder;
use rc_core::sources::live::{normalize_champion_names, AllGameData};
use serde_json::Value;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// What the last coach call asked the player to do, checked every tick (closes the loop: "you did
/// it" + the next step).
#[derive(Clone, Debug)]
pub struct Intent {
    /// Ward spot id (rc_core::wards) or None for a move.
    pub ward_spot: Option<String>,
    /// Move target (minimap 0..1).
    pub target: Option<(f64, f64)>,
    pub label: String,
    pub since: f64,
    pub ward_score_at: i64,
    pub was_covered: bool,
    /// Game time I arrived at the ward spot (standing there a moment = ward placed).
    pub at_spot_since: Option<f64>,
}

pub struct Game {
    pub intent: Option<Intent>,
    /// Ward spot just confirmed placed (spot id, Date.now() ms): the advice's marker for it is dropped.
    pub ward_done: Option<(String, f64)>,
    /// My ward score and the game time it last went up (= I placed a ward then).
    pub ward_seen: (i64, f64),
    pub static_data: Option<Arc<StaticData>>,
    pub tracker: IntelTracker,
    pub intel: Option<GameIntel>,
    pub coach: Coach,
    /// Which AI backends this PC has; optimistic until detected (like index.ts).
    pub backends: Backends,
    /// Minimap vision: fog tracker, camps, ghosts, alerts.
    pub field: crate::vision::Field,
    // ---- knowledge (Riot API / research) ----
    pub profiles: Vec<rc_core::riot::OpponentProfile>,
    pub profiles_key: String,
    /// Riot key the profiles were fetched with: a new key retries them.
    pub profiles_riot_key: String,
    pub patch_brief: Option<rc_core::patch_brief::PatchBrief>,
    pub patch_brief_requested: bool,
    pub matchup: Option<rc_core::intel::matchup::MatchupBrief>,
    pub matchup_key: String,
    // ---- situational / build ----
    pub fed_watcher: FedWatcher,
    pub team_watcher: TeamWatcher,
    pub build_trigger: BuildTrigger,
    pub build: Option<rc_core::build_advisor::BuildPanelState>,
    pub build_busy: bool,
    // ---- post-game ----
    pub match_log: MatchLog,
    pub obs: ObsRecorder,
    pub obs_game_key: String,
    pub review_done_for: String,
}

impl Game {
    pub fn new(data_dir: &std::path::Path) -> Self {
        Game {
            intent: None,
            ward_done: None,
            ward_seen: (0, -1000.0),
            static_data: None,
            tracker: IntelTracker::default(),
            intel: None,
            coach: Coach::default(),
            backends: Backends { claude: true, codex: true },
            field: crate::vision::Field::new(data_dir),
            profiles: Vec::new(),
            profiles_key: String::new(),
            profiles_riot_key: String::new(),
            patch_brief: None,
            patch_brief_requested: false,
            matchup: None,
            matchup_key: String::new(),
            fed_watcher: FedWatcher::default(),
            team_watcher: TeamWatcher::default(),
            build_trigger: BuildTrigger::default(),
            build: None,
            build_busy: false,
            match_log: MatchLog::new(),
            obs: ObsRecorder::default(),
            obs_game_key: String::new(),
            review_done_for: String::new(),
        }
    }
}

fn now_ms() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as f64).unwrap_or(0.0)
}

/// Live API left (game over / loading): forget per-game state.
pub fn on_live_lost(st: &Arc<AppState>) {
    let mut g = st.game.lock().unwrap();
    g.intel = None;
    g.build = None;
    // Per-game tip counter and the whole coach session (clocks, history, pending answers).
    g.coach.status.calls = 0;
    g.coach.reset_session();
    g.intent = None;
    g.build_trigger = Default::default();
    g.profiles_key.clear();
    g.matchup_key.clear();
    g.field.on_live_lost();
    drop(g);
    st.ui.lock().unwrap().last_advice = None;
}

/// Situational alert (rc-core) -> HUD alert.
pub fn alert_of(a: rc_core::intel::situational::AlertState) -> Alert {
    Alert { kind: a.kind.to_string(), text: a.text, confidence: a.confidence, expires_at: a.expires_at }
}

pub fn on_game_data(rt: &tokio::runtime::Handle, st: &Arc<AppState>, raw: &AllGameData) {
    let (coach_cfg, api_key_set) = {
        let s = st.settings.lock().unwrap();
        (s.value().coach.clone(), !s.value().riot.api_key.is_empty())
    };
    // Player profile text for the per-game system prompt (read before taking the game lock).
    let self_profile = rc_core::riot::self_profile::self_profile_for_coach(st.ui.lock().unwrap().player_profile.as_ref());
    let mut g = st.game.lock().unwrap();
    // English champion names everywhere (the client may be localised).
    let d = match &g.static_data {
        Some(sd) => normalize_champion_names(raw, |id| sd.name_by_id.get(id).cloned()),
        None => raw.clone(),
    };
    let sd = g.static_data.clone();
    let intel = g.tracker.update(&d, sd.as_ref().map(|s| &s.items), sd.as_ref().map(|s| &s.champion_stats));
    let now = crate::vision::now_ms();
    g.match_log.update(&intel, now);
    for a in crate::hud_state::local_alerts(&intel, now) {
        if !g.field.alerts.iter().any(|x| x.text == a.text) {
            g.field.alerts.push(a);
        }
    }
    for a in g.fed_watcher.update(&intel, now) {
        g.field.alerts.push(alert_of(a));
    }
    g.intel = Some(intel);
    crate::knowledge::maybe_load_profiles(st, &mut g, &d, api_key_set);
    crate::knowledge::maybe_load_matchup(st, &mut g);
    // Bots move nothing like people: a game with bots must never teach the fog model.
    let bots = d.all_players.iter().any(|p| p.is_bot.unwrap_or(false));
    crate::vision::on_game_data(rt, st, &mut g, &d.game_data.game_mode, d.game_data.map_number, bots);

    let b = g.backends;
    let live = pick(&coach_cfg.live_model, Task::Live, b);
    let death = pick(&coach_cfg.review_model, Task::Death, b);
    let c = &mut g.coach.config;
    // Only on Summoner's Rift (no tips in ARAM / Arena); --smoke never spends model calls.
    c.enabled = coach_cfg.enabled && d.game_data.map_number == 11 && !crate::smoke();
    c.live_model = live.model;
    c.review_model = death.model;
    c.live_effort = Some(live.effort);
    c.review_effort = Some(death.effort);
    c.period_sec = coach_cfg.period_sec as f64;
    // Build advice (an AI call) only on Summoner's Rift, like the coach.
    if d.game_data.map_number == 11 {
        crate::knowledge::maybe_build(st, &mut g, &coach_cfg.live_model, now);
    }
    crate::knowledge::maybe_patch_brief(st, &mut g);

    let action = {
        let g = &mut *g;
        g.field.expire_if_stale(now_ms());
        check_intent(g);
        let (camps, covered, brief, matchup) = match g.intel.as_ref() {
            Some(i) => (crate::vision::camp_summary(i, &g.field), crate::vision::covered_spots(i, &g.field), crate::knowledge::brief_text(g, i), matchup_for_coach(i, g.matchup.as_ref())),
            None => (Vec::new(), Vec::new(), None, None),
        };
        let ctx = CoachContext {
            intel: g.intel.as_ref(),
            ghosts: g.field.ghosts.iter().filter_map(|x| serde_json::to_value(x).ok()).collect(),
            // Same gank mute as the HUD (allies with me and the fight clearly ours): the model must not call
            // a gank the player was not shown.
            alerts: {
                let mute = g.intel.as_ref().and_then(|i| rc_core::intel::matchup::power_bar_near(i, g.matchup.as_ref(), &|c: &str| c.to_string(), &crate::knowledge::nearby(g))).filter(|p| p.local).is_some_and(|p| p.p_me >= 0.6 && !p.allies.is_empty());
                g.field.alerts.iter().filter(|x| !(mute && x.kind == "gank")).filter_map(|x| serde_json::to_value(x).ok()).collect()
            },
            profiles: g.profiles.iter().filter_map(|p| serde_json::to_value(p).ok()).collect(),
            patch_brief: brief,
            matchup,
            camps,
            covered_ward_spots: covered,
            field: g.intel.as_ref().map(|i| {
                let r2 = |v: f64| (v * 100.0).round() / 100.0;
                let red = i.me.as_ref().is_some_and(|m| m.team == "CHAOS");
                let pos = |u: &crate::vision::SeenUnit| serde_json::json!({ "champ": u.champion, "x": r2(u.x), "y": r2(u.y), "area": area_name(u.x, u.y, red) });
                let me = i.me.as_ref().map(|m| m.champion.clone()).unwrap_or_default();
                let near = crate::knowledge::nearby(g);
                let bar = rc_core::intel::matchup::power_bar_near(i, g.matchup.as_ref(), &|c: &str| c.to_string(), &near);
                let fight = bar.as_ref().filter(|p| p.local).map(|p| r2(p.p_me));
                let tracks = crate::knowledge::fog_tracks(g);
                let enemies_seen = crate::knowledge::seen(&g.field.visible_enemies);
                let danger = rc_core::intel::situational::danger_at(i, crate::knowledge::pos(g.field.my_pos), &tracks, &enemies_seen, fight.or_else(|| crate::knowledge::current_p_me(g)), &crate::knowledge::fed_names(i));
                let now = crate::vision::now_ms();
                serde_json::json!({
                    "me": g.field.my_pos.map(|(x, y)| serde_json::json!({ "x": r2(x), "y": r2(y), "area": area_name(x, y, red) })),
                    "allies": g.field.allies.iter().filter(|a| a.champion != me).map(pos).collect::<Vec<_>>(),
                    "visibleEnemies": g.field.visible_enemies.iter().map(pos).collect::<Vec<_>>(),
                    "fightHereNow": fight,
                    // Same as the player's fight bar: who is next to me, towers, its own one-line advice.
                    "nearMe": {
                        "allies": near.allies, "enemies": near.enemies.iter().map(|(c, seen)| if *seen { c.clone() } else { format!("{c} (mgła)") }).collect::<Vec<_>>(),
                        "unnamedEnemyBars": near.unknown_enemies, "underEnemyTower": near.me_under_enemy_tower, "underMyTower": near.me_under_my_tower,
                        "barTarget": bar.as_ref().filter(|p| p.local).map(|p| p.enemy.champion.clone()), "barAdvice": bar.as_ref().map(|p| p.advice.clone()),
                    },
                    "danger": danger.map(|d| serde_json::json!({ "label": d.label, "reasons": d.reasons })),
                    // Where each fogged enemy may be: the tracker's hypotheses (zones with probability).
                    "fogZones": g.field.ghosts.iter().map(|gh| serde_json::json!({ "champ": gh.champion, "best": [r2(gh.x), r2(gh.y), r2(gh.radius)], "unseenSec": gh.age.round(), "otherZones": gh.zones.iter().take(3).map(|z| serde_json::json!([r2(z.x), r2(z.y), r2(z.mass)])).collect::<Vec<_>>() })).collect::<Vec<_>>(),
                    "rivalBackOnLaneInSec": g.field.returns.iter().map(|(c, until)| serde_json::json!({ "champ": c, "sec": ((until - now) / 1000.0).max(0.0).round() })).collect::<Vec<_>>(),
                    "liveWarnings": g.field.toasts.iter().filter(|t| t.expires_at > now).map(|t| t.text.clone()).collect::<Vec<_>>(),
                    // Do I have a ward to place? Trinket type, Control Ward in inventory, last ward placed.
                    // Learned this game: their jungler / support habits (ganks per lane, side, start).
                    "enemyHabits": g.field.habits.iter().map(|(c, h)| (c.clone(), serde_json::json!(h.summary()))).collect::<serde_json::Map<_, _>>(),
                    // Off my lane (not jungler, not in base): where I am heading, so wards / calls fit the roam.
                    "roam": roam_info(g, i, red),
                    "myWards": i.me.as_ref().map(|m| {
                        let has = |id: i64| m.items.iter().any(|it| it.id == id);
                        let trinket = if has(3340) { "Stealth Ward" } else if has(3364) { "Oracle Lens" } else if has(3363) { "Farsight Alteration" } else { "brak" };
                        // Read from the HUD trinket slot: true = a charge ready, false = recharging (no ward now).
                        let ready = g.field.trinket.filter(|(_, at)| now - at < 5000.0).map(|(r, _)| r);
                        serde_json::json!({ "trinket": trinket, "trinketReady": ready, "controlWard": has(2055), "lastWardPlacedSecAgo": if g.ward_seen.1 < 0.0 { serde_json::Value::Null } else { serde_json::json!((i.game_time - g.ward_seen.1).round()) } })
                    }),
                })
            }),
            waves: g.field.waves.as_ref().map(crate::vision::waves_json),
            lane_pushes: { let now = crate::vision::now_ms(); g.field.push_seen.iter().filter(|(k, at)| now - at < 20000.0 && !k.starts_with("team:")).map(|(k, _)| k.replace('@', " ")).collect() },
            self_profile,
            // Item plan from the HUD build panel: the coach's "item" must match it.
            planned_build: g.build.as_ref().map(|b| b.items.iter().map(|it| it.name.clone()).collect()).unwrap_or_default(),
        };
        g.coach.tick(&ctx, Instant::now())
    };
    drop(g);
    match action {
        TickAction::None => {}
        TickAction::CheckAuth => check_auth(rt, st),
        TickAction::Ask(req) => {
            let st = st.clone();
            let rt2 = rt.clone();
            log::info!(target: "coach", "ask model={} prompt={}", req.call.model, req.call.prompt);
            rt.spawn(async move {
                let r = rc_core::llm::call_model::<Value>(&req.call).await.map(|r| (r.value, r.latency_ms));
                match &r {
                    Ok((v, ms)) => log::info!(target: "coach", "answer {ms:.0} ms {v}"),
                    Err(e) => log::warn!(target: "coach", "error {e}"),
                }
                let (advice, reauth) = {
                    let mut g = st.game.lock().unwrap();
                    let (mut advice, reauth) = g.coach.complete(&req, r, now_ms());
                    if let (Some(a), Some(sd)) = (advice.as_mut(), g.static_data.as_ref()) {
                        validate_item(a, &sd.items);
                    }
                    // The answer comes ~5 s late: a call about an enemy who died meanwhile ("Wejdź na Darius"
                    // after Darius is dead) is stale — drop the call, keep the rest.
                    if let (Some(a), Some(i)) = (advice.as_mut(), g.intel.as_ref()) {
                        let call = a.get("call").and_then(Value::as_str).unwrap_or("").to_lowercase();
                        let dead = i.players.iter().find(|p| !p.is_ally && p.is_dead && call.contains(&p.champion.to_lowercase()));
                        if let Some(p) = dead {
                            log::info!(target: "coach", "stale call dropped ({} is dead): {call}", p.champion);
                            a["call"] = Value::Null;
                        }
                    }
                    if let Some(a) = advice.as_ref() {
                        g.match_log.advice(a);
                        g.intent = intent_of(&g, a);
                    }
                    (advice, reauth)
                };
                if let Some(a) = advice {
                    st.ui.lock().unwrap().last_advice = Some(a);
                }
                if reauth {
                    check_auth(&rt2, &st);
                }
            });
        }
    }
}

/// Where on Summoner's Rift a minimap point is, in words for the coach (it misread raw x/y: "Idź z
/// Zaahenem na Dragon" while he was at Voidgrubs). Minimap: blue base bottom-left, y down.
/// Distance of a minimap point from a lane's line (fractions of the map).
fn lane_dist(lane: &str, x: f64, y: f64) -> f64 {
    match lane {
        "top" => x.min(y).max(0.0) - 0.07,
        "bot" => (1.0 - x).min(1.0 - y).max(0.0) - 0.07,
        _ => (x + y - 1.0).abs() / std::f64::consts::SQRT_2,
    }
    .abs()
}

/// I left my own lane (not jungler, alive, not in a base): {from, at, headingTo, towardLane, sec}.
fn roam_info(g: &Game, i: &GameIntel, red: bool) -> Option<serde_json::Value> {
    let me = i.me.as_ref().filter(|m| !m.is_dead)?;
    let lane = match me.role.as_str() { "TOP" => "top", "MIDDLE" => "mid", "BOTTOM" | "UTILITY" => "bot", _ => return None };
    let &(t, x, y) = g.field.my_trail.last()?;
    let in_base = x.hypot(1.0 - y) < 0.22 || (1.0 - x).hypot(y) < 0.22;
    if in_base || lane_dist(lane, x, y) < 0.13 || i.game_time - t > 3.0 {
        return None;
    }
    // Direction from ~6 s ago; projected 10 s ahead.
    let &(t0, x0, y0) = g.field.my_trail.iter().find(|(at, _, _)| t - at <= 8.0)?;
    let dt = (t - t0).max(1.0);
    let (vx, vy) = ((x - x0) / dt, (y - y0) / dt);
    let moving = vx.hypot(vy) > 0.004;
    let (px, py) = if moving { ((x + vx * 10.0).clamp(0.0, 1.0), (y + vy * 10.0).clamp(0.0, 1.0)) } else { (x, y) };
    let toward = ["top", "mid", "bot"].into_iter().filter(|l| *l != lane).min_by(|a, b| lane_dist(a, px, py).total_cmp(&lane_dist(b, px, py)))?;
    let off_since = g.field.my_trail.iter().rev().take_while(|(_, tx, ty)| lane_dist(lane, *tx, *ty) >= 0.13).last().map_or(0.0, |(at, _, _)| t - at);
    Some(serde_json::json!({
        "from": lane, "at": area_name(x, y, red), "headingTo": if moving { serde_json::json!(area_name(px, py, red)) } else { serde_json::Value::Null },
        "towardLane": if moving { serde_json::json!(toward) } else { serde_json::Value::Null }, "offLaneSec": off_since.round(),
    }))
}

pub fn area_name(x: f64, y: f64, red: bool) -> String {
    let (ours, theirs) = if red { ("wroga", "nasza") } else { ("nasza", "wroga") };
    let half = if x + y < 1.0 { "top" } else { "bot" };
    if x.hypot(1.0 - y) < 0.22 {
        return if red { "baza wroga".into() } else { "nasza baza".into() };
    }
    if (1.0 - x).hypot(y) < 0.22 {
        return if red { "nasza baza".into() } else { "baza wroga".into() };
    }
    if (x - 0.663).hypot(y - 0.703) < 0.07 {
        return "jama Dragona".into();
    }
    if (x - 0.337).hypot(y - 0.296) < 0.07 {
        return "jama Barona / Voidgrubs".into();
    }
    if x < 0.12 || y < 0.12 {
        return "linia top".into();
    }
    if x > 0.88 || y > 0.88 {
        return "linia bot".into();
    }
    if (x + y - 1.0).abs() < 0.07 {
        return "linia mid".into();
    }
    if (x - y).abs() < 0.07 {
        return format!("rzeka {half}");
    }
    // Below the river (y > x) is the blue side's jungle.
    // Below the river (y > x) is the blue jungle: ours on blue, theirs on red.
    let blue_jungle = y > x;
    let side = if blue_jungle != red { "nasza" } else { "wroga" };
    let _ = (ours, theirs);
    format!("{side} dżungla {half}")
}

/// Ward to place or place to go from a coach call (None when the call has neither).
fn intent_of(g: &Game, a: &Value) -> Option<Intent> {
    let intel = g.intel.as_ref()?;
    let me = intel.me.as_ref()?;
    let red = me.team == "CHAOS";
    let call = a.get("call").and_then(Value::as_str).unwrap_or("").to_string();
    if let Some(spot) = a.get("ward").and_then(|w| w.get("spot")).and_then(Value::as_str) {
        let covered = crate::vision::covered_spots(intel, &g.field).iter().any(|s| s == spot);
        return Some(Intent { ward_spot: Some(spot.to_string()), target: None, label: call, since: intel.game_time, ward_score_at: me.ward_score, was_covered: covered, at_spot_since: None });
    }
    let target = a.get("mapHint").and_then(|m| m.get("target")).and_then(Value::as_str)?;
    // A lane target means its wave (the arrow points there too), not the lane corner.
    let wave = matches!(target, "top" | "mid" | "bot").then(|| g.field.waves.as_ref().and_then(|w| w.lanes.iter().find(|l| l.lane == target)).and_then(|l| l.front)).flatten();
    let p = match wave { Some(front) => crate::hud::wave_point(target, front, red), None => crate::hud_state::map_point_for(target, red)? };
    Some(Intent { ward_spot: None, target: Some(p), label: call, since: intel.game_time, ward_score_at: me.ward_score, was_covered: false, at_spot_since: None })
}

/// Did the player do what the last call said? Then a short confirmation and the next step now.
fn check_intent(g: &mut Game) {
    if let Some((ws, t)) = g.intel.as_ref().and_then(|i| i.me.as_ref().map(|m| (m.ward_score, i.game_time))) {
        if ws > g.ward_seen.0 {
            g.ward_seen = (ws, t);
        } else if ws < g.ward_seen.0 {
            g.ward_seen = (ws, -1000.0); // new game
        }
    }
    let Some(it) = g.intent.clone() else { return };
    let Some(intel) = g.intel.as_ref() else { return };
    let Some(me) = intel.me.as_ref() else { return };
    if me.is_dead || intel.game_time - it.since > 120.0 {
        g.intent = None;
        return;
    }
    let (done, text, reason) = if let Some(spot) = &it.ward_spot {
        let covered = crate::vision::covered_spots(intel, &g.field).iter().any(|s| s == spot);
        // Only when I stand at the spot: the spot getting vision (an ally walking by) or ward score
        // going up elsewhere said "Ward postawiony" without a ward.
        let red = me.team == "CHAOS";
        let at_spot = match (rc_core::wards::ward_point(spot, red), g.field.my_pos) {
            (Some((x, y)), Some((mx, my))) => (mx - x).hypot(my - y) < 0.07,
            _ => false,
        };
        // Ward points went up = a ward placed (the user's rule); the spot getting vision counts only when I stand there.
        // The API ward score is rounded and grows slowly (it almost never moved on placing): standing at
        // the spot for 2.5 s counts as placed too.
        let gt = intel.game_time;
        // Remember that I was at the spot (kept after I leave, for the "ward left behind" check).
        let since = if at_spot { it.at_spot_since.or(Some(gt)) } else { it.at_spot_since };
        let dwell = since.is_some_and(|s0| gt - s0 >= 2.5);
        if let Some(i2) = g.intent.as_mut() {
            i2.at_spot_since = since;
        }
        // Best signal: the spot still has vision while none of us stands there = a ward is there (I
        // walked to it and left it behind). Standing there is only a hint now (no ward = no vision after).
        let spot_pt = rc_core::wards::ward_point(spot, red);
        let nobody_there = spot_pt.is_some_and(|(x, y)| {
            !g.field.allies.iter().any(|a| (a.x - x).hypot(a.y - y) < 0.06) && g.field.my_pos.is_none_or(|(mx, my)| (mx - x).hypot(my - y) >= 0.06)
        });
        let visited = it.at_spot_since.is_some() || dwell;
        let left_ward = covered && nobody_there && visited;
        let _ = dwell;
        // Trinket went from a charge to recharging after the call = a ward was placed.
        let trinket_used = g.field.trinket_used_at > 0.0 && (crate::vision::now_ms() - g.field.trinket_used_at) < (gt - it.since).max(0.0) * 1000.0 + 1000.0;
        // Evidence must belong to the requested spot: I was there, then ward points / trinket use / vision left behind.
        // Not the ward score: it grows by itself while older wards live and when killing wards ("Ward
        // postawiony" without a ward). Vision left behind counts only if the spot had none before.
        let placed = visited && ((left_ward && !it.was_covered) || trinket_used);
        if placed {
            g.ward_seen.1 = gt;
        }
        (placed, "Ward postawiony".to_string(), "ward-done")
    } else if let (Some((x, y)), Some((mx, my))) = (it.target, g.field.my_pos) {
        ((mx - x).hypot(my - y) < 0.08, "Jesteś na miejscu".to_string(), "arrived")
    } else {
        (false, String::new(), "")
    };
    if !done {
        return;
    }
    g.intent = None;
    let now = now_ms();
    if reason == "ward-done" {
        if let Some(spot) = it.ward_spot.clone() {
            g.ward_done = Some((spot, now));
        }
        g.field.toasts.push(crate::vision::Toast { id: now as u64 + 31, kind: "done".into(), team: "ally".into(), champion: None, text: format!("✓ {text}"), expires_at: now + 3000.0 });
    }
    g.coach.request_followup(reason);
}

pub fn check_auth(rt: &tokio::runtime::Handle, st: &Arc<AppState>) {
    let st = st.clone();
    rt.spawn(async move {
        // Check the backend of the selected model: a Codex-only setup must not be blocked by a missing
        // Claude login (and the other way round).
        let model = st.game.lock().unwrap().coach.config.live_model.clone();
        let (ok, err) = if rc_core::llm::is_codex_model(&model) {
            match rc_core::llm::codex::resolve_codex().await {
                Some(_) => (true, None),
                None => (false, Some("Codex nie jest zainstalowany".to_string())),
            }
        } else {
            let a = rc_core::llm::claude::auth_status().await;
            (a.logged_in, a.error)
        };
        st.game.lock().unwrap().coach.auth_result(ok, err);
    });
}

/// Startup + every 5 min: detect Claude Code login / Codex install.
pub fn spawn_backend_detection(rt: &tokio::runtime::Handle, st: &Arc<AppState>) {
    let st = st.clone();
    rt.spawn(async move {
        loop {
            let b = rc_core::llm::profiles::detect_backends().await;
            log::info!("AI backends {}", serde_json::to_string(&b).unwrap_or_default());
            st.game.lock().unwrap().backends = b;
            crate::knowledge::apply_models(&st);
            tokio::time::sleep(std::time::Duration::from_secs(300)).await;
        }
    });
}

/// Data Dragon in the background (same cache dir as the TS app).
pub fn spawn_static_data(rt: &tokio::runtime::Handle, st: &Arc<AppState>) {
    let st = st.clone();
    rt.spawn(async move {
        match rc_core::intel::ddragon::load_static_data(&st.data_dir.join("ddragon"), "en_US").await {
            Ok(sd) => {
                st.ui.lock().unwrap().ddragon_version = Some(sd.version.clone());
                st.game.lock().unwrap().static_data = Some(Arc::new(sd));
            }
            Err(e) => log::warn!("ddragon {e}"),
        }
    });
}
