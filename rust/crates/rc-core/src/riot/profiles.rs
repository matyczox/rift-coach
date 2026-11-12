//! Orchestration: 5 enemies → `OpponentProfile`s within the dev-key limits
//! (`src/main/riot/profiles.ts`).
//!
//! Request plan for a cold cache (defaults), per player:
//!   account 1 (skipped when puuid is given), league 1, match ids 1 (type=ranked;
//!   +1 type=normal only if ranked < matchCount), matches: jungler ≤ 12, laner ≤ 7
//!   network fetches, timelines: jungler ≤ 10, laner ≤ 5, mastery ≤ 1.
//! Typical cold total 85–90 ≤ maxRequests (95), under 100 req / 120 s.
//! Everything is additionally bounded by a hard deadline (default 85 s).

use super::analyze::{analyze_game, build_profile, norm_champion, GameStats, ProfileInput};
use super::cache::epoch_ms;
use super::client::{CallOptions, MatchIdsQuery, RequestBudget, RiotClient, RiotError};
use super::types::*;
use futures_util::future::{join_all, try_join_all};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::Instant;

#[derive(Clone, Debug, Default)]
pub struct ProfilePlayer {
    pub riot_id: String,
    pub champion: String,
    /// TOP | JUNGLE | MIDDLE | BOTTOM | UTILITY | UNKNOWN
    pub role: String,
    /// Known puuid (spectator-v5 / LCU) saves the account lookup.
    pub puuid: Option<String>,
    /// Numeric champion id (enables the mastery lookup).
    pub champion_id: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProfileProgress {
    pub riot_id: String,
    /// account | matches | timelines | done | error
    pub stage: &'static str,
    pub players_done: usize,
    pub players_total: usize,
    pub requests_used: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoleCaps {
    pub jungle: usize,
    pub laner: usize,
}

pub struct BuildProfilesOptions {
    /// Match ids considered per player (default 20).
    pub match_count: u32,
    /// Queues kept (default ranked solo 420, flex 440, normal draft 400, swiftplay 480).
    pub queues: Vec<i64>,
    /// Max match downloads (network) per player.
    pub max_match_fetch: RoleCaps,
    /// Max timeline downloads (network) per player.
    pub max_timelines: RoleCaps,
    /// Hard cap on network requests for the whole run (default 95).
    pub max_requests: usize,
    /// Hard wall-clock cap (default 85 000 ms).
    pub timeout_ms: u64,
    pub fetch_mastery: bool,
    pub on_progress: Option<Box<dyn Fn(&ProfileProgress) + Send + Sync>>,
    /// Epoch ms for `fetchedAt`.
    pub now: Option<Arc<dyn Fn() -> f64 + Send + Sync>>,
}

impl Default for BuildProfilesOptions {
    fn default() -> Self {
        BuildProfilesOptions {
            match_count: 20,
            queues: DEFAULT_QUEUES.to_vec(),
            max_match_fetch: RoleCaps { jungle: 12, laner: 7 },
            max_timelines: RoleCaps { jungle: 10, laner: 5 },
            max_requests: 95,
            timeout_ms: 85_000,
            fetch_mastery: true,
            on_progress: None,
            now: None,
        }
    }
}

pub const DEFAULT_QUEUES: [i64; 4] = [420, 440, 400, 480];
const RANKED: [i64; 2] = [420, 440];

/// Key rejected: stop this player. Anything else on a single match is skipped.
fn is_fatal(e: &RiotError) -> bool {
    matches!(e.status(), Some(401) | Some(403))
}

/// Budget/deadline problems are expected: skip the item, keep going.
fn is_soft(e: &RiotError) -> bool {
    matches!(e, RiotError::Budget | RiotError::Deadline)
}

struct Run<'a> {
    client: &'a RiotClient,
    opts: &'a BuildProfilesOptions,
    players_total: usize,
    global: Arc<RequestBudget>,
    base: CallOptions,
    done: Mutex<usize>,
    /// Champion name → id learned from any fetched match (for mastery lookups).
    champ_ids: Mutex<HashMap<String, i64>>,
    queues: HashSet<i64>,
}

impl Run<'_> {
    fn now_ms(&self) -> f64 {
        self.opts.now.as_ref().map(|f| f()).unwrap_or_else(epoch_ms)
    }

    fn report(&self, riot_id: &str, stage: &'static str) {
        if let Some(cb) = &self.opts.on_progress {
            let p = ProfileProgress { riot_id: riot_id.to_string(), stage, players_done: *self.done.lock().unwrap(), players_total: self.players_total, requests_used: self.global.used() };
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| cb(&p)));
        }
    }
}

pub async fn build_profiles(client: &RiotClient, players: &[ProfilePlayer], opts: &BuildProfilesOptions) -> Vec<OpponentProfile> {
    let deadline = Instant::now() + Duration::from_millis(opts.timeout_ms);
    // Never plan more requests than the regional limiter can grant before the
    // deadline (e.g. a re-run shortly after a previous one), minus a safety margin.
    let limiter_room = client.limiter_for(&client.region).available_by(deadline) as i64 - 3;
    let global = RequestBudget::new((opts.max_requests as i64).min(limiter_room).max(0) as usize, None);
    let run = Run {
        client,
        opts,
        players_total: players.len(),
        base: CallOptions { budget: Some(global.clone()), deadline: Some(deadline) },
        global,
        done: Mutex::new(0),
        champ_ids: Mutex::new(HashMap::new()),
        queues: opts.queues.iter().copied().collect(),
    };

    // Hard stop: even if something hangs, return what we have shortly after the deadline.
    let results: Mutex<Vec<Option<OpponentProfile>>> = Mutex::new(vec![None; players.len()]);
    let work = join_all(players.iter().enumerate().map(|(i, pl)| {
        let (run, results) = (&run, &results);
        async move {
            let p = profile_one(run, pl).await;
            results.lock().unwrap()[i] = Some(p);
        }
    }));
    let _ = tokio::time::timeout_at(deadline + Duration::from_millis(3000), work).await;
    let results = results.into_inner().unwrap();
    results
        .into_iter()
        .zip(players)
        .map(|(r, pl)| {
            r.unwrap_or_else(|| {
                build_profile(&ProfileInput { puuid: pl.puuid.as_deref().unwrap_or(""), riot_id: &pl.riot_id, champion: &pl.champion, role: &pl.role, rank: None, games: &[], mastery_points: None, now: run.now_ms() })
            })
        })
        .collect()
}

async fn profile_one(run: &Run<'_>, pl: &ProfilePlayer) -> OpponentProfile {
    let (client, opts, base, queues, champ_ids, global) = (run.client, run.opts, &run.base, &run.queues, &run.champ_ids, &run.global);
    let report = |id: &str, stage: &'static str| run.report(id, stage);
    {
        {
            let is_jungle = pl.role == "JUNGLE";
            let mut puuid = pl.puuid.clone().unwrap_or_default();
            let mut rank: Option<String> = None;
            let mut games: Vec<GameStats> = Vec::new();
            let mut mastery: Option<Option<f64>> = None;
            let res: Result<(), RiotError> = async {
                report(&pl.riot_id, "account");
                if puuid.is_empty() {
                    let acc = client.resolve_riot_id(&pl.riot_id, base).await?;
                    match acc {
                        Some(a) => puuid = a.puuid,
                        None => return Err(RiotError::Payload { endpoint: "account not found".into() }),
                    }
                }
                let league = async { client.get_league_entries(&puuid, base).await.ok().and_then(|e| format_rank(&e)) };
                let main = async {
                    report(&pl.riot_id, "matches");
                    let ids = list_match_ids(client, &puuid, queues, opts.match_count, base).await?;
                    let match_budget = RequestBudget::new(if is_jungle { opts.max_match_fetch.jungle } else { opts.max_match_fetch.laner }, Some(global.clone()));
                    let mo = CallOptions { budget: Some(match_budget), deadline: base.deadline };
                    let matches: Mutex<Vec<MatchDto>> = Mutex::new(Vec::new());
                    // Newest first; small parallelism, the limiter does the pacing.
                    for_each_limit(&ids, 4, |id: &String| {
                        let (mo, matches) = (&mo, &matches);
                        async move {
                            match client.get_match(id, mo).await {
                                Ok(Some(m)) if queues.contains(&m.info.queue_id) => matches.lock().unwrap().push(m),
                                Ok(_) => {}
                                Err(e) if is_fatal(&e) => return Err(e),
                                Err(_) => {}
                            }
                            Ok(())
                        }
                    })
                    .await?;
                    let mut matches = matches.into_inner().unwrap();
                    {
                        let mut ci = champ_ids.lock().unwrap();
                        for m in &matches {
                            for p in &m.info.participants {
                                ci.insert(norm_champion(&p.champion_name), p.champion_id);
                            }
                        }
                    }
                    matches.sort_by(|a, b| end_of(b).total_cmp(&end_of(a)));

                    report(&pl.riot_id, "timelines");
                    let want = pick_timeline_games(&matches, &puuid, &pl.role, &pl.champion);
                    let tl_budget = RequestBudget::new(if is_jungle { opts.max_timelines.jungle } else { opts.max_timelines.laner }, Some(global.clone()));
                    let to = CallOptions { budget: Some(tl_budget), deadline: base.deadline };
                    let timelines: Mutex<HashMap<String, TimelineDto>> = Mutex::new(HashMap::new());
                    for_each_limit(&want, 3, |m: &&MatchDto| {
                        let (to, timelines) = (&to, &timelines);
                        let id = m.metadata.match_id.clone();
                        async move {
                            match client.get_timeline(&id, to).await {
                                Ok(Some(t)) => {
                                    timelines.lock().unwrap().insert(id, t);
                                }
                                Ok(None) => {}
                                Err(e) if is_fatal(&e) => return Err(e),
                                Err(_) => {}
                            }
                            Ok(())
                        }
                    })
                    .await?;
                    let timelines = timelines.into_inner().unwrap();
                    let mut games = Vec::new();
                    for m in &matches {
                        if let Some(g) = analyze_game(m, timelines.get(&m.metadata.match_id), &puuid) {
                            games.push(g);
                        }
                    }

                    let mut mastery = None;
                    let champ = norm_champion(&pl.champion);
                    if opts.fetch_mastery && !games.iter().any(|g| norm_champion(&g.champion_name) == champ) {
                        let cid = pl.champion_id.or_else(|| champ_ids.lock().unwrap().get(&champ).copied());
                        if let Some(cid) = cid {
                            if let Ok(m) = client.get_champion_mastery(&puuid, cid, base).await {
                                mastery = Some(m.map(|m| m.champion_points as f64));
                            }
                        }
                    }
                    Ok::<_, RiotError>((games, mastery))
                };
                let (r, out) = tokio::join!(league, main);
                rank = r;
                let (g, m) = out?;
                games = g;
                mastery = m;
                Ok(())
            }
            .await;
            *run.done.lock().unwrap() += 1;
            report(&pl.riot_id, if res.is_ok() { "done" } else { "error" });
            build_profile(&ProfileInput { puuid: &puuid, riot_id: &pl.riot_id, champion: &pl.champion, role: &pl.role, rank, games: &games, mastery_points: mastery, now: run.now_ms() })
        }
    }
}

async fn list_match_ids(client: &RiotClient, puuid: &str, queues: &HashSet<i64>, count: u32, o: &CallOptions) -> Result<Vec<String>, RiotError> {
    let want_ranked = queues.iter().any(|q| RANKED.contains(q));
    let want_normal = queues.iter().any(|q| !RANKED.contains(q));
    let mut ids: Vec<String> = Vec::new();
    if queues.len() == 1 {
        let q = *queues.iter().next().unwrap();
        ids = client.get_match_ids(puuid, &MatchIdsQuery { queue: Some(q), count: Some(count), ..Default::default() }, o).await?;
    } else {
        if want_ranked {
            ids = client.get_match_ids(puuid, &MatchIdsQuery { kind: Some("ranked".into()), count: Some(count), ..Default::default() }, o).await?;
        }
        if want_normal && (ids.len() as u32) < count {
            match client.get_match_ids(puuid, &MatchIdsQuery { kind: Some("normal".into()), count: Some(count), ..Default::default() }, o).await {
                Ok(more) => {
                    for id in more {
                        if !ids.contains(&id) {
                            ids.push(id);
                        }
                    }
                }
                Err(e) if is_soft(&e) => {}
                Err(e) => return Err(e),
            }
        }
    }
    // Match ids are "<PLATFORM>_<increasing number>": sort newest first.
    ids.sort_by(|a, b| match_num(b).total_cmp(&match_num(a)));
    ids.truncate(count as usize);
    Ok(ids)
}

fn match_num(id: &str) -> f64 {
    let tail = &id[id.rfind('_').map(|i| i + 1).unwrap_or(0)..];
    tail.parse::<f64>().ok().filter(|x| x.is_finite()).unwrap_or(0.0)
}

fn end_of(m: &MatchDto) -> f64 {
    m.info.game_end_timestamp.unwrap_or(m.info.game_creation)
}

/// Which games deserve a timeline: jungler → games played as JUNGLE; laner →
/// games in the same lane (else any lane game). Same champion first, then recency.
pub fn pick_timeline_games<'a>(matches: &'a [MatchDto], puuid: &str, role: &str, champion: &str) -> Vec<&'a MatchDto> {
    let champ = norm_champion(champion);
    let rows: Vec<(&MatchDto, &MatchParticipantDto)> = matches.iter().filter_map(|m| m.info.participants.iter().find(|x| x.puuid == puuid).map(|p| (m, p))).collect();
    let lane_pos = |pos: &str| if pos == "UTILITY" { "BOTTOM".to_string() } else { pos.to_string() };
    let tp = |p: &MatchParticipantDto| p.team_position.clone().unwrap_or_default();
    let mut eligible: Vec<_> = rows.iter().filter(|(_, p)| if role == "UNKNOWN" { !tp(p).is_empty() } else { lane_pos(&tp(p)) == lane_pos(role) }).cloned().collect();
    if eligible.is_empty() && role != "JUNGLE" {
        eligible = rows.iter().filter(|(_, p)| !tp(p).is_empty() && tp(p) != "JUNGLE").cloned().collect();
    }
    eligible.sort_by(|a, b| {
        let ca = norm_champion(&a.1.champion_name) == champ;
        let cb = norm_champion(&b.1.champion_name) == champ;
        cb.cmp(&ca).then(end_of(b.0).total_cmp(&end_of(a.0)))
    });
    eligible.into_iter().map(|r| r.0).collect()
}

/// "GOLD II 45 LP", apex tiers without division, " (flex)" when only flex is ranked.
pub fn format_rank(entries: &[LeagueEntryDto]) -> Option<String> {
    let e = entries.iter().find(|x| x.queue_type == "RANKED_SOLO_5x5").or_else(|| entries.iter().find(|x| x.queue_type == "RANKED_FLEX_SR"))?;
    let tier = e.tier.as_deref().filter(|t| !t.is_empty())?;
    let apex = ["MASTER", "GRANDMASTER", "CHALLENGER"].contains(&tier);
    let lp = format!("{} LP", e.league_points.unwrap_or(0));
    let flex = if e.queue_type == "RANKED_FLEX_SR" { " (flex)" } else { "" };
    let div = match e.rank.as_deref().filter(|r| !r.is_empty()) {
        Some(r) if !apex => format!(" {r}"),
        _ => String::new(),
    };
    Some(format!("{tier}{div} {lp}{flex}"))
}

/// `forEachLimit`: `limit` workers over `items` in order; the first error stops the run.
async fn for_each_limit<'a, T, F, Fut>(items: &'a [T], limit: usize, f: F) -> Result<(), RiotError>
where
    F: Fn(&'a T) -> Fut,
    Fut: std::future::Future<Output = Result<(), RiotError>>,
{
    let next = Mutex::new(0usize);
    let workers = (0..limit.min(items.len())).map(|_| {
        let (next, f) = (&next, &f);
        async move {
            loop {
                let i = {
                    let mut n = next.lock().unwrap();
                    if *n >= items.len() {
                        return Ok::<(), RiotError>(());
                    }
                    *n += 1;
                    *n - 1
                };
                f(&items[i]).await?;
            }
        }
    });
    try_join_all(workers).await.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks() {
        let e = |q: &str, t: &str, r: &str, lp: i64| LeagueEntryDto { queue_type: q.into(), tier: Some(t.into()), rank: Some(r.into()), league_points: Some(lp), ..Default::default() };
        assert_eq!(format_rank(&[e("RANKED_FLEX_SR", "SILVER", "I", 3), e("RANKED_SOLO_5x5", "GOLD", "II", 45)]).as_deref(), Some("GOLD II 45 LP"));
        assert_eq!(format_rank(&[e("RANKED_SOLO_5x5", "MASTER", "I", 120)]).as_deref(), Some("MASTER 120 LP"));
        assert_eq!(format_rank(&[e("RANKED_FLEX_SR", "SILVER", "I", 3)]).as_deref(), Some("SILVER I 3 LP (flex)"));
        assert_eq!(format_rank(&[]), None);
        assert_eq!(match_num("EUN1_123"), 123.0);
        assert_eq!(match_num("x"), 0.0);
    }
}
