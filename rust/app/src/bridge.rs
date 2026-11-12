//! `window.rc` methods (RcApi) behind the single `rc_call` command.

use crate::state::AppState;
use serde_json::{json, Value};
use std::sync::Arc;

const NO_CAPTURE: &str = "Przechwytywanie obrazu gry nie jest jeszcze dostępne w wersji Rust.";
const NO_RECORDING: &str = "Nagrywanie meczów nie jest jeszcze dostępne w wersji Rust.";
const NO_REPLAY: &str = "Odtwarzanie nagrań nie jest jeszcze dostępne w wersji Rust.";
const NO_HARVEST: &str = "Zbieranie danych z powtórek nie jest jeszcze dostępne w wersji Rust.";

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Null)
}

pub fn call(app: &tauri::AppHandle, st: &Arc<AppState>, method: &str, args: &[Value]) -> Result<Value, String> {
    match method {
        "getStatus" => serde_json::to_value(st.status()).map_err(|e| e.to_string()),
        "updateSettings" => {
            let patch = arg(args, 0);
            let mut s = st.settings.lock().unwrap();
            let next = s.update(&patch).map_err(|e| format!("Nie udało się zapisać ustawień: {e}"))?.clone();
            drop(s);
            crate::on_settings_changed(app, st);
            Ok(next)
        }
        "listRecordings" => Ok(Value::Array(rc_core::files::list_recordings(&st.recordings_dir()))),
        "deleteRecording" => {
            let id = arg(args, 0);
            let id = id.as_str().ok_or("Brak id nagrania.")?;
            rc_core::files::delete_recording(&st.recordings_dir(), id).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "openRecordingsFolder" => {
            let dir = st.recordings_dir();
            let _ = std::fs::create_dir_all(&dir);
            std::process::Command::new("explorer.exe").arg(&dir).spawn().map_err(|e| e.to_string())?;
            Ok(json!(""))
        }
        "listReviews" => Ok(Value::Array(rc_core::files::list_reviews(&st.reviews_dir()))),
        "listReplayFiles" => {
            let recs = rc_core::files::list_recordings(&st.recordings_dir());
            Ok(Value::Array(rc_core::files::list_replay_files(&rc_core::files::replay_dirs(), &recs)))
        }
        "grabScene" | "autoCalibrate" | "setCalibration" => Err(NO_CAPTURE.into()),
        // Nothing can be open or running yet: the "stop/close" calls are no-ops like in TS when idle.
        "stopRecording" | "replayClose" | "replayPause" | "harvestStop" => Ok(Value::Null),
        "startRecording" => Err(NO_RECORDING.into()),
        "replayOpen" | "replayPlay" | "replaySeek" | "replaySpeed" => Err(NO_REPLAY.into()),
        "harvestStart" | "harvestContinue" => Err(NO_HARVEST.into()),
        other => Err(format!("Nieznana metoda: {other}")),
    }
}
