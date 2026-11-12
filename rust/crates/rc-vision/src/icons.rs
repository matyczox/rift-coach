//! Champion square icons from Data Dragon (port of `src/main/vision/icons.ts`).
//!
//! Reading only: files are cached on disk as `<cache_dir>/<version>-<ddragonId>.png` (the TS
//! app's layout under `%APPDATA%\Rift Coach\ddragon\icons`). Downloading missing icons is left
//! to the caller (the app already has an HTTP client); [`icon_url`] gives the URL.

use crate::image::{load_png, ImageError, Rgba};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub fn icon_url(version: &str, ddragon_id: &str) -> String {
    format!("https://ddragon.leagueoflegends.com/cdn/{version}/img/champion/{ddragon_id}.png")
}

pub fn icon_path(cache_dir: &Path, version: &str, ddragon_id: &str) -> PathBuf {
    cache_dir.join(format!("{version}-{ddragon_id}.png"))
}

/// Decode a cached icon.
pub fn load_champion_icon(cache_dir: &Path, version: &str, ddragon_id: &str) -> Result<Rgba, ImageError> {
    load_png(icon_path(cache_dir, version, ddragon_id))
}

/// Champion display name -> Data Dragon id, from a `champion.json` `data` object.
#[derive(Clone, Debug, Default)]
pub struct ChampionIds {
    map: HashMap<String, String>,
}

impl ChampionIds {
    /// `champions` = the parsed `champion.json` (an object with a `data` map).
    pub fn from_champion_json(champions: &serde_json::Value) -> ChampionIds {
        let mut map = HashMap::new();
        if let Some(data) = champions.get("data").and_then(|d| d.as_object()) {
            for c in data.values() {
                let (Some(id), Some(name)) = (c.get("id").and_then(|v| v.as_str()), c.get("name").and_then(|v| v.as_str())) else { continue };
                map.insert(name.to_string(), id.to_string());
                map.insert(id.to_string(), id.to_string());
            }
        }
        ChampionIds { map }
    }

    /// Exact name/id, else the first word ("Nunu i Willump" -> "Nunu & Willump" -> "Nunu"),
    /// else the name itself (same fallback chain as scripts/dev/detect-cache.ts).
    pub fn resolve(&self, name: &str) -> String {
        if let Some(id) = self.map.get(name) {
            return id.clone();
        }
        let first = name.split(' ').next().unwrap_or(name);
        let mut keys: Vec<&String> = self.map.keys().collect();
        keys.sort();
        for k in keys {
            if k.split([' ', '&']).next() == Some(first) {
                return self.map[k].clone();
            }
        }
        name.to_string()
    }
}
