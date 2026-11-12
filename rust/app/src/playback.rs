//! Playing back a recorded session in the control window (replayOpen / replayPlay / ... in
//! index.ts) with `rc_core::replayer::Replayer`. A 33 ms ticker thread advances the virtual
//! clock; frames go to the Minimapa tab, API samples into the IntelTracker like live data.

use crate::state::AppState;
use rc_core::replayer::{ReplayEvent, Replayer};
use rc_core::sources::live::{normalize_champion_names, AllGameData};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::Emitter;

#[derive(Default)]
pub struct ReplayCtl {
    pub player: Option<Replayer>,
    /// Newest API sample of the open recording (status().game while a recording is open).
    pub game: Option<AllGameData>,
    ticker: bool,
}

fn clock() -> f64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0
}

/// PNG width/height from the IHDR chunk.
fn png_size(png: &[u8]) -> (u32, u32) {
    if png.len() >= 24 && &png[12..16] == b"IHDR" {
        let w = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
        let h = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
        (w, h)
    } else {
        (0, 0)
    }
}

fn handle(st: &Arc<AppState>, events: Vec<ReplayEvent>) {
    for ev in events {
        match ev {
            ReplayEvent::Frame { offset_ms, png } => {
                use base64::Engine;
                let (w, h) = png_size(&png);
                let url = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&png));
                let _ = st.app.emit_to("main", "minimapFrame", json!({ "at": crate::vision::now_ms(), "offsetMs": offset_ms, "width": w, "height": h, "dataUrl": url }));
            }
            ReplayEvent::Api { data, .. } => {
                let Ok(raw) = serde_json::from_value::<AllGameData>(data) else { continue };
                let mut g = st.game.lock().unwrap();
                let d = match &g.static_data {
                    Some(sd) => normalize_champion_names(&raw, |id| sd.name_by_id.get(id).cloned()),
                    None => raw,
                };
                let sd = g.static_data.clone();
                let intel = g.tracker.update(&d, sd.as_ref().map(|s| &s.items), sd.as_ref().map(|s| &s.champion_stats));
                g.intel = Some(intel);
                drop(g);
                st.replay.lock().unwrap().game = Some(d);
            }
            ReplayEvent::Status(_) => {}
        }
    }
}

fn with_player(st: &Arc<AppState>, f: impl FnOnce(&mut Replayer) -> Vec<ReplayEvent>) -> Option<Value> {
    let (events, status) = {
        let mut r = st.replay.lock().unwrap();
        let p = r.player.as_mut()?;
        let ev = f(p);
        (ev, serde_json::to_value(&p.status).ok())
    };
    handle(st, events);
    status
}

pub fn open(st: &Arc<AppState>, id: &str) -> Result<Value, String> {
    close(st);
    let p = Replayer::open(&st.recordings_dir(), id).map_err(|e| format!("Nie udało się otworzyć nagrania: {e}"))?;
    let start_ticker = {
        let mut r = st.replay.lock().unwrap();
        r.player = Some(p);
        !std::mem::replace(&mut r.ticker, true)
    };
    if start_ticker {
        let st2 = st.clone();
        std::thread::Builder::new()
            .name("rc-replay".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_millis(33));
                let ev = {
                    let mut r = st2.replay.lock().unwrap();
                    match r.player.as_mut() {
                        Some(p) => p.tick(clock()),
                        None => {
                            r.ticker = false;
                            return;
                        }
                    }
                };
                handle(&st2, ev);
            })
            .map_err(|e| e.to_string())?;
    }
    with_player(st, |p| p.seek(0.0)).ok_or_else(|| "Nagranie zamknięte.".into())
}

pub fn play(st: &Arc<AppState>) {
    with_player(st, |p| p.play(clock()));
}
pub fn pause(st: &Arc<AppState>) {
    with_player(st, |p| p.pause());
}
pub fn seek(st: &Arc<AppState>, ms: f64) {
    with_player(st, |p| p.seek(ms));
}
pub fn speed(st: &Arc<AppState>, s: f64) {
    with_player(st, |p| p.set_speed(s));
}

pub fn close(st: &AppState) {
    let mut r = st.replay.lock().unwrap();
    r.player = None;
    r.game = None;
}

/// The recording being deleted is open: close it first (deleteRecording).
pub fn close_if(st: &AppState, id: &str) {
    let open = st.replay.lock().unwrap().player.as_ref().is_some_and(|p| p.status.session_id == id);
    if open {
        close(st);
    }
}

pub fn is_open(st: &AppState) -> bool {
    st.replay.lock().unwrap().player.is_some()
}
