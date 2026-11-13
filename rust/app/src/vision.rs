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
    /// Other likely places (x, y, radius, mass), drawn as weaker zones.
    pub zones: Vec<rc_vision::tracker::Zone>,
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

/// What an enemy jungler / support did so far this game (learned live, reset each game).
#[derive(Clone, Debug, Default)]
pub struct Habit {
    /// Sightings by map side: [top side, mid, bot side].
    pub side: [u32; 3],
    /// Ganks seen per lane ("top" | "mid" | "bot").
    pub ganks: std::collections::HashMap<&'static str, u32>,
    /// First sighting in the first 2 minutes: (area name, game time) = where he started.
    pub start: Option<(String, f64)>,
    /// Last gank counted per lane (game time), to count one gank once.
    pub last_gank: std::collections::HashMap<&'static str, f64>,
}

impl Habit {
    /// "ganki: bot 3, mid 1; czas: bot 55%, top 30%, mid 15%; start: wroga dżungla top" for the coach.
    pub fn summary(&self) -> String {
        let total: u32 = self.side.iter().sum::<u32>().max(1);
        let mut g: Vec<(&&str, &u32)> = self.ganks.iter().collect();
        g.sort_by(|a, b| b.1.cmp(a.1));
        let ganks = if g.is_empty() { "brak".to_string() } else { g.iter().map(|(l, n)| format!("{l} {n}")).collect::<Vec<_>>().join(", ") };
        let pct = |i: usize| (self.side[i] as f64 * 100.0 / total as f64).round();
        let start = self.start.as_ref().map(|(a, t)| format!("; start: {a} ({}:{:02})", (*t as i64) / 60, (*t as i64) % 60)).unwrap_or_default();
        format!("ganki: {ganks}; czas: top {}%, mid {}%, bot {}%{start}", pct(0), pct(1), pct(2))
    }
}

/// Per-game vision state (lives in `Game`, under the same lock as the intel).
pub struct Field {
    pub toasts: Vec<Toast>,
    /// Unguarded lane pushes announced: ("champion@lane", Date.now() ms).
    pub push_seen: Vec<(String, f64)>,
    /// Enemies seen near me on the minimap: champion -> last game time (kept 3 s for the fight bar).
    pub near_seen: std::collections::HashMap<String, f64>,
    /// Enemy health bars on my screen in the last read: (count, game time). Unnamed ones count too.
    pub enemy_bars: (usize, f64),
    /// Lane pushes warned: "champion@lane" -> stage (1 coming, 2 near our tower, 3 hitting it).
    pub push_stage: std::collections::HashMap<String, u8>,
    /// Enemy jungler / support habits learned this game.
    pub habits: std::collections::HashMap<String, Habit>,
    /// Allies last seen on the minimap: champion -> (x, y, game time). An ally standing on an enemy
    /// hides under his icon, so "nobody there" needs a few seconds of memory.
    pub ally_last: std::collections::HashMap<String, (f64, f64, f64)>,
    /// Fog "X invade" guess: (shown since, quiet until) in Date.now() ms.
    pub invade_guess: (f64, f64),
    /// Ally fights announced: (ally, Date.now() ms).
    pub fight_seen: Vec<(String, f64)>,
    /// Go help this ally (fight worth joining): (ally, until Date.now() ms) -> arrow on the minimap.
    pub help: Option<(String, f64)>,
    /// Teamfight at an objective pit with our people there: (label, x, y, until Date.now() ms) -> arrow.
    pub help_point: Option<(String, f64, f64, f64)>,
    /// Last fight toast: (toast id, enemies in it) - dropped once they are all dead.
    pub fight_toast: Option<(u64, Vec<String>)>,
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
    /// When my portrait was last seen (Date.now() ms): the position expires after a few seconds.
    pub my_pos_at: f64,
    /// Last trusted spot of my icon (kept when my_pos goes unknown): (x, y, Date.now() ms).
    pub my_last_real: Option<(f64, f64, f64)>,
    /// Last time my icon sat in the middle of the camera box (camera locked on me): Date.now() ms.
    pub cam_locked_at: f64,
    /// Last time my icon was seen AWAY from the camera box centre (camera unlocked / panned).
    pub cam_unlocked_at: f64,
    /// Size of the camera rectangle when fully on the minimap (w, h); at the map edge it is cut off.
    pub cam_size: (f64, f64),
    /// Trinket read from the HUD: (has a charge, Date.now() ms of the reading).
    pub trinket: Option<(bool, f64)>,
    /// Last time the trinket went from ready to recharging (= I placed a ward), Date.now() ms.
    pub trinket_used_at: f64,
    /// Where my position came from on the last frame: icon | camera | held | none (for the log).
    pub pos_src: &'static str,
    /// Last vision result (Date.now() ms): when vision stops, everything spatial expires.
    pub last_vision_at: f64,
    /// CommunityDragon was unreachable at roster load (square icons in use): retry at this time (ms).
    pub icon_retry_at: f64,
    /// Recalls already announced: (champion, recalled_at game time). Vision runs faster than the Live
    /// API, so the same recall shows up on several frames.
    pub recall_seen: Vec<(String, f64)>,
    /// Enemies hidden under another portrait (teamfight): champion -> game time it started.
    pub occluded_since: std::collections::HashMap<String, f64>,
    pub allies: Vec<SeenUnit>,
    pub visible_enemies: Vec<SeenUnit>,
    /// Fog mask (P(visible) per cell, `mask_grid`², row-major).
    pub mask: Option<Vec<f32>>,
    pub mask_grid: usize,
    /// Minion waves per lane from the last vision frame (smoothed in the vision worker).
    pub waves: Option<rc_vision::waves::WaveReading>,
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
            fight_seen: Vec::new(),
            invade_guess: (0.0, 0.0),
            ally_last: Default::default(),
            habits: Default::default(),
            push_stage: Default::default(),
            enemy_bars: (0, 0.0),
            near_seen: Default::default(),
            help: None,
            fight_toast: None,
            help_point: None,
            prev_dead: Vec::new(),
            tracker,
            camps: CampTracker::default(),
            camp_status: Vec::new(),
            ghosts: Vec::new(),
            alerts: Vec::new(),
            my_pos: None,
            my_pos_at: 0.0,
            my_last_real: None,
            cam_locked_at: 0.0,
            cam_unlocked_at: 0.0,
            cam_size: (0.268, 0.15),
            trinket: None,
            trinket_used_at: 0.0,
            pos_src: "none",
            last_vision_at: 0.0,
            icon_retry_at: 0.0,
            recall_seen: Vec::new(),
            occluded_since: std::collections::HashMap::new(),
            allies: Vec::new(),
            visible_enemies: Vec::new(),
            mask: None,
            mask_grid: 64,
            waves: None,
            roster_key: String::new(),
            roster_ready: false,
            detect: false,
            map11: false,
        }
    }

    /// Vision stopped (no result for 2 s) while the Live API goes on: positions, ghosts and the fog
    /// mask are unknown now, not "as last seen".
    pub fn is_stale(&self, now: f64) -> bool {
        now - self.last_vision_at > 2000.0
    }

    pub fn expire_if_stale(&mut self, now: f64) {
        if self.is_stale(now) {
            self.my_pos = None;
            self.allies.clear();
            self.visible_enemies.clear();
            self.ghosts.clear();
            self.mask = None;
            self.waves = None;
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
        self.fight_seen.clear();
        self.invade_guess = (0.0, 0.0);
        self.ally_last.clear();
        self.habits.clear();
        self.push_stage.clear();
        self.enemy_bars = (0, 0.0);
        self.near_seen.clear();
        self.help = None;
        self.fight_toast = None;
        self.help_point = None;
        self.prev_dead.clear();
        self.ghosts.clear();
        self.my_pos = None;
        self.recall_seen.clear();
        self.roster_key.clear();
        self.roster_ready = false;
        self.allies.clear();
        self.visible_enemies.clear();
        self.mask = None;
        self.waves = None;
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
/// In-game minimap icon from CommunityDragon (cached). `net` = may use the network; `net_failed` is
/// set when CommunityDragon did not answer (not a 404), so the caller stops trying for this load.
async fn load_game_circle_icon(dir: &Path, dd_id: &str, net: bool, net_failed: &mut bool) -> Option<rc_vision::Rgba> {
    let id = dd_id.to_lowercase();
    let file = dir.join(format!("cd-{id}-0.png"));
    let miss = dir.join(format!("cd-{id}-0.png.missing"));
    if !file.exists() && !miss.exists() {
        if !net {
            return None;
        }
        let _ = std::fs::create_dir_all(dir);
        let url = format!("https://raw.communitydragon.org/latest/game/assets/characters/{id}/hud/{id}_circle_0.png");
        // Short: 10 champions x 10 s left the detector off for the first ~100 s of a game when it was down.
        let res = match rc_core::intel::ddragon::web_client().get(url).timeout(Duration::from_secs(3)).send().await {
            Ok(r) => r,
            Err(_) => {
                *net_failed = true;
                return None;
            }
        };
        if res.status().as_u16() == 404 {
            let _ = std::fs::write(&miss, b"");
            return None;
        }
        if !res.status().is_success() {
            *net_failed = true;
            return None;
        }
        std::fs::write(&file, res.bytes().await.ok()?).ok()?;
    }
    if file.exists() { rc_vision::image::load_png(&file).ok() } else { None }
}

/// Circle icon if possible (network only while CommunityDragon answers), else the Data Dragon square.
/// `square_used` is set when the square was used because CommunityDragon was unreachable.
async fn load_icon(dir: &Path, version: &str, dd_id: &str, net_failed: &mut bool, square_used: &mut bool) -> anyhow::Result<rc_vision::Rgba> {
    // 1) cache, 2) the local League install (always there, no server), 3) CommunityDragon.
    if !dir.join(format!("cd-{}-0.png", dd_id.to_lowercase())).exists() {
        let d = dd_id.to_string();
        if let Ok(Some(icon)) = tokio::task::spawn_blocking(move || crate::game_icons::circle_icon(&d)).await {
            return Ok(icon);
        }
    }
    let net = !*net_failed;
    if let Some(icon) = load_game_circle_icon(dir, dd_id, net, net_failed).await {
        return Ok(icon);
    }
    if *net_failed && !dir.join(format!("cd-{}-0.png.missing", dd_id.to_lowercase())).exists() {
        *square_used = true;
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
        // Square icons because CommunityDragon was down: try again every 2 min, swap in the circle icons.
        if g.field.icon_retry_at > 0.0 && now_ms() >= g.field.icon_retry_at && g.field.roster_ready {
            g.field.icon_retry_at = 0.0;
        } else {
            return;
        }
    }
    let retry_only = key == g.field.roster_key;
    g.field.roster_key = key.clone();
    g.field.roster_ready = false;
    g.field.map11 = map == 11;
    let players: Vec<(String, String)> = intel.players.iter().map(|p| (p.champion.clone(), p.team.clone())).collect();
    let st = st.clone();
    rt.spawn(async move {
        let dir = icons_dir(&st);
        let mut champions = Vec::new();
        let mut err = None;
        let (mut net_failed, mut square_used) = (false, false);
        // Other maps: no templates (the detector only knows Summoner's Rift); health bars still work.
        if map == 11 {
            for (champ, team) in &players {
                let dd = sd.champion_ids.get(champ).cloned().unwrap_or_else(|| champ.clone());
                match load_icon(&dir, &sd.version, &dd, &mut net_failed, &mut square_used).await {
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
        // A retry only swaps the icons: keep the tracker's current knowledge.
        if !retry_only {
            g.field.camps.reset();
            if let Some(t) = g.field.tracker.as_mut() {
                t.reset();
            }
        }
        g.field.icon_retry_at = if square_used { now_ms() + 120_000.0 } else { 0.0 };
        if square_used {
            log::warn!("vision: CommunityDragon unreachable, square icons for now (retry in 2 min)");
        }
        g.field.roster_ready = true;
        log::info!("vision roster ready (map {map}, detector {})", g.field.detect);
    });
}

/// Per Live API sample (onGameData): learning switch + roster.
pub fn on_game_data(rt: &tokio::runtime::Handle, st: &Arc<AppState>, g: &mut Game, game_mode: &str, map: i64, bots: bool) {
    if let Some(t) = g.field.tracker.as_mut() {
        // Never from replays (also not with --allow-replay): only real games teach the fog model.
        t.learning_enabled = LEARN_MODES.contains(&game_mode) && map == 11 && !st.replay_open() && !bots;
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
    let mut last_frame_at = 0u64;
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
        // Same image again (WGC repeats its last frame with a new seq when nothing new arrives) or an
        // image older than 2 s: not a new observation of the game.
        if f.at_ms <= last_frame_at || (now_ms() as u64).saturating_sub(f.at_ms) > 2000 {
            continue;
        }
        last_frame_at = f.at_ms;
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
/// `lost` = towers his team already lost in that lane (he meets the wave at the first one standing).
pub fn return_eta(p: &PlayerIntel, my_pos: Option<(f64, f64)>, game_t: f64, lost: u8) -> f64 {
    use rc_vision::tracker::{enemy::homeguard_extra_secs, predictor::fountain_of, MAP_UNITS};
    let team = Team::parse(&p.team).unwrap_or(Team::Chaos);
    let (fx, fy) = fountain_of(team);
    let _ = my_pos;
    // "Na linii" = at his own outer tower of that lane (where he meets the wave), ORDER side; CHAOS
    // is the point mirror. (Was my position: from my side of the lane it came out far too long.)
    // ORDER towers of the lane: outer, inner, inhibitor.
    let towers = match p.role.as_str() {
        "TOP" => [(0.066, 0.298), (0.102, 0.549), (0.079, 0.712)],
        "BOTTOM" | "UTILITY" => [(0.706, 0.931), (0.465, 0.900), (0.288, 0.916)],
        _ => [(0.393, 0.570), (0.339, 0.676), (0.246, 0.751)],
    };
    let order_outer = towers[(lost as usize).min(2)];
    let target = if team == Team::Order { order_outer } else { (1.0 - order_outer.0, 1.0 - order_outer.1) };
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
        // Inside our base (nexus area): that is the nexus, not a lane ("Sona pcha top" from our base).
        let lane = if x.hypot(1.0 - y) < 0.24 {
            "nexus"
        } else if x < 0.18 && y > 0.45 && y < 0.88 {
            "top"
        } else if y > 0.82 && x > 0.12 && x < 0.55 {
            "bot"
        } else if (x - (1.0 - y)).abs() < 0.08 && x > 0.2 && x < 0.45 {
            "mid"
        } else {
            continue;
        };
        if allies.iter().any(|a| (a.x - e.x).hypot(a.y - e.y) < 0.18) {
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
        "gromp" => "Gromp",
        "blue" => "Blue",
        "wolves" => "Wolves",
        "raptors" => "Raptors",
        "red" => "Red",
        "krugs" => "Krugs",
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
    let Game { intel, tracker, field, .. } = g;
    let Some(intel) = intel.as_ref() else { return };
    // A blue bar can be my own mana strip ("Maokai obok" while he stood top and I mid): only allies
    // whose minimap icon is near me (or hidden under my portrait) can have a bar on my screen.
    let me = field.my_pos;
    let close = |c: &str| match (me, field.allies.iter().find(|a| a.champion == c)) {
        (Some((mx, my)), Some(a)) => (a.x - mx).hypot(a.y - my) < 0.2,
        _ => true,
    };
    let cands: Vec<ChampionCandidate> = intel.players.iter().filter(|p| p.is_ally && !p.is_me && !p.is_dead && close(&p.champion)).map(|p| ChampionCandidate { champion: p.champion.clone(), level: p.level.max(0) as u32, screen: None }).collect();
    for (bar, c) in bars.iter().zip(match_bars_to_champions(bars, &cands, &MatchOptions::default())) {
        if let Some(c) = c {
            tracker.set_hp(&c, bar.pct, intel.game_time);
            tracker.set_onscreen(&c, intel.game_time);
            tracker.set_shield(&c, bar.shield_pct, intel.game_time);
        }
    }
}

fn apply_hp(g: &mut Game, bars: &[HealthBar]) {
    let Game { intel, tracker, field, .. } = g;
    let Some(intel) = intel.as_ref() else { return };
    field.enemy_bars = (bars.len(), intel.game_time);
    let cands: Vec<ChampionCandidate> = intel.players.iter().filter(|p| !p.is_ally && !p.is_dead).map(|p| ChampionCandidate { champion: p.champion.clone(), level: p.level.max(0) as u32, screen: None }).collect();
    for (bar, c) in bars.iter().zip(match_bars_to_champions(bars, &cands, &MatchOptions::default())) {
        if let Some(c) = c {
            tracker.set_hp(&c, bar.pct, intel.game_time);
            tracker.set_onscreen(&c, intel.game_time);
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
    // Frame older than 2 s (queue / worker delay): not the game as it is now.
    if now_ms() - r.at as f64 > 2000.0 {
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
    if let Some(ready) = r.trinket_ready {
        let now = now_ms();
        if guard.field.trinket.is_some_and(|(was, _)| was) && !ready {
            guard.field.trinket_used_at = now;
        }
        guard.field.trinket = Some((ready, now));
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
    f.last_vision_at = now;
    let my_team = intel.me.as_ref().map(|m| m.team.as_str()).unwrap_or("ORDER");
    let my_team_t = Team::parse(my_team).unwrap_or(Team::Order);
    if let Some(me) = intel.me.as_ref() {
        // My icon: of all detections with my champion on my team, the one nearest to where I just was
        // (a stray match in our base made me jump base <-> dragon mid-fight and hid the fight bar).
        let mine: Vec<_> = r.detections.iter().filter(|d| d.champion == me.champion && d.team == my_team_t).collect();
        let pick = match f.my_pos.filter(|_| now - f.my_pos_at < 3000.0) {
            Some((px, py)) => mine.iter().min_by(|a, b| (a.x - px).hypot(a.y - py).total_cmp(&(b.x - px).hypot(b.y - py))).copied(),
            None => mine.first().copied(),
        };
        // A jump of more than ~1/3 of the map within 3 s is not walking (recall takes 8 s): ignore it.
        let pick = pick.filter(|d| f.my_pos.filter(|_| now - f.my_pos_at < 3000.0).is_none_or(|(px, py)| (d.x - px).hypot(d.y - py) < 0.3));
        // A stray "me" on our fountain while I was fighting far away: only after a death, or once a
        // recall (8 s) could have happened since my last real spot.
        let fountain = if my_team_t == Team::Order { (0.04, 0.96) } else { (0.96, 0.04) };
        let just_respawned = f.prev_dead.contains(&me.champion);
        let pick = pick.filter(|d| {
            let at_fountain = (d.x - fountain.0).hypot(d.y - fountain.1) < 0.12;
            !at_fountain || just_respawned || f.my_last_real.is_none_or(|(x, y, at)| (d.x - x).hypot(d.y - y) < 0.3 || now - at > 8500.0)
        });
        if let Some(d) = pick {
            f.my_pos = Some((d.x, d.y));
            f.my_pos_at = now;
            f.my_last_real = Some((d.x, d.y, now));
            f.pos_src = "icon";
            match r.camera.map(|(cx, cy)| (cx - d.x).hypot(cy - d.y)) {
                Some(dist) if dist < 0.05 => f.cam_locked_at = now,
                Some(dist) if dist > 0.09 => f.cam_unlocked_at = now,
                _ => {}
            }
        } else if let (Some((sx, sy)), Some((x0, y0, x1, y1)), false) = (r.me_screen, r.camera_rect, me.is_dead) {
            // My green bar on screen + the camera rectangle (= what the screen shows) -> my spot on the
            // map, whatever the camera does (locked or not). A box cut by the map edge gets its usual size
            // back from the side that is still there.
            let (cw, ch) = f.cam_size;
            let (x0, x1) = if x0 <= 0.01 && x1 - x0 < cw * 0.95 { (x1 - cw, x1) } else if x1 >= 0.99 && x1 - x0 < cw * 0.95 { (x0, x0 + cw) } else { (x0, x1) };
            let (y0, y1) = if y0 <= 0.01 && y1 - y0 < ch * 0.95 { (y1 - ch, y1) } else if y1 >= 0.99 && y1 - y0 < ch * 0.95 { (y0, y0 + ch) } else { (y0, y1) };
            f.my_pos = Some((x0 + sx * (x1 - x0), y0 + sy * (y1 - y0)));
            f.my_pos_at = now;
            f.pos_src = "screen";
        } else if let Some((cx, cy)) = r.camera.filter(|_| !me.is_dead && now - f.cam_locked_at < 30000.0 && f.cam_locked_at > f.cam_unlocked_at) {
            // My icon hidden (under others in a fight): with the camera locked on me (seen in the last
            // 30 s) the camera box centre IS my position.
            f.my_pos = Some((cx, cy));
            f.my_pos_at = now;
            f.pos_src = "camera";
        } else if me.is_dead || now - f.my_pos_at > if f.my_pos.is_some_and(|(x, y)| r.detections.iter().any(|d| (d.x - x).hypot(d.y - y) < 0.045)) { 4000.0 } else { 3000.0 } {
            // (My icon under someone else's in a fight: keep my last spot up to 8 s, not "unknown" —
            // without it the fight bar only counted the one enemy whose health bar had a name.)
            // Unknown, not "where I was": an old position fed alerts, the fight bar and the coach.
            f.my_pos = None;
            f.pos_src = "none";
        } else {
            f.pos_src = "held";
        }
    }
    if let Some((x0, y0, x1, y1)) = r.camera_rect.filter(|(x0, y0, x1, y1)| *x0 > 0.01 && *y0 > 0.01 && *x1 < 0.99 && *y1 < 0.99) {
        f.cam_size = (x1 - x0, y1 - y0);
    }
    let prev_allies = std::mem::take(&mut f.allies);
    f.allies = r.detections.iter().filter(|d| d.team == my_team_t).map(|d| with_vel(&prev_allies, d)).collect();
    // An ally right next to me hides under my portrait (or another one): still there, not gone. Keep him
    // at his last spot for up to 5 s so the fight bar counts him ("Sejuani was with me, not counted").
    let tnow = intel.game_time;
    let mut kept = std::collections::HashSet::new();
    for u in &prev_allies {
        if f.allies.iter().any(|a| a.champion == u.champion) {
            continue;
        }
        if !r.detections.iter().any(|d| d.champion != u.champion && (d.x - u.x).hypot(d.y - u.y) < 0.035) {
            continue;
        }
        let since = *f.occluded_since.entry(format!("ally:{}", u.champion)).or_insert(tnow);
        kept.insert(format!("ally:{}", u.champion));
        if tnow - since <= 5.0 {
            f.allies.push(SeenUnit { vx: 0.0, vy: 0.0, ..u.clone() });
        }
    }
    f.occluded_since.retain(|k, _| !k.starts_with("ally:") || kept.contains(k));
    f.visible_enemies = r.detections.iter().filter(|d| d.team != my_team_t).map(|d| with_vel(&f.visible_enemies, d)).collect();
    for a in &f.allies {
        f.ally_last.insert(a.champion.clone(), (a.x, a.y, intel.game_time));
    }
    // Learn their jungler's / support's habits live: which side he spends time on, where he started,
    // which lane he ganks ("ciągle gankuje bota") — fed to the coach and to the gank warning.
    {
        let red = my_team_t == Team::Chaos;
        for p in intel.players.iter().filter(|p| !p.is_ally && (p.role == "JUNGLE" || p.role == "UTILITY")) {
            let Some(e) = f.visible_enemies.iter().find(|e| e.champion == p.champion) else { continue };
            let h = f.habits.entry(p.champion.clone()).or_default();
            let s = e.x + e.y;
            h.side[if s < 0.85 { 0 } else if s > 1.15 { 2 } else { 1 }] += 1;
            if h.start.is_none() && intel.game_time < 120.0 {
                h.start = Some((crate::game::area_name(e.x, e.y, red), intel.game_time));
            }
            // A gank: he is right next to one of our laners standing on a lane, with their laner there too.
            for a in &f.allies {
                if (a.x - e.x).hypot(a.y - e.y) > 0.07 {
                    continue;
                }
                let area = crate::game::area_name(a.x, a.y, red);
                let lane: &'static str = match area.as_str() { "linia top" => "top", "linia mid" => "mid", "linia bot" => "bot", _ => continue };
                if h.last_gank.get(lane).is_some_and(|t| intel.game_time - t < 40.0) {
                    continue;
                }
                h.last_gank.insert(lane, intel.game_time);
                *h.ganks.entry(lane).or_insert(0) += 1;
            }
        }
    }
    if let Some((mx, my)) = f.my_pos {
        for e in f.visible_enemies.iter().filter(|e| (e.x - mx).hypot(e.y - my) < 0.15) {
            f.near_seen.insert(e.champion.clone(), intel.game_time);
        }
    }
    f.near_seen.retain(|_, t| intel.game_time - *t <= 3.0);
    // Enemy pushing a lane on our half with none of us near -> toast (once per champion+lane / 60 s).
    // Me too, by my own position (my icon can be missing from the detections: "pcha mid" while I stood mid).
    let mut guards = f.allies.clone();
    if let Some((x, y)) = f.my_pos {
        guards.push(SeenUnit { champion: String::new(), x, y, vx: 0.0, vy: 0.0 });
    }
    // Not in the first minutes: everyone is just walking to lane.
    let pushes = if intel.game_time > 150.0 { unguarded_pushes(&f.visible_enemies, &guards, my_team_t) } else { Vec::new() };
    for (champ, lane) in pushes {
        let role = match lane { "top" => "TOP", "bot" => "BOTTOM", "mid" => "MIDDLE", _ => "" };
        if !role.is_empty() && intel.players.iter().any(|p| p.champion == champ && p.role == role) {
            continue;
        }
        let key = format!("{champ}@{lane}");
        if f.push_seen.iter().any(|(k, at)| *k == key && now - at < 60000.0) {
            continue;
        }
        f.push_seen.retain(|(k, _)| *k != key);
        f.push_seen.push((key, now));
        let text = if lane == "nexus" { format!("{champ} w naszej bazie — broń nexusa") } else { format!("{champ} pcha {lane} — nikt nie pilnuje") };
        f.toasts.push(Toast { id: now as u64 + 7 + f.toasts.len() as u64, kind: "push".into(), team: "enemy".into(), champion: Some(champ.clone()), text, expires_at: now + 5000.0 });
    }
    // An ally fighting somewhere I can reach (jungler vs invader, skirmish): "go help" right away,
    // locally. Not plain laning: a jungler involved, or 3+ champions in the scrap.
    if let Some((mx, my)) = f.my_pos {
        let me_c = intel.me.as_ref().map(|m| m.champion.clone()).unwrap_or_default();
        let jungler = |c: &str| intel.players.iter().any(|p| p.champion == c && p.role == "JUNGLE");
        for a in f.allies.iter().filter(|a| a.champion != me_c) {
            let d_me = (a.x - mx).hypot(a.y - my);
            let foes: Vec<&SeenUnit> = f.visible_enemies.iter().filter(|e| (e.x - a.x).hypot(e.y - a.y) < 0.045).collect();
            let friends = f.allies.iter().filter(|b| b.champion != me_c && (b.x - a.x).hypot(b.y - a.y) < 0.045).count();
            let Some(foe) = foes.first() else { continue };
            // A teamfight (4+ champions in it) anywhere is worth walking to from further away (~half the
            // map); a small skirmish only when I am close.
            let teamfight = friends + foes.len() >= 4;
            if !(0.12..if teamfight { 0.55 } else { 0.3 }).contains(&d_me) {
                continue;
            }
            if !(jungler(&a.champion) || foes.iter().any(|e| jungler(&e.champion)) || friends + foes.len() >= 3) {
                continue;
            }
            if f.fight_seen.iter().any(|(k, at)| *k == a.champion && now - at < 30000.0) {
                continue;
            }
            f.fight_seen.retain(|(k, _)| *k != a.champion);
            f.fight_seen.push((a.champion.clone(), now));
            // Worth going? Count both sides around the fight: visible ones plus fogged enemies the tracker
            // most likely puts there ("bił się z Kayle, a za nią cała zgraja"). Me arriving counts for us.
            const AREA: f64 = 0.09;
            let ours = 1 + f.allies.iter().filter(|b| b.champion != me_c && (b.x - a.x).hypot(b.y - a.y) < AREA).count();
            let seen_foes = f.visible_enemies.iter().filter(|e| (e.x - a.x).hypot(e.y - a.y) < AREA).count();
            let fog_foes = intel
                .players
                .iter()
                .filter(|p| !p.is_ally && !p.is_dead && !f.visible_enemies.iter().any(|e| e.champion == p.champion))
                .filter(|p| f.tracker.as_ref().and_then(|tr| tr.mass_near(&p.champion, (a.x, a.y), AREA)).is_some_and(|m| m > 0.4))
                .count();
            let theirs = seen_foes + fog_foes;
            let more = if theirs > 1 { format!(" +{}", theirs - 1) } else { String::new() };
            let fog = if fog_foes > 0 { format!(" ({fog_foes} we mgle)") } else { String::new() };
            let _ = (&more, &fog, foe);
            // Plain words: "Pomóż Sona — 2v2 z tobą" / "Nie idź do Sona — 1v3" (was "Sona vs X +1 → pomóż").
            let (kind, team, text) = match (theirs > ours, teamfight) {
                (true, true) => ("fight-bad", "enemy", format!("Teamfight {}v{} — nie idź", ours, theirs)),
                (true, false) => ("fight-bad", "enemy", format!("Nie idź do {} — {}v{}", a.champion, ours, theirs)),
                (false, true) => ("fight", "ally", format!("Teamfight → idź, {}v{} z tobą", ours, theirs)),
                (false, false) => ("fight", "ally", format!("Pomóż {} — {}v{} z tobą", a.champion, ours, theirs)),
            };
            f.toasts.push(Toast { id: now as u64 + 13 + f.toasts.len() as u64, kind: kind.into(), team: team.into(), champion: Some(a.champion.clone()), text, expires_at: now + 6000.0 });
            f.help = (theirs <= ours).then(|| (a.champion.clone(), now + 8000.0));
            let id = f.toasts.last().map(|t| t.id).unwrap_or(0);
            f.fight_toast = Some((id, f.visible_enemies.iter().filter(|e| (e.x - a.x).hypot(e.y - a.y) < AREA).map(|e| e.champion.clone()).collect()));
            break;
        }
    }
    // Their laner seen on his lane pushing towards our tower (their wave on our side, or him walking at our
    // tower) with none of us near: warn before he is AT the tower.
    for (role, lane) in [("TOP", "top"), ("MIDDLE", "mid"), ("BOTTOM", "bot")] {
        if intel.game_time < 150.0 || intel.me.as_ref().is_some_and(|m| m.role == role) {
            continue;
        }
        let Some(e) = intel.players.iter().find(|p| !p.is_ally && p.role == role && !p.is_dead) else { continue };
        let Some(v) = f.visible_enemies.iter().find(|v| v.champion == e.champion) else { continue };
        let order_outer = match lane { "top" => (0.066, 0.298), "bot" => (0.706, 0.931), _ => (0.393, 0.570) };
        let tower = if my_team_t == Team::Order { order_outer } else { (1.0 - order_outer.0, 1.0 - order_outer.1) };
        let d = (v.x - tower.0).hypot(v.y - tower.1);
        // On his lane between the river and our tower (not walking mid, not in his base).
        let on_lane = match lane {
            "top" => if my_team_t == Team::Order { v.x < 0.2 || v.y < 0.2 } else { v.x > 0.8 || v.y > 0.8 },
            "bot" => if my_team_t == Team::Order { v.x > 0.8 || v.y > 0.8 } else { v.x < 0.2 || v.y < 0.2 },
            _ => (v.x - (1.0 - v.y)).abs() < 0.08,
        };
        let our_base = if my_team_t == Team::Order { (0.04, 0.96) } else { (0.96, 0.04) };
        if (v.x - our_base.0).hypot(v.y - our_base.1) < 0.24 {
            continue; // in our base: the nexus warning above covers it
        }
        let base_key = format!("{}@{lane}", e.champion);
        // Their outer tower of the lane: he must be on OUR half (closer to our tower than to his).
        let their_tower = (1.0 - tower.0, 1.0 - tower.1);
        let our_half = d < (v.x - their_tower.0).hypot(v.y - their_tower.1);
        if !on_lane || !our_half || d > 0.4 {
            // Back on his side: the next push warns from the start again.
            f.push_stage.remove(&base_key);
            continue;
        }
        if d > 0.22 {
            continue;
        }
        let speed = v.vx.hypot(v.vy);
        let heading = speed > 0.0008 && ((tower.0 - v.x) * v.vx + (tower.1 - v.y) * v.vy) / (d.max(1e-6) * speed) > 0.6;
        let wave_ours = f.waves.as_ref().and_then(|w| w.lanes.iter().find(|l| l.lane == lane)).is_some_and(|l| matches!(l.state.as_str(), "our_side" | "under_our_tower"));
        // Someone of us next to him now, or a moment ago (icon hidden under his): no warning.
        let guarded = f.allies.iter().any(|a| (a.x - v.x).hypot(a.y - v.y) < 0.18)
            || f.ally_last.values().any(|(x, y, t)| intel.game_time - t <= 6.0 && (x - v.x).hypot(y - v.y) < 0.18)
            || f.my_pos.is_some_and(|(x, y)| (x - v.x).hypot(y - v.y) < 0.18);
        // Their wave really on our side; walking at our tower alone is not a push (lane start).
        let _ = heading;
        if guarded || !wave_ours {
            continue;
        }
        // Warn again only when he gets closer (coming -> near our tower -> hitting it), not on a timer.
        let stage: u8 = if d < 0.06 { 3 } else if d < 0.12 { 2 } else { 1 };
        if f.push_stage.get(&base_key).is_some_and(|s| *s >= stage) {
            continue;
        }
        f.push_stage.insert(base_key.clone(), stage);
        f.push_seen.retain(|(k, _)| *k != base_key);
        f.push_seen.push((base_key, now));
        let text = match stage {
            3 => format!("{} bije naszą wieżę {lane} — nikt nie pilnuje", e.champion),
            2 => format!("{} blisko naszej wieży {lane}", e.champion),
            _ => format!("{} pcha {lane} w stronę naszej wieży", e.champion),
        };
        f.toasts.push(Toast { id: now as u64 + 17 + f.toasts.len() as u64, kind: "push".into(), team: "enemy".into(), champion: Some(e.champion.clone()), text, expires_at: now + 6000.0 });
    }
    // Fight over (all its enemies dead): the "X vs Y" toast and the help arrow go now, not after 6 s.
    if let Some((id, foes)) = f.fight_toast.clone() {
        let over = !foes.is_empty() && foes.iter().all(|c| intel.players.iter().any(|p| p.champion == *c && p.is_dead));
        if over || !f.toasts.iter().any(|t| t.id == id) {
            if over {
                f.toasts.retain(|t| t.id != id);
                f.help = None;
            }
            f.fight_toast = None;
        }
    }
    // Teamplay prompts (gank ask, follow-up, ...).
    let (allies_s, enemies_s) = (crate::knowledge::seen(&f.allies), crate::knowledge::seen(&f.visible_enemies));
    for a in team_watcher.update(intel, crate::knowledge::pos(f.my_pos), &allies_s, &enemies_s, p_me, now) {
        let al = crate::game::alert_of(a);
        // Team prompts ("Teemo obok → pinguj, wchodźcie", "MF idzie do ciebie"): shown as a green alert
        // under my champion (the top bar never showed these kinds). Same text at most every 20 s.
        if intel.game_time > 150.0 && matches!(al.kind.as_str(), "gank-ask" | "ally") {
            let key = format!("team:{}", al.text);
            if !f.push_seen.iter().any(|(k, at)| *k == key && now - at < 20000.0) {
                f.push_seen.retain(|(k, _)| *k != key);
                f.push_seen.push((key, now));
                f.toasts.push(Toast { id: now as u64 + 23 + f.toasts.len() as u64, kind: "fight".into(), team: "ally".into(), champion: None, text: al.text.clone(), expires_at: now + 5000.0 });
            }
        }
        f.alerts.push(al);
    }
    f.mask = r.fog_mask.clone();
    f.mask_grid = r.fog_grid;
    f.waves = r.waves.clone();
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
    // Camp cleared on the enemy side: a soft hint for their jungler's fog estimate (it may have been a
    // laner or one of us), never a detection / learning label.
    let mut camp_hints: Vec<(String, (f64, f64))> = Vec::new();
    let enemy_jg = intel.players.iter().find(|p| !p.is_ally && p.role == "JUNGLE");
    for ev in new_events.iter().filter(|e| e.kind == CampEventKind::Cleared) {
        let Some(camp) = CAMPS.iter().find(|c| c.id == ev.id) else { continue };
        let side = side_str(camp.side);
        let fresh = ev.earliest.is_none_or(|e| ev.at - e <= 10.0);
        if let Some(jg) = enemy_jg {
            if side != my_team && camp.side != CampSide::Neutral && fresh && !r.detections.iter().any(|d| d.champion == jg.champion) {
                camp_hints.push((jg.champion.clone(), (camp.x, camp.y)));
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
    // Hidden under another portrait (portraits overlap in a fight): not gone. Keep him where he was for up
    // to 5 s instead of letting the fog mask wipe every hypothesis near the fight (which left only
    // "recalled" ones and put the ghost in base). Only while still Visible: no fog episode, nothing learned.
    if let Some(tr) = f.tracker.as_ref() {
        let mut still = std::collections::HashSet::new();
        for e in tr.all() {
            if e.state != rc_vision::tracker::TrackState::Visible || dets.iter().any(|d| d.champion == e.champion) {
                continue;
            }
            let Some(l) = e.last else { continue };
            let covered = r.detections.iter().any(|d| d.champion != e.champion && (d.x - l.x).hypot(d.y - l.y) < 0.035);
            if !covered {
                continue;
            }
            let since = *f.occluded_since.entry(e.champion.clone()).or_insert(t);
            // Remember the start while he stays covered, so the 5 s limit really ends.
            still.insert(e.champion.clone());
            if t - since <= 5.0 {
                dets.push(Detection { champion: e.champion.clone(), team: e.team, x: l.x, y: l.y, score: 0.5, margin: 0.0, votes: 0.0 });
            }
        }
        f.occluded_since.retain(|c, _| c.starts_with("ally:") || still.contains(c));
    }
    let tracker = f.tracker.as_mut().unwrap();
    for (c, q) in &camp_hints {
        tracker.soft_hint(c, *q);
    }
    let tracks = tracker.update(&FrameInput { t, detections: &dets, visible, enemies: &enemies, dragon_in: obj_in(dragon), baron_in: obj_in(baron) });
    // Proven recall (bought items while unseen) of any enemy -> short top-right toast.
    // (Auto-Flash stays off: dashes look the same on the minimap; Flash timers come from the hotkey.)
    // Lane opponent back in base (proven back or respawn) -> ETA to lane, until seen again.
    // New recalls only (once per recall, not once per vision frame).
    let mut new_recalls: Vec<String> = Vec::new();
    for tr in &tracks {
        // His health bar was on my screen a moment ago: he was not in base (the minimap only lost him).
        let seen_on_screen = intel.players.iter().any(|p| p.champion == tr.champion && p.hp_seen_at.is_some_and(|s| t - s < 8.0));
        if let Some(at) = tr.recalled_at.filter(|at| t - at < 5.0 && !seen_on_screen) {
            if !f.recall_seen.iter().any(|(c, a)| *c == tr.champion && (*a - at).abs() < 1e-6) {
                f.recall_seen.retain(|(c, _)| *c != tr.champion);
                f.recall_seen.push((tr.champion.clone(), at));
                new_recalls.push(tr.champion.clone());
            }
        }
    }
    if let Some(opp) = lane_opponent(intel) {
        let backed = new_recalls.contains(&opp.champion);
        let respawned = !opp.is_dead && f.prev_dead.contains(&opp.champion);
        let counting = f.returns.iter().any(|(c, _)| *c == opp.champion);
        let lane_name = match opp.role.as_str() { "TOP" => "top", "BOTTOM" | "UTILITY" => "bot", _ => "mid" };
        let their_lost = intel.lanes.iter().find(|l| l.lane == lane_name).map_or(0, |l| l.theirs_lost);
        if opp.is_dead && opp.respawn_in > 0.0 {
            // Dead: respawn timer + walk, refreshed every frame from the game's own timer (it was set
            // only once he respawned, seconds late, and then counted too long).
            let eta = opp.respawn_in + return_eta(opp, f.my_pos, t, their_lost);
            f.returns.retain(|(c, _)| *c != opp.champion);
            f.returns.push((opp.champion.clone(), now + eta * 1000.0));
        } else if backed || (respawned && !counting) {
            let eta = return_eta(opp, f.my_pos, t, their_lost);
            f.returns.retain(|(c, _)| *c != opp.champion);
            f.returns.push((opp.champion.clone(), now + eta * 1000.0));
        }
        // Back already: seen on the minimap, or his health bar on my screen (minimap can miss him).
        let on_screen = opp.onscreen_at.is_some_and(|s| t - s < 2.0);
        if !opp.is_dead && (on_screen || f.visible_enemies.iter().any(|e| e.champion == opp.champion)) {
            f.returns.retain(|(c, _)| *c != opp.champion);
        }
    }
    f.returns.retain(|(_, until)| *until > now - 3000.0);
    f.prev_dead = intel.players.iter().filter(|p| !p.is_ally && p.is_dead).map(|p| p.champion.clone()).collect();
    for tr in &tracks {
        if new_recalls.contains(&tr.champion) && intel.players.iter().any(|x| x.champion == tr.champion && !x.is_ally) {
            f.toasts.push(Toast { id: now as u64 ^ (f.toasts.len() as u64) << 1, kind: "recall".into(), team: "enemy".into(), champion: Some(tr.champion.clone()), text: format!("{} w bazie", tr.champion), expires_at: now + 3500.0 });
        }
    }
    f.ghosts = tracker
        .ghosts(t)
        .into_iter()
        .map(|gh| {
            let icon_url = icons.iter().find(|(c, _)| *c == gh.champion).map(|(_, u)| u.clone()).unwrap_or_default();
            MinimapGhost { champion: gh.champion, x: gh.x, y: gh.y, radius: gh.radius, age: gh.age, confidence: gh.confidence, icon_url, zones: gh.zones }
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
    let my_role = intel.me.as_ref().map(|m| rc_vision::tracker::Role::parse(&m.role));
    let new_alerts = {
        let my_lane: Option<&'static str> = intel.me.as_ref().and_then(|m| match m.role.as_str() { "TOP" => Some("top"), "MIDDLE" => Some("mid"), "BOTTOM" | "UTILITY" => Some("bot"), _ => None });
        let habit_boost = |c: &str| my_lane.and_then(|l| f.habits.get(c).and_then(|h| h.ganks.get(l))).map_or(1.0, |n| (1.0 + 0.15 * *n as f64).min(1.5));
        let near_me = |c: &str| f.my_pos.and_then(|p| f.tracker.as_ref().and_then(|tr| tr.mass_near(c, p, 0.11))).map(|m| (m * habit_boost(c)).min(1.0));
        tracker_alerts(&tracks, f.my_pos, my_team_t, my_role, t, now, &near_me)
    };
    // Can the enemies be taking an objective right now? Seen at the pit, or likely there in the fog (all
    // tracker hypotheses): P(at least one there). Shown with a % like a gank warning.
    if let Some(tr) = f.tracker.as_ref() {
        const PITS: [(&[&str], (f64, f64)); 2] = [(&["dragon", "elder"], (0.663, 0.703)), (&["baron", "herald", "grubs", "atakhan"], (0.337, 0.296))];
        let mut best: Option<(f64, String)> = None;
        for (kinds, pit) in PITS {
            let Some(o) = intel.objectives.iter().find(|o| kinds.contains(&o.kind) && o.alive) else { continue };
            let mut miss = 1.0;
            for p in intel.players.iter().filter(|p| !p.is_ally && !p.is_dead) {
                let m = match f.visible_enemies.iter().find(|e| e.champion == p.champion) {
                    Some(e) => if (e.x - pit.0).hypot(e.y - pit.1) < 0.1 { 1.0 } else { 0.0 },
                    // A fog guess counts fully only for their jungler (Smite); laners / support smeared
                    // near the pit lit "Mogą robić Dragon" for ~1200 frames a game.
                    None => tr.mass_near(&p.champion, pit, 0.1).unwrap_or(0.0) * if p.role == "JUNGLE" { 1.0 } else { 0.4 },
                };
                miss *= 1.0 - m.min(0.95);
            }
            let p = 1.0 - miss;
            let name = match o.kind { "dragon" => "Dragon", "elder" => "Elder Dragon", "baron" => "Baron", "herald" => "Herald", "grubs" => "Voidgrubs", _ => "Atakhan" };
            // Shown from 50 %, then kept down to 25 % (it flickered on/off around the threshold).
            let showing = f.alerts.iter().any(|x| x.kind == "objective" && x.expires_at > now && x.text.ends_with(name));
            // Our people at the pit too = a teamfight there: arrow to go (up to ~half the map away).
            let ours_there = f.allies.iter().filter(|a| (a.x - pit.0).hypot(a.y - pit.1) < 0.12).count();
            let far = f.my_pos.is_some_and(|(x, y)| { let d = (x - pit.0).hypot(y - pit.1); d > 0.1 && d < 0.55 });
            // We are taking it (2+ of us at the pit) and their jungler (Smite) may be close: steal risk.
            if ours_there >= 2 {
                if let Some(j) = intel.players.iter().find(|p| !p.is_ally && !p.is_dead && p.role == "JUNGLE") {
                    let m = match f.visible_enemies.iter().find(|e| e.champion == j.champion) {
                        Some(e) => if (e.x - pit.0).hypot(e.y - pit.1) < 0.18 { 1.0 } else { 0.0 },
                        None => tr.mass_near(&j.champion, pit, 0.18).unwrap_or(0.0),
                    };
                    if m >= 0.35 {
                        let al = Alert::new("objective", format!("{} może ukraść {name}", j.champion), m.min(0.9), now + 4000.0);
                        match f.alerts.iter().position(|x| x.kind == "objective") {
                            Some(i) => f.alerts[i] = al,
                            None => f.alerts.push(al),
                        }
                        continue;
                    }
                }
            }
            if p >= 0.5 && ours_there >= 1 && far {
                f.help_point = Some((format!("Teamfight przy {name} → idź"), pit.0, pit.1, now + 6000.0));
            }
            if p >= if showing { 0.35 } else { 0.6 } && best.as_ref().is_none_or(|b| p > b.0) {
                best = Some((p, format!("Mogą robić {name}")));
            }
        }
        if let Some((p, text)) = best {
            let al = Alert::new("objective", text, p.min(0.9), now + 6000.0);
            match f.alerts.iter().position(|x| x.kind == al.kind) {
                Some(i) => f.alerts[i] = al,
                None => f.alerts.push(al),
            }
        }
    }
    for a in new_alerts {
        let kind = serde_json::to_value(a.kind).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
        if kind == "invade" {
            let (since, quiet) = f.invade_guess;
            if now < quiet {
                continue;
            }
            if since == 0.0 || now - since > 6000.0 {
                f.invade_guess = (now, 0.0);
            } else if now - since > 5000.0 {
                f.invade_guess = (0.0, now + 40000.0);
                continue;
            }
        }
        let al = Alert { kind, text: a.text, confidence: a.confidence, expires_at: a.expires_at };
        match f.alerts.iter().position(|x| x.kind == al.kind) {
            Some(i) => f.alerts[i] = al,
            None => f.alerts.push(al),
        }
    }
}

// ---------- coach context ----------

/// Wave state for the coach: {"top": {"ally", "enemy", "front", "state"}, ...}; lanes without
/// minions carry the state only.
pub fn waves_json(w: &rc_vision::waves::WaveReading) -> Value {
    let mut o = serde_json::Map::new();
    for l in &w.lanes {
        let v = match l.front {
            Some(front) => serde_json::json!({ "ally": l.ally, "enemy": l.enemy, "front": front, "state": l.state.as_str() }),
            None => serde_json::json!({ "ally": l.ally, "enemy": l.enemy, "state": l.state.as_str() }),
        };
        o.insert(l.lane.to_string(), v);
    }
    Value::Object(o)
}

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
