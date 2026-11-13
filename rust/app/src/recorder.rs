//! Recording a live game to disk (the writing half of `src/main/recorder.ts`; loading and
//! housekeeping live in `rc_core::recorder`). Same folder layout as the TS app:
//!   meta.json            RecordingSummary + calibration
//!   frames/<ms>.png      minimap crops, <ms> = offset from start
//!   scenes/<ms>.png      full-window frames (PNG: no JPEG encoder here; TS wrote .jpg)
//!   api.jsonl.gz         {o, d} Live API samples (1 Hz), o = offset ms
//!   lcu.jsonl            {o, phase} gameflow changes

use flate2::write::GzEncoder;
use rc_core::sources::live::{to_game_info, AllGameData};
use rc_core::types::RecordingStatus;
use serde_json::{json, Map, Value};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Game modes worth recording automatically (index.ts SUPPORTED_MODES).
pub const SUPPORTED_MODES: [&str; 6] = ["CLASSIC", "PRACTICETOOL", "ARAM", "URF", "CHERRY", "SWIFTPLAY"];

pub struct Recorder {
    pub root: PathBuf,
    pub status: RecordingStatus,
    dir: Option<PathBuf>,
    api: Option<GzEncoder<std::fs::File>>,
    last_frame_at: f64,
    last_api_at: f64,
    pub last_scene_at: f64,
    meta: Map<String, Value>,
    extra: Map<String, Value>,
}

fn now_ms() -> f64 {
    rc_core::jsutil::now_ms().floor()
}

impl Recorder {
    pub fn new(root: &Path) -> Recorder {
        let _ = std::fs::create_dir_all(root);
        Recorder { root: root.to_path_buf(), status: RecordingStatus::default(), dir: None, api: None, last_frame_at: 0.0, last_api_at: 0.0, last_scene_at: 0.0, meta: Map::new(), extra: Map::new() }
    }

    pub fn active(&self) -> bool {
        self.status.active
    }

    pub fn start(&mut self, extra: Map<String, Value>) {
        if self.status.active {
            return;
        }
        let started_at = now_ms();
        let id = rc_core::jsutil::iso_id(started_at);
        let dir = self.root.join(&id);
        if std::fs::create_dir_all(dir.join("frames")).is_err() || std::fs::create_dir_all(dir.join("scenes")).is_err() {
            log::warn!("recording: cannot create {}", dir.display());
            return;
        }
        self.api = std::fs::File::create(dir.join("api.jsonl.gz")).ok().map(|f| GzEncoder::new(f, flate2::Compression::default()));
        self.dir = Some(dir);
        self.extra = extra;
        self.meta = match json!({ "id": id, "startedAt": started_at, "durationMs": 0, "frames": 0, "bytes": 0, "gameMode": null, "champion": null, "allies": [], "enemies": [] }) {
            Value::Object(m) => m,
            _ => Map::new(),
        };
        self.status = RecordingStatus { active: true, session_id: Some(id), frames: 0, api_samples: 0, bytes: 0, started_at: Some(started_at) };
        self.last_frame_at = 0.0;
        self.last_api_at = 0.0;
        self.last_scene_at = 0.0;
        self.write_meta();
    }

    pub fn stop(&mut self) {
        if !self.status.active {
            return;
        }
        if let Some(gz) = self.api.take() {
            let _ = gz.finish();
        }
        self.write_meta();
        self.status.active = false;
        self.dir = None;
    }

    /// Minimap crop at `at` (epoch ms), throttled to `record_fps`.
    pub fn frame(&mut self, png: &[u8], at: f64, record_fps: u32) {
        let (Some(dir), Some(start)) = (&self.dir, self.status.started_at) else { return };
        if !self.status.active || at - self.last_frame_at < 1000.0 / record_fps.max(1) as f64 - 5.0 {
            return;
        }
        self.last_frame_at = at;
        let o = (at - start) as i64;
        if std::fs::write(dir.join("frames").join(format!("{o}.png")), png).is_ok() {
            self.status.frames += 1;
            self.status.bytes += png.len() as u64;
            if self.status.frames % 50 == 0 {
                self.write_meta();
            }
        }
    }

    /// Where the next full-window frame goes (None when not recording).
    pub fn scene_path(&self, at: f64) -> Option<PathBuf> {
        let (Some(dir), Some(start)) = (&self.dir, self.status.started_at) else { return None };
        Some(dir.join("scenes").join(format!("{}.png", (at - start) as i64)))
    }

    pub fn add_bytes(&mut self, n: u64) {
        self.status.bytes += n;
    }

    pub fn api_sample(&mut self, d: &AllGameData, at: f64) {
        let Some(start) = self.status.started_at else { return };
        if !self.status.active || at - self.last_api_at < 950.0 {
            return;
        }
        let Some(gz) = self.api.as_mut() else { return };
        self.last_api_at = at;
        let line = format!("{}\n", json!({ "o": (at - start) as i64, "d": d }));
        // Sync flush per line: a crash mid-game still leaves a readable log (Z_SYNC_FLUSH in TS).
        if gz.write_all(line.as_bytes()).and_then(|_| gz.flush()).is_err() {
            return;
        }
        self.status.api_samples += 1;
        self.status.bytes += line.len() as u64 / 8; // rough compressed size
        let g = to_game_info(d);
        self.meta.insert("gameMode".into(), json!(g.game_mode));
        self.meta.insert("champion".into(), json!(g.active_champion));
        self.meta.insert("allies".into(), json!(g.allies));
        self.meta.insert("enemies".into(), json!(g.enemies));
    }

    pub fn lcu_phase(&mut self, phase: &str) {
        let (Some(dir), Some(start)) = (&self.dir, self.status.started_at) else { return };
        if !self.status.active {
            return;
        }
        let line = format!("{}\n", json!({ "o": (now_ms() - start) as i64, "phase": phase }));
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("lcu.jsonl")) {
            let _ = f.write_all(line.as_bytes());
        }
    }

    fn write_meta(&mut self) {
        let Some(dir) = &self.dir else { return };
        let started = self.meta.get("startedAt").and_then(Value::as_f64).unwrap_or(0.0);
        self.meta.insert("durationMs".into(), json!((now_ms() - started) as i64));
        self.meta.insert("frames".into(), json!(self.status.frames));
        self.meta.insert("bytes".into(), json!(self.status.bytes));
        let mut all = self.meta.clone();
        for (k, v) in &self.extra {
            all.insert(k.clone(), v.clone());
        }
        let _ = std::fs::write(dir.join("meta.json"), rc_core::jsutil::json_pretty(&Value::Object(all)));
    }
}

fn dir_size(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            _ => e.metadata().map(|m| m.len()).unwrap_or(0),
        })
        .sum()
}

/// listRecordings: recordings without a byte count (harvests) get it once, stored in meta.json.
pub fn list_with_sizes(st: &crate::state::AppState) -> Vec<Value> {
    let root = st.recordings_dir();
    let active = {
        let r = st.recorder.lock().unwrap();
        r.status.session_id.clone().filter(|_| r.active())
    };
    let mut list = rc_core::files::list_recordings(&root);
    for m in list.iter_mut() {
        let Some(id) = m.get("id").and_then(Value::as_str).map(String::from) else { continue };
        if m.get("bytes").and_then(Value::as_f64).unwrap_or(0.0) > 0.0 || active.as_deref() == Some(id.as_str()) {
            continue;
        }
        let Some(base) = Path::new(&id).file_name() else { continue };
        let dir = root.join(base);
        m["bytes"] = json!(dir_size(&dir));
        let _ = std::fs::write(dir.join("meta.json"), rc_core::jsutil::json_pretty(&*m));
    }
    list
}

/// startRecording: meta gets the calibration and app version (index.ts).
pub fn start_recording(st: &crate::state::AppState) {
    let mut extra = Map::new();
    extra.insert("calibration".into(), st.settings.lock().unwrap().value().calibration.clone().unwrap_or(Value::Null));
    extra.insert("appVersion".into(), json!(st.version));
    st.recorder.lock().unwrap().start(extra);
}

pub fn stop_recording(st: &crate::state::AppState) {
    st.recorder.lock().unwrap().stop();
}

/// Disk housekeeping (30 s after start, like index.ts): keep the newest 5 sessions, none older than 14 days.
pub fn prune(st: &crate::state::AppState) {
    let active = {
        let r = st.recorder.lock().unwrap();
        r.status.session_id.clone().filter(|_| r.active())
    };
    let opts = rc_core::recorder::PruneOptions { active, ..Default::default() };
    let freed = rc_core::recorder::prune_recordings(&st.recordings_dir(), &opts);
    if freed > 0 {
        log::info!("pruned {} MB", freed / 1_000_000);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_session_layout() {
        let d = tempfile::tempdir().unwrap();
        let mut r = Recorder::new(d.path());
        let mut extra = Map::new();
        extra.insert("appVersion".into(), json!("0.1.0"));
        r.start(extra);
        assert!(r.active());
        let at = r.status.started_at.unwrap() + 1000.0;
        r.frame(b"png1", at, 30);
        r.frame(b"png2", at + 10.0, 30); // throttled (30 fps -> 28 ms gap)
        r.frame(b"png3", at + 40.0, 30);
        r.api_sample(&AllGameData::default(), at);
        r.lcu_phase("InProgress");
        r.stop();
        let id = r.status.session_id.clone().unwrap();
        let s = rc_core::recorder::load_session(d.path(), &id).unwrap();
        assert_eq!(s.frames.len(), 2);
        assert_eq!(s.api.len(), 1);
        let meta: Value = serde_json::from_str(&std::fs::read_to_string(d.path().join(&id).join("meta.json")).unwrap()).unwrap();
        assert_eq!(meta["frames"], 2);
        assert_eq!(meta["appVersion"], "0.1.0");
        assert!(d.path().join(&id).join("lcu.jsonl").exists());
        assert_eq!(rc_core::files::list_recordings(d.path()).len(), 1);
    }
}
