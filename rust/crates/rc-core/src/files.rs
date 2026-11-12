//! On-disk lists the UI shows: recordings (meta.json per session), post-game
//! reviews, downloaded .rofl replays. Same folders and rules as the TS app.

use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

fn num(v: &Value, k: &str) -> f64 {
    v.get(k).and_then(Value::as_f64).unwrap_or(0.0)
}

/// Recorder.list: every `<root>/<id>/meta.json`, newest first.
pub fn list_recordings(root: &Path) -> Vec<Value> {
    let mut out: Vec<Value> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| std::fs::read_to_string(e.path().join("meta.json")).ok())
        .filter_map(|t| serde_json::from_str(&t).ok())
        .collect();
    out.sort_by(|a, b| num(b, "startedAt").total_cmp(&num(a, "startedAt")));
    out
}

/// Recorder.delete: only removes a folder that is a recording (has meta.json).
pub fn delete_recording(root: &Path, id: &str) -> std::io::Result<()> {
    let base = Path::new(id).file_name().map(PathBuf::from).unwrap_or_default();
    if base.as_os_str().is_empty() {
        return Ok(());
    }
    let dir = root.join(base);
    if dir.join("meta.json").exists() {
        std::fs::remove_dir_all(dir)?;
    }
    Ok(())
}

/// listReviews: `<dir>/*.json`, newest `createdAt` first.
pub fn list_reviews(dir: &Path) -> Vec<Value> {
    let mut out: Vec<Value> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|t| serde_json::from_str(&t).ok())
        .collect();
    out.sort_by(|a, b| num(b, "createdAt").total_cmp(&num(a, "createdAt")));
    out
}

pub fn replay_dirs() -> Vec<PathBuf> {
    std::env::var_os("USERPROFILE").map(|h| vec![PathBuf::from(h).join("Documents").join("League of Legends").join("Replays")]).unwrap_or_default()
}

/// listReplayFiles: `*-<gameId>.rofl`, harvested = a replay recording with frames exists.
pub fn list_replay_files(dirs: &[PathBuf], recordings: &[Value]) -> Vec<Value> {
    let done: HashSet<i64> = recordings
        .iter()
        .filter(|r| r.get("source").and_then(Value::as_str) == Some("replay") && num(r, "frames") > 0.0)
        .filter_map(|r| r.get("gameId").and_then(Value::as_i64))
        .filter(|g| *g != 0)
        .collect();
    let re = regex::Regex::new(r"(?i)-(\d+)\.rofl$").unwrap();
    let mut out = Vec::new();
    for dir in dirs {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let Some(m) = re.captures(&name) else { continue };
            let Ok(game_id) = m[1].parse::<i64>() else { continue };
            let Ok(md) = e.metadata() else { continue };
            let modified = md.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0);
            out.push(serde_json::json!({
                "gameId": game_id, "name": name,
                "sizeMb": (md.len() as f64 / 1e5).round() / 10.0,
                "modifiedAt": modified, "harvested": done.contains(&game_id),
            }));
        }
    }
    out.sort_by(|a, b| num(b, "modifiedAt").total_cmp(&num(a, "modifiedAt")));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn recordings_reviews_and_replays() {
        let d = tempfile::tempdir().unwrap();
        let rec = d.path().join("recordings");
        for (id, t) in [("a", 1), ("b", 3)] {
            std::fs::create_dir_all(rec.join(id)).unwrap();
            std::fs::write(rec.join(id).join("meta.json"), json!({ "id": id, "startedAt": t, "source": "replay", "gameId": 77, "frames": 5 }).to_string()).unwrap();
        }
        std::fs::create_dir_all(rec.join("junk")).unwrap();
        let list = list_recordings(&rec);
        assert_eq!(list.iter().map(|r| r["id"].as_str().unwrap()).collect::<Vec<_>>(), ["b", "a"]);
        delete_recording(&rec, "junk").unwrap();
        assert!(rec.join("junk").exists()); // not a recording: untouched
        delete_recording(&rec, "../a").unwrap(); // basename only
        assert!(!rec.join("a").exists());

        let rev = d.path().join("reviews");
        std::fs::create_dir_all(&rev).unwrap();
        std::fs::write(rev.join("1.json"), json!({ "createdAt": 1 }).to_string()).unwrap();
        std::fs::write(rev.join("2.json"), json!({ "createdAt": 2 }).to_string()).unwrap();
        std::fs::write(rev.join("bad.json"), "{").unwrap();
        assert_eq!(list_reviews(&rev).iter().map(|r| r["createdAt"].as_i64().unwrap()).collect::<Vec<_>>(), [2, 1]);

        let rp = d.path().join("Replays");
        std::fs::create_dir_all(&rp).unwrap();
        std::fs::write(rp.join("EUN1-77.rofl"), vec![0u8; 250_000]).unwrap();
        std::fs::write(rp.join("other.txt"), "x").unwrap();
        let files = list_replay_files(&[rp], &list_recordings(&rec));
        assert_eq!(files.len(), 1);
        assert_eq!(files[0]["gameId"], 77);
        assert_eq!(files[0]["sizeMb"], 0.3);
        assert_eq!(files[0]["harvested"], true);
    }
}
