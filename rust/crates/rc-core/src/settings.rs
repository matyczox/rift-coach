//! Settings: same file and schema as the Electron app (`src/main/settings.ts`):
//! `%APPDATA%\Rift Coach\settings.json`, defaults merged shallowly with nested
//! `hotkeys` / `coach` / `riot`, unknown keys kept, numbers clamped.
//!
//! The Riot API key is part of the settings (the UI shows it in its input), but
//! it is never logged: `Settings` has a redacting `Debug`.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

/// `DEFAULT_SETTINGS` from settings.ts, key order preserved.
pub fn defaults() -> Value {
    json!({
        "calibration": null,
        "minimapFps": 10,
        "recordFps": 5,
        "sceneEverySec": 5,
        "autoRecord": false,
        "overlayVisible": true,
        "hotkeys": { "toggleOverlay": "Alt+Shift+O", "toggleRecording": "Alt+Shift+R", "enemyFlash": "Alt+Shift+F" },
        "coach": { "enabled": true, "liveModel": "gpt-6.1-terra", "reviewModel": "claude-opus-5-5", "periodSec": 25, "researchModel": "claude-opus-5-5", "draftModel": "gpt-6.1-terra", "fast": true },
        "riot": { "apiKey": "", "platform": "eun1" },
        "autoRunes": true,
        "launchAtLogin": false
    })
}

/// JS `Math.round` (halves go towards +infinity).
fn js_round(v: f64) -> f64 {
    (v + 0.5).floor()
}

/// `clamp` from settings.ts; non-numbers fall back to `fallback` first.
fn clamp(v: Option<&Value>, fallback: f64, lo: f64, hi: f64) -> Value {
    let x = v.and_then(Value::as_f64).unwrap_or(fallback);
    let r = js_round(x).max(lo).min(hi);
    json!(r as i64)
}

fn merge_obj(base: &Value, over: Option<&Value>) -> Value {
    let mut m = base.as_object().cloned().unwrap_or_default();
    if let Some(Value::Object(o)) = over {
        for (k, v) in o {
            m.insert(k.clone(), v.clone());
        }
    }
    Value::Object(m)
}

/// `sanitize` from settings.ts on raw JSON (keeps keys the Rust side doesn't know).
pub fn sanitize(raw: &Value) -> Value {
    let d = defaults();
    let empty = Map::new();
    let s = raw.as_object().unwrap_or(&empty);
    let mut m = d.as_object().unwrap().clone();
    for (k, v) in s {
        m.insert(k.clone(), v.clone());
    }
    for k in ["hotkeys", "coach", "riot"] {
        let merged = merge_obj(&d[k], s.get(k));
        m.insert(k.into(), merged);
    }
    let coach = m.get_mut("coach").unwrap().as_object_mut().unwrap();
    let period = clamp(coach.get("periodSec"), 25.0, 10.0, 120.0);
    coach.insert("periodSec".into(), period);
    let riot = m.get_mut("riot").unwrap().as_object_mut().unwrap();
    let key = riot.get("apiKey").and_then(Value::as_str).unwrap_or("").trim().to_string();
    riot.insert("apiKey".into(), Value::String(key));
    let minimap = clamp(m.get("minimapFps"), 10.0, 2.0, 20.0);
    let minimap_n = minimap.as_f64().unwrap();
    m.insert("minimapFps".into(), minimap);
    let rec = clamp(m.get("recordFps"), 5.0, 1.0, minimap_n);
    m.insert("recordFps".into(), rec);
    let scene = clamp(m.get("sceneEverySec"), 5.0, 0.0, 60.0);
    m.insert("sceneEverySec".into(), scene);
    Value::Object(m)
}

/// Typed view of the settings the Rust side uses.
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub calibration: Option<Value>,
    pub minimap_fps: u32,
    pub record_fps: u32,
    pub scene_every_sec: u32,
    pub auto_record: bool,
    pub overlay_visible: bool,
    pub hotkeys: Hotkeys,
    pub coach: CoachSettings,
    pub riot: RiotSettings,
    pub auto_runes: bool,
    pub launch_at_login: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Hotkeys {
    pub toggle_overlay: String,
    pub toggle_recording: String,
    pub enemy_flash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CoachSettings {
    pub enabled: bool,
    pub live_model: String,
    pub review_model: String,
    pub period_sec: u32,
    pub research_model: String,
    pub draft_model: String,
    /// Codex fast mode for Terra (faster, uses more of the plan). `None` = on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fast: Option<bool>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RiotSettings {
    pub api_key: String,
    pub platform: String,
}

impl std::fmt::Debug for RiotSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RiotSettings")
            .field("api_key", &if self.api_key.is_empty() { "<none>" } else { "<redacted>" })
            .field("platform", &self.platform)
            .finish()
    }
}

impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field("minimap_fps", &self.minimap_fps)
            .field("coach", &self.coach)
            .field("riot", &self.riot)
            .finish_non_exhaustive()
    }
}

/// Typed view; a field with an unexpected type falls back to its default.
pub fn typed(v: &Value) -> Settings {
    serde_json::from_value(v.clone()).unwrap_or_else(|_| {
        // Repair field by field against the defaults.
        let d = defaults();
        let mut m = d.as_object().unwrap().clone();
        if let Some(o) = v.as_object() {
            for (k, dv) in d.as_object().unwrap() {
                if let Some(x) = o.get(k) {
                    let ok = match (dv, x) {
                        (Value::Object(dm), Value::Object(xm)) => {
                            let mut mm = dm.clone();
                            for (kk, dd) in dm {
                                if let Some(xx) = xm.get(kk) {
                                    if std::mem::discriminant(dd) == std::mem::discriminant(xx) {
                                        mm.insert(kk.clone(), xx.clone());
                                    }
                                }
                            }
                            Some(Value::Object(mm))
                        }
                        (Value::Null, x) => Some(x.clone()),
                        (dv, x) if std::mem::discriminant(dv) == std::mem::discriminant(x) => Some(x.clone()),
                        _ => None,
                    };
                    if let Some(val) = ok {
                        m.insert(k.clone(), val);
                    }
                }
            }
        }
        serde_json::from_value(Value::Object(m)).expect("defaults are valid")
    })
}

/// `%APPDATA%\Rift Coach` (Electron's userData for productName "Rift Coach").
pub fn user_data_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("Rift Coach")
}

pub struct SettingsStore {
    raw: Value,
    value: Settings,
    file: PathBuf,
}

impl SettingsStore {
    pub fn open(dir: &Path) -> Self {
        let _ = std::fs::create_dir_all(dir);
        let file = dir.join("settings.json");
        let parsed = std::fs::read_to_string(&file).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()).unwrap_or(json!({}));
        let raw = sanitize(&parsed);
        let value = typed(&raw);
        SettingsStore { raw, value, file }
    }

    pub fn value(&self) -> &Settings {
        &self.value
    }

    /// Full JSON (incl. keys unknown to Rust), what `getStatus().settings` returns.
    pub fn json(&self) -> &Value {
        &self.raw
    }

    /// `update(patch)`: shallow merge like `{ ...value, ...patch }`, sanitize, write.
    pub fn update(&mut self, patch: &Value) -> std::io::Result<&Value> {
        let mut m = self.raw.as_object().cloned().unwrap_or_default();
        if let Value::Object(p) = patch {
            for (k, v) in p {
                m.insert(k.clone(), v.clone());
            }
        }
        self.raw = sanitize(&Value::Object(m));
        self.value = typed(&self.raw);
        let text = serde_json::to_string_pretty(&self.raw).expect("json");
        std::fs::write(&self.file, text)?;
        Ok(&self.raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_empty() {
        let s = sanitize(&json!({}));
        assert_eq!(s, defaults());
        let t = typed(&s);
        assert_eq!(t.coach.period_sec, 25);
        assert_eq!(t.riot.platform, "eun1");
    }

    #[test]
    fn clamps_and_trims_like_ts() {
        let s = sanitize(&json!({
            "minimapFps": 50, "recordFps": 30, "sceneEverySec": -3,
            "coach": { "periodSec": 4.5, "liveModel": "claude-sonnet-5-5" },
            "riot": { "apiKey": "  RGAPI-x  " },
            "extra": { "kept": true }
        }));
        assert_eq!(s["minimapFps"], 20);
        assert_eq!(s["recordFps"], 20); // clamped to minimapFps
        assert_eq!(s["sceneEverySec"], 0);
        assert_eq!(s["coach"]["periodSec"], 10);
        assert_eq!(s["coach"]["liveModel"], "claude-sonnet-5-5");
        assert_eq!(s["coach"]["reviewModel"], "claude-opus-5-5"); // nested default kept
        assert_eq!(s["riot"]["apiKey"], "RGAPI-x");
        assert_eq!(s["riot"]["platform"], "eun1");
        assert_eq!(s["extra"]["kept"], true);
    }

    #[test]
    fn js_rounding() {
        assert_eq!(sanitize(&json!({"minimapFps": 2.5}))["minimapFps"], 3);
        assert_eq!(sanitize(&json!({"minimapFps": 12.49}))["minimapFps"], 12);
    }

    #[test]
    fn update_is_shallow_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = SettingsStore::open(dir.path());
        st.update(&json!({ "autoRunes": false, "coach": { "enabled": false } })).unwrap();
        assert!(!st.value().auto_runes);
        assert!(!st.value().coach.enabled);
        // Shallow merge like `{ ...value, ...patch }`: other coach keys come back as defaults.
        assert_eq!(st.value().coach.period_sec, 25);
        let st2 = SettingsStore::open(dir.path());
        assert!(!st2.value().auto_runes);
        let text = std::fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(text.starts_with("{\n  \"calibration\": null,"));
    }

    #[test]
    fn debug_redacts_key() {
        let t = typed(&sanitize(&json!({ "riot": { "apiKey": "RGAPI-secret" } })));
        let dbg = format!("{t:?}");
        assert!(!dbg.contains("RGAPI-secret"));
        assert!(dbg.contains("<redacted>"));
    }

    #[test]
    fn wrong_types_fall_back() {
        let t = typed(&sanitize(&json!({ "autoRunes": "yes", "hotkeys": { "enemyFlash": 5 } })));
        assert!(t.auto_runes);
        assert_eq!(t.hotkeys.enemy_flash, "Alt+Shift+F");
    }
}
