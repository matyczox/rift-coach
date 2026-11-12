//! Live Client Data API poller (`src/main/sources/liveApi.ts`).

use crate::sources::http::get_json;
use crate::types::{GameInfo, SourceState};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

pub const URL_ALL: &str = "https://127.0.0.1:2999/liveclientdata/allgamedata";
/// `new LiveApi()` default interval.
pub const POLL_MS: u64 = 500;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivePlayer {
    pub riot_id: Option<String>,
    pub summoner_name: Option<String>,
    #[serde(default)]
    pub current_gold: f64,
    #[serde(default)]
    pub level: i64,
    #[serde(default)]
    pub champion_stats: Map<String, Value>,
}

impl ActivePlayer {
    pub fn stat(&self, k: &str) -> Option<f64> {
        self.champion_stats.get(k).and_then(Value::as_f64)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveItem {
    #[serde(rename = "itemID", default)]
    pub item_id: i64,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub price: f64,
    #[serde(default)]
    pub slot: i64,
    #[serde(default)]
    pub count: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scores {
    #[serde(default)]
    pub kills: i64,
    #[serde(default)]
    pub deaths: i64,
    #[serde(default)]
    pub assists: i64,
    #[serde(default)]
    pub creep_score: i64,
    #[serde(default)]
    pub ward_score: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LivePlayer {
    pub champion_name: String,
    pub raw_champion_name: Option<String>,
    pub team: String,
    #[serde(default)]
    pub position: String,
    #[serde(default)]
    pub is_dead: bool,
    #[serde(default)]
    pub respawn_timer: f64,
    #[serde(default)]
    pub level: i64,
    pub riot_id: Option<String>,
    pub riot_id_game_name: Option<String>,
    pub summoner_name: Option<String>,
    pub is_bot: Option<bool>,
    #[serde(default)]
    pub items: Vec<LiveItem>,
    #[serde(default)]
    pub scores: Scores,
    /// Object in API order (summonerSpellOne, summonerSpellTwo).
    #[serde(default)]
    pub summoner_spells: Option<Map<String, Value>>,
}

impl LivePlayer {
    pub fn spell_names(&self) -> Vec<String> {
        self.summoner_spells
            .as_ref()
            .map(|m| m.values().filter_map(|v| v.get("displayName").and_then(Value::as_str).map(String::from)).collect())
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Events {
    #[serde(rename = "Events", default)]
    pub events: Vec<Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameData {
    #[serde(default)]
    pub game_mode: String,
    #[serde(default)]
    pub game_time: f64,
    #[serde(default)]
    pub map_number: i64,
}

/// Subset of /liveclientdata/allgamedata we rely on.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AllGameData {
    pub active_player: ActivePlayer,
    pub all_players: Vec<LivePlayer>,
    #[serde(default)]
    pub events: Events,
    pub game_data: GameData,
}

/// Data Dragon id from rawChampionName ("game_character_displayname_MonkeyKing"), else the name without non-letters.
pub fn dd_id(p: &LivePlayer) -> String {
    if let Some(raw) = &p.raw_champion_name {
        if let Some(i) = raw.find("displayname_") {
            let id = &raw[i + "displayname_".len()..];
            if !id.is_empty() {
                return id.to_string();
            }
        }
    }
    p.champion_name.chars().filter(|c| c.is_ascii_alphabetic()).collect()
}

fn id_of(p: &LivePlayer) -> Option<&str> {
    p.riot_id.as_deref().or(p.summoner_name.as_deref())
}

pub fn to_game_info(d: &AllGameData) -> GameInfo {
    let me_id = d.active_player.riot_id.as_deref().or(d.active_player.summoner_name.as_deref());
    let me = d.all_players.iter().find(|p| id_of(p) == me_id && me_id.is_some());
    let my_team = me.map(|p| p.team.as_str()).unwrap_or("ORDER");
    let allies: Vec<&LivePlayer> = d.all_players.iter().filter(|p| p.team == my_team).collect();
    let enemies: Vec<&LivePlayer> = d.all_players.iter().filter(|p| p.team != my_team).collect();
    GameInfo {
        game_mode: d.game_data.game_mode.clone(),
        game_time: d.game_data.game_time,
        map_number: d.game_data.map_number,
        active_champion: me.map(|p| p.champion_name.clone()),
        allies: allies.iter().map(|p| p.champion_name.clone()).collect(),
        enemies: enemies.iter().map(|p| p.champion_name.clone()).collect(),
        ally_ids: Some(allies.iter().map(|p| dd_id(p)).collect()),
        enemy_ids: Some(enemies.iter().map(|p| dd_id(p)).collect()),
    }
}

/// Localised championName -> English display name via rawChampionName (normalizeChampionNames).
pub fn normalize_champion_names(d: &AllGameData, name_by_id: impl Fn(&str) -> Option<String>) -> AllGameData {
    let mut out = d.clone();
    for p in &mut out.all_players {
        let id = p.raw_champion_name.as_deref().and_then(|r| r.find("displayname_").map(|i| r[i + 12..].to_string()));
        if let Some(en) = id.and_then(|id| name_by_id(&id)) {
            if en != p.champion_name {
                p.champion_name = en;
            }
        }
    }
    out
}

/// Parse a response; during loading the API answers but gameData may be incomplete.
pub fn parse(v: Value) -> Option<AllGameData> {
    if !v.get("gameData").is_some_and(Value::is_object) || !v.get("allPlayers").is_some_and(Value::is_array) {
        return None;
    }
    serde_json::from_value(v).ok()
}

#[derive(Default)]
pub struct LiveShared {
    pub state: SourceState,
    pub last: Option<AllGameData>,
}

pub enum LiveEvent {
    State(SourceState),
    /// Fresh snapshot and the wall-clock time it was received.
    Data(Box<AllGameData>, Instant),
}

/// Start polling; state/last are readable at any time, events go to the channel.
pub fn spawn(rt: &tokio::runtime::Handle, interval_ms: u64) -> (Arc<Mutex<LiveShared>>, mpsc::UnboundedReceiver<LiveEvent>) {
    let shared = Arc::new(Mutex::new(LiveShared::default()));
    let (tx, rx) = mpsc::unbounded_channel();
    let sh = shared.clone();
    rt.spawn(async move {
        loop {
            let started = Instant::now();
            let res = get_json::<Value>(URL_ALL, None, 1500).await.ok().and_then(parse);
            let changed = {
                let mut s = sh.lock().unwrap();
                let new_state = if res.is_some() { SourceState::Connected } else { SourceState::Waiting };
                s.last = res.clone();
                let ch = s.state != new_state;
                s.state = new_state;
                ch.then_some(new_state)
            };
            if let Some(st) = changed {
                let _ = tx.send(LiveEvent::State(st));
            }
            if let Some(d) = res {
                let _ = tx.send(LiveEvent::Data(Box::new(d), Instant::now()));
            }
            if tx.is_closed() {
                break;
            }
            let wait = Duration::from_millis(interval_ms).saturating_sub(started.elapsed());
            tokio::time::sleep(wait.max(Duration::from_millis(10))).await;
        }
    });
    (shared, rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn fixture() -> AllGameData {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/allgamedata.json")).unwrap();
        parse(serde_json::from_str(&text).unwrap()).expect("fixture parses")
    }

    #[test]
    fn splits_teams_relative_to_active_player() {
        let g = to_game_info(&fixture());
        assert_eq!(g.active_champion.as_deref(), Some("Ambessa"));
        assert!(g.allies.contains(&"Fizz".to_string()));
        assert_eq!(g.enemies, ["Lulu", "Tristana", "Xin Zhao", "Warwick", "Twisted Fate"]);
        assert_eq!(g.enemy_ids.as_ref().unwrap()[2], "XinZhao");
        assert_eq!(g.enemy_ids.as_ref().unwrap()[4], "TwistedFate");
        let js = serde_json::to_value(&g).unwrap();
        assert!(js.get("allyIds").is_some() && js.get("gameMode").is_some());
    }

    #[test]
    fn loading_response_is_rejected() {
        assert!(parse(serde_json::json!({ "activePlayer": {}, "gameData": {} })).is_none());
    }

    #[test]
    fn normalizes_localised_names() {
        let mut d = fixture();
        d.all_players[7].champion_name = "Xin Zhao (PL)".into();
        let n = normalize_champion_names(&d, |id| (id == "XinZhao").then(|| "Xin Zhao".to_string()));
        assert_eq!(n.all_players[7].champion_name, "Xin Zhao");
    }

    #[test]
    fn spells_in_api_order() {
        let d = fixture();
        assert_eq!(d.all_players[0].spell_names().len(), 2);
    }
}
