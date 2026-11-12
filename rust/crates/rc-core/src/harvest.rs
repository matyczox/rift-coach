//! Collects training/evaluation data from an open replay (`src/main/harvest.ts`).
//!
//! Which champions the minimap draws depends on the team picked in the replay HUD
//! ("Mgła wojny"), NOT on the Replay API fogOfWar flag. So the replay is played
//! once per team view (blue pass → `frames/`, red pass → `red/`, all → `all/`).
//! The HUD team can't be switched through the API and we never send input to the
//! game, so between passes the harvester waits for the user (`waiting-fog`,
//! resumed by [`Harvester::continue_pass`]).
//!
//! Output (normal recording folder, frames named by game time in ms):
//!   frames/<gameMs>.png, red/<gameMs>.png, all/<gameMs>.png
//!   api.jsonl.gz   {o: gameMs, d: allgamedata}
//!   meta.json      source: 'replay', passesDone
//!
//! Capture stays in the app: it calls [`Harvester::frame`] with every minimap PNG.
//! The Replay API is behind [`ReplayControl`] (default: [`LiveReplayApi`]).

use crate::llm::caller::BoxFuture;
use crate::sources::replay::Playback;
use anyhow::{bail, Result};
use flate2::write::GzEncoder;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FogTeam {
    Blue,
    Red,
    All,
}

impl FogTeam {
    /// Session sub-folder for each team's view (frames/ stays the default blue view).
    pub fn dir(self) -> &'static str {
        match self {
            FogTeam::Blue => "frames",
            FogTeam::Red => "red",
            FogTeam::All => "all",
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            FogTeam::Blue => "blue",
            FogTeam::Red => "red",
            FogTeam::All => "all",
        }
    }
}

/// `HarvestStatus` (shared/types.ts) as the harvester sees it (the app adds `queue`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HarvestState {
    /// idle | running | waiting-fog | done | error
    pub state: String,
    pub game_id: Option<i64>,
    pub pass: Option<FogTeam>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub need_fog: Option<FogTeam>,
    pub position_sec: f64,
    pub length_sec: f64,
    pub frames: u64,
    pub error: Option<String>,
}

impl Default for HarvestState {
    fn default() -> Self {
        HarvestState { state: "idle".into(), game_id: None, pass: None, need_fog: None, position_sec: 0.0, length_sec: 0.0, frames: 0, error: None }
    }
}

/// The Replay API calls the harvester needs (injectable for tests).
pub trait ReplayControl: Send + Sync {
    fn playback(&self) -> BoxFuture<'_, Result<Playback>>;
    fn set_playback<'a>(&'a self, p: &'a Value) -> BoxFuture<'a, Result<()>>;
    fn set_render<'a>(&'a self, r: &'a Value) -> BoxFuture<'a, Result<()>>;
    fn seek(&self, time: f64) -> BoxFuture<'_, Result<Playback>>;
    /// `https://127.0.0.1:2999/liveclientdata/allgamedata` while the replay plays.
    fn allgamedata(&self) -> BoxFuture<'_, Result<Value>>;
}

/// The real Replay API on 127.0.0.1:2999.
pub struct LiveReplayApi;

impl ReplayControl for LiveReplayApi {
    fn playback(&self) -> BoxFuture<'_, Result<Playback>> {
        Box::pin(crate::sources::replay::playback())
    }
    fn set_playback<'a>(&'a self, p: &'a Value) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move { crate::sources::replay::set_playback(p).await.map(|_| ()) })
    }
    fn set_render<'a>(&'a self, r: &'a Value) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move { crate::sources::replay::set_render(r).await.map(|_| ()) })
    }
    fn seek(&self, time: f64) -> BoxFuture<'_, Result<Playback>> {
        Box::pin(crate::sources::replay::seek(time, 8000))
    }
    fn allgamedata(&self) -> BoxFuture<'_, Result<Value>> {
        Box::pin(crate::sources::http::get_json::<Value>("https://127.0.0.1:2999/liveclientdata/allgamedata", None, 1500))
    }
}

#[derive(Clone, Debug, Default)]
pub struct HarvestRequest {
    pub game_id: Option<i64>,
    pub speed: f64,
    /// Extra meta.json fields (e.g. the .rofl name).
    pub meta: Map<String, Value>,
    /// Team views to record, in order (default blue, red).
    pub passes: Option<Vec<FogTeam>>,
    /// The HUD is already on passes[0] (no wait before the first pass).
    pub confirm_first: bool,
    /// Add passes to an existing harvest folder (e.g. a missing red pass).
    pub session_id: Option<String>,
}

struct Pb {
    at: Instant,
    time: f64,
    speed: f64,
    paused: bool,
}

struct Inner {
    status: HarvestState,
    dir: Option<PathBuf>,
    pb: Option<Pb>,
    last_frame_game_ms: i64,
    stop: bool,
}

/// One harvester per app; `run` drives a whole harvest, the other methods are
/// called from other tasks/threads (capture, UI).
pub struct Harvester {
    root: PathBuf,
    inner: Mutex<Inner>,
    resume: tokio::sync::Notify,
    /// Called on every status change (the app forwards it to the UI).
    on_status: Mutex<Option<Box<dyn Fn(&HarvestState) + Send + Sync>>>,
    /// Poll interval of the playback loop (120 ms like TS; tests shorten it).
    pub poll_ms: u64,
    /// Pause after seeking to 0 before playing (400 ms like TS).
    pub settle_ms: u64,
}

impl Harvester {
    pub fn new(root: &Path) -> Arc<Harvester> {
        Self::with_timing(root, 120, 400)
    }

    /// Custom poll / settle times (tests).
    pub fn with_timing(root: &Path, poll_ms: u64, settle_ms: u64) -> Arc<Harvester> {
        Arc::new(Harvester {
            root: root.to_path_buf(),
            inner: Mutex::new(Inner { status: HarvestState::default(), dir: None, pb: None, last_frame_game_ms: -1, stop: false }),
            resume: tokio::sync::Notify::new(),
            on_status: Mutex::new(None),
            poll_ms,
            settle_ms,
        })
    }

    pub fn set_on_status(&self, f: impl Fn(&HarvestState) + Send + Sync + 'static) {
        *self.on_status.lock().unwrap() = Some(Box::new(f));
    }

    pub fn status(&self) -> HarvestState {
        self.inner.lock().unwrap().status.clone()
    }

    pub fn running(&self) -> bool {
        matches!(self.status().state.as_str(), "running" | "waiting-fog")
    }

    fn emit(&self) {
        let s = self.status();
        if let Some(f) = self.on_status.lock().unwrap().as_ref() {
            f(&s);
        }
    }

    /// Called for every captured minimap crop while running (`at` = capture instant).
    pub fn frame(&self, png: &[u8], at: Instant) {
        let mut g = self.inner.lock().unwrap();
        let inner = &mut *g;
        let (Some(dir), Some(pb), Some(pass)) = (&inner.dir, &inner.pb, inner.status.pass) else { return };
        if inner.status.state != "running" || pb.paused {
            return;
        }
        // Interpolate game time at capture moment from the last playback poll.
        let since = if at >= pb.at { at.duration_since(pb.at).as_secs_f64() } else { -pb.at.duration_since(at).as_secs_f64() };
        let game_ms = crate::intel::js_round((pb.time + since * pb.speed) * 1000.0) as i64;
        if game_ms <= inner.last_frame_game_ms {
            return;
        }
        inner.last_frame_game_ms = game_ms;
        if std::fs::write(dir.join(pass.dir()).join(format!("{game_ms}.png")), png).is_ok() {
            inner.status.frames += 1;
        }
    }

    /// User confirmed the HUD fog team is set for the pending pass.
    pub fn continue_pass(&self) {
        self.resume.notify_waiters();
    }

    pub fn stop(&self) {
        self.inner.lock().unwrap().stop = true;
        self.resume.notify_waiters();
    }

    fn stopped(&self) -> bool {
        self.inner.lock().unwrap().stop
    }

    fn set_status(&self, f: impl FnOnce(&mut HarvestState)) {
        f(&mut self.inner.lock().unwrap().status);
    }

    /// Run a harvest to the end (or stop/error). Errors only if already running
    /// or no replay is open; later failures end in state "error".
    pub async fn run(&self, api: &dyn ReplayControl, req: &HarvestRequest) -> Result<()> {
        if self.running() {
            bail!("harvest already running");
        }
        self.inner.lock().unwrap().stop = false;
        let passes = req.passes.clone().unwrap_or_else(|| vec![FogTeam::Blue, FogTeam::Red]);
        let first = api.playback().await?; // fails if no replay is open
        let id = req.session_id.clone().unwrap_or_else(|| format!("replay-{}-{}", req.game_id.map(|g| g.to_string()).unwrap_or_else(|| "unknown".into()), crate::jsutil::iso_id(crate::jsutil::now_ms())));
        let base = Path::new(&id).file_name().map(PathBuf::from).unwrap_or_default();
        let dir = self.root.join(base);
        for p in &passes {
            std::fs::create_dir_all(dir.join(p.dir()))?;
        }
        let meta_file = dir.join("meta.json");
        let prev_meta: Map<String, Value> = std::fs::read_to_string(&meta_file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let mut meta = Map::new();
        for (k, v) in [
            ("id", json!(id)),
            ("startedAt", json!(crate::jsutil::now_ms().floor())),
            ("durationMs", json!(crate::intel::js_round(first.length * 1000.0))),
            ("frames", json!(0)),
            ("bytes", json!(0)),
            ("source", json!("replay")),
            ("gameId", json!(req.game_id)),
            ("speed", json!(req.speed)),
            ("gameMode", Value::Null),
            ("champion", Value::Null),
            ("allies", json!([])),
            ("enemies", json!([])),
            ("passesDone", json!([])),
        ] {
            meta.insert(k.into(), v);
        }
        for (k, v) in prev_meta.iter().chain(req.meta.iter()) {
            meta.insert(k.clone(), v.clone());
        }
        {
            let mut g = self.inner.lock().unwrap();
            g.dir = Some(dir.clone());
            g.status = HarvestState { state: "running".into(), game_id: req.game_id, pass: None, need_fog: None, position_sec: 0.0, length_sec: first.length, frames: 0, error: None };
        }
        let api_file = dir.join("api.jsonl.gz");
        let gz: Option<Mutex<GzEncoder<std::fs::File>>> = if api_file.exists() { None } else { std::fs::File::create(&api_file).ok().map(|f| Mutex::new(GzEncoder::new(f, flate2::Compression::default()))) };
        let meta = Mutex::new(meta);

        let result: Result<()> = async {
            for (k, &pass) in passes.iter().enumerate() {
                if self.stopped() {
                    break;
                }
                let _ = api.set_playback(&json!({ "paused": true })).await;
                if k > 0 || !req.confirm_first {
                    // Register before announcing the wait, so a quick confirm is not lost;
                    // confirms sent while not waiting are ignored (like TS).
                    let notified = self.resume.notified();
                    tokio::pin!(notified);
                    notified.as_mut().enable();
                    self.set_status(|s| {
                        s.state = "waiting-fog".into();
                        s.pass = Some(pass);
                        s.need_fog = Some(pass);
                    });
                    self.emit();
                    if !self.stopped() {
                        notified.await;
                    }
                    if self.stopped() {
                        break;
                    }
                }
                self.set_status(|s| {
                    s.state = "running".into();
                    s.pass = Some(pass);
                    s.need_fog = None;
                });
                self.inner.lock().unwrap().last_frame_game_ms = -1;
                api.set_render(&json!({ "fogOfWar": true })).await?;
                api.seek(0.0).await?;
                tokio::time::sleep(Duration::from_millis(self.settle_ms)).await;
                api.set_playback(&json!({ "speed": req.speed, "paused": false })).await?;
                let mut last_api: Option<Instant> = None;
                loop {
                    if self.stopped() {
                        break;
                    }
                    let pb = api.playback().await?;
                    {
                        let mut g = self.inner.lock().unwrap();
                        g.pb = Some(Pb { at: Instant::now(), time: pb.time, speed: pb.speed, paused: pb.paused || pb.seeking });
                        g.status.position_sec = pb.time;
                    }
                    self.emit();
                    if let Some(gz) = gz.as_ref().filter(|_| k == 0 && last_api.is_none_or(|t| t.elapsed() > Duration::from_millis(250))) {
                        last_api = Some(Instant::now());
                        // TS fires this without awaiting; a failed read (seeking) is retried next poll.
                        if let Ok(d) = api.allgamedata().await {
                            let line = format!("{}\n", serde_json::to_string(&json!({ "o": crate::intel::js_round(pb.time * 1000.0), "d": d })).unwrap_or_default());
                            let _ = gz.lock().unwrap().write_all(line.as_bytes());
                            let mut m = meta.lock().unwrap();
                            m.insert("gameMode".into(), d.pointer("/gameData/gameMode").cloned().unwrap_or(Value::Null));
                            let team = |t: &str| -> Value { json!(d.get("allPlayers").and_then(Value::as_array).map(|a| a.iter().filter(|p| p.get("team").and_then(Value::as_str) == Some(t)).filter_map(|p| p.get("championName").cloned()).collect::<Vec<_>>()).unwrap_or_default()) };
                            m.insert("allies".into(), team("ORDER"));
                            m.insert("enemies".into(), team("CHAOS"));
                        }
                    }
                    if pb.time >= pb.length - 0.5 || (pb.paused && pb.time > 5.0) {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(self.poll_ms)).await;
                }
                self.inner.lock().unwrap().pb = None;
                let _ = api.set_playback(&json!({ "paused": true })).await;
                if !self.stopped() {
                    let mut m = meta.lock().unwrap();
                    let done = m.entry("passesDone").or_insert_with(|| json!([]));
                    if let Value::Array(a) = done {
                        if !a.iter().any(|x| x == pass.as_str()) {
                            a.push(json!(pass.as_str()));
                        }
                    }
                }
            }
            Ok(())
        }
        .await;

        let stopped = self.stopped();
        self.set_status(|s| {
            match &result {
                Ok(()) => s.state = if stopped { "idle".into() } else { "done".into() },
                Err(e) => {
                    s.state = "error".into();
                    s.error = Some(format!("Error: {e}"));
                }
            }
            s.need_fog = None;
        });
        let frames = {
            let mut g = self.inner.lock().unwrap();
            g.pb = None;
            g.status.frames
        };
        if let Some(gz) = gz {
            let _ = gz.into_inner().unwrap().finish();
        }
        let mut meta = meta.into_inner().unwrap();
        let prev_frames = prev_meta.get("frames").and_then(Value::as_u64).unwrap_or(0);
        meta.insert("frames".into(), json!(prev_frames + frames));
        std::fs::write(&meta_file, crate::jsutil::json_pretty(&Value::Object(meta)))?;
        self.emit();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Fake replay: time advances 10 s per playback poll while playing.
    struct FakeReplay {
        t: Mutex<f64>,
        paused: Mutex<bool>,
        polls: AtomicU32,
        renders: Mutex<Vec<Value>>,
    }

    impl ReplayControl for FakeReplay {
        fn playback(&self) -> BoxFuture<'_, Result<Playback>> {
            self.polls.fetch_add(1, Ordering::SeqCst);
            let mut t = self.t.lock().unwrap();
            let paused = *self.paused.lock().unwrap();
            if !paused {
                *t = (*t + 10.0).min(60.0);
            }
            let pb = Playback { length: 60.0, paused, seeking: false, speed: 8.0, time: *t };
            Box::pin(async move { Ok(pb) })
        }
        fn set_playback<'a>(&'a self, p: &'a Value) -> BoxFuture<'a, Result<()>> {
            if let Some(b) = p.get("paused").and_then(Value::as_bool) {
                *self.paused.lock().unwrap() = b;
            }
            Box::pin(async { Ok(()) })
        }
        fn set_render<'a>(&'a self, r: &'a Value) -> BoxFuture<'a, Result<()>> {
            self.renders.lock().unwrap().push(r.clone());
            Box::pin(async { Ok(()) })
        }
        fn seek(&self, time: f64) -> BoxFuture<'_, Result<Playback>> {
            *self.t.lock().unwrap() = time;
            Box::pin(async move { Ok(Playback { length: 60.0, paused: true, seeking: false, speed: 8.0, time }) })
        }
        fn allgamedata(&self) -> BoxFuture<'_, Result<Value>> {
            Box::pin(async { Ok(json!({ "gameData": { "gameMode": "CLASSIC" }, "allPlayers": [{ "championName": "Ahri", "team": "ORDER" }, { "championName": "Zed", "team": "CHAOS" }] })) })
        }
    }

    fn fake() -> FakeReplay {
        FakeReplay { t: Mutex::new(0.0), paused: Mutex::new(true), polls: AtomicU32::new(0), renders: Mutex::new(vec![]) }
    }

    #[tokio::test]
    async fn two_passes_with_wait_and_meta() {
        let root = tempfile::tempdir().unwrap();
        let h = Harvester::with_timing(root.path(), 1, 1);
        let api = Arc::new(fake());
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let (s2, h2) = (seen.clone(), h.clone());
        h.set_on_status(move |s| {
            s2.lock().unwrap().push(s.state.clone());
            // Capture thread: one frame per status update while running.
            if s.state == "running" {
                h2.frame(b"png", Instant::now());
            }
        });
        let (hr, ar) = (h.clone(), api.clone());
        let task = tokio::spawn(async move {
            let req = HarvestRequest { game_id: Some(77), speed: 8.0, confirm_first: true, session_id: Some("replay-77-x".into()), ..Default::default() };
            hr.run(&*ar, &req).await
        });
        // Wait for the red pass to ask for the HUD switch, then confirm.
        while h.status().state != "waiting-fog" {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        assert_eq!(h.status().need_fog, Some(FogTeam::Red));
        assert!(h.run(&*api, &HarvestRequest::default()).await.is_err()); // already running
        h.continue_pass();
        task.await.unwrap().unwrap();
        let st = h.status();
        assert_eq!(st.state, "done");
        assert!(st.frames >= 2);
        let dir = root.path().join("replay-77-x");
        assert!(std::fs::read_dir(dir.join("frames")).unwrap().count() >= 1);
        assert!(std::fs::read_dir(dir.join("red")).unwrap().count() >= 1);
        let meta: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("meta.json")).unwrap()).unwrap();
        assert_eq!(meta["passesDone"], json!(["blue", "red"]));
        assert_eq!(meta["source"], "replay");
        assert_eq!(meta["gameId"], 77);
        assert_eq!(meta["durationMs"], 60000);
        assert_eq!(meta["gameMode"], "CLASSIC");
        assert_eq!(meta["allies"], json!(["Ahri"]));
        assert_eq!(meta["frames"], st.frames);
        let s = crate::recorder::load_session(root.path(), "replay-77-x").unwrap();
        assert!(!s.api.is_empty());
        assert_eq!(api.renders.lock().unwrap()[0], json!({ "fogOfWar": true }));
        assert!(seen.lock().unwrap().contains(&"waiting-fog".to_string()));

        // A second run adding the "all" pass keeps passesDone/frames and the api log.
        let req = HarvestRequest { game_id: Some(77), speed: 8.0, passes: Some(vec![FogTeam::All]), confirm_first: true, session_id: Some("replay-77-x".into()), ..Default::default() };
        h.run(&*api, &req).await.unwrap();
        let meta2: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("meta.json")).unwrap()).unwrap();
        assert_eq!(meta2["passesDone"], json!(["blue", "red", "all"]));
        assert!(meta2["frames"].as_u64().unwrap() > st.frames);
    }

    #[tokio::test]
    async fn stop_while_waiting_and_no_replay() {
        let root = tempfile::tempdir().unwrap();
        let h = Harvester::new(root.path());
        let api = fake();
        let hh = h.clone();
        let stopper = tokio::spawn(async move {
            while hh.status().state != "waiting-fog" {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            hh.stop();
        });
        h.run(&api, &HarvestRequest { game_id: None, speed: 4.0, ..Default::default() }).await.unwrap();
        stopper.await.unwrap();
        assert_eq!(h.status().state, "idle");
        let id = std::fs::read_dir(root.path()).unwrap().next().unwrap().unwrap().file_name().to_string_lossy().into_owned();
        assert!(id.starts_with("replay-unknown-"), "{id}");

        struct NoReplay;
        impl ReplayControl for NoReplay {
            fn playback(&self) -> BoxFuture<'_, Result<Playback>> {
                Box::pin(async { bail!("connection refused") })
            }
            fn set_playback<'a>(&'a self, _: &'a Value) -> BoxFuture<'a, Result<()>> {
                Box::pin(async { Ok(()) })
            }
            fn set_render<'a>(&'a self, _: &'a Value) -> BoxFuture<'a, Result<()>> {
                Box::pin(async { Ok(()) })
            }
            fn seek(&self, _: f64) -> BoxFuture<'_, Result<Playback>> {
                Box::pin(async { bail!("x") })
            }
            fn allgamedata(&self) -> BoxFuture<'_, Result<Value>> {
                Box::pin(async { bail!("x") })
            }
        }
        assert!(Harvester::new(root.path()).run(&NoReplay, &HarvestRequest::default()).await.is_err());
    }
}
