//! Shared helpers for the dev examples: harvested replay sessions under
//! `%APPDATA%\Rift Coach\recordings\<session>` (read-only) and the Data Dragon cache.
#![allow(dead_code)]

use rc_vision::detect::{build_template, DetectParams, Team, Template};
use rc_vision::icons::{load_champion_icon, ChampionIds};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub fn root() -> PathBuf {
    PathBuf::from(std::env::var("APPDATA").expect("APPDATA")).join("Rift Coach")
}

/// Session dir from an argument: absolute path or a session id under recordings/.
pub fn session_dir(arg: &str) -> PathBuf {
    let p = PathBuf::from(arg);
    if p.is_dir() {
        p
    } else {
        root().join("recordings").join(arg)
    }
}

pub struct Ddragon {
    pub version: String,
    pub raw: serde_json::Value,
    pub ids: ChampionIds,
}

pub fn ddragon() -> Ddragon {
    let dir = root().join("ddragon");
    let version = std::fs::read_to_string(dir.join("latest.txt")).expect("ddragon latest.txt").trim().to_string();
    let raw: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join(format!("{version}-pl_PL.json"))).expect("ddragon json")).unwrap();
    let ids = ChampionIds::from_champion_json(&raw["champions"]);
    Ddragon { version, raw, ids }
}

pub struct Meta {
    pub allies: Vec<String>,
    pub enemies: Vec<String>,
}

pub fn meta(dir: &Path) -> Meta {
    let m: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("meta.json")).unwrap()).unwrap();
    let list = |k: &str| m[k].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    Meta { allies: list("allies"), enemies: list("enemies") }
}

/// Templates exactly as scripts/dev/detect-cache.ts builds them (allies = ORDER, enemies = CHAOS).
pub fn templates(dd: &Ddragon, meta: &Meta, width: usize, p: &DetectParams) -> Vec<Template> {
    let icons = root().join("ddragon").join("icons");
    let mut out = Vec::new();
    for (team, list) in [(Team::Order, &meta.allies), (Team::Chaos, &meta.enemies)] {
        for c in list {
            let id = dd.ids.resolve(c);
            let icon = load_champion_icon(&icons, &dd.version, &id).unwrap_or_else(|e| panic!("icon {c} ({id}): {e}"));
            out.push(build_template(c, team, &icon, width, p));
        }
    }
    out
}

/// Frame times (ms) in a session sub-folder, sorted.
pub fn frame_times(dir: &Path, sub: &str) -> Vec<u64> {
    let mut v: Vec<u64> = std::fs::read_dir(dir.join(sub))
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok()?.strip_suffix(".png")?.parse().ok())
        .collect();
    v.sort();
    v
}

#[derive(serde::Deserialize, Clone, Debug)]
pub struct CachedDet {
    pub c: String,
    pub t: String,
    pub x: f64,
    pub y: f64,
    pub s: f64,
}

pub fn cached(dir: &Path, sub: &str) -> BTreeMap<u64, Vec<CachedDet>> {
    let raw: BTreeMap<String, Vec<CachedDet>> = serde_json::from_slice(&std::fs::read(dir.join(format!("detections-{sub}.json"))).unwrap()).unwrap();
    raw.into_iter().map(|(k, v)| (k.parse().unwrap(), v)).collect()
}
