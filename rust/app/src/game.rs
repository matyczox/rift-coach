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

pub struct Game {
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
            static_data: None,
            tracker: IntelTracker::default(),
            intel: None,
            coach: Coach::default(),
            backends: Backends { claude: true, codex: true },
            field: crate::vision::Field::new(data_dir),
            profiles: Vec::new(),
            profiles_key: String::new(),
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
    // Per-game tip counter.
    g.coach.status.calls = 0;
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
    crate::vision::on_game_data(rt, st, &mut g, &d.game_data.game_mode, d.game_data.map_number);

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
    crate::knowledge::maybe_build(st, &mut g, &coach_cfg.live_model, now);
    crate::knowledge::maybe_patch_brief(st, &mut g);

    let action = {
        let g = &mut *g;
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

pub fn check_auth(rt: &tokio::runtime::Handle, st: &Arc<AppState>) {
    let st = st.clone();
    rt.spawn(async move {
        let a = rc_core::llm::claude::auth_status().await;
        st.game.lock().unwrap().coach.auth_result(a.logged_in, a.error);
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
        match rc_core::intel::ddragon::load_static_data(&st.data_dir.join("ddragon"), "pl_PL").await {
            Ok(sd) => {
                st.ui.lock().unwrap().ddragon_version = Some(sd.version.clone());
                st.game.lock().unwrap().static_data = Some(Arc::new(sd));
            }
            Err(e) => log::warn!("ddragon {e}"),
        }
    });
}
