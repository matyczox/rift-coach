//! Pure functions: match-v5 match + timeline JSON → opponent profile metrics
//! (`src/main/riot/analyze.ts`).
//!
//! Summoner's Rift coordinates (match-v5 timeline): x right, y up, roughly
//! 0..14870 x 0..14980. Blue (teamId 100, ORDER) base is bottom-left, red (200,
//! CHAOS) top-right. Mid lane runs along x = y, the river along x + y ≈ 14820.
//!
//! Jungle quadrants (lane-relative, identical for both teams): own jungle is
//! below the river for blue and above it for red; "top side" is x < y (between
//! top lane and mid) for BOTH teams, "bot side" x > y. Mirroring for red applies
//! to which half is *own* jungle; the top/bot label is the lane the quadrant
//! borders, which is what matters to the coach.

use super::types::*;
use crate::intel::js_round;

pub const MAP_SIZE: f64 = 14870.0;
pub const RIVER_SUM: f64 = 14820.0;
const RIVER_HALF: f64 = 900.0;
pub const DRAGON_PIT: Position = Position { x: 9866.0, y: 4414.0 };
pub const BARON_PIT: Position = Position { x: 5007.0, y: 10471.0 };
const BASE_EDGE: f64 = 4600.0;
const MIN: f64 = 60_000.0;

/// 100 = blue, 200 = red.
pub type TeamId = i64;

// ---------------- geometry ----------------

pub fn in_base(p: Position) -> Option<TeamId> {
    if p.x < BASE_EDGE && p.y < BASE_EDGE {
        return Some(100);
    }
    if p.x > RIVER_SUM - BASE_EDGE && p.y > RIVER_SUM - BASE_EDGE {
        return Some(200);
    }
    None
}

fn top_area(p: Position, m: f64) -> bool {
    let (x, y) = (p.x, p.y);
    (x < 1900.0 + m && y > 4500.0 - m) || (y > 12900.0 - m && x < 10300.0 + m) || (x < 3800.0 + m && y > 11000.0 - m)
}

/// Whether `p` lies in the given lane corridor (outside both bases); `margin` widens it.
pub fn in_lane_area(p: Position, lane: Lane, margin: f64) -> bool {
    if in_base(p).is_some() {
        return false;
    }
    match lane {
        Lane::Top => top_area(p, margin),
        Lane::Bottom => top_area(Position { x: p.y, y: p.x }, margin),
        Lane::Middle => {
            let s = p.x + p.y;
            (p.x - p.y).abs() < 1200.0 + margin && s > 6600.0 && s < 23000.0
        }
    }
}

pub fn classify_lane(p: Position, margin: f64) -> Option<Lane> {
    [Lane::Top, Lane::Bottom, Lane::Middle].into_iter().find(|&l| in_lane_area(p, l, margin))
}

/// Which team's jungle `p` is in (not lane, not base, not river).
pub fn jungle_owner(p: Position) -> Option<TeamId> {
    if in_base(p).is_some() || classify_lane(p, 0.0).is_some() {
        return None;
    }
    let s = p.x + p.y;
    if s < RIVER_SUM - RIVER_HALF {
        Some(100)
    } else if s > RIVER_SUM + RIVER_HALF {
        Some(200)
    } else {
        None
    }
}

/// Side of `team`'s own jungle `p` is in ("top" borders top lane / "bot"), or None.
pub fn classify_jungle_side(p: Position, team: TeamId) -> Option<&'static str> {
    if jungle_owner(p) != Some(team) {
        return None;
    }
    let n = if team == 100 { p } else { Position { x: MAP_SIZE - p.x, y: MAP_SIZE - p.y } };
    let toward_top = if team == 100 { n.x < n.y } else { n.x > n.y };
    Some(if toward_top { "top" } else { "bot" })
}

pub fn dist(a: Position, b: Position) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

// ---------------- timeline helpers ----------------

pub fn find_participant<'a>(m: &'a MatchDto, puuid: &str) -> Option<&'a MatchParticipantDto> {
    m.info.participants.iter().find(|p| p.puuid == puuid)
}

/// participantId of `puuid` in a timeline (falls back to the match participant id).
pub fn timeline_participant_id(tl: &TimelineDto, puuid: &str, fallback: Option<i64>) -> Option<i64> {
    if let Some(id) = tl.info.participants.as_ref().and_then(|ps| ps.iter().find(|p| p.puuid == puuid)).map(|p| p.participant_id).filter(|&i| i != 0) {
        return Some(id);
    }
    if let Some(i) = tl.metadata.participants.iter().position(|p| p == puuid) {
        return Some(i as i64 + 1);
    }
    fallback
}

pub fn involved(ev: &TimelineEventDto, pid: i64) -> bool {
    ev.killer_id == Some(pid) || ev.victim_id == Some(pid) || ev.assisting_participant_ids.as_ref().is_some_and(|a| a.contains(&pid))
}

fn events<'a>(tl: &'a TimelineDto, kind: &'a str) -> impl Iterator<Item = &'a TimelineEventDto> + 'a {
    tl.info.frames.iter().flat_map(|f| f.events.iter()).filter(move |e| kind.is_empty() || e.kind == kind)
}

fn sorted_events<'a>(tl: &'a TimelineDto, kind: &'a str) -> Vec<&'a TimelineEventDto> {
    let mut v: Vec<_> = events(tl, kind).collect();
    v.sort_by(|a, b| a.timestamp.total_cmp(&b.timestamp));
    v
}

/// Positions of `pid` per frame, ascending.
fn track(tl: &TimelineDto, pid: i64) -> Vec<(f64, Position)> {
    let mut out: Vec<(f64, Position)> = tl.info.frames.iter().filter_map(|f| f.participant_frames.get(&pid.to_string()).and_then(|pf| pf.position).map(|p| (f.timestamp, p))).collect();
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out
}

/// Position of `pid` at frame minute `m` (frame closest to m*60s, within 20 s).
pub fn position_at_minute(tl: &TimelineDto, pid: i64, m: f64) -> Option<Position> {
    let mut best: Option<(f64, Position)> = None;
    for (t, pos) in track(tl, pid) {
        let d = (t - m * MIN).abs();
        if d <= 20_000.0 && best.is_none_or(|b| d < b.0) {
            best = Some((d, pos));
        }
    }
    best.map(|b| b.1)
}

/// Linear interpolation of the per-minute track.
pub fn position_at(tl: &TimelineDto, pid: i64, t: f64) -> Option<Position> {
    let tr = track(tl, pid);
    let mut prev = *tr.first()?;
    if t <= prev.0 {
        return Some(prev.1);
    }
    for &s in &tr {
        if s.0 >= t {
            let k = if s.0 == prev.0 { 0.0 } else { (t - prev.0) / (s.0 - prev.0) };
            return Some(Position { x: prev.1.x + (s.1.x - prev.1.x) * k, y: prev.1.y + (s.1.y - prev.1.y) * k });
        }
        prev = s;
    }
    Some(prev.1)
}

// ---------------- per-game metrics ----------------

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EarlyStats {
    pub kills15: u32,
    pub deaths15: u32,
    pub deaths10: u32,
    /// Kills before 15:00 with no assisting participants.
    pub solo_kills15: u32,
}

pub fn early_stats(tl: &TimelineDto, pid: i64) -> EarlyStats {
    let mut r = EarlyStats::default();
    for e in events(tl, "CHAMPION_KILL") {
        if e.timestamp >= 15.0 * MIN {
            continue;
        }
        if e.killer_id == Some(pid) {
            r.kills15 += 1;
            if e.assisting_participant_ids.as_ref().is_none_or(|a| a.is_empty()) {
                r.solo_kills15 += 1;
            }
        }
        if e.victim_id == Some(pid) {
            r.deaths15 += 1;
            if e.timestamp < 10.0 * MIN {
                r.deaths10 += 1;
            }
        }
    }
    r
}

pub fn is_solo_kill(e: &TimelineEventDto) -> bool {
    e.kind == "CHAMPION_KILL" && e.killer_id.is_some_and(|k| k > 0) && e.assisting_participant_ids.as_ref().is_none_or(|a| a.is_empty())
}

pub fn lane_of_position(team_position: &str) -> Option<Lane> {
    match team_position {
        "TOP" => Some(Lane::Top),
        "MIDDLE" => Some(Lane::Middle),
        "BOTTOM" | "UTILITY" => Some(Lane::Bottom),
        _ => None,
    }
}

/// Laner roamed: took part in a champion kill between 1:30 and 14:00 outside
/// their own lane corridor (+1000 units) and outside their own base.
pub fn roamed_in_game(tl: &TimelineDto, pid: i64, lane: Lane, team: TeamId) -> bool {
    for e in events(tl, "CHAMPION_KILL") {
        let Some(pos) = e.position else { continue };
        if e.timestamp < 90_000.0 || e.timestamp >= 14.0 * MIN || !involved(e, pid) {
            continue;
        }
        if in_base(pos) == Some(team) {
            continue;
        }
        if !in_lane_area(pos, lane, 1000.0) {
            return true;
        }
    }
    false
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FirstGank {
    pub minute: f64,
    pub lane: Lane,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JungleGameStats {
    pub start_side: Option<&'static str>,
    pub first_gank: Option<FirstGank>,
    pub invaded: bool,
    /// None when no dragon was taken in the game.
    pub dragon_present: Option<bool>,
}

const GANK_AFTER: f64 = 150_000.0;
const GANK_UNTIL: f64 = 15.0 * MIN;
const INVADE_BEFORE: f64 = 4.0 * MIN;
const DRAGON_RADIUS: f64 = 2500.0;

pub fn analyze_jungle_game(tl: &TimelineDto, pid: i64, team: TeamId) -> JungleGameStats {
    // Start side: minute-2 position (at/just after the first camp), else minute 1.
    let mut start_side = None;
    for m in [2.0, 1.0] {
        if let Some(side) = position_at_minute(tl, pid, m).and_then(|p| classify_jungle_side(p, team)) {
            start_side = Some(side);
            break;
        }
    }

    // First gank: earliest kill/death/assist in a lane corridor (+600) or first
    // per-minute position inside a lane corridor, after 2:30.
    let mut first: Option<(f64, Lane)> = None;
    let mut consider = |t: f64, lane: Option<Lane>| {
        if let Some(l) = lane {
            if t >= GANK_AFTER && t < GANK_UNTIL && first.is_none_or(|f| t < f.0) {
                first = Some((t, l));
            }
        }
    };
    for e in events(tl, "CHAMPION_KILL") {
        if let Some(p) = e.position {
            if involved(e, pid) {
                consider(e.timestamp, classify_lane(p, 600.0));
            }
        }
    }
    let tr = track(tl, pid);
    for &(t, p) in &tr {
        consider(t, classify_lane(p, 0.0));
    }

    // Invade: any position (frame or own kill event) in the enemy jungle before 4:00.
    let enemy = if team == 100 { 200 } else { 100 };
    let mut invaded = tr.iter().any(|&(t, p)| t > 0.0 && t < INVADE_BEFORE && jungle_owner(p) == Some(enemy));
    if !invaded {
        invaded = events(tl, "CHAMPION_KILL").any(|e| e.timestamp < INVADE_BEFORE && e.position.is_some_and(|p| involved(e, pid) && jungle_owner(p) == Some(enemy)));
    }

    // Dragon presence at the first dragon of the game (either team).
    let mut dragon_present = None;
    if let Some(dragon) = sorted_events(tl, "ELITE_MONSTER_KILL").into_iter().find(|e| e.monster_type.as_deref() == Some("DRAGON")) {
        if dragon.killer_id == Some(pid) || dragon.assisting_participant_ids.as_ref().is_some_and(|a| a.contains(&pid)) {
            dragon_present = Some(true);
        } else {
            let p = position_at(tl, pid, dragon.timestamp);
            dragon_present = Some(p.is_some_and(|p| dist(p, dragon.position.unwrap_or(DRAGON_PIT)) <= DRAGON_RADIUS));
        }
    }

    JungleGameStats { start_side, first_gank: first.map(|(t, lane)| FirstGank { minute: t / MIN, lane }), invaded, dragon_present }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GameStats {
    pub match_id: String,
    pub queue_id: i64,
    /// Epoch ms of game end (for recency ordering).
    pub ended_at: f64,
    pub duration_sec: f64,
    pub win: bool,
    pub champion_name: String,
    pub champion_id: i64,
    pub team_position: String,
    pub team_id: TeamId,
    pub kills: i64,
    pub deaths: i64,
    pub assists: i64,
    pub cs_per_min: f64,
    pub fb_participation: bool,
    /// Whole-game solo kills from match challenges (if present).
    pub challenge_solo_kills: Option<f64>,
    pub early: Option<EarlyStats>,
    pub roamed: Option<bool>,
    pub jungle: Option<JungleGameStats>,
}

pub fn duration_seconds(m: &MatchDto) -> f64 {
    let d = m.info.game_duration;
    if m.info.game_end_timestamp.is_some() {
        d
    } else {
        d / 1000.0
    }
}

/// True for remakes / early surrenders that should not count.
pub fn is_remake(m: &MatchDto, p: Option<&MatchParticipantDto>) -> bool {
    duration_seconds(m) < 300.0 || p.is_some_and(|p| p.game_ended_in_early_surrender == Some(true))
}

pub fn analyze_game(m: &MatchDto, timeline: Option<&TimelineDto>, puuid: &str) -> Option<GameStats> {
    let p = find_participant(m, puuid)?;
    if is_remake(m, Some(p)) {
        return None;
    }
    let duration_sec = duration_seconds(m);
    let team: TeamId = if p.team_id == 200 { 200 } else { 100 };
    let ended_at = m.info.game_end_timestamp.unwrap_or(m.info.game_creation + duration_sec * 1000.0);
    let team_position = p.team_position.clone().unwrap_or_default();
    let mut g = GameStats {
        match_id: m.metadata.match_id.clone(),
        queue_id: m.info.queue_id,
        ended_at,
        duration_sec,
        win: p.win,
        champion_name: p.champion_name.clone(),
        champion_id: p.champion_id,
        team_position: team_position.clone(),
        team_id: team,
        kills: p.kills,
        deaths: p.deaths,
        assists: p.assists,
        cs_per_min: (p.total_minions_killed.unwrap_or(0.0) + p.neutral_minions_killed.unwrap_or(0.0)) / (duration_sec / 60.0).max(1.0),
        fb_participation: p.first_blood_kill == Some(true) || p.first_blood_assist == Some(true),
        challenge_solo_kills: p.challenges.as_ref().and_then(|c| c.solo_kills),
        early: None,
        roamed: None,
        jungle: None,
    };
    if let Some(tl) = timeline {
        if let Some(pid) = timeline_participant_id(tl, puuid, Some(p.participant_id)).filter(|&x| x != 0) {
            g.early = Some(early_stats(tl, pid));
            if let Some(lane) = lane_of_position(&team_position) {
                g.roamed = Some(roamed_in_game(tl, pid, lane, team));
            }
            if team_position == "JUNGLE" {
                g.jungle = Some(analyze_jungle_game(tl, pid, team));
            }
        }
    }
    Some(g)
}

// ---------------- aggregation ----------------

fn round(x: f64, d: i32) -> f64 {
    let k = 10f64.powi(d);
    js_round(x * k) / k
}

fn avg(xs: impl IntoIterator<Item = f64>) -> f64 {
    let v: Vec<f64> = xs.into_iter().collect();
    if v.is_empty() {
        0.0
    } else {
        v.iter().sum::<f64>() / v.len() as f64
    }
}

/// Aggression 0..1:
///   A = 0.45 · min(1, (kills15 + deaths15) / 6)
///     + 0.30 · min(1, soloKills15 / 1.0)
///     + 0.25 · min(1, firstBloodRate / 0.4)
pub fn aggression_score(involvement15: f64, solo_kills15: f64, first_blood_rate: f64) -> f64 {
    let a = 0.45 * (involvement15 / 6.0).min(1.0) + 0.30 * (solo_kills15 / 1.0).min(1.0) + 0.25 * (first_blood_rate / 0.4).min(1.0);
    round(a.clamp(0.0, 1.0), 3)
}

/// (win, endedAt) pairs → wins, losses, streak from the most recent game.
pub fn recent_form(games: &[(bool, f64)]) -> RecentForm {
    let mut sorted = games.to_vec();
    sorted.sort_by(|a, b| b.1.total_cmp(&a.1));
    let wins = sorted.iter().filter(|g| g.0).count();
    let mut streak: i64 = 0;
    for &(win, _) in &sorted {
        if streak == 0 {
            streak = if win { 1 } else { -1 };
        } else if win && streak > 0 {
            streak += 1;
        } else if !win && streak < 0 {
            streak -= 1;
        } else {
            break;
        }
    }
    RecentForm { wins, losses: sorted.len() - wins, streak }
}

pub fn norm_champion(s: &str) -> String {
    let n: String = s.to_lowercase().chars().filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit()).collect();
    match n.as_str() {
        "wukong" => "monkeyking".into(),
        "nunuwillump" => "nunu".into(),
        "renataglasc" => "renata".into(),
        _ => n,
    }
}

pub fn median(xs: &[f64]) -> Option<f64> {
    if xs.is_empty() {
        return None;
    }
    let mut s = xs.to_vec();
    s.sort_by(f64::total_cmp);
    let m = s.len() >> 1;
    Some(if s.len() % 2 == 1 { s[m] } else { (s[m - 1] + s[m]) / 2.0 })
}

pub fn aggregate_jungle(games: &[&JungleGameStats]) -> Option<JunglePathing> {
    if games.is_empty() {
        return None;
    }
    let sided: Vec<_> = games.iter().filter(|j| j.start_side.is_some()).collect();
    let top = sided.iter().filter(|j| j.start_side == Some("top")).count();
    let ganks: Vec<FirstGank> = games.iter().filter_map(|j| j.first_gank).collect();
    let mut lane_dist: Vec<(Lane, f64)> = Vec::new();
    for g in &ganks {
        match lane_dist.iter_mut().find(|(l, _)| *l == g.lane) {
            Some(e) => e.1 += 1.0,
            None => lane_dist.push((g.lane, 1.0)),
        }
    }
    for e in &mut lane_dist {
        e.1 = round(e.1 / ganks.len() as f64, 3);
    }
    let with_dragon: Vec<_> = games.iter().filter(|j| j.dragon_present.is_some()).collect();
    let med = median(&ganks.iter().map(|g| g.minute).collect::<Vec<_>>());
    let n = games.len() as f64;
    Some(JunglePathing {
        start_top_side: if sided.is_empty() { 0.0 } else { round(top as f64 / sided.len() as f64, 3) },
        start_bot_side: if sided.is_empty() { 0.0 } else { round((sided.len() - top) as f64 / sided.len() as f64, 3) },
        first_gank_minute: med.map(|m| round(m, 2)),
        first_gank_lane: LaneShares(lane_dist),
        early_invade_rate: round(games.iter().filter(|j| j.invaded).count() as f64 / n, 3),
        dragon_presence: if with_dragon.is_empty() { 0.0 } else { round(with_dragon.iter().filter(|j| j.dragon_present == Some(true)).count() as f64 / with_dragon.len() as f64, 3) },
        sample_games: games.len(),
    })
}

pub struct ProfileInput<'a> {
    pub puuid: &'a str,
    pub riot_id: &'a str,
    pub champion: &'a str,
    pub role: &'a str,
    pub rank: Option<String>,
    pub games: &'a [GameStats],
    /// Mastery points on `champion` if fetched: Some(None) = never played, None = unknown.
    pub mastery_points: Option<Option<f64>>,
    pub now: f64,
}

fn same_lane(team_position: &str, role: &str) -> bool {
    matches!((lane_of_position(team_position), lane_of_position(role)), (Some(a), Some(b)) if a == b)
}

pub fn build_profile(inp: &ProfileInput) -> OpponentProfile {
    let games = inp.games;
    let n = games.len();
    let champ = norm_champion(inp.champion);
    let on_champ: Vec<&GameStats> = games.iter().filter(|g| norm_champion(&g.champion_name) == champ).collect();
    let champ_wins = on_champ.iter().filter(|g| g.win).count();
    let fb_rate = if n > 0 { games.iter().filter(|g| g.fb_participation).count() as f64 / n as f64 } else { 0.0 };

    // Early stats: timeline games when available, otherwise a rough estimate
    // scaling whole-game numbers by the share of the game before 10/15 min.
    let tl: Vec<&EarlyStats> = games.iter().filter_map(|g| g.early.as_ref()).collect();
    let (k15, d15, d10, solo15);
    if !tl.is_empty() {
        k15 = avg(tl.iter().map(|e| e.kills15 as f64));
        d15 = avg(tl.iter().map(|e| e.deaths15 as f64));
        d10 = avg(tl.iter().map(|e| e.deaths10 as f64));
        solo15 = avg(tl.iter().map(|e| e.solo_kills15 as f64));
    } else {
        let f = |g: &GameStats, min: f64| ((min * 60.0) / g.duration_sec.max(1.0)).min(1.0);
        k15 = avg(games.iter().map(|g| g.kills as f64 * f(g, 15.0)));
        d15 = avg(games.iter().map(|g| g.deaths as f64 * f(g, 15.0)));
        d10 = avg(games.iter().map(|g| g.deaths as f64 * f(g, 10.0)));
        solo15 = avg(games.iter().map(|g| g.challenge_solo_kills.unwrap_or(0.0) * f(g, 15.0)));
    }

    let role = inp.role;
    let roam_games: Vec<&GameStats> = games.iter().filter(|g| g.roamed.is_some() && (role == "JUNGLE" || role == "UNKNOWN" || same_lane(&g.team_position, role))).collect();
    let roam_sample: Vec<&GameStats> = if !roam_games.is_empty() { roam_games } else { games.iter().filter(|g| g.roamed.is_some()).collect() };
    let roam_rate = if role == "JUNGLE" || roam_sample.is_empty() { 0.0 } else { roam_sample.iter().filter(|g| g.roamed == Some(true)).count() as f64 / roam_sample.len() as f64 };

    let jungle_games: Vec<&JungleGameStats> = games.iter().filter_map(|g| g.jungle.as_ref()).collect();
    let jungle = if role == "JUNGLE" || (role == "UNKNOWN" && jungle_games.len() * 2 >= tl.len() && !jungle_games.is_empty()) { aggregate_jungle(&jungle_games) } else { None };

    let mut profile = OpponentProfile {
        puuid: inp.puuid.to_string(),
        riot_id: inp.riot_id.to_string(),
        champion: inp.champion.to_string(),
        role: role.to_string(),
        rank: inp.rank.clone(),
        games_analyzed: n,
        aggression: if n > 0 { aggression_score(k15 + d15, solo15, fb_rate) } else { 0.0 },
        first_blood_rate: round(fb_rate, 3),
        deaths_before10: round(d10, 2),
        kills_before15: round(k15, 2),
        roam_rate: round(roam_rate, 3),
        champion_games: on_champ.len(),
        champion_win_rate: if on_champ.is_empty() { None } else { Some(round(champ_wins as f64 / on_champ.len() as f64, 3)) },
        recent_form: recent_form(&games.iter().map(|g| (g.win, g.ended_at)).collect::<Vec<_>>()),
        jungle,
        summary: String::new(),
        fetched_at: inp.now,
    };
    let lane_games: Vec<&GameStats> = games.iter().filter(|g| g.team_position != "JUNGLE").collect();
    let cs = if !lane_games.is_empty() { avg(lane_games.iter().map(|g| g.cs_per_min)) } else { avg(games.iter().map(|g| g.cs_per_min)) };
    profile.summary = summarize(&profile, &SummaryExtras { cs_per_min: Some(cs), roam_sample: Some(roam_sample.len()), mastery_points: inp.mastery_points });
    profile
}

// ---------------- summary ----------------

fn lane_short(l: Lane) -> &'static str {
    match l {
        Lane::Top => "top",
        Lane::Middle => "mid",
        Lane::Bottom => "bot",
    }
}

/// Polish plural: 1 → one, 2-4 (not 12-14) → few, else many.
pub fn pl_plural<'a>(n: i64, one: &'a str, few: &'a str, many: &'a str) -> &'a str {
    if n == 1 {
        return one;
    }
    let (d, h) = (n % 10, n % 100);
    if (2..=4).contains(&d) && !(12..=14).contains(&h) {
        few
    } else {
        many
    }
}

pub fn fmt_minute(min: f64) -> String {
    let s = js_round(min * 60.0) as i64;
    format!("{}:{:02}", s.div_euclid(60), s.rem_euclid(60))
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SummaryExtras {
    pub cs_per_min: Option<f64>,
    pub roam_sample: Option<usize>,
    /// Some(None) = mastery fetched and none (never played).
    pub mastery_points: Option<Option<f64>>,
}

/// One short Polish line (≤ ~6 words) with the 1–3 most decision-relevant facts.
pub fn summarize(p: &OpponentProfile, x: &SummaryExtras) -> String {
    if p.games_analyzed == 0 {
        return "brak danych".into();
    }
    struct Fact {
        order: u8,
        prio: f64,
        text: String,
    }
    let mut facts: Vec<Fact> = Vec::new();
    let mut push = |order: u8, prio: f64, text: String| facts.push(Fact { order, prio, text });
    let enough = p.games_analyzed >= 3;
    let is_jungle = p.role == "JUNGLE";

    if enough {
        if p.aggression >= 0.6 {
            push(0, if is_jungle { 0.8 } else { 0.9 }, "agresywny".into());
        } else if p.aggression <= 0.3 {
            let farmer = !is_jungle && x.cs_per_min.unwrap_or(0.0) >= 7.0;
            push(0, if is_jungle { 0.6 } else { 0.75 }, if farmer { "pasywny farmer" } else { "pasywny" }.into());
        }
    }

    if let Some(j) = p.jungle.as_ref().filter(|j| j.sample_games >= 3) {
        if j.start_bot_side >= 0.6 {
            push(1, 0.9 + j.start_bot_side / 100.0, "start dół".into());
        } else if j.start_top_side >= 0.6 {
            push(1, 0.9 + j.start_top_side / 100.0, "start góra".into());
        }
        let mut lanes = j.first_gank_lane.0.clone();
        lanes.sort_by(|a, b| b.1.total_cmp(&a.1)); // stable, like JS sort
        if let (Some(top), Some(minute)) = (lanes.first(), j.first_gank_minute) {
            if top.1 >= 0.5 {
                push(2, 0.85, format!("gank {} ~{}", lane_short(top.0), fmt_minute(minute)));
            }
        }
        if j.early_invade_rate >= 0.4 {
            push(3, 0.7, "invaduje".into());
        }
    }

    if !is_jungle && x.roam_sample.unwrap_or(0) >= 3 && p.roam_rate >= 0.4 {
        push(4, 0.85, "roamuje".into());
    }

    let cg = p.champion_games as i64;
    if cg == 0 {
        if matches!(x.mastery_points, Some(None)) || matches!(x.mastery_points, Some(Some(m)) if m < 20_000.0) {
            push(5, 0.82, "nowa postać".into());
        }
    } else if cg <= 2 && p.games_analyzed >= 5 {
        push(5, 0.8, format!("{cg} {} na postaci", pl_plural(cg, "gra", "gry", "gier")));
    } else if cg >= 6 && cg as f64 / p.games_analyzed as f64 >= 0.6 {
        push(5, 0.55, "main postaci".into());
    }

    let s = p.recent_form.streak;
    if s <= -3 {
        push(6, 0.65, format!("{} {} z rzędu", -s, pl_plural(-s, "porażka", "porażki", "porażek")));
    } else if s >= 4 {
        push(6, 0.5, format!("{s} {} z rzędu", pl_plural(s, "wygrana", "wygrane", "wygranych")));
    }

    facts.sort_by(|a, b| b.prio.total_cmp(&a.prio));
    facts.truncate(3);
    let words = |fs: &[Fact]| fs.iter().map(|f| f.text.split(' ').count()).sum::<usize>();
    while facts.len() > 1 && words(&facts) > 6 {
        facts.pop();
    }
    if facts.is_empty() {
        return if enough { "bez wyraźnych wzorców" } else { "mało danych" }.into();
    }
    facts.sort_by_key(|f| f.order);
    facts.iter().map(|f| f.text.as_str()).collect::<Vec<_>>().join(", ")
}
