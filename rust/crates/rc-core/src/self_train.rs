//! Self-training from your own games, no replays needed (`src/main/tracker/selfTrain.ts`):
//!  1. during a real game, once a second, store what the app saw (enemy detections,
//!     a coarse fog mask, enemy states) — a few hundred KB per game ([`ObsRecorder`]);
//!  2. after the game, the Riot API timeline gives every player's true position each
//!     minute ([`timeline_truth`]);
//!  3. replay the observations through a fresh tracker and, at each minute mark where
//!     an enemy was in fog, credit the behaviours that explained the true position
//!     ([`train_from_observations`]).
//!
//! The fog tracker is in rc-vision, so step 3 goes through the [`ObsTracker`] trait;
//! the app implements it for `rc_vision::tracker::EnemyTracker` (new tracker with
//! 250 particles, `learning_enabled = true`, detections with score 0.9, margin 0.5,
//! votes 1 — the values the TS code used).

use crate::riot::types::{MatchDto, Position, TimelineDto};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

/// One observation per second.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObsFrame {
    /// game seconds
    pub t: f64,
    /// enemy detections: champion, x, y
    pub d: Vec<(String, f64, f64)>,
    /// 32x32 fog mask, base64 bitset (1 = visible)
    pub m: String,
    /// champion, role, isDead(0/1), moveSpeed, itemGold
    pub e: Vec<(String, String, u8, f64, f64)>,
    /// dragon / baron seconds to spawn
    pub dr: Option<f64>,
    pub br: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObsLog {
    pub game_id: Option<i64>,
    /// ORDER | CHAOS
    pub team: String,
    pub enemy_team: String,
    pub frames: Vec<ObsFrame>,
}

const G: usize = 32;

/// `size`×`size` mask (≥ 0.5 = visible) → 32×32 bitset, base64.
pub fn pack_mask(mask: &[f32], size: usize) -> String {
    let mut bits = vec![0u8; G * G / 8];
    for y in 0..G {
        for x in 0..G {
            let v = mask.get((y * size / G) * size + (x * size / G)).copied().unwrap_or(0.0);
            if v >= 0.5 {
                let i = y * G + x;
                bits[i >> 3] |= 1 << (i & 7);
            }
        }
    }
    base64::engine::general_purpose::STANDARD.encode(bits)
}

/// Base64 bitset → `visible(x, y)` on normalized minimap coordinates.
pub fn unpack_mask(s: &str) -> impl Fn(f64, f64) -> f64 {
    let bits = base64::engine::general_purpose::STANDARD.decode(s).unwrap_or_default();
    move |x: f64, y: f64| {
        let gx = ((x * G as f64).floor().max(0.0) as usize).min(G - 1);
        let gy = ((y * G as f64).floor().max(0.0) as usize).min(G - 1);
        let i = gy * G + gx;
        (bits.get(i >> 3).copied().unwrap_or(0) >> (i & 7) & 1) as f64
    }
}

/// One frame as the app collects it (mask unpacked).
pub struct ObsInput<'a> {
    pub t: f64,
    pub d: Vec<(String, f64, f64)>,
    pub mask: Option<&'a [f32]>,
    /// Side length of `mask` (64 in the TS app).
    pub mask_size: usize,
    pub e: Vec<(String, String, u8, f64, f64)>,
    pub dr: Option<f64>,
    pub br: Option<f64>,
}

#[derive(Default)]
pub struct ObsRecorder {
    pub log: Option<ObsLog>,
    last_t: f64,
}

impl ObsRecorder {
    pub fn start(&mut self, team: &str) {
        let enemy = if team == "ORDER" { "CHAOS" } else { "ORDER" };
        self.log = Some(ObsLog { game_id: None, team: team.into(), enemy_team: enemy.into(), frames: vec![] });
        self.last_t = -1.0;
    }

    pub fn add(&mut self, f: ObsInput) {
        let Some(log) = self.log.as_mut() else { return };
        let Some(mask) = f.mask else { return };
        if f.t - self.last_t < 1.0 {
            return;
        }
        self.last_t = f.t;
        log.frames.push(ObsFrame { t: crate::intel::js_round(f.t * 10.0) / 10.0, d: f.d, m: pack_mask(mask, f.mask_size), e: f.e, dr: f.dr, br: f.br });
    }

    /// `<dir>/<gameId or now>.json.gz`; None for short logs (< 120 frames ≈ 2 min).
    pub fn save(&mut self, dir: &Path, game_id: Option<i64>) -> Option<PathBuf> {
        let log = self.log.as_mut()?;
        if log.frames.len() < 120 {
            return None;
        }
        log.game_id = game_id;
        std::fs::create_dir_all(dir).ok()?;
        let name = game_id.map(|g| g.to_string()).unwrap_or_else(|| (crate::jsutil::now_ms() as i64).to_string());
        let file = dir.join(format!("{name}.json.gz"));
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(crate::jsutil::json_compact(log).as_bytes()).ok()?;
        std::fs::write(&file, gz.finish().ok()?).ok()?;
        Some(file)
    }
}

pub fn load_obs(file: &Path) -> anyhow::Result<ObsLog> {
    let raw = std::fs::read(file)?;
    let mut text = String::new();
    std::io::Read::read_to_string(&mut flate2::read::GzDecoder::new(&raw[..]), &mut text)?;
    Ok(serde_json::from_str(&text)?)
}

/// `<userData>/model/observations`
pub fn obs_dir(user_data: &Path) -> PathBuf {
    user_data.join("model").join("observations")
}

/// Summoner's Rift game units -> minimap coords (y flipped: minimap y grows downwards).
pub fn riot_to_minimap(p: Position) -> (f64, f64) {
    (((p.x + 120.0) / 14990.0).clamp(0.0, 0.999), (1.0 - (p.y + 120.0) / 15100.0).clamp(0.0, 0.999))
}

/// True minimap position of `champion` at timeline frame `minute` (index.ts `truth`).
/// `name_fix` maps match-v5 champion names to the names the tracker uses
/// (Data Dragon id → display name; identity if unknown).
pub fn timeline_truth(m: &MatchDto, tl: &TimelineDto, champion: &str, minute: usize, name_fix: &dyn Fn(&str) -> String) -> Option<(f64, f64)> {
    let frame = tl.info.frames.get(minute)?;
    let mut pfs: Vec<_> = frame.participant_frames.values().collect();
    pfs.sort_by_key(|pf| pf.participant_id); // JS iterates integer keys ascending
    for pf in pfs {
        let Some(p) = m.info.participants.iter().find(|p| p.participant_id == pf.participant_id) else { continue };
        if name_fix(&p.champion_name) == champion {
            if let Some(pos) = pf.position {
                return Some(riot_to_minimap(pos));
            }
        }
    }
    None
}

/// What the training loop sees of a track.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackView {
    pub champion: String,
    /// `state === 'fog'`
    pub in_fog: bool,
    /// (x, y, radius)
    pub estimate: Option<(f64, f64, f64)>,
    /// last confirmed position
    pub last: Option<(f64, f64)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObsEnemy {
    pub champion: String,
    pub role: String,
    pub is_dead: bool,
    pub move_speed: f64,
    pub item_gold: f64,
}

/// Thin view of the rc-vision fog tracker (implemented by the app).
pub trait ObsTracker {
    /// One tracker frame; `visible(x, y)` is the unpacked fog mask.
    #[allow(clippy::too_many_arguments)]
    fn update(&mut self, t: f64, detections: &[(String, f64, f64)], visible: &dyn Fn(f64, f64) -> f64, enemies: &[ObsEnemy], enemy_team: &str, dragon_in: Option<f64>, baron_in: Option<f64>) -> Vec<TrackView>;
    /// Credit the behaviours that explain the true position `q` of `champion` at `t`.
    fn train_at(&mut self, champion: &str, q: (f64, f64), t: f64);
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrainResult {
    pub samples: usize,
    pub err_model: f64,
    pub err_baseline: f64,
    pub inside: f64,
}

/// Replay a game's observations and learn from the timeline truth.
/// Returns how the model did vs the "stayed where last seen" baseline on fogged enemies.
pub fn train_from_observations(log: &ObsLog, tracker: &mut dyn ObsTracker, truth: &dyn Fn(&str, usize) -> Option<(f64, f64)>) -> TrainResult {
    let (mut n, mut err_model, mut err_base, mut inside) = (0usize, 0.0, 0.0, 0usize);
    let mut next_minute = 1usize;
    for f in &log.frames {
        let enemies: Vec<ObsEnemy> = f.e.iter().map(|(c, r, dead, ms, gold)| ObsEnemy { champion: c.clone(), role: r.clone(), is_dead: *dead == 1, move_speed: *ms, item_gold: *gold }).collect();
        let visible = unpack_mask(&f.m);
        let tracks = tracker.update(f.t, &f.d, &visible, &enemies, &log.enemy_team, f.dr, f.br);
        if f.t >= next_minute as f64 * 60.0 {
            for tr in &tracks {
                let (true, Some(est), Some(last)) = (tr.in_fog, tr.estimate, tr.last) else { continue };
                let Some(q) = truth(&tr.champion, next_minute) else { continue };
                let em = (q.0 - est.0).hypot(q.1 - est.1);
                let eb = (q.0 - last.0).hypot(q.1 - last.1);
                n += 1;
                err_model += em;
                err_base += eb;
                if em <= est.2 {
                    inside += 1;
                }
                tracker.train_at(&tr.champion, q, f.t);
            }
            next_minute += 1;
        }
    }
    let nf = n as f64;
    TrainResult { samples: n, err_model: if n > 0 { err_model / nf } else { 0.0 }, err_baseline: if n > 0 { err_base / nf } else { 0.0 }, inside: if n > 0 { inside as f64 / nf } else { 0.0 } }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::riot::types::*;

    #[test]
    fn mask_round_trip() {
        let mut m = vec![0f32; 64 * 64];
        for y in 0..32 {
            for x in 0..64 {
                m[y * 64 + x] = 1.0; // top half visible
            }
        }
        let s = pack_mask(&m, 64);
        assert_eq!(s.len(), 172); // 128 bytes base64
        let v = unpack_mask(&s);
        assert_eq!(v(0.1, 0.2), 1.0);
        assert_eq!(v(0.9, 0.7), 0.0);
        assert_eq!(v(-5.0, 9.0), 0.0); // clamped
        assert_eq!(riot_to_minimap(Position { x: -120.0, y: -120.0 }), (0.0, 0.999));
    }

    #[test]
    fn recorder_saves_and_loads() {
        let d = tempfile::tempdir().unwrap();
        let mask = vec![1f32; 64 * 64];
        let mut r = ObsRecorder::default();
        r.add(ObsInput { t: 1.0, d: vec![], mask: Some(&mask), mask_size: 64, e: vec![], dr: None, br: None }); // not started: ignored
        r.start("ORDER");
        for k in 0..150 {
            let t = k as f64 * 0.5 + 0.04;
            r.add(ObsInput { t, d: vec![("Zed".into(), 0.5, 0.5)], mask: Some(&mask), mask_size: 64, e: vec![("Zed".into(), "MIDDLE".into(), 0, 345.0, 1200.0)], dr: Some(30.0), br: None });
        }
        assert_eq!(r.log.as_ref().unwrap().frames.len(), 75); // one per second
        assert!(r.save(d.path(), Some(1)).is_none()); // < 120 frames
        for k in 0..60 {
            r.add(ObsInput { t: 100.0 + k as f64, d: vec![], mask: Some(&mask), mask_size: 64, e: vec![], dr: None, br: None });
        }
        let f = r.save(d.path(), Some(7_000_123)).unwrap();
        assert_eq!(f, d.path().join("7000123.json.gz"));
        let log = load_obs(&f).unwrap();
        assert_eq!(log.game_id, Some(7_000_123));
        assert_eq!(log.enemy_team, "CHAOS");
        assert_eq!(log.frames[1].t, 1.0);
        assert_eq!(log.frames[0].e[0].1, "MIDDLE");
        // JSON shape like TS: tuples as arrays, gameId/enemyTeam camelCase.
        let text = crate::jsutil::json_compact(&log);
        assert!(text.starts_with("{\"gameId\":7000123,\"team\":\"ORDER\",\"enemyTeam\":\"CHAOS\",\"frames\":[{\"t\":0,\"d\":[[\"Zed\",0.5,0.5]]"), "{}", &text[..120]);
    }

    /// Tracker stub: enemy always fogged, estimate fixed, last at (0.1, 0.1).
    struct Stub {
        trained: Vec<(String, (f64, f64), f64)>,
        updates: usize,
    }
    impl ObsTracker for Stub {
        fn update(&mut self, _t: f64, _d: &[(String, f64, f64)], visible: &dyn Fn(f64, f64) -> f64, enemies: &[ObsEnemy], team: &str, _dr: Option<f64>, _br: Option<f64>) -> Vec<TrackView> {
            assert_eq!(team, "CHAOS");
            assert_eq!(visible(0.5, 0.5), 1.0);
            self.updates += 1;
            enemies.iter().map(|e| TrackView { champion: e.champion.clone(), in_fog: true, estimate: Some((0.5, 0.5, 0.1)), last: Some((0.1, 0.1)) }).collect()
        }
        fn train_at(&mut self, c: &str, q: (f64, f64), t: f64) {
            self.trained.push((c.into(), q, t));
        }
    }

    #[test]
    fn trains_at_minute_marks() {
        let m = pack_mask(&vec![1f32; 64 * 64], 64);
        let frames = (0..=130).map(|t| ObsFrame { t: t as f64, d: vec![], m: m.clone(), e: vec![("Zed".into(), "MIDDLE".into(), 0, 345.0, 0.0)], dr: None, br: None }).collect();
        let log = ObsLog { game_id: Some(1), team: "ORDER".into(), enemy_team: "CHAOS".into(), frames };
        let mut s = Stub { trained: vec![], updates: 0 };
        let r = train_from_observations(&log, &mut s, &|c, minute| if c == "Zed" && minute == 1 { Some((0.55, 0.5)) } else if minute == 2 { Some((0.5, 0.9)) } else { None });
        assert_eq!(s.updates, 131);
        assert_eq!(r.samples, 2);
        assert_eq!(s.trained[0], ("Zed".into(), (0.55, 0.5), 60.0));
        assert!((r.inside - 0.5).abs() < 1e-9);
        assert!((r.err_model - (0.05 + 0.4) / 2.0).abs() < 1e-9);
    }

    #[test]
    fn truth_from_timeline() {
        let p = |id: i64, name: &str| MatchParticipantDto { participant_id: id, champion_name: name.into(), ..Default::default() };
        let m = MatchDto { info: MatchInfo { participants: vec![p(1, "MonkeyKing"), p(2, "Ahri")], ..Default::default() }, ..Default::default() };
        let mut pf = std::collections::BTreeMap::new();
        pf.insert("1".to_string(), ParticipantFrameDto { participant_id: 1, position: Some(Position { x: 14870.0, y: 14980.0 }), ..Default::default() });
        pf.insert("2".to_string(), ParticipantFrameDto { participant_id: 2, position: None, ..Default::default() });
        let tl = TimelineDto { info: TimelineInfo { frames: vec![TimelineFrameDto::default(), TimelineFrameDto { participant_frames: pf, ..Default::default() }], ..Default::default() }, ..Default::default() };
        let fix = |n: &str| if n == "MonkeyKing" { "Wukong".to_string() } else { n.to_string() };
        assert_eq!(timeline_truth(&m, &tl, "Wukong", 1, &fix), Some((0.999, 0.0)));
        assert_eq!(timeline_truth(&m, &tl, "Ahri", 1, &fix), None);
        assert_eq!(timeline_truth(&m, &tl, "Wukong", 5, &fix), None);
    }
}
