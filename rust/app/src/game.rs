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
}

pub struct Game {
    pub intent: Option<Intent>,
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
            alerts: g.field.alerts.iter().filter_map(|x| serde_json::to_value(x).ok()).collect(),
            profiles: g.profiles.iter().filter_map(|p| serde_json::to_value(p).ok()).collect(),
            patch_brief: brief,
            matchup,
            camps,
            covered_ward_spots: covered,
            field: g.intel.as_ref().map(|i| {
                let r2 = |v: f64| (v * 100.0).round() / 100.0;
                let pos = |u: &crate::vision::SeenUnit| serde_json::json!({ "champ": u.champion, "x": r2(u.x), "y": r2(u.y) });
                let me = i.me.as_ref().map(|m| m.champion.clone()).unwrap_or_default();
                let fight = rc_core::intel::matchup::power_bar_near(i, g.matchup.as_ref(), &|c: &str| c.to_string(), &crate::knowledge::nearby(g)).filter(|p| p.local).map(|p| r2(p.p_me));
                serde_json::json!({
                    "me": g.field.my_pos.map(|(x, y)| serde_json::json!({ "x": r2(x), "y": r2(y) })),
                    "allies": g.field.allies.iter().filter(|a| a.champion != me).map(pos).collect::<Vec<_>>(),
                    "visibleEnemies": g.field.visible_enemies.iter().map(pos).collect::<Vec<_>>(),
                    "fightHereNow": fight,
                })
            }),
            waves: g.field.waves.as_ref().map(crate::vision::waves_json),
            lane_pushes: { let now = crate::vision::now_ms(); g.field.push_seen.iter().filter(|(_, at)| now - at < 20000.0).map(|(k, _)| k.replace('@', " ")).collect() },
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
            rt.spawn(async move {
                let r = rc_core::llm::call_model::<Value>(&req.call).await.map(|r| (r.value, r.latency_ms));
                let (advice, reauth) = {
                    let mut g = st.game.lock().unwrap();
                    let (mut advice, reauth) = g.coach.complete(&req, r, now_ms());
                    if let (Some(a), Some(sd)) = (advice.as_mut(), g.static_data.as_ref()) {
                        validate_item(a, &sd.items);
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

/// Ward to place or place to go from a coach call (None when the call has neither).
fn intent_of(g: &Game, a: &Value) -> Option<Intent> {
    let intel = g.intel.as_ref()?;
    let me = intel.me.as_ref()?;
    let red = me.team == "CHAOS";
    let call = a.get("call").and_then(Value::as_str).unwrap_or("").to_string();
    if let Some(spot) = a.get("ward").and_then(|w| w.get("spot")).and_then(Value::as_str) {
        let covered = crate::vision::covered_spots(intel, &g.field).iter().any(|s| s == spot);
        return Some(Intent { ward_spot: Some(spot.to_string()), target: None, label: call, since: intel.game_time, ward_score_at: me.ward_score, was_covered: covered });
    }
    let target = a.get("mapHint").and_then(|m| m.get("target")).and_then(Value::as_str)?;
    let p = crate::hud_state::map_point_for(target, red)?;
    Some(Intent { ward_spot: None, target: Some(p), label: call, since: intel.game_time, ward_score_at: me.ward_score, was_covered: false })
}

/// Did the player do what the last call said? Then a short confirmation and the next step now.
fn check_intent(g: &mut Game) {
    let Some(it) = g.intent.clone() else { return };
    let Some(intel) = g.intel.as_ref() else { return };
    let Some(me) = intel.me.as_ref() else { return };
    if me.is_dead || intel.game_time - it.since > 120.0 {
        g.intent = None;
        return;
    }
    let (done, text, reason) = if let Some(spot) = &it.ward_spot {
        let covered = crate::vision::covered_spots(intel, &g.field).iter().any(|s| s == spot);
        let placed = me.ward_score > it.ward_score_at || (covered && !it.was_covered);
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
    g.field.toasts.push(crate::vision::Toast { id: now as u64 + 31, kind: "done".into(), team: "ally".into(), champion: None, text: format!("✓ {text}"), expires_at: now + 3000.0 });
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
