//! Process-wide state: settings, data sources, statuses shown in the UI.

use rc_core::settings::SettingsStore;
use rc_core::sources::lcu::{LcuClient, LcuShared};
use rc_core::sources::live::LiveShared;
use rc_core::types::*;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub struct AppState {
    pub version: String,
    pub data_dir: PathBuf,
    pub settings: Mutex<SettingsStore>,
    pub live: Arc<Mutex<LiveShared>>,
    pub lcu: Arc<Mutex<LcuShared>>,
    pub lcu_client: LcuClient,
    pub replay_api: Arc<Mutex<SourceState>>,
    pub ui: Mutex<UiState>,
    pub game: Mutex<crate::game::Game>,
    pub vision: crate::vision::VisionHub,
    pub hud: crate::hud::Hud,
    pub app: tauri::AppHandle,
    pub rt: tokio::runtime::Handle,
    pub recorder: Mutex<crate::recorder::Recorder>,
    pub harvester: Arc<rc_core::harvest::Harvester>,
    pub harvest: Mutex<crate::harvest::HarvestCtl>,
    pub replay: Mutex<crate::playback::ReplayCtl>,
}

/// Parts of AppStatus owned by the coach / intel / runes side.
pub struct UiState {
    pub ddragon_version: Option<String>,
    pub last_advice: Option<Value>,
    pub profiles: ProfilesStatus,
    pub runes: RunesStatus,
    /// When the Live API went away (game over / loading); None while connected.
    pub live_lost_at: Option<std::time::Instant>,
    /// The player's own habits + review lessons (`<userData>/player-profile.json`).
    pub player_profile: Option<rc_core::riot::PlayerSelfProfile>,
    pub self_profile_busy: bool,

    pub quitting: bool,
}

impl Default for UiState {
    fn default() -> Self {
        UiState {
            ddragon_version: None,
            last_advice: None,
            profiles: ProfilesStatus { state: "idle".into(), count: 0, error: None },
            runes: RunesStatus::idle(),
            live_lost_at: None,
            player_profile: None,
            self_profile_busy: false,
            quitting: false,
        }
    }
}

/// `<userData>/recordings`, or `--recordings-root=<dir>` (testing without touching real recordings).
pub fn recordings_root(data_dir: &std::path::Path) -> PathBuf {
    crate::arg_value("recordings-root").map(PathBuf::from).unwrap_or_else(|| data_dir.join("recordings"))
}

impl AppState {
    /// Replays are not live games: no vision, no HUD, no coach (index.ts checks replayApiState).
    /// `--allow-replay` (testing) treats an open replay like a live game.
    pub fn replay_blocks(&self) -> bool {
        self.replay_open() && !crate::allow_replay()
    }

    /// A League replay is open and its Replay API answers (`replayApiState === 'connected'`).
    pub fn replay_open(&self) -> bool {
        *self.replay_api.lock().unwrap() == SourceState::Connected
    }

    pub fn recordings_dir(&self) -> PathBuf {
        recordings_root(&self.data_dir)
    }
    pub fn reviews_dir(&self) -> PathBuf {
        match crate::arg_value("reviews-dir") {
            Some(d) => PathBuf::from(d),
            None => self.data_dir.join("reviews"),
        }
    }

    /// `status()` from index.ts.
    pub fn status(&self) -> AppStatus {
        let (settings_json, auto_runes) = {
            let s = self.settings.lock().unwrap();
            (s.json().clone(), s.value().auto_runes)
        };
        // A recorded session open in the app's own player replaces the live game info.
        let (replay, replay_game) = {
            let r = self.replay.lock().unwrap();
            match r.player.as_ref() {
                Some(p) => (serde_json::to_value(&p.status).ok(), Some(r.game.as_ref().map(rc_core::sources::live::to_game_info))),
                None => (None, None),
            }
        };
        let (live_state, game) = {
            let l = self.live.lock().unwrap();
            (l.state, replay_game.unwrap_or_else(|| l.last.as_ref().map(rc_core::sources::live::to_game_info)))
        };
        let (lcu_state, phase, me) = {
            let l = self.lcu.lock().unwrap();
            (l.state, l.phase.clone(), l.me.clone())
        };
        // Highest-mastery champion key -> English name (header splash).
        let me = me.map(|mut p| {
            if p.top_champion.is_none() {
                if let (Some(k), Some(sd)) = (p.top_champion_key, self.game.lock().unwrap().static_data.clone()) {
                    p.top_champion = sd.champion_by_key.get(&k).cloned();
                }
            }
            p
        });
        let coach = {
            let c = &self.game.lock().unwrap().coach.status;
            CoachStatus { state: c.state.clone(), model: c.model.clone(), calls: c.calls, last_error: c.last_error.clone(), last_latency_ms: c.last_latency_ms }
        };
        let recording = self.recorder.lock().unwrap().status.clone();
        let harvest = crate::harvest::status(self);
        let ui = self.ui.lock().unwrap();
        AppStatus {
            version: self.version.clone(),
            ddragon_version: ui.ddragon_version.clone(),
            me,
            player_profile: ui.player_profile.clone(),
            live_api: live_state,
            lcu: lcu_state,
            lcu_phase: phase,
            game,
            capture: self.vision.status(),
            recording,
            replay,
            replay_api: *self.replay_api.lock().unwrap(),
            harvest,
            coach,
            last_advice: ui.last_advice.clone(),
            profiles: ui.profiles.clone(),
            runes: if auto_runes { ui.runes.clone() } else { RunesStatus { state: "off".into(), label: None, error: None } },
            settings: settings_json,
        }
    }
}
