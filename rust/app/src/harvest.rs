//! Replay harvesting (index.ts `harvestStart` / `harvestOpen` / queue): collect fog / no-fog
//! minimap frames from League replays with `rc_core::harvest::Harvester`. The capture side
//! feeds `Harvester::frame` from `vision::frame_loop`. We never send input to the game: the
//! replay is controlled only through its Replay API, and the HUD fog team is switched by the
//! user (state `waiting-fog`).

use crate::state::AppState;
use rc_core::harvest::{HarvestRequest, LiveReplayApi, ReplayControl};
use rc_core::llm::caller::BoxFuture;
use rc_core::sources::replay::Playback;
use rc_core::types::{HarvestStatus, SourceState};
use serde_json::{json, Map, Value};
use std::sync::Arc;
use std::time::Duration;

#[derive(Default)]
pub struct HarvestCtl {
    /// Replay ids still to harvest (excluding the current one).
    pub queue: Vec<i64>,
    /// Replay being launched through the League client.
    pub launching: Option<i64>,
    pub cancelled: bool,
    /// Launch failure of a queued replay (the harvester itself never ran).
    pub error: Option<(i64, String)>,
}

/// `harvest` in AppStatus: the harvester's state + launching / queue.
pub fn status(st: &AppState) -> HarvestStatus {
    let h = st.harvester.status();
    let ctl = st.harvest.lock().unwrap();
    let mut s = HarvestStatus {
        state: h.state.clone(),
        game_id: h.game_id,
        pass: h.pass.map(|p| p.as_str().to_string()),
        position_sec: h.position_sec,
        length_sec: h.length_sec,
        frames: h.frames,
        error: h.error.clone(),
        queue: ctl.queue.clone(),
        need_fog: h.need_fog.map(|p| p.as_str().to_string()),
    };
    if let Some(id) = ctl.launching {
        s.state = "launching".into();
        s.game_id = Some(id);
    } else if let Some((id, e)) = ctl.error.as_ref().filter(|_| !st.harvester.running()) {
        s.state = "error".into();
        s.game_id = Some(*id);
        s.error = Some(e.clone());
    }
    s
}

/// Harvest the replay that is open now, with a fresh calibration for the replay HUD.
async fn harvest_open(st: &Arc<AppState>, speed: f64, game_id: Option<i64>) -> anyhow::Result<()> {
    st.vision.capture.set_fps(30.0);
    for _ in 0..20 {
        if st.vision.capture.status().state == rc_vision::capture::CaptureState::Connected {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    // Let a frame arrive at 30 fps before looking for the minimap.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (st2, app) = (st.clone(), st.app.clone());
    let _ = tokio::task::spawn_blocking(move || crate::vision::auto_calibrate(&app, &st2)).await;
    let mut meta = Map::new();
    meta.insert("calibration".into(), st.settings.lock().unwrap().value().calibration.clone().unwrap_or(Value::Null));
    meta.insert("appVersion".into(), json!(st.version));
    let req = HarvestRequest { game_id, speed, meta, confirm_first: false, ..Default::default() };
    let r = st.harvester.run(&LiveReplayApi, &req).await;
    let fps = st.settings.lock().unwrap().value().minimap_fps.max(1) as f64;
    st.vision.capture.set_fps(fps);
    r
}

async fn wait_replay_ready(timeout: Duration) -> anyhow::Result<()> {
    let end = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < end {
        if let Ok(pb) = rc_core::sources::replay::playback().await {
            if pb.length > 0.0 && pb.time > 0.5 {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    anyhow::bail!("Powtórka nie wystartowała (czy EnableReplayApi=1 jest w game.cfg?)")
}

/// Close the replay game process — only the one the Replay API itself names (closeReplayProcess).
/// Never called for a live game: the Replay API answers only while a replay is open.
async fn close_replay_process() {
    let Ok(g) = rc_core::sources::replay::game().await else { return };
    if g.process_id > 0 {
        let mut c = std::process::Command::new("taskkill");
        c.args(["/PID", &g.process_id.to_string(), "/F"]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let _ = c.output();
    }
    for _ in 0..30 {
        if rc_core::sources::replay::playback().await.is_err() {
            return;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Game id of the replay that is open now (LCU gameflow session `gameData.gameId`).
async fn open_replay_game_id(st: &AppState) -> Option<i64> {
    let g = st.lcu_client.get::<Value>("/lol-gameflow/v1/session").await.ok()?;
    // Only while the client says a replay is being watched: in "Lobby" the session still holds
    // the LAST PLAYED game (seen: a replay opened from the lobby reported the previous ARAM id).
    if g.get("phase").and_then(Value::as_str) != Some("WatchInProgress") {
        return None;
    }
    g.pointer("/gameData/gameId").and_then(Value::as_i64).filter(|id| *id > 0)
}

/// RcApi `harvestStart({ gameIds?, speed? })`.
pub fn start(st: &Arc<AppState>, opts: &Value) -> Result<(), String> {
    {
        let ctl = st.harvest.lock().unwrap();
        if st.harvester.running() || ctl.launching.is_some() {
            return Err("Zbieranie już trwa".into());
        }
    }
    let speed = opts.get("speed").and_then(Value::as_f64).unwrap_or(8.0).clamp(1.0, 16.0);
    let ids: Vec<i64> = opts.get("gameIds").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_i64).collect()).unwrap_or_default();
    {
        let mut ctl = st.harvest.lock().unwrap();
        ctl.cancelled = false;
        ctl.error = None;
    }
    if ids.is_empty() {
        if !st.replay_open() {
            return Err("Najpierw otwórz powtórkę w kliencie League.".into());
        }
        let st = st.clone();
        st.rt.clone().spawn(async move {
            let id = open_replay_game_id(&st).await;
            if let Err(e) = harvest_open(&st, speed, id).await {
                log::warn!("harvest {e}");
            }
        });
        return Ok(());
    }
    if st.live.lock().unwrap().state == SourceState::Connected && !st.replay_open() {
        return Err("Trwa mecz — zbieranie z powtórek po meczu".into());
    }
    st.harvest.lock().unwrap().queue = ids;
    let st = st.clone();
    st.rt.clone().spawn(async move {
        loop {
            let id = {
                let mut ctl = st.harvest.lock().unwrap();
                if ctl.cancelled || ctl.queue.is_empty() {
                    break;
                }
                let id = ctl.queue.remove(0);
                ctl.launching = Some(id);
                id
            };
            let res: anyhow::Result<()> = async {
                if st.replay_open() {
                    close_replay_process().await;
                }
                st.lcu_client.post(&format!("/lol-replays/v1/rofls/{id}/watch"), &json!({ "componentType": "replay-button_match-history" })).await?;
                wait_replay_ready(Duration::from_secs(180)).await?;
                tokio::time::sleep(Duration::from_secs(3)).await; // let the HUD settle before calibrating
                st.harvest.lock().unwrap().launching = None;
                harvest_open(&st, speed, Some(id)).await
            }
            .await;
            let cancelled = {
                let mut ctl = st.harvest.lock().unwrap();
                ctl.launching = None;
                if let Err(e) = res {
                    ctl.error = Some((id, e.to_string()));
                }
                ctl.cancelled
            };
            if !cancelled {
                close_replay_process().await;
            }
        }
        st.harvest.lock().unwrap().queue.clear();
    });
    Ok(())
}

/// `--smoke-harvest`: harvest the open replay, one "all" pass with the HUD as it is (no waits).
pub fn smoke_start(st: &Arc<AppState>) {
    let st = st.clone();
    st.rt.clone().spawn(async move {
        st.vision.capture.set_fps(30.0);
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let mut meta = Map::new();
        meta.insert("appVersion".into(), json!(st.version));
        let game_id = open_replay_game_id(&st).await;
        println!("SMOKE-HARVEST gameId {game_id:?}");
        let req = HarvestRequest { game_id, speed: 8.0, meta, passes: Some(vec![rc_core::harvest::FogTeam::All]), confirm_first: true, session_id: None };
        if let Err(e) = st.harvester.run(&NoClicks, &req).await {
            println!("SMOKE-HARVEST error {e}");
        }
    });
}

/// The real Replay API without the HUD fog-team clicks (smoke runs never send input).
struct NoClicks;

impl ReplayControl for NoClicks {
    fn playback(&self) -> BoxFuture<'_, anyhow::Result<Playback>> {
        LiveReplayApi.playback()
    }
    fn set_playback<'a>(&'a self, p: &'a Value) -> BoxFuture<'a, anyhow::Result<()>> {
        LiveReplayApi.set_playback(p)
    }
    fn set_render<'a>(&'a self, r: &'a Value) -> BoxFuture<'a, anyhow::Result<()>> {
        LiveReplayApi.set_render(r)
    }
    fn seek(&self, time: f64) -> BoxFuture<'_, anyhow::Result<Playback>> {
        LiveReplayApi.seek(time)
    }
    fn allgamedata(&self) -> BoxFuture<'_, anyhow::Result<Value>> {
        LiveReplayApi.allgamedata()
    }
}

pub fn stop(st: &AppState) {
    {
        let mut ctl = st.harvest.lock().unwrap();
        ctl.cancelled = true;
        ctl.queue.clear();
    }
    st.harvester.stop();
}
