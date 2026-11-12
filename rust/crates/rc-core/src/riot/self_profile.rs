//! Profile of the PLAYER (not the enemies) — `src/main/riot/selfProfile.ts`: habits from
//! their recent Summoner's Rift games (Riot match-v5 + timelines) and recurring lessons
//! from the app's own post-game reviews. The coach gets it in the per-game system prompt,
//! so its calls aim at the player's weak spots.
//!
//! Wiring (app): resolve the player's PUUID from the Riot ID `gameName#tagLine` of the LCU
//! current summoner (NOT the LCU puuid: dev-key PUUIDs differ) — [`build_for_riot_id`];
//! rebuild every [`REBUILD_MS`] and a few minutes after each game; lessons refreshed from
//! the reviews on every refresh. Stored in `<userData>/player-profile.json`.

use super::analyze::{duration_seconds, early_stats, find_participant, timeline_participant_id};
use super::client::{CallOptions, MatchIdsQuery, RiotClient, RiotError};
use super::types::MatchDto;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

/// Ranked solo/flex, draft, swiftplay, quickplay (Summoner's Rift).
pub const QUEUES: [i64; 5] = [420, 440, 400, 480, 490];
pub const MATCHES: usize = 15;
pub const TIMELINES: usize = 8;
/// Rebuild from the Riot API at most this often (unless forced after a game).
pub const REBUILD_MS: f64 = 6.0 * 3600_000.0;
pub const FILE_NAME: &str = "player-profile.json";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChampionGames {
    pub name: String,
    pub games: u32,
    pub wins: u32,
}

/// `PlayerSelfProfile` from shared/types.ts (same JSON).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PlayerSelfProfile {
    pub games: u32,
    pub wins: u32,
    pub main_role: Option<String>,
    pub champions: Vec<ChampionGames>,
    pub kda: f64,
    pub deaths_per_game: f64,
    pub cs_per_min: f64,
    pub vision_per_min: f64,
    pub kill_participation: f64,
    #[serde(rename = "deathsBefore10")]
    pub deaths_before10: Option<f64>,
    pub weaknesses: Vec<String>,
    pub lessons: Vec<String>,
    pub summary: String,
    /// Epoch ms.
    pub updated_at: f64,
}

fn minutes(m: &MatchDto) -> f64 {
    duration_seconds(m) / 60.0
}

/// JS `+x.toFixed(d)`.
fn fixed(v: f64, d: i32) -> f64 {
    let m = 10f64.powi(d);
    (v * m).round() / m
}

fn role_pl(r: &str) -> &str {
    match r {
        "TOP" => "top",
        "JUNGLE" => "jungle",
        "MIDDLE" => "mid",
        "BOTTOM" => "ADC",
        "UTILITY" => "support",
        _ => r,
    }
}

/// A Summoner's Rift game of `puuid` that counts (queue, >= 10 min).
pub fn counts(m: &MatchDto, puuid: &str) -> bool {
    QUEUES.contains(&m.info.queue_id) && minutes(m) >= 10.0 && find_participant(m, puuid).is_some()
}

/// Recent games -> habits. Fails only on fatal API errors (bad key) when listing the matches;
/// single match / timeline failures are skipped.
pub async fn build_self_profile(client: &RiotClient, puuid: &str, reviews: &[Value], now_ms: f64) -> Result<PlayerSelfProfile, RiotError> {
    let o = CallOptions::default();
    let ids = client.get_match_ids(puuid, &MatchIdsQuery { count: Some(30), ..Default::default() }, &o).await?;
    let mut matches: Vec<MatchDto> = Vec::new();
    for id in ids {
        if matches.len() >= MATCHES {
            break;
        }
        if let Ok(Some(m)) = client.get_match(&id, &o).await {
            if counts(&m, puuid) {
                matches.push(m);
            }
        }
    }
    let (mut deaths10, mut tl_count) = (0u32, 0u32);
    for m in matches.iter().take(TIMELINES) {
        let Ok(Some(tl)) = client.get_timeline(&m.metadata.match_id, &o).await else { continue };
        let fallback = find_participant(m, puuid).map(|p| p.participant_id);
        if let Some(pid) = timeline_participant_id(&tl, puuid, fallback) {
            deaths10 += early_stats(&tl, pid).deaths10;
            tl_count += 1;
        }
    }
    let early = (tl_count > 0).then(|| deaths10 as f64 / tl_count as f64);
    Ok(profile_from(&matches, puuid, early, reviews, now_ms))
}

/// Resolve "Name#TAG" with account-v1, then [`build_self_profile`]. `Ok(None)` = account not found.
pub async fn build_for_riot_id(client: &RiotClient, riot_id: &str, reviews: &[Value], now_ms: f64) -> Result<Option<PlayerSelfProfile>, RiotError> {
    let Some(acc) = client.resolve_riot_id(riot_id, &CallOptions::default()).await? else { return Ok(None) };
    if acc.puuid.is_empty() {
        return Ok(None);
    }
    build_self_profile(client, &acc.puuid, reviews, now_ms).await.map(Some)
}

/// Aggregates of counted games (`counts`) + average deaths before 10:00 from the timelines.
pub fn profile_from(matches: &[MatchDto], puuid: &str, deaths_before10: Option<f64>, reviews: &[Value], now_ms: f64) -> PlayerSelfProfile {
    let (mut wins, mut k, mut d, mut a) = (0u32, 0i64, 0i64, 0i64);
    let (mut cs, mut mins, mut vision, mut kp_sum) = (0f64, 0f64, 0f64, 0f64);
    // Insertion order kept so ties sort like the JS Map + stable sort.
    let mut roles: Vec<(String, u32)> = Vec::new();
    let mut champs: Vec<ChampionGames> = Vec::new();
    let mut n = 0u32;
    for m in matches {
        let Some(p) = find_participant(m, puuid) else { continue };
        n += 1;
        let t = minutes(m);
        if p.win {
            wins += 1;
        }
        k += p.kills;
        d += p.deaths;
        a += p.assists;
        mins += t;
        cs += p.total_minions_killed.unwrap_or(0.0) + p.neutral_minions_killed.unwrap_or(0.0);
        vision += p.vision_score.unwrap_or(0.0);
        let team_kills: i64 = m.info.participants.iter().filter(|x| x.team_id == p.team_id).map(|x| x.kills).sum();
        kp_sum += if team_kills != 0 { (p.kills + p.assists) as f64 / team_kills as f64 } else { 0.0 };
        if let Some(pos) = p.team_position.as_deref().filter(|s| !s.is_empty()) {
            match roles.iter_mut().find(|r| r.0 == pos) {
                Some(r) => r.1 += 1,
                None => roles.push((pos.to_string(), 1)),
            }
        }
        let i = champs.iter().position(|c| c.name == p.champion_name).unwrap_or_else(|| {
            champs.push(ChampionGames { name: p.champion_name.clone(), games: 0, wins: 0 });
            champs.len() - 1
        });
        champs[i].games += 1;
        if p.win {
            champs[i].wins += 1;
        }
    }
    roles.sort_by(|x, y| y.1.cmp(&x.1));
    champs.sort_by(|x, y| y.games.cmp(&x.games));
    champs.truncate(5);
    let main_role = roles.first().map(|r| r.0.clone());
    let nf = n as f64;
    let mut p = PlayerSelfProfile {
        games: n,
        wins,
        main_role,
        champions: champs,
        kda: if n > 0 { fixed((k + a) as f64 / d.max(1) as f64, 2) } else { 0.0 },
        deaths_per_game: if n > 0 { fixed(d as f64 / nf, 1) } else { 0.0 },
        cs_per_min: if mins > 0.0 { fixed(cs / mins, 1) } else { 0.0 },
        vision_per_min: if mins > 0.0 { fixed(vision / mins, 2) } else { 0.0 },
        kill_participation: if n > 0 { fixed(kp_sum / nf, 2) } else { 0.0 },
        deaths_before10: deaths_before10.map(|x| fixed(x, 1)),
        weaknesses: Vec::new(),
        lessons: lessons_from(reviews),
        summary: String::new(),
        updated_at: now_ms,
    };
    p.weaknesses = weaknesses(&p);
    p.summary = summarize(&p);
    p
}

/// Weak spots, thresholds per role (supports and junglers don't farm like laners).
pub fn weaknesses(p: &PlayerSelfProfile) -> Vec<String> {
    let mut w = Vec::new();
    let role = p.main_role.as_deref();
    let laner = role.is_some_and(|r| r != "UTILITY" && r != "JUNGLE");
    if laner && p.cs_per_min < 6.5 {
        w.push(format!("niska farma ({} CS/min)", p.cs_per_min));
    }
    if let Some(d10) = p.deaths_before10.filter(|&d| d >= 1.5) {
        w.push(format!("dużo śmierci przed 10. minutą ({d10}/mecz)"));
    }
    if p.deaths_per_game >= 7.0 {
        w.push(format!("dużo śmierci w meczu ({}/mecz)", p.deaths_per_game));
    }
    if p.vision_per_min < if role == Some("UTILITY") { 1.5 } else { 0.6 } {
        w.push(format!("mało wizji ({}/min)", p.vision_per_min));
    }
    if p.kill_participation < 0.45 {
        w.push(format!("mały udział w walkach ({}% KP)", (p.kill_participation * 100.0).round()));
    }
    w
}

/// JS `s.slice(0, 40)` (UTF-16 units).
fn prefix40(s: &str) -> Vec<u16> {
    s.encode_utf16().take(40).collect()
}

/// The most important point of the last few post-game reviews (newest first, no duplicates).
/// `reviews` are MatchReview JSON objects (`createdAt`, `improve`).
pub fn lessons_from(reviews: &[Value]) -> Vec<String> {
    let mut sorted: Vec<&Value> = reviews.iter().collect();
    let created = |r: &Value| r.get("createdAt").and_then(Value::as_f64).unwrap_or(0.0);
    sorted.sort_by(|x, y| created(y).total_cmp(&created(x)));
    let mut out: Vec<String> = Vec::new();
    for r in sorted.into_iter().take(8) {
        let l = r.get("improve").and_then(Value::as_array).and_then(|a| a.first()).and_then(Value::as_str).map(str::trim).unwrap_or("");
        if !l.is_empty() && !out.iter().any(|o| prefix40(o) == prefix40(l)) {
            out.push(l.to_string());
        }
        if out.len() >= 4 {
            break;
        }
    }
    out
}

pub fn summarize(p: &PlayerSelfProfile) -> String {
    let role = p.main_role.as_deref().map(role_pl).unwrap_or("różne role");
    let top = p.champions.iter().take(3).map(|c| c.name.as_str()).collect::<Vec<_>>().join(", ");
    format!(
        "{} ostatnich gier na SR ({}W {}L), głównie {role}{}. KDA {}, {} CS/min, {} śmierci/mecz{}, KP {}%.",
        p.games,
        p.wins,
        p.games - p.wins,
        if top.is_empty() { String::new() } else { format!(", postacie: {top}") },
        p.kda,
        p.cs_per_min,
        p.deaths_per_game,
        p.deaths_before10.map(|d| format!(" ({d} przed 10. min)")).unwrap_or_default(),
        (p.kill_participation * 100.0).round()
    )
}

/// Text block for the coach's per-game system prompt.
pub fn self_profile_for_coach(p: Option<&PlayerSelfProfile>) -> Option<String> {
    let p = p?;
    if p.games == 0 && p.lessons.is_empty() {
        return None;
    }
    let mut lines = vec![format!("Profil gracza (Ty): {}", p.summary)];
    if !p.weaknesses.is_empty() {
        lines.push(format!("Jego słabe strony: {}.", p.weaknesses.join("; ")));
    }
    if !p.lessons.is_empty() {
        lines.push(format!("Najważniejsze uwagi z jego ostatnich analiz:\n- {}", p.lessons.join("\n- ")));
    }
    lines.push("Gdy sytuacja dotyczy jego słabej strony (np. wychodzi daleko bez wizji, a często ginie od ganków), przypomnij mu o tym krótko w poradzie.".into());
    Some(lines.join("\n"))
}

/// True when the profile should be rebuilt from the Riot API now.
pub fn needs_rebuild(p: Option<&PlayerSelfProfile>, now_ms: f64, force: bool) -> bool {
    force || p.is_none_or(|p| now_ms - p.updated_at >= REBUILD_MS)
}

pub fn load_self_profile(file: &Path) -> Option<PlayerSelfProfile> {
    serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()
}

pub fn save_self_profile(file: &Path, p: &PlayerSelfProfile) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(file, crate::jsutil::json_pretty(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn prof(role: Option<&str>) -> PlayerSelfProfile {
        PlayerSelfProfile { games: 10, wins: 5, main_role: role.map(String::from), cs_per_min: 7.0, vision_per_min: 1.0, kill_participation: 0.5, deaths_per_game: 4.0, ..Default::default() }
    }

    #[test]
    fn weakness_thresholds_depend_on_role() {
        assert!(weaknesses(&prof(Some("MIDDLE"))).is_empty());
        let mut p = prof(Some("MIDDLE"));
        p.cs_per_min = 5.2;
        p.deaths_before10 = Some(1.5);
        p.deaths_per_game = 7.0;
        p.vision_per_min = 0.55;
        p.kill_participation = 0.4;
        assert_eq!(
            weaknesses(&p),
            vec!["niska farma (5.2 CS/min)", "dużo śmierci przed 10. minutą (1.5/mecz)", "dużo śmierci w meczu (7/mecz)", "mało wizji (0.55/min)", "mały udział w walkach (40% KP)"]
        );
        // Supports: no farm check, higher vision bar.
        let mut s = prof(Some("UTILITY"));
        s.cs_per_min = 1.0;
        assert_eq!(weaknesses(&s), vec!["mało wizji (1/min)"]);
        let mut j = prof(Some("JUNGLE"));
        j.cs_per_min = 5.0;
        assert!(weaknesses(&j).is_empty());
    }

    #[test]
    fn lessons_newest_first_deduplicated_max_four() {
        let r = |t: f64, s: &str| json!({ "createdAt": t, "improve": [s, "drugi punkt"] });
        let reviews = vec![
            r(1.0, "najstarsza"),
            r(5.0, "  Wardy przed smokiem  "),
            r(4.0, "Wardy przed smokiem (inna końcówka tylko po 40 znakach ... )"),
            r(3.0, "Mniej facecheckow"),
            json!({ "createdAt": 6.0, "improve": [] }),
            r(2.0, "Back przy 1300 g"),
            r(1.5, "Piąta lekcja"),
        ];
        assert_eq!(lessons_from(&reviews), vec!["Wardy przed smokiem", "Wardy przed smokiem (inna końcówka tylko po 40 znakach ... )", "Mniej facecheckow", "Back przy 1300 g"]);
        let dup = vec![r(2.0, "Ta sama lekcja, bardzo długa, ponad czterdzieści znaków: A"), r(1.0, "Ta sama lekcja, bardzo długa, ponad czterdzieści znaków: B")];
        assert_eq!(lessons_from(&dup).len(), 1);
    }

    #[test]
    fn coach_text_and_rebuild_window() {
        assert_eq!(self_profile_for_coach(None), None);
        assert_eq!(self_profile_for_coach(Some(&PlayerSelfProfile::default())), None);
        let lessons_only = PlayerSelfProfile { lessons: vec!["Wardy".into()], ..Default::default() };
        assert!(self_profile_for_coach(Some(&lessons_only)).unwrap().contains("- Wardy"));
        assert!(needs_rebuild(None, 0.0, false));
        let p = PlayerSelfProfile { updated_at: 1000.0, ..Default::default() };
        assert!(!needs_rebuild(Some(&p), 1000.0 + REBUILD_MS - 1.0, false));
        assert!(needs_rebuild(Some(&p), 1000.0 + REBUILD_MS, false));
        assert!(needs_rebuild(Some(&p), 1000.0, true));
    }
}
