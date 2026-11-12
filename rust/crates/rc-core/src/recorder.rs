//! Recording folders on disk (`src/main/recorder.ts`, non-capture parts):
//! loading a session for replay/offline analysis and disk housekeeping.
//!
//! Session layout (one folder per recording):
//!   meta.json            RecordingSummary + calibration
//!   frames/<ms>.png      minimap crops, <ms> = offset from start
//!   scenes/<ms>.jpg      full-window JPEGs
//!   api.jsonl.gz         {o, d} Live API samples (1 Hz), o = offset ms
//!   lcu.jsonl            {o, phase} gameflow changes
//! (Listing/deleting recordings lives in `crate::files`.)

use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Loaded recording for replay and offline analysis.
#[derive(Clone, Debug)]
pub struct Session {
    pub id: String,
    pub dir: PathBuf,
    pub meta: Value,
    /// Frame offsets in ms, ascending.
    pub frames: Vec<i64>,
    pub scenes: Vec<i64>,
    /// (o, allgamedata)
    pub api: Vec<(f64, Value)>,
    pub duration_ms: f64,
}

/// JS `parseInt(name, 10)`: leading integer of a file name.
fn parse_int_prefix(s: &str) -> Option<i64> {
    let t = s.trim_start();
    let (sign, rest) = match t.strip_prefix('-') {
        Some(r) => (-1, r),
        None => (1, t.strip_prefix('+').unwrap_or(t)),
    };
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse::<i64>().ok().map(|n| sign * n)
}

fn offsets(dir: &Path) -> Vec<i64> {
    let mut v: Vec<i64> = std::fs::read_dir(dir).into_iter().flatten().flatten().filter_map(|e| parse_int_prefix(&e.file_name().to_string_lossy())).collect();
    v.sort();
    v
}

/// Gunzip that tolerates streams never closed (app crash mid-recording): keeps what decoded.
pub fn gunzip_lenient(bytes: &[u8]) -> Vec<u8> {
    let mut d = flate2::read::GzDecoder::new(bytes);
    let mut out = Vec::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        match d.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(_) => break,
        }
    }
    out
}

pub fn load_session(root: &Path, id: &str) -> anyhow::Result<Session> {
    let base = Path::new(id).file_name().map(PathBuf::from).unwrap_or_default();
    let dir = root.join(base);
    let meta: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("meta.json"))?)?;
    let frames = offsets(&dir.join("frames"));
    let scenes = offsets(&dir.join("scenes"));
    let mut api = Vec::new();
    let api_file = dir.join("api.jsonl.gz");
    if std::fs::metadata(&api_file).is_ok_and(|m| m.len() > 0) {
        let raw = std::fs::read(&api_file).unwrap_or_default();
        let text = String::from_utf8_lossy(&gunzip_lenient(&raw)).into_owned();
        for line in text.split('\n').filter(|l| !l.is_empty()) {
            if let Ok(v) = serde_json::from_str::<Value>(line) {
                let o = v.get("o").and_then(Value::as_f64).unwrap_or(0.0);
                api.push((o, v.get("d").cloned().unwrap_or(Value::Null)));
            }
        }
    }
    let duration_ms = [frames.last().map(|&x| x as f64), api.last().map(|a| a.0), scenes.last().map(|&x| x as f64)].into_iter().flatten().fold(0.0, f64::max);
    Ok(Session { id: id.to_string(), dir, meta, frames, scenes, api, duration_ms })
}

pub fn frame_path(s: &Session, o: i64) -> PathBuf {
    s.dir.join("frames").join(format!("{o}.png"))
}

#[derive(Clone, Debug)]
pub struct PruneOptions {
    pub keep_full: usize,
    pub max_age_days: f64,
    /// Session being written: never touched.
    pub active: Option<String>,
    /// Epoch ms (default: now).
    pub now_ms: Option<f64>,
}

impl Default for PruneOptions {
    fn default() -> Self {
        PruneOptions { keep_full: 5, max_age_days: 14.0, active: None, now_ms: None }
    }
}

/// Disk housekeeping: keep the newest `keep_full` sessions, delete every older
/// one entirely, and delete any session older than `max_age_days`. Folders
/// without a readable meta.json are not recordings and are left alone.
/// Returns bytes freed.
pub fn prune_recordings(root: &Path, opts: &PruneOptions) -> u64 {
    let now = opts.now_ms.unwrap_or_else(crate::jsutil::now_ms);
    let max_age_ms = opts.max_age_days * 86_400_000.0;
    let mut sessions: Vec<(PathBuf, f64)> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| opts.active.as_deref() != Some(&*e.file_name().to_string_lossy()))
        .filter_map(|e| {
            let meta: Value = serde_json::from_str(&std::fs::read_to_string(e.path().join("meta.json")).ok()?).ok()?;
            if meta.is_null() {
                return None;
            }
            Some((e.path(), meta.get("startedAt").and_then(Value::as_f64).unwrap_or(f64::NAN)))
        })
        .collect();
    sessions.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut freed = 0;
    for (idx, (dir, started)) in sessions.iter().enumerate() {
        if idx < opts.keep_full && now - started <= max_age_ms {
            continue;
        }
        freed += dir_size(dir);
        let _ = std::fs::remove_dir_all(dir);
    }
    freed
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

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use serde_json::json;
    use std::io::Write;

    const DAY: f64 = 86_400_000.0;

    fn mk(root: &Path, id: &str, started: f64, bytes: usize) {
        let d = root.join(id);
        std::fs::create_dir_all(d.join("frames")).unwrap();
        std::fs::write(d.join("meta.json"), json!({ "id": id, "startedAt": started }).to_string()).unwrap();
        std::fs::write(d.join("frames").join("0.png"), vec![0u8; bytes]).unwrap();
    }

    #[test]
    fn prune_keeps_newest_five_and_drops_old() {
        let t = tempfile::tempdir().unwrap();
        let now = 100.0 * DAY;
        for k in 0..7 {
            mk(t.path(), &format!("s{k}"), now - k as f64 * 3600_000.0, 100);
        }
        mk(t.path(), "old", now - 20.0 * DAY, 50); // would be newest-5 by count? no: 8th; age rule anyway
        mk(t.path(), "active", now - 30.0 * DAY, 10);
        std::fs::create_dir_all(t.path().join("not-a-recording")).unwrap();
        let meta_len = |id: &str| std::fs::metadata(t.path().join(id).join("meta.json")).unwrap().len();
        let expect = meta_len("s5") + meta_len("s6") + meta_len("old") + 100 + 100 + 50;
        let freed = prune_recordings(t.path(), &PruneOptions { active: Some("active".into()), now_ms: Some(now), ..Default::default() });
        assert_eq!(freed, expect);
        let mut left: Vec<String> = std::fs::read_dir(t.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, ["active", "not-a-recording", "s0", "s1", "s2", "s3", "s4"]);
        // Age rule applies inside the newest five too.
        let freed = prune_recordings(t.path(), &PruneOptions { now_ms: Some(now + 14.0 * DAY + 1.0), ..Default::default() });
        assert!(freed > 0);
        assert!(!t.path().join("s0").exists() && !t.path().join("active").exists());
        assert_eq!(prune_recordings(&t.path().join("missing"), &PruneOptions::default()), 0);
    }

    #[test]
    fn load_session_tolerates_unclosed_gzip() {
        let t = tempfile::tempdir().unwrap();
        mk(t.path(), "a", 1.0, 1);
        let d = t.path().join("a");
        std::fs::write(d.join("frames").join("2500.png"), b"x").unwrap();
        std::fs::create_dir_all(d.join("scenes")).unwrap();
        std::fs::write(d.join("scenes").join("4000.jpg"), b"x").unwrap();
        let mut gz = GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(b"{\"o\":0,\"d\":{\"x\":1}}\n{\"o\":3000,\"d\":{\"x\":2}}\nbroken\n").unwrap();
        gz.flush().unwrap(); // sync flush, never finished (crash)
        let partial = gz.get_ref().clone();
        std::fs::write(d.join("api.jsonl.gz"), partial).unwrap();
        let s = load_session(t.path(), "../a").unwrap();
        assert_eq!(s.frames, [0, 2500]);
        assert_eq!(s.scenes, [4000]);
        assert_eq!(s.api.len(), 2);
        assert_eq!(s.api[1].1["x"], 2);
        assert_eq!(s.duration_ms, 4000.0);
        assert_eq!(frame_path(&s, 2500), d.join("frames").join("2500.png"));
    }
}
