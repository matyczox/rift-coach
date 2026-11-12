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
}

/// Parts of AppStatus owned by the coach / intel / runes side.
pub struct UiState {
    pub ddragon_version: Option<String>,
    pub coach: CoachStatus,
    pub last_advice: Option<Value>,
    pub profiles: ProfilesStatus,
    pub runes: RunesStatus,
    /// Game info from the replay being played back (none yet in the Rust port).
    pub quitting: bool,
}

impl Default for UiState {
    fn default() -> Self {
        UiState {
            ddragon_version: None,
            // Coach starts 'off' and checks the Claude login on its first tick in a game (coach.ts).
            coach: CoachStatus { state: "off".into(), model: "claude-sonnet-5-5".into(), calls: 0, last_error: None, last_latency_ms: None },
            last_advice: None,
            profiles: ProfilesStatus { state: "idle".into(), count: 0, error: None },
            runes: RunesStatus::idle(),
            quitting: false,
        }
    }
}

impl AppState {
    pub fn recordings_dir(&self) -> PathBuf {
        self.data_dir.join("recordings")
    }
    pub fn reviews_dir(&self) -> PathBuf {
        self.data_dir.join("reviews")
    }

    /// `status()` from index.ts.
    pub fn status(&self) -> AppStatus {
        let (settings_json, auto_runes) = {
            let s = self.settings.lock().unwrap();
            (s.json().clone(), s.value().auto_runes)
        };
        let (live_state, game) = {
            let l = self.live.lock().unwrap();
            (l.state, l.last.as_ref().map(rc_core::sources::live::to_game_info))
        };
        let (lcu_state, phase, me) = {
            let l = self.lcu.lock().unwrap();
            (l.state, l.phase.clone(), l.me.clone())
        };
        let ui = self.ui.lock().unwrap();
        AppStatus {
            version: self.version.clone(),
            ddragon_version: ui.ddragon_version.clone(),
            me,
            live_api: live_state,
            lcu: lcu_state,
            lcu_phase: phase,
            game,
            capture: CaptureStatus::default(),
            recording: RecordingStatus::default(),
            replay: None,
            replay_api: *self.replay_api.lock().unwrap(),
            harvest: HarvestStatus::default(),
            coach: ui.coach.clone(),
            last_advice: ui.last_advice.clone(),
            profiles: ui.profiles.clone(),
            runes: if auto_runes { ui.runes.clone() } else { RunesStatus { state: "off".into(), label: None, error: None } },
            settings: settings_json,
        }
    }
}
