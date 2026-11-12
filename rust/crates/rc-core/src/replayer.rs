//! Plays a recorded session back on a virtual clock (`src/main/replay.ts`).
//! The TS version ran its own 33 ms interval and emitted events; here the app
//! owns the timer and calls [`Replayer::tick`] (~every 33 ms while playing), and
//! every call returns the events to forward — consumers treat them exactly like
//! live data.

use crate::recorder::{frame_path, load_session, Session};
use serde::Serialize;
use serde_json::Value;
use std::path::Path;

/// Index of the last element <= t in an ascending slice, or None.
pub fn last_at_or_before<T>(arr: &[T], t: f64, key: impl Fn(&T) -> f64) -> Option<usize> {
    let n = arr.partition_point(|v| key(v) <= t);
    n.checked_sub(1)
}

/// `ReplayStatus` from shared/types.ts.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayStatus {
    pub session_id: String,
    pub playing: bool,
    pub speed: f64,
    /// Milliseconds from the start of the recording.
    pub position_ms: f64,
    pub duration_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ReplayEvent {
    /// Minimap crop recorded at `offset_ms`.
    Frame { offset_ms: i64, png: Vec<u8> },
    /// Live API sample (allgamedata) recorded at `offset_ms`.
    Api { data: Value, offset_ms: f64 },
    /// Status changed (play/pause/seek/speed, and every tick).
    Status(ReplayStatus),
}

pub struct Replayer {
    pub session: Session,
    pub status: ReplayStatus,
    last_tick: f64,
    frame_idx: Option<usize>,
    api_idx: Option<usize>,
}

impl Replayer {
    pub fn open(root: &Path, id: &str) -> anyhow::Result<Replayer> {
        Ok(Self::from_session(load_session(root, id)?))
    }

    pub fn from_session(session: Session) -> Replayer {
        let status = ReplayStatus { session_id: session.id.clone(), playing: false, speed: 1.0, position_ms: 0.0, duration_ms: session.duration_ms };
        Replayer { session, status, last_tick: 0.0, frame_idx: None, api_idx: None }
    }

    /// `now_ms`: any monotonic millisecond clock (the same one passed to `tick`).
    pub fn play(&mut self, now_ms: f64) -> Vec<ReplayEvent> {
        if self.status.playing {
            return vec![];
        }
        let mut out = vec![];
        if self.status.position_ms >= self.status.duration_ms {
            out = self.seek(0.0);
        }
        self.status.playing = true;
        self.last_tick = now_ms;
        out.push(ReplayEvent::Status(self.status.clone()));
        out
    }

    pub fn pause(&mut self) -> Vec<ReplayEvent> {
        self.status.playing = false;
        vec![ReplayEvent::Status(self.status.clone())]
    }

    pub fn set_speed(&mut self, speed: f64) -> Vec<ReplayEvent> {
        self.status.speed = speed.clamp(0.25, 16.0);
        vec![ReplayEvent::Status(self.status.clone())]
    }

    pub fn seek(&mut self, position_ms: f64) -> Vec<ReplayEvent> {
        self.status.position_ms = position_ms.max(0.0).min(self.status.duration_ms);
        self.frame_idx = None;
        self.api_idx = None;
        let mut out = self.emit_up_to(self.status.position_ms);
        out.push(ReplayEvent::Status(self.status.clone()));
        out
    }

    /// Advance the virtual clock; pauses itself at the end.
    pub fn tick(&mut self, now_ms: f64) -> Vec<ReplayEvent> {
        if !self.status.playing {
            return vec![];
        }
        self.status.position_ms += (now_ms - self.last_tick) * self.status.speed;
        self.last_tick = now_ms;
        if self.status.position_ms >= self.status.duration_ms {
            self.status.position_ms = self.status.duration_ms;
            let mut out = self.emit_up_to(self.status.position_ms);
            out.extend(self.pause());
            return out;
        }
        self.emit_up_to(self.status.position_ms)
    }

    /// Emit only the newest frame/api sample at or before t (skip frames at high speed).
    fn emit_up_to(&mut self, t: f64) -> Vec<ReplayEvent> {
        let mut out = vec![];
        let s = &self.session;
        if let Some(fi) = last_at_or_before(&s.frames, t, |&v| v as f64) {
            if Some(fi) != self.frame_idx {
                self.frame_idx = Some(fi);
                let o = s.frames[fi];
                if let Ok(png) = std::fs::read(frame_path(s, o)) {
                    out.push(ReplayEvent::Frame { offset_ms: o, png });
                }
            }
        }
        if let Some(ai) = last_at_or_before(&s.api, t, |v| v.0) {
            if Some(ai) != self.api_idx {
                self.api_idx = Some(ai);
                out.push(ReplayEvent::Api { data: s.api[ai].1.clone(), offset_ms: s.api[ai].0 });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    #[test]
    fn binary_search() {
        let a = [0, 100, 200, 200, 300];
        assert_eq!(last_at_or_before(&a, -1.0, |&v| v as f64), None);
        assert_eq!(last_at_or_before(&a, 0.0, |&v| v as f64), Some(0));
        assert_eq!(last_at_or_before(&a, 250.0, |&v| v as f64), Some(3));
        assert_eq!(last_at_or_before(&a, 999.0, |&v| v as f64), Some(4));
        assert_eq!(last_at_or_before::<i64>(&[], 5.0, |&v| v as f64), None);
    }

    #[test]
    fn plays_on_virtual_clock() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("s");
        std::fs::create_dir_all(dir.join("frames")).unwrap();
        for o in [0, 500, 1000] {
            std::fs::write(dir.join("frames").join(format!("{o}.png")), [o as u8]).unwrap();
        }
        let s = Session { id: "s".into(), dir: PathBuf::from(&dir), meta: json!({}), frames: vec![0, 500, 1000], scenes: vec![], api: vec![(0.0, json!({ "a": 0 })), (800.0, json!({ "a": 1 }))], duration_ms: 1000.0 };
        let mut r = Replayer::from_session(s);
        let ev = r.play(10_000.0);
        assert!(matches!(ev.last(), Some(ReplayEvent::Status(st)) if st.playing));
        let ev = r.tick(10_100.0);
        assert_eq!(ev.len(), 2); // frame 0 + api 0
        assert!(r.tick(10_200.0).is_empty()); // nothing new
        r.set_speed(100.0);
        assert_eq!(r.status.speed, 16.0);
        let ev = r.tick(10_250.0); // +800 ms → 1000 = end
        assert!(matches!(ev[0], ReplayEvent::Frame { offset_ms: 1000, .. }));
        assert!(matches!(&ev[1], ReplayEvent::Api { offset_ms, .. } if *offset_ms == 800.0));
        assert!(!r.status.playing);
        assert_eq!(r.status.position_ms, 1000.0);
        let ev = r.seek(600.0);
        assert!(matches!(ev[0], ReplayEvent::Frame { offset_ms: 500, ref png } if png == &vec![244u8]));
        // play at the end restarts from 0
        r.seek(1000.0);
        let ev = r.play(0.0);
        assert!(matches!(ev[0], ReplayEvent::Frame { offset_ms: 0, .. }));
        assert_eq!(serde_json::to_value(&r.status).unwrap()["sessionId"], "s");
    }
}
