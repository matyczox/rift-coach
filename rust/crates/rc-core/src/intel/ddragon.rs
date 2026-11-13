//! Static data from Riot's Data Dragon CDN, cached on disk per patch
//! (`src/main/intel/ddragon.ts`). Same cache files as the TS app:
//! `<dir>/latest.txt` and `<dir>/<version>-<locale>.json` = `{items, champions, itemsEn}`.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

pub const CDN: &str = "https://ddragon.leagueoflegends.com";

#[derive(Clone, Debug, PartialEq)]
pub struct ItemData {
    pub id: i64,
    pub name: String,
    /// Full shop price.
    pub total: f64,
    /// Final item (nothing builds from it) and not a consumable/trinket/boots-tier-1.
    pub completed: bool,
    pub tags: Vec<String>,
    /// English name (models often answer in English).
    pub name_en: Option<String>,
    pub flat_ms: f64,
    pub pct_ms: f64,
    /// Bought in the shop (false: items that evolve by themselves, e.g. jungle pets, support quest items).
    pub purchasable: bool,
    /// Recipe: component item ids (Data Dragon "from").
    pub from: Vec<i64>,
}

#[derive(Clone, Debug, Default)]
pub struct StaticData {
    pub version: String,
    pub items: HashMap<i64, ItemData>,
    /// Display name (and id) -> ddragon id ("Twisted Fate" -> "TwistedFate").
    pub champion_ids: HashMap<String, String>,
    /// Display name -> (move speed, attack range).
    pub champion_stats: HashMap<String, (f64, f64)>,
    /// LCU championId -> display name.
    pub champion_by_key: HashMap<i64, String>,
    /// Ddragon id ("MonkeyKing") -> English display name ("Wukong").
    pub name_by_id: HashMap<String, String>,
    /// Display name -> classes from the game data ("Ahri" -> ["Mage", "Assassin"]).
    pub champion_tags: HashMap<String, Vec<String>>,
}

pub fn champion_icon_url(version: &str, ddragon_id: &str) -> String {
    format!("{CDN}/cdn/{version}/img/champion/{ddragon_id}.png")
}

/// "16.22.1" -> "16.22"
pub fn patch_of(version: &str) -> String {
    version.split('.').take(2).collect::<Vec<_>>().join(".")
}

pub fn parse_items(j: &Value) -> HashMap<i64, ItemData> {
    let mut items = HashMap::new();
    let Some(data) = j.get("data").and_then(Value::as_object) else { return items };
    for (id, it) in data {
        let Ok(nid) = id.parse::<i64>() else { continue };
        // Summoner's Rift shop items only (Arena/other-mode copies use 6-digit ids or other maps).
        let map11_off = it.pointer("/maps/11").and_then(Value::as_bool) == Some(false);
        if nid >= 100000 || map11_off || it.get("inStore").and_then(Value::as_bool) == Some(false) || it.get("requiredAlly").is_some_and(|v| !v.is_null()) {
            continue;
        }
        let tags: Vec<String> = it.get("tags").and_then(Value::as_array).map(|a| a.iter().filter_map(|t| t.as_str().map(String::from)).collect()).unwrap_or_default();
        let has = |t: &str| tags.iter().any(|x| x == t);
        let total = it.pointer("/gold/total").and_then(Value::as_f64).unwrap_or(0.0);
        let purchasable = it.pointer("/gold/purchasable").and_then(Value::as_bool).unwrap_or(false);
        let consumed = it.get("consumed").and_then(Value::as_bool).unwrap_or(false);
        let builds_into = it.get("into").and_then(Value::as_array).is_some_and(|a| !a.is_empty());
        let completed = purchasable && !consumed && !builds_into && !has("Trinket") && !has("Consumable") && if has("Boots") { total >= 900.0 } else { total >= 2000.0 };
        let stat = |k: &str| it.get("stats").and_then(|s| s.get(k)).and_then(Value::as_f64).unwrap_or(0.0);
        items.insert(
            nid,
            ItemData {
                id: nid,
                name: it.get("name").and_then(Value::as_str).unwrap_or("").into(),
                total,
                completed,
                tags,
                name_en: None,
                flat_ms: stat("FlatMovementSpeedMod"),
                pct_ms: stat("PercentMovementSpeedMod"),
                purchasable,
                from: it.get("from").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().and_then(|s| s.parse().ok())).collect()).unwrap_or_default(),
            },
        );
    }
    items
}

/// Build StaticData from the cached raw JSON `{items, champions, itemsEn}`.
pub fn from_raw(version: &str, raw: &Value) -> StaticData {
    let mut sd = StaticData { version: version.into(), ..Default::default() };
    if let Some(champs) = raw.pointer("/champions/data").and_then(Value::as_object) {
        for c in champs.values() {
            let (Some(id), Some(name)) = (c.get("id").and_then(Value::as_str), c.get("name").and_then(Value::as_str)) else { continue };
            sd.champion_ids.insert(name.into(), id.into());
            sd.champion_ids.insert(id.into(), id.into());
            let ms = c.pointer("/stats/movespeed").and_then(Value::as_f64).unwrap_or(0.0);
            let range = c.pointer("/stats/attackrange").and_then(Value::as_f64).unwrap_or(0.0);
            sd.champion_stats.insert(name.into(), (ms, range));
            if let Some(k) = c.get("key").and_then(Value::as_str).and_then(|k| k.parse().ok()) {
                sd.champion_by_key.insert(k, name.into());
            }
            sd.name_by_id.insert(id.into(), name.into());
            let tags: Vec<String> = c.get("tags").and_then(Value::as_array).map(|a| a.iter().filter_map(|t| t.as_str().map(String::from)).collect()).unwrap_or_default();
            sd.champion_tags.insert(name.into(), tags);
        }
    }
    sd.items = parse_items(raw.get("items").unwrap_or(&Value::Null));
    if let Some(en) = raw.pointer("/itemsEn/data").and_then(Value::as_object) {
        for (id, it) in en {
            if let (Ok(n), Some(name)) = (id.parse::<i64>(), it.get("name").and_then(Value::as_str)) {
                if let Some(x) = sd.items.get_mut(&n) {
                    x.name_en = Some(name.into());
                }
            }
        }
    }
    sd
}

pub fn web_client() -> reqwest::Client {
    reqwest::Client::builder().timeout(Duration::from_secs(10)).build().expect("http client")
}

async fn fetch_json(c: &reqwest::Client, url: &str) -> Result<Value> {
    let res = c.get(url).send().await?;
    if !res.status().is_success() {
        return Err(anyhow!("{url}: HTTP {}", res.status().as_u16()));
    }
    Ok(res.json().await?)
}

/// Load current patch data (network once per patch, then disk). Falls back to the newest cache offline.
pub async fn load_static_data(cache_dir: &Path, locale: &str) -> Result<StaticData> {
    std::fs::create_dir_all(cache_dir)?;
    let c = web_client();
    let latest_file = cache_dir.join("latest.txt");
    let version = match fetch_json(&c, &format!("{CDN}/api/versions.json")).await.ok().and_then(|v| v.get(0).and_then(Value::as_str).map(String::from)) {
        Some(v) => {
            let _ = std::fs::write(&latest_file, &v);
            v
        }
        None => std::fs::read_to_string(&latest_file).map_err(|_| anyhow!("Data Dragon unreachable and no cache"))?.trim().to_string(),
    };
    let file = cache_dir.join(format!("{version}-{locale}.json"));
    let mut raw: Value = match std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()) {
        Some(v) => v,
        None => {
            let (u1, u2, u3) = (
                format!("{CDN}/cdn/{version}/data/{locale}/item.json"),
                format!("{CDN}/cdn/{version}/data/en_US/champion.json"),
                format!("{CDN}/cdn/{version}/data/en_US/item.json"),
            );
            let (items, champions, items_en) = tokio::join!(fetch_json(&c, &u1), fetch_json(&c, &u2), fetch_json(&c, &u3));
            let raw = json!({ "items": items?, "champions": champions?, "itemsEn": items_en? });
            std::fs::write(&file, serde_json::to_string(&raw)?)?;
            raw
        }
    };
    if raw.get("itemsEn").is_none_or(Value::is_null) {
        if let Ok(en) = fetch_json(&c, &format!("{CDN}/cdn/{version}/data/en_US/item.json")).await {
            raw["itemsEn"] = en;
            let _ = std::fs::write(&file, serde_json::to_string(&raw)?);
        }
    }
    Ok(from_raw(&version, &raw))
}

/// League movement speed soft caps applied to the raw value.
pub fn soft_cap_move_speed(raw: f64) -> f64 {
    if raw > 490.0 {
        raw * 0.5 + 230.0
    } else if raw > 415.0 {
        raw * 0.8 + 83.0
    } else if raw < 220.0 {
        raw * 0.5 + 110.0
    } else {
        raw
    }
}

/// Out-of-combat move speed estimate from base speed and item bonuses (no runes/summoners/buffs).
pub fn estimate_move_speed(base: f64, items: &[(f64, f64)]) -> f64 {
    let flat: f64 = items.iter().map(|i| i.0).sum();
    let pct: f64 = items.iter().map(|i| i.1).sum();
    crate::intel::js_round(soft_cap_move_speed((base + flat) * (1.0 + pct)))
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Same items as tests/intel.test.ts.
    pub fn test_items() -> HashMap<i64, ItemData> {
        parse_items(&json!({ "data": {
            "1055": { "name": "Ostrze Dorana", "gold": { "total": 450, "purchasable": true }, "tags": ["Damage"] },
            "6333": { "name": "Taniec Śmierci", "gold": { "total": 3300, "purchasable": true }, "tags": ["Damage"] },
            "3174": { "name": "Opancerzone Natarcie", "gold": { "total": 1000, "purchasable": true }, "tags": ["Boots"] },
            "1036": { "name": "Długi Miecz", "gold": { "total": 350, "purchasable": true }, "into": ["6333"], "tags": ["Damage"] },
            "223333": { "name": "Arena copy", "gold": { "total": 3300, "purchasable": true } },
            "3340": { "name": "Trinket", "gold": { "total": 0, "purchasable": true }, "tags": ["Trinket"], "maps": { "11": false } }
        }}))
    }

    #[test]
    fn flags_completed_items_and_components() {
        let items = test_items();
        assert!(items[&6333].completed);
        assert!(!items[&1036].completed);
        assert!(!items[&1055].completed);
        assert!(items[&3174].completed); // tier-2 boots
        assert!(!items.contains_key(&223333));
        assert!(!items.contains_key(&3340));
        assert!(items[&6333].purchasable);
        let pet = parse_items(&json!({ "data": { "1101": { "name": "Pet", "gold": { "total": 450, "purchasable": false } } } }));
        assert!(!pet[&1101].purchasable && !pet[&1101].completed);
    }

    #[test]
    fn champions_and_move_speed() {
        let raw = json!({ "champions": { "data": { "MonkeyKing": { "id": "MonkeyKing", "key": "62", "name": "Wukong", "stats": { "movespeed": 340, "attackrange": 175 } } } }, "items": { "data": {} } });
        let sd = from_raw("16.22.1", &raw);
        assert_eq!(sd.champion_ids["Wukong"], "MonkeyKing");
        assert_eq!(sd.champion_by_key[&62], "Wukong");
        assert_eq!(sd.name_by_id["MonkeyKing"], "Wukong");
        assert_eq!(patch_of("16.22.1"), "16.22");
        assert_eq!(estimate_move_speed(340.0, &[(45.0, 0.0)]), 385.0);
        assert_eq!(estimate_move_speed(400.0, &[(45.0, 0.1)]), (489.5f64 * 0.8 + 83.0 + 0.5).floor());
    }
}
