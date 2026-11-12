//! Riot Replay API (127.0.0.1:2999/replay/*), `src/main/sources/replayApi.ts`.
//! Needs EnableReplayApi=1 in game.cfg.

use crate::sources::http::{get_json, send_json};
use crate::types::SourceState;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const BASE: &str = "https://127.0.0.1:2999";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Playback {
    #[serde(default)]
    pub length: f64,
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub seeking: bool,
    #[serde(default)]
    pub speed: f64,
    #[serde(default)]
    pub time: f64,
}

pub async fn playback() -> Result<Playback> {
    get_json(&format!("{BASE}/replay/playback"), None, 1500).await
}

/// Partial update: `{ time?, paused?, speed? }`.
pub async fn set_playback(p: &Value) -> Result<Option<Value>> {
    send_json("POST", &format!("{BASE}/replay/playback"), p, None, 3000).await
}

pub async fn set_render(r: &Value) -> Result<Option<Value>> {
    send_json("POST", &format!("{BASE}/replay/render"), r, None, 3000).await
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayGame {
    #[serde(rename = "processID", default)]
    pub process_id: i64,
}

pub async fn game() -> Result<ReplayGame> {
    get_json(&format!("{BASE}/replay/game"), None, 1500).await
}

/// Seek while paused and wait until the engine has landed on `time`.
pub async fn seek(time: f64, timeout_ms: u64) -> Result<Playback> {
    set_playback(&serde_json::json!({ "time": time, "paused": true })).await?;
    let end = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        let pb = playback().await?;
        if !pb.seeking && (pb.time - time).abs() < 0.25 {
            return Ok(pb);
        }
        if Instant::now() > end {
            bail!("seek to {time} timed out");
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
}

/// index.ts: every 1 s, `replayApiState = playback ok ? 'connected' : 'waiting'`.
pub fn spawn_state_poller(rt: &tokio::runtime::Handle) -> Arc<Mutex<SourceState>> {
    let st = Arc::new(Mutex::new(SourceState::Waiting));
    let s = st.clone();
    rt.spawn(async move {
        loop {
            let ok = playback().await.is_ok();
            *s.lock().unwrap() = if ok { SourceState::Connected } else { SourceState::Waiting };
            tokio::time::sleep(Duration::from_millis(1000)).await;
        }
    });
    st
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_parses() {
        let p: Playback = serde_json::from_str(r#"{"length":1800.5,"paused":false,"seeking":false,"speed":8.0,"time":12.25}"#).unwrap();
        assert_eq!(p.speed, 8.0);
        let g: ReplayGame = serde_json::from_str(r#"{"processID":4242}"#).unwrap();
        assert_eq!(g.process_id, 4242);
    }
}
