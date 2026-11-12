//! Shapes shared with the UI (`src/shared/types.ts`, `src/shared/intel.ts`).
//! Serialized with camelCase field names exactly like the TypeScript types.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SourceState {
    Off,
    #[default]
    Waiting,
    Connected,
    Error,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameInfo {
    pub game_mode: String,
    pub game_time: f64,
    pub map_number: i64,
    pub active_champion: Option<String>,
    pub allies: Vec<String>,
    pub enemies: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ally_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enemy_ids: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureStatus {
    pub state: SourceState,
    pub window_width: Option<u32>,
    pub window_height: Option<u32>,
    pub fps: f64,
    pub last_frame_at: Option<f64>,
    pub error: Option<String>,
}

impl Default for CaptureStatus {
    fn default() -> Self {
        // Screen capture lives in rc-vision (other crate, not wired yet).
        CaptureStatus { state: SourceState::Off, window_width: None, window_height: None, fps: 0.0, last_frame_at: None, error: None }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RecordingStatus {
    pub active: bool,
    pub session_id: Option<String>,
    pub frames: u64,
    pub api_samples: u64,
    pub bytes: u64,
    pub started_at: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarvestStatus {
    pub state: String,
    pub game_id: Option<i64>,
    pub pass: Option<String>,
    pub position_sec: f64,
    pub length_sec: f64,
    pub frames: u64,
    pub error: Option<String>,
    pub queue: Vec<i64>,
    /// waiting-fog: the replay HUD team view the user must pick before continuing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub need_fog: Option<String>,
}

impl Default for HarvestStatus {
    fn default() -> Self {
        HarvestStatus { state: "idle".into(), game_id: None, pass: None, position_sec: 0.0, length_sec: 0.0, frames: 0, error: None, queue: vec![], need_fog: None }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoachStatus {
    pub state: String, // off | not-logged-in | idle | thinking | error
    pub model: String,
    pub calls: u64,
    pub last_error: Option<String>,
    pub last_latency_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfilesStatus {
    pub state: String,
    pub count: u32,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunesStatus {
    pub state: String, // idle | working | set | no-slot | error | off
    pub label: Option<String>,
    pub error: Option<String>,
}

impl RunesStatus {
    pub fn idle() -> Self {
        RunesStatus { state: "idle".into(), label: None, error: None }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RankEntry {
    pub queue: String, // solo | flex
    pub tier: String,
    pub division: String,
    pub lp: i64,
    pub wins: i64,
    pub losses: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerProfile {
    pub game_name: String,
    pub tag_line: String,
    pub level: i64,
    pub icon_id: i64,
    pub ranks: Vec<RankEntry>,
    /// Highest-mastery champion (English name) for the header splash; filled by the app from Data Dragon.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_champion: Option<String>,
    /// Champion key of the highest mastery (LCU), resolved to `top_champion` by the app.
    #[serde(skip)]
    pub top_champion_key: Option<i64>,
}

/// `AppStatus` from types.ts.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStatus {
    pub version: String,
    pub ddragon_version: Option<String>,
    pub me: Option<PlayerProfile>,
    /// Player profile for the coach (null until built: needs a Riot API key).
    pub player_profile: Option<crate::riot::self_profile::PlayerSelfProfile>,
    pub live_api: SourceState,
    pub lcu: SourceState,
    pub lcu_phase: Option<String>,
    pub game: Option<GameInfo>,
    pub capture: CaptureStatus,
    pub recording: RecordingStatus,
    pub replay: Option<Value>,
    pub replay_api: SourceState,
    pub harvest: HarvestStatus,
    pub coach: CoachStatus,
    pub last_advice: Option<Value>,
    pub profiles: ProfilesStatus,
    pub runes: RunesStatus,
    /// Full settings JSON (same object the UI gets from the Electron app).
    pub settings: Value,
}
