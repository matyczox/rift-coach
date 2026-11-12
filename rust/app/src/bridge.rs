//! `window.rc` methods (RcApi) behind the single `rc_call` command.

use crate::state::AppState;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Null)
}

/// Files in `dir` whose name ends with `suffix` ("" = all).
fn count(dir: &Path, suffix: &str) -> usize {
    std::fs::read_dir(dir).map(|rd| rd.filter_map(Result::ok).filter(|e| e.file_name().to_string_lossy().ends_with(suffix)).count()).unwrap_or(0)
}

/// `getLearning` (LearningStats in shared/types.ts).
pub fn learning(st: &AppState) -> Value {
    let d = &st.data_dir;
    let fog = {
        let g = st.game.lock().unwrap();
        match g.field.tracker.as_ref() {
            Some(t) => {
                let m = &t.learner.model;
                let better = if m.score.n > 0 { Some(if m.score.log_model > m.score.log_baseline { 1 } else { 0 }) } else { None };
                json!({ "samples": m.score.n, "hits": m.score.hits, "betterThanBaseline": better, "patterns": m.multipliers.len() })
            }
            None => json!({ "samples": 0, "hits": 0, "betterThanBaseline": null, "patterns": 0 }),
        }
    };
    json!({
        "fog": fog,
        "observedGames": count(&rc_core::self_train::obs_dir(d), ".json.gz"),
        "profiles": count(&d.join("riot-cache"), ""),
        "matchups": count(&d.join("matchups"), ".json"),
        "patchBriefs": count(&d.join("patch"), ".json"),
        "reviews": count(&st.reviews_dir(), ".json"),
        "recordings": rc_core::files::list_recordings(&st.recordings_dir()).len(),
    })
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
        "listRecordings" => Ok(Value::Array(crate::recorder::list_with_sizes(st))),
        "deleteRecording" => {
            let id = arg(args, 0);
            let id = id.as_str().ok_or("Brak id nagrania.")?;
            {
                let r = st.recorder.lock().unwrap();
                if r.active() && r.status.session_id.as_deref() == Some(id) {
                    return Err("Nagrywanie trwa.".into());
                }
            }
            crate::playback::close_if(st, id);
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
        "getLearning" => Ok(learning(st)),
        "listReplayFiles" => {
            let recs = rc_core::files::list_recordings(&st.recordings_dir());
            Ok(Value::Array(rc_core::files::list_replay_files(&rc_core::files::replay_dirs(), &recs)))
        }
        "grabScene" => Ok(crate::vision::grab_scene(st)),
        "autoCalibrate" => crate::vision::auto_calibrate(app, st),
        "setCalibration" => crate::vision::apply_calibration(app, st, &arg(args, 0)).map(|_| Value::Null),
        "startRecording" => {
            crate::recorder::start_recording(st);
            Ok(Value::Null)
        }
        "stopRecording" => {
            crate::recorder::stop_recording(st);
            Ok(Value::Null)
        }
        "replayOpen" => {
            let id = arg(args, 0);
            crate::playback::open(st, id.as_str().ok_or("Brak id nagrania.")?)
        }
        "replayPlay" => {
            crate::playback::play(st);
            Ok(Value::Null)
        }
        "replayPause" => {
            crate::playback::pause(st);
            Ok(Value::Null)
        }
        "replaySeek" => {
            crate::playback::seek(st, arg(args, 0).as_f64().unwrap_or(0.0));
            Ok(Value::Null)
        }
        "replaySpeed" => {
            crate::playback::speed(st, arg(args, 0).as_f64().unwrap_or(1.0));
            Ok(Value::Null)
        }
        "replayClose" => {
            crate::playback::close(st);
            Ok(Value::Null)
        }
        "harvestStart" => crate::harvest::start(st, &arg(args, 0)).map(|_| Value::Null),
        "harvestContinue" => {
            st.harvester.continue_pass();
            Ok(Value::Null)
        }
        "harvestStop" => {
            crate::harvest::stop(st);
            Ok(Value::Null)
        }
        other => Err(format!("Nieznana metoda: {other}")),
    }
}
