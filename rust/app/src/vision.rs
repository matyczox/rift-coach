//! Minimap vision in the app, like `src/main/index.ts`: game-window capture -> minimap preview
//! for the control window, VisionService (detector / fog / camps / health bars) -> fog tracker,
//! ghosts and alerts for the HUD, health bars into the IntelTracker.
//!
//! Threads: `rc-capture` (rc-vision, newest frame only), `rc-frames` (this file: hands the
//! newest frame to the preview and to the VisionService), `rc-vision` + its 2-thread detector
//! pool (rc-vision, <= 5 fps, newest frame wins) and `rc-vision-results` (tracker update).
//! Nothing here ever focuses the game or sends it input: capture only reads its surface.

use crate::game::Game;
use crate::state::AppState;
use rc_core::intel::game_intel::{GameIntel, PlayerIntel};
use rc_vision::calibration::find_minimap;
use rc_vision::camps::{CampEventKind, CampKind, CampSide, CampState, CampStatus, CampTracker, CAMPS};
use rc_vision::capture::{crop_rect, CaptureState, CaptureThread, GAME_WINDOW_TITLE};
use rc_vision::healthbars::{match_bars_to_champions, ChampionCandidate, HealthBar, MatchOptions};
use rc_vision::image::encode_png;
use rc_vision::tracker::alerts::tracker_alerts;
use rc_vision::tracker::{Role, TrackState};
use rc_vision::{Calibration, Detection, EnemyInput, EnemyTracker, FrameInput, GridTerrain, Hint, Learner, Roster, RosterEntry, Team, Vision, VisionParams, VisionResult, VisionService};
use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{Emitter, Manager};

/// The fog model learns only from real Summoner's Rift games.
const LEARN_MODES: [&str; 2] = ["CLASSIC", "SWIFTPLAY"];
/// Vision worker rate (TS VisionService: ~5 fps, newest frame wins).
const VISION_FPS: f64 = 5.0;
/// Control-window preview: <= ~10 fps.
const PREVIEW_GAP_MS: u64 = 90;

pub fn now_ms() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as f64).unwrap_or(0.0)
}

/// `AlertState` from shared/intel.ts (all kinds, not only the tracker's).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Alert {
    pub kind: String,
    pub text: String,
    pub confidence: f64,
    /// Wall-clock ms.
    pub expires_at: f64,
}

impl Alert {
    pub fn new(kind: &str, text: impl Into<String>, confidence: f64, expires_at: f64) -> Alert {
        Alert { kind: kind.into(), text: text.into(), confidence, expires_at }
    }
}

/// `MinimapGhost` from shared/intel.ts.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MinimapGhost {
    pub champion: String,
    pub x: f64,
    pub y: f64,
    pub radius: f64,
    pub age: f64,
    pub confidence: f64,
    pub icon_url: String,
}

/// A champion seen on the minimap this frame, with a smoothed velocity.
#[derive(Clone, Debug)]
pub struct SeenUnit {
    pub champion: String,
    pub x: f64,
    pub y: f64,
    pub vx: f64,
    pub vy: f64,
}

/// Bump when the detector / tracker changed so much that what was learned before is wrong.
/// 2: minimap icons from the game (CommunityDragon circle_0) — before that some champions (Briar)
///    were not detected and the fog model learned from bad sightings.
const FOG_MODEL_VERSION: &str = "2";

/// Once per FOG_MODEL_VERSION: move the learned fog model and its observations to
/// model-old-v<previous>/ and start learning from scratch (also on other people's installs).
fn reset_old_fog_model(dir: &Path) {
    let marker = dir.join("version.txt");
    let current = std::fs::read_to_string(&marker).unwrap_or_default();
    if current.trim() == FOG_MODEL_VERSION {
        return;
    }
    let _ = std::fs::create_dir_all(dir);
    let model = dir.join("fog-model.json");
    let obs = dir.join("observations");
    let had = model.exists() || std::fs::read_dir(&obs).map(|mut d| d.next().is_some()).unwrap_or(false);
    if had {
        let old = dir.with_file_name(format!("model-old-v{}", if current.trim().is_empty() { "1" } else { current.trim() }));
        let _ = std::fs::create_dir_all(&old);
        if model.exists() {
            let _ = std::fs::rename(&model, old.join("fog-model.json"));
        }
        if let Ok(rd) = std::fs::read_dir(&obs) {
            let _ = std::fs::create_dir_all(old.join("observations"));
            for e in rd.flatten() {
                let _ = std::fs::rename(e.path(), old.join("observations").join(e.file_name()));
            }
        }
        log::info!("fog model reset (v{} -> v{FOG_MODEL_VERSION}), old one in {}", current.trim(), old.display());
    }
    let _ = std::fs::write(&marker, FOG_MODEL_VERSION);
}

/// Short top-right notification (enemy back, Baron / Elder taken). Flies away when it expires.
#[derive(Clone, Debug)]
pub struct Toast {
    pub id: u64,
    /// recall
    pub kind: String,
    /// ally | enemy
    pub team: String,
    pub champion: Option<String>,
    pub text: String,
    /// Date.now() ms
    pub expires_at: f64,
}

/// Per-game vision state (lives in `Game`, under the same lock as the intel).
pub struct Field {
    pub toasts: Vec<Toast>,
    /// Unguarded lane pushes announced: ("champion@lane", Date.now() ms).
    pub push_seen: Vec<(String, f64)>,
    /// Lane opponent walking back to lane: (champion, ETA as Date.now() ms).
    pub returns: Vec<(String, f64)>,
    /// Enemies dead in the last frame (respawn detection).
    pub prev_dead: Vec<String>,
    pub tracker: Option<EnemyTracker>,
    pub camps: CampTracker,
    pub camp_status: Vec<CampStatus>,
    pub ghosts: Vec<MinimapGhost>,
    pub alerts: Vec<Alert>,
    pub my_pos: Option<(f64, f64)>,
    pub allies: Vec<SeenUnit>,
    pub visible_enemies: Vec<SeenUnit>,
    /// Fog mask (P(visible) per cell, `mask_grid`², row-major).
    pub mask: Option<Vec<f32>>,
    pub mask_grid: usize,
    /// Champions + my team of the roster given to the VisionService ('' = none).
    pub roster_key: String,
    /// Roster loaded into the VisionService (frames may be pushed).
    pub roster_ready: bool,
    /// Detector has champion templates (Summoner's Rift only).
    pub detect: bool,
    /// Summoner's Rift (map 11): the only map the detector / tracker know.
    pub map11: bool,
}

impl Field {
    pub fn new(data_dir: &Path) -> Field {
        reset_old_fog_model(&data_dir.join("model"));
        let learner = Learner::new(Some(data_dir.join("model").join("fog-model.json")));
        let tracker = std::panic::catch_unwind(|| EnemyTracker::new(Arc::new(GridTerrain::from_asset(128)), learner, 400)).map_err(|_| log::warn!("terrain: no walkable grid")).ok();
        Field {
            toasts: Vec::new(),
            returns: Vec::new(),
            push_seen: Vec::new(),
            prev_dead: Vec::new(),
            tracker,
            camps: CampTracker::default(),
            camp_status: Vec::new(),
            ghosts: Vec::new(),
            alerts: Vec::new(),
            my_pos: None,
            allies: Vec::new(),
            visible_enemies: Vec::new(),
            mask: None,
            mask_grid: 64,
            roster_key: String::new(),
            roster_ready: false,
            detect: false,
            map11: false,
        }
    }

    /// P(visible) at normalized minimap coords (same cell lookup as index.ts `visibleFn`).
    pub fn visible_at(&self, x: f64, y: f64) -> Option<f64> {
        self.mask.as_deref().map(|m| mask_at(m, self.mask_grid, x, y))
    }

    /// Live API left (game over / loading): forget per-game state, keep what was learned.
    pub fn on_live_lost(&mut self) {
        self.alerts.clear();
        self.toasts.clear();
        self.returns.clear();
        self.push_seen.clear();
        self.prev_dead.clear();
        self.ghosts.clear();
        self.my_pos = None;
        self.roster_key.clear();
        self.roster_ready = false;
        self.allies.clear();
        self.visible_enemies.clear();
        self.mask = None;
        self.save_learner();
    }

    pub fn save_learner(&mut self) {
        if let Some(t) = self.tracker.as_mut() {
            if let Err(e) = t.learner.save() {
                log::warn!("fog model save {e}");
            }
        }
    }

    /// Positions of enemies the tracker currently sees (detector hints).
    fn hints(&self) -> Vec<Hint> {
        self.tracker.as_ref().map(|t| t.all().into_iter().filter(|t| t.state == TrackState::Visible).filter_map(|t| t.last.map(|l| Hint { x: l.x, y: l.y, team: t.team })).collect()).unwrap_or_default()
    }
}

fn mask_at(m: &[f32], grid: usize, x: f64, y: f64) -> f64 {
    let g = grid as f64;
    let cx = ((x * g).floor()).clamp(0.0, g - 1.0) as usize;
    let cy = ((y * g).floor()).clamp(0.0, g - 1.0) as usize;
    m.get(cy * grid + cx).copied().unwrap_or(0.0) as f64
}

/// Capture + vision worker, shared by the whole app.
pub struct VisionHub {
    pub capture: CaptureThread,
    pub service: VisionService,
    calibration: Mutex<Option<Calibration>>,
    /// Show the minimap outline on the HUD until then (after a calibration).
    pub frame_until: Mutex<Option<Instant>>,
    rx: Mutex<Option<mpsc::Receiver<VisionResult>>>,
}

impl VisionHub {
    pub fn new(fps: f64, calibration: Option<Calibration>) -> VisionHub {
        let (tx, rx) = mpsc::channel();
        let vision = Vision::new(Roster { champions: Vec::new(), own_team: Team::Order, relative_colors: true }, calibration, VisionParams::default());
        VisionHub {
            capture: CaptureThread::spawn(GAME_WINDOW_TITLE, fps),
            service: VisionService::start(vision, VISION_FPS, tx),
            calibration: Mutex::new(calibration),
            frame_until: Mutex::new(None),
            rx: Mutex::new(Some(rx)),
        }
    }

    pub fn calibration(&self) -> Option<Calibration> {
        *self.calibration.lock().unwrap()
    }

    fn set_calibration(&self, c: Option<Calibration>) {
        let mut cur = self.calibration.lock().unwrap();
        if *cur != c {
            *cur = c;
            self.service.set_calibration(c);
        }
    }

    pub fn show_frame(&self) -> bool {
        self.frame_until.lock().unwrap().is_some_and(|t| Instant::now() < t)
    }

    /// Capture status in the shape the UI expects.
    pub fn status(&self) -> rc_core::types::CaptureStatus {
        use rc_core::types::SourceState;
        let s = self.capture.status();
        rc_core::types::CaptureStatus {
            state: match s.state {
                CaptureState::Off => SourceState::Off,
                CaptureState::Waiting => SourceState::Waiting,
                CaptureState::Connected => SourceState::Connected,
                CaptureState::Error => SourceState::Error,
            },
            window_width: s.window_width.map(|v| v as u32),
            window_height: s.window_height.map(|v| v as u32),
            fps: s.fps,
            last_frame_at: s.last_frame_at.map(|v| v as f64),
            // "window not found" while waiting is normal, not an error for the UI.
            error: if s.state == CaptureState::Error { s.error } else { None },
        }
    }

    /// Newest captured frame if it is recent (<= 3 s) and the game window is still captured.
    fn fresh_frame(&self) -> Option<rc_vision::capture::CapturedFrame> {
        if self.capture.status().state != CaptureState::Connected {
            return None;
        }
        let (_, f) = self.capture.latest()?;
        (now_ms() - f.at_ms as f64 <= 3000.0).then_some(f)
    }
}

pub fn calibration_from_settings(v: Option<&Value>) -> Option<Calibration> {
    v.and_then(|c| serde_json::from_value::<Calibration>(c.clone()).ok()).filter(|c| c.window_width > 0 && c.window_height > 0 && c.rect.width > 8)
}

/// Settings changed: capture rate and calibration follow (index.ts `capture.configure`).
pub fn sync_settings(st: &AppState) {
    let (fps, cal) = {
        let s = st.settings.lock().unwrap();
        (s.value().minimap_fps, calibration_from_settings(s.value().calibration.as_ref()))
    };
    st.vision.capture.set_fps(fps.max(1) as f64);
    st.vision.set_calibration(cal);
}

// ---------- calibration (RcApi) ----------

fn data_url_png(png: &[u8]) -> String {
    use base64::Engine;
    format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png))
}

/// `grabScene`: one full game-window frame as a PNG data URL (null without a game window).
pub fn grab_scene(st: &AppState) -> Value {
    match st.vision.fresh_frame() {
        Some(f) => json!({ "dataUrl": data_url_png(&encode_png(&f.image)), "width": f.image.width, "height": f.image.height }),
        None => Value::Null,
    }
}

/// `autoCalibrate`: find the minimap square in the newest frame and save it.
pub fn auto_calibrate(app: &tauri::AppHandle, st: &Arc<AppState>) -> Result<Value, String> {
    let Some(f) = st.vision.fresh_frame() else { return Ok(Value::Null) };
    let Some(found) = find_minimap(&f.image.data, f.image.width, f.image.height) else { return Ok(Value::Null) };
    let c = json!({ "rect": found.rect, "windowWidth": f.image.width, "windowHeight": f.image.height, "source": "auto", "updatedAt": now_ms() as u64 });
    apply_calibration(app, st, &c)?;
    Ok(c)
}

/// `setCalibration` / after autoCalibrate: save, crop with it, flash the outline on the HUD.
pub fn apply_calibration(app: &tauri::AppHandle, st: &Arc<AppState>, c: &Value) -> Result<(), String> {
    if calibration_from_settings(Some(c)).is_none() {
        return Err("Nieprawidłowa kalibracja.".into());
    }
    st.settings.lock().unwrap().update(&json!({ "calibration": c })).map_err(|e| format!("Nie udało się zapisać ustawień: {e}"))?;
    sync_settings(st);
    *st.vision.frame_until.lock().unwrap() = Some(Instant::now() + Duration::from_millis(4000));
    crate::hud::place(app, st);
    Ok(())
}

// ---------- roster ----------

/// Game setting "relative team colours" (enemies always red on the minimap). Default on.
pub fn read_relative_colors() -> bool {
    for p in ["C:/Riot Games/League of Legends/Config/game.cfg", "D:/Riot Games/League of Legends/Config/game.cfg"] {
        let Ok(text) = std::fs::read_to_string(p) else { continue };
        for line in text.lines() {
            if let Some(v) = line.strip_prefix("RelativeTeamColors=") {
                if let Some(c) = v.chars().next().filter(char::is_ascii_digit) {
                    return c == '1';
                }
            }
        }
    }
    true
}

/// The icon the GAME draws on the minimap (CommunityDragon `<champ>_circle_0.png`; the minimap ignores
/// skins). For most champions it equals the Data Dragon square, but Briar's square matched her minimap
/// icon at 0.25 (others 0.99), so she was never detected. Same lookup as `src/main/vision/icons.ts`.
async fn load_game_circle_icon(dir: &Path, dd_id: &str) -> Option<rc_vision::Rgba> {
    let id = dd_id.to_lowercase();
    let file = dir.join(format!("cd-{id}-0.png"));
    let miss = dir.join(format!("cd-{id}-0.png.missing"));
    if !file.exists() && !miss.exists() {
        let _ = std::fs::create_dir_all(dir);
        let url = format!("https://raw.communitydragon.org/latest/game/assets/characters/{id}/hud/{id}_circle_0.png");
        let res = rc_core::intel::ddragon::web_client().get(url).timeout(Duration::from_secs(10)).send().await.ok()?;
        if res.status().as_u16() == 404 {
            let _ = std::fs::write(&miss, b"");
            return None;
        }
        if !res.status().is_success() {
            return None;
        }
        std::fs::write(&file, res.bytes().await.ok()?).ok()?;
    }
    if file.exists() { rc_vision::image::load_png(&file).ok() } else { None }
}

async fn load_icon(dir: &Path, version: &str, dd_id: &str) -> anyhow::Result<rc_vision::Rgba> {
    if let Some(icon) = load_game_circle_icon(dir, dd_id).await {
        return Ok(icon);
    }
    let file = rc_vision::icons::icon_path(dir, version, dd_id);
    if !file.exists() {
        std::fs::create_dir_all(dir)?;
        let res = rc_core::intel::ddragon::web_client().get(rc_vision::icons::icon_url(version, dd_id)).timeout(Duration::from_secs(10)).send().await?;
        if !res.status().is_success() {
            anyhow::bail!("icon {dd_id}: HTTP {}", res.status());
        }
        std::fs::write(&file, res.bytes().await?)?;
    }
    Ok(rc_vision::image::load_png(&file)?)
}

fn icons_dir(st: &AppState) -> PathBuf {
    st.data_dir.join("ddragon").join("icons")
}

/// Give the vision worker this game's 10 champions (icons from Data Dragon) — `syncVisionRoster`.
pub fn sync_roster(rt: &tokio::runtime::Handle, st: &Arc<AppState>, g: &mut Game, map: i64) {
    let (Some(intel), Some(sd)) = (g.intel.as_ref(), g.static_data.clone()) else { return };
    let my_team = intel.me.as_ref().map(|m| m.team.clone()).unwrap_or_else(|| "ORDER".into());
    let key = format!("{}{my_team}|{map}", intel.players.iter().map(|p| p.champion.as_str()).collect::<Vec<_>>().join("|"));
    if key == g.field.roster_key {
        return;
    }
    g.field.roster_key = key.clone();
    g.field.roster_ready = false;
    g.field.map11 = map == 11;
    let players: Vec<(String, String)> = intel.players.iter().map(|p| (p.champion.clone(), p.team.clone())).collect();
    let st = st.clone();
    rt.spawn(async move {
        let dir = icons_dir(&st);
        let mut champions = Vec::new();
        let mut err = None;
        // Other maps: no templates (the detector only knows Summoner's Rift); health bars still work.
        if map == 11 {
            for (champ, team) in &players {
                let dd = sd.champion_ids.get(champ).cloned().unwrap_or_else(|| champ.clone());
                match load_icon(&dir, &sd.version, &dd).await {
                    Ok(icon) => champions.push(RosterEntry { champion: champ.clone(), team: Team::parse(team).unwrap_or(Team::Order), icon }),
                    Err(e) => err = Some(e),
                }
            }
        }
        let mut g = st.game.lock().unwrap();
        if g.field.roster_key != key {
            return; // a newer roster is on its way
        }
        if let Some(e) = err {
            log::warn!("vision roster {e}");
            g.field.roster_key.clear(); // retry on the next Live API sample
            return;
        }
        g.field.detect = !champions.is_empty();
        st.vision.service.set_roster(Roster { champions, own_team: Team::parse(&my_team).unwrap_or(Team::Order), relative_colors: read_relative_colors() });
        g.field.camps.reset();
        if let Some(t) = g.field.tracker.as_mut() {
            t.reset();
        }
        g.field.roster_ready = true;
        log::info!("vision roster ready (map {map}, detector {})", g.field.detect);
    });
}

/// Per Live API sample (onGameData): learning switch + roster.
pub fn on_game_data(rt: &tokio::runtime::Handle, st: &Arc<AppState>, g: &mut Game, game_mode: &str, map: i64) {
    if let Some(t) = g.field.tracker.as_mut() {
        // Never from replays (also not with --allow-replay): only real games teach the fog model.
        t.learning_enabled = LEARN_MODES.contains(&game_mode) && map == 11 && !st.replay_open();
    }
    sync_roster(rt, st, g, map);
}

// ---------- frames ----------

fn control_visible(app: &tauri::AppHandle) -> bool {
    app.get_webview_window("main").is_some_and(|w| w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(true))
}

/// `rc-frames` + `rc-vision-results` threads.
pub fn spawn_threads(app: &tauri::AppHandle, st: &Arc<AppState>) {
    let rx = st.vision.rx.lock().unwrap().take().expect("vision threads started twice");
    let (app2, st2) = (app.clone(), st.clone());
    std::thread::Builder::new()
        .name("rc-frames".into())
        .spawn(move || frame_loop(&app2, &st2))
        .expect("spawn frame thread");
    let (app3, st3) = (app.clone(), st.clone());
    std::thread::Builder::new()
        .name("rc-vision-results".into())
        .spawn(move || {
            for r in rx {
                on_vision(&app3, &st3, r);
            }
        })
        .expect("spawn vision result thread");
}

/// The user placed the minimap by hand (calibration.source === 'manual').
fn manual_calibration(st: &AppState) -> bool {
    st.settings.lock().unwrap().value().calibration.as_ref().and_then(|c| c.get("source")).and_then(Value::as_str) == Some("manual")
}

fn frame_loop(app: &tauri::AppHandle, st: &Arc<AppState>) {
    let mut last_seq = 0u64;
    let mut last_preview = 0u64;
    let mut preview_on = false;
    let mut preview_checked = Instant::now() - Duration::from_secs(1);
    let mut was_connected = false;
    let mut auto_cal_at: Option<Instant> = None;
    loop {
        std::thread::sleep(Duration::from_millis(15));
        // Game window found: find the minimap by itself at every game start (HUD scale /
        // resolution may have changed), unless the user placed it by hand.
        let connected = st.vision.capture.status().state == CaptureState::Connected;
        if connected && !was_connected && !manual_calibration(st) {
            auto_cal_at = Some(Instant::now() + Duration::from_millis(1500));
        }
        was_connected = connected;
        if auto_cal_at.is_some_and(|t| Instant::now() >= t) {
            auto_cal_at = None;
            if let Err(e) = auto_calibrate(app, st) {
                log::warn!("auto calibration {e}");
            }
        }
        let Some((seq, f)) = st.vision.capture.latest() else { continue };
        if seq == last_seq {
            continue;
        }
        last_seq = seq;
        let cal = st.vision.calibration();
        // Minimap preview for the control window (Minimapa tab), only while it is shown.
        if preview_checked.elapsed() >= Duration::from_millis(500) {
            preview_checked = Instant::now();
            preview_on = control_visible(app);
        }
        // Recording / replay harvest need every crop as PNG; the preview reuses the same encode.
        let recording = st.recorder.lock().unwrap().active();
        let harvesting = st.harvester.running();
        // A recording open in the app's player owns the preview (index.ts: `if (!replay) sendFrame`).
        let want_preview = preview_on && !crate::playback::is_open(st) && f.at_ms.saturating_sub(last_preview) >= PREVIEW_GAP_MS;
        if let Some(c) = cal.filter(|_| want_preview || recording || harvesting) {
            if let Some(crop) = crop_rect(&f.image, c.rect_for(f.image.width as i32, f.image.height as i32)) {
                let png = encode_png(&crop);
                if harvesting {
                    let age = Duration::from_millis((now_ms() as u64).saturating_sub(f.at_ms));
                    st.harvester.frame(&png, Instant::now().checked_sub(age).unwrap_or_else(Instant::now));
                }
                if recording {
                    let fps = st.settings.lock().unwrap().value().record_fps;
                    st.recorder.lock().unwrap().frame(&png, f.at_ms as f64, fps);
                }
                if want_preview {
                    last_preview = f.at_ms;
                    let frame = json!({ "at": f.at_ms, "width": crop.width, "height": crop.height, "dataUrl": data_url_png(&png) });
                    let _ = app.emit_to("main", "minimapFrame", frame);
                }
            }
        }
        // Full-window frame every `sceneEverySec` while recording (encoded off this thread).
        if recording {
            let every = st.settings.lock().unwrap().value().scene_every_sec as f64 * 1000.0;
            let path = {
                let mut r = st.recorder.lock().unwrap();
                let due = every > 0.0 && f.at_ms as f64 - r.last_scene_at >= every;
                if due {
                    r.last_scene_at = f.at_ms as f64;
                }
                due.then(|| r.scene_path(f.at_ms as f64)).flatten()
            };
            if let Some(path) = path {
                let (img, st2) = (f.image.clone(), st.clone());
                std::thread::spawn(move || {
                    // Half resolution: a full 1440p PNG is ~5 MB (TS wrote ~0.3 MB JPEGs).
                    let png = encode_png(&rc_vision::image::resize(&img, img.width / 2, img.height / 2));
                    if std::fs::write(&path, &png).is_ok() {
                        st2.recorder.lock().unwrap().add_bytes(png.len() as u64);
                    }
                });
            }
        }
        // Live game only: detect + track (health bars on every map).
        let hints = {
            let g = st.game.lock().unwrap();
            let ok = g.intel.is_some() && g.field.roster_ready && !st.replay_blocks();
            ok.then(|| g.field.hints())
        };
        if let Some(h) = hints {
            st.vision.service.push(f.image.clone(), f.at_ms, Some(h));
        }
    }
}

// ---------- results ----------

/// Seconds for an enemy leaving their fountain to reach the lane: to me if I am on the map (not in
/// my base), else to their lane's middle; straight line x1.15 for pathing, at their move speed,
/// minus the Homeguard head start.
pub fn return_eta(p: &PlayerIntel, my_pos: Option<(f64, f64)>, game_t: f64) -> f64 {
    use rc_vision::tracker::{enemy::homeguard_extra_secs, predictor::fountain_of, MAP_UNITS};
    let team = Team::parse(&p.team).unwrap_or(Team::Chaos);
    let (fx, fy) = fountain_of(team);
    let lane = match p.role.as_str() {
        "TOP" => (0.13, 0.18),
        "BOTTOM" | "UTILITY" => (0.82, 0.87),
        _ => (0.5, 0.5),
    };
    let (bx, by) = fountain_of(team.other());
    let target = my_pos.filter(|&(x, y)| (x - bx).hypot(y - by) > 0.2).unwrap_or(lane);
    let dist = (target.0 - fx).hypot(target.1 - fy) * MAP_UNITS * 1.15;
    let speed = if p.move_speed > 0.0 { p.move_speed } else { 345.0 };
    (dist / speed - homeguard_extra_secs(30.0, game_t)).max(5.0)
}

/// Visible enemies standing in a lane on OUR half of the map with no ally (me included) within ~1/4
/// of the map: (champion, "top" | "mid" | "bot"). Coordinates are mirrored for red side so that our
/// base is always bottom-left (reflected across the diagonal, which keeps top lane = top).
pub fn unguarded_pushes(enemies: &[SeenUnit], allies: &[SeenUnit], my_team: Team) -> Vec<(String, &'static str)> {
    let flip = my_team == Team::Chaos;
    let mut out = Vec::new();
    for e in enemies {
        let (x, y) = if flip { (e.y, e.x) } else { (e.x, e.y) };
        let lane = if x < 0.18 && y > 0.45 && y < 0.88 {
            "top"
        } else if y > 0.82 && x > 0.12 && x < 0.55 {
            "bot"
        } else if (x - (1.0 - y)).abs() < 0.08 && x > 0.2 && x < 0.45 {
            "mid"
        } else {
            continue;
        };
        if allies.iter().any(|a| (a.x - e.x).hypot(a.y - e.y) < 0.25) {
            continue;
        }
        out.push((e.champion.clone(), lane));
    }
    out
}

pub fn lane_opponent(intel: &GameIntel) -> Option<&PlayerIntel> {
    let me = intel.me.as_ref()?;
    if me.role == "UNKNOWN" {
        return None;
    }
    intel.players.iter().find(|p| !p.is_ally && p.role == me.role)
}

fn side_str(s: CampSide) -> &'static str {
    match s {
        CampSide::Order => "ORDER",
        CampSide::Chaos => "CHAOS",
        CampSide::Neutral => "neutral",
    }
}

pub fn camp_label(id: &str) -> String {
    let base = id.strip_prefix("blue_").or_else(|| id.strip_prefix("red_")).unwrap_or(id);
    match base {
        "gromp" => "gromp",
        "blue" => "niebieskim buffie",
        "wolves" => "wilkach",
        "raptors" => "raptorach",
        "red" => "czerwonym buffie",
        "krugs" => "krugach",
        _ => return id.to_string(),
    }
    .to_string()
}

pub fn champion_icon(g: &Game, champion: &str) -> String {
    let Some(sd) = g.static_data.as_ref() else { return String::new() };
    let id = sd.champion_ids.get(champion).cloned().unwrap_or_else(|| champion.chars().filter(|c| c.is_ascii_alphabetic()).collect());
    rc_core::intel::ddragon::champion_icon_url(&sd.version, &id)
}

/// Enemy health bars -> IntelTracker (capture 'hp' in index.ts).
/// Allies' health bars -> their HP for the fight estimate (my own HP comes from the Live API).
/// Ally portraits above the minimap -> HP and mana of every ally. Slots follow the team order of the
/// Live API with me left out. (Read every HP tick; more reliable than bars over champions.)
fn apply_team_frames(g: &mut Game, frames: &[rc_vision::teamframes::AllyFrame]) {
    let Game { intel, tracker, .. } = g;
    let Some(intel) = intel.as_ref() else { return };
    let allies: Vec<&PlayerIntel> = intel.players.iter().filter(|p| p.is_ally && !p.is_me).collect();
    if frames.len() != 4 || allies.len() != 4 {
        return;
    }
    for f in frames {
        if let Some(p) = allies.get(f.slot) {
            if !p.is_dead {
                tracker.set_hp(&p.champion, f.hp, intel.game_time);
                tracker.set_mana(&p.champion, f.mana, intel.game_time);
                tracker.set_shield(&p.champion, f.shield, intel.game_time);
            }
        }
    }
}

fn apply_ally_hp(g: &mut Game, bars: &[HealthBar]) {
    let Game { intel, tracker, .. } = g;
    let Some(intel) = intel.as_ref() else { return };
    let cands: Vec<ChampionCandidate> = intel.players.iter().filter(|p| p.is_ally && !p.is_me && !p.is_dead).map(|p| ChampionCandidate { champion: p.champion.clone(), level: p.level.max(0) as u32, screen: None }).collect();
    for (bar, c) in bars.iter().zip(match_bars_to_champions(bars, &cands, &MatchOptions::default())) {
        if let Some(c) = c {
            tracker.set_hp(&c, bar.pct, intel.game_time);
            tracker.set_shield(&c, bar.shield_pct, intel.game_time);
        }
    }
}

fn apply_hp(g: &mut Game, bars: &[HealthBar]) {
    let Game { intel, tracker, field, .. } = g;
    let Some(intel) = intel.as_ref() else { return };
    let cands: Vec<ChampionCandidate> = intel.players.iter().filter(|p| !p.is_ally && !p.is_dead).map(|p| ChampionCandidate { champion: p.champion.clone(), level: p.level.max(0) as u32, screen: None }).collect();
    for (bar, c) in bars.iter().zip(match_bars_to_champions(bars, &cands, &MatchOptions::default())) {
        if let Some(c) = c {
            tracker.set_hp(&c, bar.pct, intel.game_time);
            tracker.set_shield(&c, bar.shield_pct, intel.game_time);
            if bar.mana.is_some() {
                tracker.set_mana(&c, bar.mana, intel.game_time);
            }
        }
    }
    // Only one enemy bar on screen and only my lane opponent is near me on the minimap -> it's him.
    if let ([bar], Some(opp), Some(me)) = (bars, lane_opponent(intel), field.my_pos) {
        if bar.level.is_none() {
            let near: Vec<&SeenUnit> = field.visible_enemies.iter().filter(|e| (e.x - me.0).hypot(e.y - me.1) < 0.08).collect();
            if near.len() == 1 && near[0].champion == opp.champion {
                tracker.set_hp(&opp.champion, bar.pct, intel.game_time);
            }
        }
    }
}

fn with_vel(prev: &[SeenUnit], d: &Detection) -> SeenUnit {
    match prev.iter().find(|u| u.champion == d.champion) {
        Some(p) => SeenUnit { champion: d.champion.clone(), x: d.x, y: d.y, vx: p.vx * 0.6 + (d.x - p.x) * 0.4, vy: p.vy * 0.6 + (d.y - p.y) * 0.4 },
        None => SeenUnit { champion: d.champion.clone(), x: d.x, y: d.y, vx: 0.0, vy: 0.0 },
    }
}

/// `onVision` from index.ts.
pub fn on_vision(_app: &tauri::AppHandle, st: &Arc<AppState>, r: VisionResult) {
    if st.replay_blocks() {
        return;
    }
    let mut guard = st.game.lock().unwrap();
    if guard.intel.is_none() {
        return;
    }
    if let Some(bars) = &r.hp {
        apply_hp(&mut guard, bars);
    }
    if let Some(bars) = &r.hp_allies {
        apply_ally_hp(&mut guard, bars);
    }
    if let Some(frames) = &r.team_frames {
        apply_team_frames(&mut guard, frames);
    }
    if !guard.field.detect || !guard.field.map11 || r.width == 0 || guard.field.tracker.is_none() {
        return;
    }
    let icons: Vec<(String, String)> = guard.intel.as_ref().unwrap().players.iter().map(|p| (p.champion.clone(), champion_icon(&guard, &p.champion))).collect();
    let p_me = crate::knowledge::current_p_me(&guard);
    let Game { intel, field: f, team_watcher, obs, obs_game_key, profiles, .. } = &mut *guard;
    let intel = intel.as_ref().unwrap();
    let now = now_ms();
    let my_team = intel.me.as_ref().map(|m| m.team.as_str()).unwrap_or("ORDER");
    let my_team_t = Team::parse(my_team).unwrap_or(Team::Order);
    if let Some(me) = intel.me.as_ref() {
        if let Some(d) = r.detections.iter().find(|d| d.champion == me.champion) {
            f.my_pos = Some((d.x, d.y));
        }
    }
    f.allies = r.detections.iter().filter(|d| d.team == my_team_t).map(|d| with_vel(&f.allies, d)).collect();
    f.visible_enemies = r.detections.iter().filter(|d| d.team != my_team_t).map(|d| with_vel(&f.visible_enemies, d)).collect();
    // Enemy pushing a lane on our half with none of us near -> toast (once per champion+lane / 60 s).
    for (champ, lane) in unguarded_pushes(&f.visible_enemies, &f.allies, my_team_t) {
        let key = format!("{champ}@{lane}");
        if f.push_seen.iter().any(|(k, at)| *k == key && now - at < 60000.0) {
            continue;
        }
        f.push_seen.retain(|(k, _)| *k != key);
        f.push_seen.push((key, now));
        f.toasts.push(Toast { id: now as u64 + 7 + f.toasts.len() as u64, kind: "push".into(), team: "enemy".into(), champion: Some(champ.clone()), text: format!("{champ} pcha {lane} — nikt nie pilnuje"), expires_at: now + 5000.0 });
    }
    // Teamplay prompts (gank ask, follow-up, ...).
    let (allies_s, enemies_s) = (crate::knowledge::seen(&f.allies), crate::knowledge::seen(&f.visible_enemies));
    for a in team_watcher.update(intel, crate::knowledge::pos(f.my_pos), &allies_s, &enemies_s, p_me, now) {
        f.alerts.push(crate::game::alert_of(a));
    }
    f.mask = r.fog_mask.clone();
    f.mask_grid = r.fog_grid;
    let grid = r.fog_grid;
    let mask_ref = r.fog_mask.as_deref();
    let vis_fn = |x: f64, y: f64| mask_at(mask_ref.unwrap_or(&[]), grid, x, y);
    let visible: Option<&dyn Fn(f64, f64) -> f64> = if mask_ref.is_some() { Some(&vis_fn) } else { None };

    let enemies: Vec<EnemyInput> = intel
        .players
        .iter()
        .filter(|p| !p.is_ally)
        .map(|p| EnemyInput {
            champion: p.champion.clone(),
            team: Team::parse(&p.team).unwrap_or(my_team_t.other()),
            role: Role::parse(&p.role),
            is_dead: p.is_dead,
            move_speed: p.move_speed,
            likely_recall: false,
            start_top_side: profiles.iter().find(|pr| pr.champion == p.champion).and_then(|pr| pr.jungle.as_ref()).map(|j| j.start_top_side),
            // Shop-only items: evolving pets / support quest items would fake a recall.
            item_gold: Some(p.shop_gold),
            has_flash: p.summoner_spells.iter().any(|s| {
                let l = s.to_lowercase();
                l.contains("flash") || l.contains("błysk")
            }),
        })
        .collect();
    let dragon = intel.objectives.iter().find(|o| o.kind == "dragon");
    let baron = intel.objectives.iter().find(|o| o.kind == "baron");
    let t = intel.game_time;
    let obj_in = |o: Option<&rc_core::intel::game_intel::ObjectiveTimer>| o.map(|o| if o.alive { 0.0 } else { o.spawns_at.unwrap_or(t) - t });

    // Jungle camps: an enemy camp just died -> their jungler was there (a sighting for the tracker);
    // one of OUR camps died while our jungler is elsewhere -> invade.
    let n0 = f.camps.events.len();
    f.camp_status = f.camps.update(t, &r.camps, visible);
    let new_events: Vec<_> = f.camps.events[n0..].to_vec();
    let mut camp_sightings: Vec<Detection> = Vec::new();
    let enemy_jg = intel.players.iter().find(|p| !p.is_ally && p.role == "JUNGLE");
    for ev in new_events.iter().filter(|e| e.kind == CampEventKind::Cleared) {
        let Some(camp) = CAMPS.iter().find(|c| c.id == ev.id) else { continue };
        let side = side_str(camp.side);
        let fresh = ev.earliest.is_none_or(|e| ev.at - e <= 10.0);
        if let Some(jg) = enemy_jg {
            if side != my_team && camp.side != CampSide::Neutral && fresh && !r.detections.iter().any(|d| d.champion == jg.champion) {
                camp_sightings.push(Detection { champion: jg.champion.clone(), team: Team::parse(&jg.team).unwrap_or(my_team_t.other()), x: camp.x, y: camp.y, score: 0.7, margin: 0.0, votes: 0.0 });
            }
        }
        if side == my_team {
            let my_jg = intel.players.iter().find(|p| p.is_ally && !p.is_me && p.role == "JUNGLE");
            let my_jg_pos = my_jg.and_then(|j| f.allies.iter().find(|a| a.champion == j.champion));
            let ours = my_jg_pos.is_some_and(|p| (p.x - camp.x).hypot(p.y - camp.y) < 0.08);
            let me_there = f.my_pos.is_some_and(|p| (p.0 - camp.x).hypot(p.1 - camp.y) < 0.08);
            if !ours && !me_there {
                f.alerts.push(Alert::new("invade", format!("Invade {}", camp_label(camp.id)), 0.85, now + 6000.0));
            }
        }
    }
    let mut dets: Vec<Detection> = r.detections.iter().filter(|d| d.team != my_team_t).cloned().collect();
    dets.extend(camp_sightings);
    let tracker = f.tracker.as_mut().unwrap();
    let tracks = tracker.update(&FrameInput { t, detections: &dets, visible, enemies: &enemies, dragon_in: obj_in(dragon), baron_in: obj_in(baron) });
    // Proven recall (bought items while unseen) of any enemy -> short top-right toast.
    // (Auto-Flash stays off: dashes look the same on the minimap; Flash timers come from the hotkey.)
    // Lane opponent back in base (proven back or respawn) -> ETA to lane, until seen again.
    if let Some(opp) = lane_opponent(intel) {
        let backed = tracks.iter().any(|tr| tr.champion == opp.champion && tr.recalled_at == Some(t));
        let respawned = !opp.is_dead && f.prev_dead.contains(&opp.champion);
        if backed || respawned {
            let eta = return_eta(opp, f.my_pos, t);
            f.returns.retain(|(c, _)| *c != opp.champion);
            f.returns.push((opp.champion.clone(), now + eta * 1000.0));
        }
        if opp.is_dead || f.visible_enemies.iter().any(|e| e.champion == opp.champion) {
            f.returns.retain(|(c, _)| *c != opp.champion);
        }
    }
    f.returns.retain(|(_, until)| *until > now - 3000.0);
    f.prev_dead = intel.players.iter().filter(|p| !p.is_ally && p.is_dead).map(|p| p.champion.clone()).collect();
    for tr in &tracks {
        if tr.recalled_at == Some(t) && intel.players.iter().any(|x| x.champion == tr.champion && !x.is_ally) {
            f.toasts.push(Toast { id: now as u64 ^ (f.toasts.len() as u64) << 1, kind: "recall".into(), team: "enemy".into(), champion: Some(tr.champion.clone()), text: format!("{} w bazie", tr.champion), expires_at: now + 3500.0 });
        }
    }
    f.ghosts = tracker
        .ghosts(t)
        .into_iter()
        .map(|gh| {
            let icon_url = icons.iter().find(|(c, _)| *c == gh.champion).map(|(_, u)| u.clone()).unwrap_or_default();
            MinimapGhost { champion: gh.champion, x: gh.x, y: gh.y, radius: gh.radius, age: gh.age, confidence: gh.confidence, icon_url }
        })
        .collect();
    // Self-training: one observation per second of what we saw (real Summoner's Rift games only).
    if tracker.learning_enabled {
        let gk = intel.players.iter().map(|p| p.champion.as_str()).collect::<Vec<_>>().join("|");
        if gk != *obs_game_key {
            *obs_game_key = gk;
            obs.start(my_team);
        }
        let r4 = |v: f64| (v * 10000.0).round() / 10000.0;
        let obj_round = |o: Option<f64>| o.map(f64::round);
        obs.add(rc_core::self_train::ObsInput {
            t,
            d: r.detections.iter().filter(|d| d.team != my_team_t).map(|d| (d.champion.clone(), r4(d.x), r4(d.y))).collect(),
            mask: r.fog_mask.as_deref(),
            mask_size: r.fog_grid,
            e: intel.players.iter().filter(|p| !p.is_ally).map(|p| (p.champion.clone(), p.role.clone(), p.is_dead as u8, p.move_speed, p.shop_gold)).collect(),
            dr: obj_round(obj_in(dragon)),
            br: obj_round(obj_in(baron)),
        });
    }
    for a in tracker_alerts(&tracks, f.my_pos, my_team_t, t, now) {
        let kind = serde_json::to_value(a.kind).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
        let al = Alert { kind, text: a.text, confidence: a.confidence, expires_at: a.expires_at };
        match f.alerts.iter().position(|x| x.kind == al.kind) {
            Some(i) => f.alerts[i] = al,
            None => f.alerts.push(al),
        }
    }
}

// ---------- coach context ----------

/// Ward spots our team already sees (from the fog mask).
pub fn covered_spots(intel: &GameIntel, f: &Field) -> Vec<String> {
    if f.mask.is_none() {
        return Vec::new();
    }
    let red = intel.me.as_ref().is_some_and(|m| m.team == "CHAOS");
    rc_core::wards::WARD_SPOTS.iter().filter(|s| rc_core::wards::ward_point(s.id, red).and_then(|p| f.visible_at(p.0, p.1)).is_some_and(|v| v > 0.6)).map(|s| s.id.to_string()).collect()
}

/// Short camp facts for the coach: buff respawns (both sides) and the enemy jungler's last camp.
pub fn camp_summary(intel: &GameIntel, f: &Field) -> Vec<String> {
    let mut out = Vec::new();
    let my_team = intel.me.as_ref().map(|m| m.team.as_str()).unwrap_or("ORDER");
    for s in &f.camp_status {
        let Some(camp) = CAMPS.iter().find(|c| c.id == s.id) else { continue };
        if camp.kind != CampKind::Buff || s.state != CampState::Down {
            continue;
        }
        let Some(at) = s.respawn_at else { continue };
        let left = (at - intel.game_time).round();
        if left > 0.0 && left < 90.0 {
            let whose = if side_str(camp.side) == my_team { "twój" } else { "wrogi" };
            let colour = if camp.id.ends_with("blue") { "niebieski" } else { "czerwony" };
            out.push(format!("{whose} {colour} buff za {left} s"));
        }
    }
    let last = f.camps.events.iter().rev().find(|e| {
        e.kind == CampEventKind::Cleared && CAMPS.iter().find(|c| c.id == e.id).is_some_and(|c| side_str(c.side) != my_team && c.side != CampSide::Neutral)
    });
    if let Some(e) = last {
        if intel.game_time - e.at < 60.0 {
            out.push(format!("wrogi jungler wyczyścił {} {} s temu", camp_label(&e.id), (intel.game_time - e.at).round()));
        }
    }
    out
}

/// One-line state for `--smoke`.
pub fn smoke_summary(st: &AppState) -> Value {
    let cap = st.vision.capture.status();
    let g = st.game.lock().unwrap();
    let f = &g.field;
    let r3 = |v: f64| (v * 1000.0).round() / 1000.0;
    json!({
        "capture": cap,
        "calibration": st.vision.calibration(),
        "visionAvgMs": *st.vision.service.avg_ms.lock().unwrap(),
        "rosterReady": f.roster_ready,
        "detector": f.detect,
        "map11": f.map11,
        "learning": f.tracker.as_ref().map(|t| t.learning_enabled),
        "myPos": f.my_pos,
        "allies": f.allies.iter().map(|u| &u.champion).collect::<Vec<_>>(),
        "visibleEnemies": f.visible_enemies.iter().map(|u| &u.champion).collect::<Vec<_>>(),
        "tracks": f.tracker.as_ref().map(|t| t.all().iter().map(|t| json!([t.champion, t.state])).collect::<Vec<_>>()),
        "ghosts": f.ghosts.iter().map(|g| json!([g.champion, r3(g.x), r3(g.y), g.age.round()])).collect::<Vec<_>>(),
        "alerts": f.alerts.iter().map(|a| &a.text).collect::<Vec<_>>(),
        "camps": f.camp_status.iter().filter(|c| c.state != CampState::Unknown).map(|c| json!([c.id, c.state])).collect::<Vec<_>>(),
        "fogVisibleShare": f.mask.as_ref().map(|m| r3(m.iter().filter(|v| **v > 0.5).count() as f64 / m.len().max(1) as f64)),
        "hpEnemies": g.intel.as_ref().map(|i| i.players.iter().filter(|p| p.hp_pct.is_some()).map(|p| json!([p.champion, p.hp_pct])).collect::<Vec<_>>()),
    })
}

#[cfg(test)]
mod push_tests {
    use super::*;
    fn u(c: &str, x: f64, y: f64) -> SeenUnit {
        SeenUnit { champion: c.into(), x, y, vx: 0.0, vy: 0.0 }
    }
    #[test]
    fn unguarded_push_on_our_half_only() {
        // Blue: Yasuo deep in our top lane, our nearest player far away in bot.
        let got = unguarded_pushes(&[u("Yasuo", 0.08, 0.6)], &[u("Akali", 0.5, 0.9)], Team::Order);
        assert_eq!(got, vec![("Yasuo".to_string(), "top")]);
        // Someone of ours is there: no alert.
        assert!(unguarded_pushes(&[u("Yasuo", 0.08, 0.6)], &[u("Teemo", 0.1, 0.5)], Team::Order).is_empty());
        // Enemy on THEIR half of top lane: not a threat to call.
        assert!(unguarded_pushes(&[u("Yasuo", 0.3, 0.07)], &[], Team::Order).is_empty());
        // Red side: mirrored (our top lane half is the top edge, right part).
        assert_eq!(unguarded_pushes(&[u("Jhin", 0.6, 0.08)], &[], Team::Chaos), vec![("Jhin".to_string(), "top")]);
    }
}

#[cfg(test)]
mod fog_reset_tests {
    use super::*;
    #[test]
    fn old_model_moved_once() {
        let root = std::env::temp_dir().join(format!("rc-reset-{}", std::process::id()));
        let dir = root.join("model");
        std::fs::create_dir_all(dir.join("observations")).unwrap();
        std::fs::write(dir.join("fog-model.json"), "{}").unwrap();
        std::fs::write(dir.join("observations").join("1.json.gz"), "x").unwrap();
        reset_old_fog_model(&dir);
        assert!(!dir.join("fog-model.json").exists());
        assert!(root.join("model-old-v1").join("fog-model.json").exists());
        assert!(root.join("model-old-v1").join("observations").join("1.json.gz").exists());
        // Second start: new learning is kept.
        std::fs::write(dir.join("fog-model.json"), "{\"new\":1}").unwrap();
        reset_old_fog_model(&dir);
        assert!(dir.join("fog-model.json").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
