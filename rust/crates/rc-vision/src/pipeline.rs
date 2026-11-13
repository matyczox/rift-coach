//! Per-frame vision pipeline (port of `src/main/vision/worker.ts` + `service.ts`).
//!
//! [`Vision::process`] takes a full game-window frame, crops the minimap with the calibration
//! and returns detections, the fog mask, camp readings and (throttled) enemy health bars.
//! [`VisionService`] runs a `Vision` on a background thread at <= `max_fps`, always on the
//! newest pushed frame (older pending frames are dropped), like the TS worker + service.

use crate::background::load_reference;
use crate::calibration::Calibration;
use crate::camps::{read_camps, CampReading, ReadCampsParams};
use crate::detect::{build_template, detect_prepared, DetectParams, Detection, Hint, Prepared, Region, Sweep, Team, Template};
use crate::fog::{sample_mask, FogParams, FogReader};
use crate::healthbars::{read_health_bars, BarColor, HealthBar, ReadOptions};
use crate::image::Rgba;
use crate::waves::{read_waves, WaveParams, WaveReading, WaveSmoother};
use serde::Serialize;
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct RosterEntry {
    pub champion: String,
    pub team: Team,
    /// Data Dragon square icon.
    pub icon: Rgba,
}

#[derive(Clone, Debug)]
pub struct Roster {
    pub champions: Vec<RosterEntry>,
    pub own_team: Team,
    /// Game setting "relative team colours": enemies are red whatever side you are on.
    pub relative_colors: bool,
}

impl Roster {
    /// Which team has red rings / red health bars.
    pub fn red_team(&self) -> Team {
        if self.relative_colors {
            self.own_team.other()
        } else {
            Team::Chaos
        }
    }
    /// Colour of the ENEMY health bars.
    pub fn enemy_bar_color(&self) -> BarColor {
        if self.red_team() == self.own_team {
            BarColor::Blue
        } else {
            BarColor::Red
        }
    }
}

#[derive(Clone, Debug)]
pub struct VisionParams {
    /// Base detector parameters (own_team / red_team / sweep are set per frame from the roster).
    pub detect: DetectParams,
    pub fog: FogParams,
    pub fog_grid: usize,
    pub camps: ReadCampsParams,
    /// Full fallback sweep every N-th frame; otherwise only around hints (TS: 5).
    pub full_sweep_every: u32,
    /// Radius of the regional sweep around each hint (normalized; TS: 0.08).
    pub region_r: f64,
    /// Health bars at most every N ms (None = off; TS app: 350).
    pub hp_every_ms: Option<u64>,
    /// Worker threads for the detector (small on purpose: the game needs the CPU).
    pub threads: usize,
}

impl Default for VisionParams {
    fn default() -> Self {
        VisionParams { detect: DetectParams::default(), fog: FogParams::default(), fog_grid: 64, camps: ReadCampsParams::default(), full_sweep_every: 5, region_r: 0.08, hp_every_ms: Some(200), threads: 2 }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisionResult {
    /// Caller timestamp of the frame (ms).
    pub at: u64,
    /// Minimap crop width (px); 0 when no minimap could be cropped.
    pub width: usize,
    pub detections: Vec<Detection>,
    /// `fog_grid`² P(visible) per cell, row-major, y down (None without minimap/reference).
    pub fog_mask: Option<Vec<f32>>,
    pub fog_grid: usize,
    pub camps: Vec<CampReading>,
    /// Enemy health bars when read this frame (throttled by `hp_every_ms`).
    pub hp: Option<Vec<HealthBar>>,
    /// Allies' health bars (same read, the other colour).
    #[serde(skip)]
    pub hp_allies: Option<Vec<HealthBar>>,
    /// Ally portraits above the minimap: HP / mana of every ally (team order, me excluded).
    #[serde(skip)]
    pub team_frames: Option<Vec<crate::teamframes::AllyFrame>>,
    /// Minion waves per lane (smoothed over ~2 s), None without a minimap.
    #[serde(skip)]
    pub waves: Option<WaveReading>,
    /// Processing time (ms).
    pub ms: f64,
}

impl VisionResult {
    /// P(visible) at normalized minimap coords, if the mask is known.
    pub fn visible(&self, x: f64, y: f64) -> Option<f64> {
        self.fog_mask.as_ref().map(|m| sample_mask(m, self.fog_grid, x, y))
    }
}

pub struct Vision {
    roster: Roster,
    calibration: Option<Calibration>,
    params: VisionParams,
    templates: Vec<Template>,
    templates_for: usize,
    fog: Option<(usize, FogReader)>,
    frame_no: u64,
    last_hp_at: Option<u64>,
    hints: Vec<Hint>,
    pool: rayon::ThreadPool,
    waves: WaveSmoother,
}

impl Vision {
    pub fn new(roster: Roster, calibration: Option<Calibration>, params: VisionParams) -> Vision {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(params.threads.max(1)).thread_name(|i| format!("rc-vision-{i}")).build().expect("vision thread pool");
        Vision { roster, calibration, params, templates: Vec::new(), templates_for: 0, fog: None, frame_no: 0, last_hp_at: None, hints: Vec::new(), pool, waves: WaveSmoother::new(2000) }
    }

    pub fn set_roster(&mut self, roster: Roster) {
        self.roster = roster;
        self.templates_for = 0;
        self.frame_no = 0;
        self.waves.reset();
    }

    pub fn set_calibration(&mut self, c: Option<Calibration>) {
        self.calibration = c;
    }

    pub fn roster(&self) -> &Roster {
        &self.roster
    }

    /// Positions to also try and to sweep around on regional frames (TS: last positions of
    /// enemies the tracker currently sees). Kept until replaced.
    pub fn set_hints(&mut self, hints: Vec<Hint>) {
        self.hints = hints;
    }

    /// Full game-window frame -> minimap crop (via calibration) + health bars.
    pub fn process(&mut self, frame: &Rgba, at: u64) -> VisionResult {
        let t0 = Instant::now();
        let minimap = self.calibration.and_then(|c| {
            let r = c.rect_for(frame.width as i32, frame.height as i32);
            if r.x < 0 || r.y < 0 || r.width <= 8 || (r.x + r.width) as usize > frame.width || (r.y + r.height) as usize > frame.height {
                None
            } else {
                Some(frame.crop(r.x as usize, r.y as usize, r.width as usize, r.height as usize))
            }
        });
        let mut res = match &minimap {
            Some(m) => self.process_minimap(m, at),
            None => VisionResult { at, width: 0, detections: Vec::new(), fog_mask: None, fog_grid: self.params.fog_grid, camps: Vec::new(), hp: None, hp_allies: None, team_frames: None, waves: None, ms: 0.0 },
        };
        if let Some(every) = self.params.hp_every_ms {
            if self.last_hp_at.is_none_or(|l| at.saturating_sub(l) >= every || at < l) {
                self.last_hp_at = Some(at);
                let color = self.roster.enemy_bar_color();
                res.hp = Some(read_health_bars(frame, &ReadOptions::new(color)));
                let ally = if color == BarColor::Red { BarColor::Blue } else { BarColor::Red };
                res.hp_allies = Some(read_health_bars(frame, &ReadOptions::new(ally)));
                if let Some(c) = self.calibration {
                    let r = c.rect_for(frame.width as i32, frame.height as i32);
                    res.team_frames = Some(crate::teamframes::read_team_frames(frame, (r.x, r.y, r.width, r.height)));
                }
            }
        }
        res.ms = t0.elapsed().as_secs_f64() * 1e3;
        res
    }

    /// Minimap crop only (no health bars).
    pub fn process_minimap(&mut self, img: &Rgba, at: u64) -> VisionResult {
        let t0 = Instant::now();
        if self.templates_for != img.width && !self.roster.champions.is_empty() {
            let p = &self.params.detect;
            self.templates = self.roster.champions.iter().map(|r| build_template(&r.champion, r.team, &r.icon, img.width, p)).collect();
            self.templates_for = img.width;
        }
        if self.fog.as_ref().is_none_or(|(w, _)| *w != img.width || img.width != img.height) && img.width == img.height {
            self.fog = Some((img.width, FogReader::new(&load_reference(img.width), self.params.fog_grid, self.params.fog)));
        }
        let full = self.frame_no % self.params.full_sweep_every.max(1) as u64 == 0;
        self.frame_no += 1;
        let sweep = if full { Sweep::Full } else { Sweep::Regions(self.hints.iter().map(|h| Region { x: h.x, y: h.y, r: self.params.region_r }).collect()) };
        let params = DetectParams { own_team: Some(self.roster.own_team), red_team: Some(self.roster.red_team()), sweep, ..self.params.detect.clone() };
        let (templates, hints) = (&self.templates, &self.hints);
        let detections = if templates.is_empty() {
            Vec::new()
        } else {
            self.pool.install(|| {
                let prep = Prepared::new(img);
                detect_prepared(&prep, templates, &params, hints).detections
            })
        };
        let fog_mask = match &self.fog {
            Some((w, r)) if *w == img.width && img.width == img.height => Some(r.read(img)),
            _ => None,
        };
        let camps = read_camps(img, &self.params.camps);
        // Minion waves (~0.3 ms at 341 px): portrait rings share the minion colours -> masked.
        let portraits: Vec<(f64, f64)> = detections.iter().map(|d| (d.x, d.y)).collect();
        let flip = self.roster.own_team == Team::Chaos;
        let blue_is_ally = self.roster.red_team() != self.roster.own_team;
        let raw = read_waves(img, &portraits, flip, blue_is_ally, fog_mask.as_deref().map(|m| (m, self.params.fog_grid)), &WaveParams::default());
        let waves = Some(self.waves.push(at, raw));
        VisionResult { at, width: img.width, detections, fog_mask, fog_grid: self.params.fog_grid, camps, hp: None, hp_allies: None, team_frames: None, waves, ms: t0.elapsed().as_secs_f64() * 1e3 }
    }
}

struct Pending {
    frame: Option<(Arc<Rgba>, u64, Option<Vec<Hint>>)>,
    roster: Option<Roster>,
    calibration: Option<Option<Calibration>>,
    stop: bool,
}

/// Background vision worker: newest frame wins, at most `max_fps` frames per second.
pub struct VisionService {
    shared: Arc<(Mutex<Pending>, Condvar)>,
    handle: Option<std::thread::JoinHandle<()>>,
    /// Smoothed processing time (ms).
    pub avg_ms: Arc<Mutex<f64>>,
}

impl VisionService {
    /// Results are sent to `out` (drop the receiver to discard them).
    pub fn start(vision: Vision, max_fps: f64, out: mpsc::Sender<VisionResult>) -> VisionService {
        let shared = Arc::new((Mutex::new(Pending { frame: None, roster: None, calibration: None, stop: false }), Condvar::new()));
        let avg_ms = Arc::new(Mutex::new(0.0));
        let (sh, avg) = (shared.clone(), avg_ms.clone());
        let period = Duration::from_secs_f64(1.0 / max_fps.clamp(0.2, 60.0));
        let handle = std::thread::Builder::new()
            .name("rc-vision".into())
            .spawn(move || {
                let mut vision = vision;
                let mut last_start: Option<Instant> = None;
                loop {
                    let job = {
                        let (m, cv) = &*sh;
                        let mut p = m.lock().unwrap();
                        loop {
                            if p.stop {
                                return;
                            }
                            if let Some(r) = p.roster.take() {
                                vision.set_roster(r);
                            }
                            if let Some(c) = p.calibration.take() {
                                vision.set_calibration(c);
                            }
                            if p.frame.is_some() {
                                let wait = last_start.map(|s| period.saturating_sub(s.elapsed())).unwrap_or_default();
                                if wait.is_zero() {
                                    break p.frame.take().unwrap();
                                }
                                p = cv.wait_timeout(p, wait).unwrap().0;
                            } else {
                                p = cv.wait(p).unwrap();
                            }
                        }
                    };
                    last_start = Some(Instant::now());
                    let (frame, at, hints) = job;
                    if let Some(h) = hints {
                        vision.set_hints(h);
                    }
                    let res = vision.process(&frame, at);
                    {
                        let mut a = avg.lock().unwrap();
                        *a = if *a == 0.0 { res.ms } else { *a * 0.9 + res.ms * 0.1 };
                    }
                    if out.send(res).is_err() {
                        return;
                    }
                }
            })
            .expect("spawn vision thread");
        VisionService { shared, handle: Some(handle), avg_ms }
    }

    /// Queue a full game frame (replaces any frame still waiting). `hints` = tracker positions.
    pub fn push(&self, frame: impl Into<Arc<Rgba>>, at: u64, hints: Option<Vec<Hint>>) {
        let (m, cv) = &*self.shared;
        m.lock().unwrap().frame = Some((frame.into(), at, hints));
        cv.notify_one();
    }

    pub fn set_roster(&self, roster: Roster) {
        let (m, cv) = &*self.shared;
        m.lock().unwrap().roster = Some(roster);
        cv.notify_one();
    }

    pub fn set_calibration(&self, c: Option<Calibration>) {
        let (m, cv) = &*self.shared;
        m.lock().unwrap().calibration = Some(c);
        cv.notify_one();
    }
}

impl Drop for VisionService {
    fn drop(&mut self) {
        let (m, cv) = &*self.shared;
        m.lock().unwrap().stop = true;
        cv.notify_all();
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
