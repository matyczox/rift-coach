//! Minimal subsets of the Riot Web API DTOs we read (match-v5, account-v1,
//! league-v4, champion-mastery-v4, spectator-v5) — `src/main/riot/types.ts` —
//! plus the opponent profile shapes from `src/shared/intel.ts`.
//! Everything that may be missing in older payloads is optional / defaulted.

use serde::{Deserialize, Serialize, Serializer};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AccountDto {
    pub puuid: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag_line: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LeagueEntryDto {
    /// RANKED_SOLO_5x5 | RANKED_FLEX_SR | ...
    pub queue_type: String,
    pub tier: Option<String>,
    pub rank: Option<String>,
    pub league_points: Option<i64>,
    pub wins: Option<i64>,
    pub losses: Option<i64>,
    pub puuid: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChampionMasteryDto {
    pub puuid: String,
    pub champion_id: i64,
    pub champion_level: i64,
    pub champion_points: i64,
    pub last_play_time: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Challenges {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub solo_kills: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MatchParticipantDto {
    pub puuid: String,
    pub participant_id: i64,
    /// 100 blue (ORDER), 200 red (CHAOS)
    pub team_id: i64,
    pub champion_id: i64,
    /// Internal name, e.g. "MonkeyKing".
    pub champion_name: String,
    /// TOP | JUNGLE | MIDDLE | BOTTOM | UTILITY | ''
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_position: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub individual_position: Option<String>,
    pub win: bool,
    pub kills: i64,
    pub deaths: i64,
    pub assists: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_blood_kill: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_blood_assist: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_minions_killed: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub neutral_minions_killed: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_ended_in_early_surrender: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub riot_id_game_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub riot_id_tagline: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub challenges: Option<Challenges>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MatchMetadata {
    pub match_id: String,
    pub participants: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MatchInfo {
    pub game_creation: f64,
    /// Seconds when gameEndTimestamp is present (patch 11.20+), milliseconds before.
    pub game_duration: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_end_timestamp: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_start_timestamp: Option<f64>,
    pub queue_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub map_id: Option<i64>,
    pub participants: Vec<MatchParticipantDto>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MatchDto {
    pub metadata: MatchMetadata,
    pub info: MatchInfo,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ParticipantFrameDto {
    pub participant_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<Position>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_gold: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minions_killed: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jungle_minions_killed: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TimelineEventDto {
    /// CHAMPION_KILL | ELITE_MONSTER_KILL | WARD_PLACED | ...
    #[serde(rename = "type")]
    pub kind: String,
    /// ms since game start
    pub timestamp: f64,
    /// 0 = minion/tower execute
    #[serde(skip_serializing_if = "Option::is_none")]
    pub killer_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub victim_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assisting_participant_ids: Option<Vec<i64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<Position>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub killer_team_id: Option<i64>,
    /// DRAGON | RIFTHERALD | BARON_NASHOR | HORDE | ATAKHAN
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monster_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monster_sub_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participant_id: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TimelineFrameDto {
    pub timestamp: f64,
    /// Keyed by participant id ("1".."10").
    pub participant_frames: BTreeMap<String, ParticipantFrameDto>,
    pub events: Vec<TimelineEventDto>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TimelineParticipant {
    pub participant_id: i64,
    pub puuid: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TimelineInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame_interval: Option<f64>,
    pub frames: Vec<TimelineFrameDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participants: Option<Vec<TimelineParticipant>>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TimelineDto {
    pub metadata: MatchMetadata,
    pub info: TimelineInfo,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CurrentGameParticipantDto {
    pub puuid: Option<String>,
    pub team_id: i64,
    pub champion_id: i64,
    /// "Name#TAG"
    pub riot_id: Option<String>,
    pub spell1_id: Option<i64>,
    pub spell2_id: Option<i64>,
    pub bot: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CurrentGameInfoDto {
    pub game_id: i64,
    pub game_mode: Option<String>,
    pub game_queue_config_id: Option<i64>,
    pub game_start_time: Option<f64>,
    pub game_length: Option<f64>,
    pub platform_id: Option<String>,
    pub participants: Vec<CurrentGameParticipantDto>,
}

// ---------------- shared shapes (src/shared/intel.ts) ----------------

/// Lane corridor on Summoner's Rift.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Lane {
    #[serde(rename = "TOP")]
    Top,
    #[serde(rename = "MIDDLE")]
    Middle,
    #[serde(rename = "BOTTOM")]
    Bottom,
}

impl Lane {
    pub fn as_str(self) -> &'static str {
        match self {
            Lane::Top => "TOP",
            Lane::Middle => "MIDDLE",
            Lane::Bottom => "BOTTOM",
        }
    }
}

/// `Partial<Record<Lane, number>>` keeping insertion order (serialized as an object).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LaneShares(pub Vec<(Lane, f64)>);

impl LaneShares {
    pub fn get(&self, l: Lane) -> Option<f64> {
        self.0.iter().find(|(k, _)| *k == l).map(|(_, v)| *v)
    }
}

impl Serialize for LaneShares {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut m = s.serialize_map(Some(self.0.len()))?;
        for (k, v) in &self.0 {
            m.serialize_entry(k.as_str(), v)?;
        }
        m.end()
    }
}

impl<'de> Deserialize<'de> for LaneShares {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let m = serde_json::Map::<String, serde_json::Value>::deserialize(d)?;
        let mut out = Vec::new();
        for (k, v) in m {
            let lane = match k.as_str() {
                "TOP" => Lane::Top,
                "MIDDLE" => Lane::Middle,
                "BOTTOM" => Lane::Bottom,
                _ => continue,
            };
            out.push((lane, v.as_f64().unwrap_or(0.0)));
        }
        Ok(LaneShares(out))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JunglePathing {
    /// Share of games starting on top-side / bot-side of their own jungle.
    pub start_top_side: f64,
    pub start_bot_side: f64,
    /// Median first-gank minute and its lane distribution (0..1).
    pub first_gank_minute: Option<f64>,
    pub first_gank_lane: LaneShares,
    /// Share of games where they entered the enemy jungle before 4:00.
    pub early_invade_rate: f64,
    /// Share of first dragons they were present at.
    pub dragon_presence: f64,
    pub sample_games: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentForm {
    pub wins: usize,
    pub losses: usize,
    /// > 0 wins, < 0 losses
    pub streak: i64,
}

/// `OpponentProfile` from shared/intel.ts (same JSON).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpponentProfile {
    pub puuid: String,
    pub riot_id: String,
    pub champion: String,
    /// TOP | JUNGLE | MIDDLE | BOTTOM | UTILITY | UNKNOWN
    pub role: String,
    /// "GOLD II 45 LP"
    pub rank: Option<String>,
    pub games_analyzed: usize,
    /// 0..1, derived from early kills+deaths, solo kills, first blood rate.
    pub aggression: f64,
    pub first_blood_rate: f64,
    pub deaths_before10: f64,
    pub kills_before15: f64,
    pub roam_rate: f64,
    pub champion_games: usize,
    pub champion_win_rate: Option<f64>,
    pub recent_form: RecentForm,
    pub jungle: Option<JunglePathing>,
    /// One-line Polish summary, e.g. "agresywny, start dół, gank bot ~3:20".
    pub summary: String,
    pub fetched_at: f64,
}
