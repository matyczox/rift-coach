//! Starter data shipped with the app (`defaults/seed.json`): the author's matchup / rune research and
//! overlay layout, so a fresh install starts with what was learned so far. Only fills what is missing;
//! never overwrites the user's own files or settings.
use serde_json::Value;
use std::path::Path;

static SEED: &str = include_str!("../defaults/seed.json");

pub fn apply(data_dir: &Path, settings: &mut rc_core::settings::SettingsStore) {
    let Ok(seed) = serde_json::from_str::<Value>(SEED) else { return };
    for sub in ["matchups", "runes"] {
        let Some(files) = seed.get(sub).and_then(Value::as_object) else { continue };
        let dir = data_dir.join(sub);
        let _ = std::fs::create_dir_all(&dir);
        for (name, v) in files {
            let p = dir.join(name);
            if !p.exists() && !name.contains(['/', '\\']) {
                if let Ok(txt) = serde_json::to_string_pretty(v) {
                    let _ = std::fs::write(&p, txt);
                }
            }
        }
    }
    if settings.json().get("hudLayout").is_none_or(Value::is_null) {
        if let Some(layout) = seed.get("hudLayout").filter(|v| v.is_object()) {
            let _ = settings.update(&serde_json::json!({ "hudLayout": layout }));
        }
    }
}
