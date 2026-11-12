//! Port of tests/riot.test.ts: synthetic match-v5 fixtures, analysis, limiter
//! (in src/riot/limiter.rs), client (fake HTTP) and buildProfiles orchestration.

use rc_core::llm::caller::BoxFuture;
use rc_core::riot::analyze::*;
use rc_core::riot::client::*;
use rc_core::riot::profiles::*;
use rc_core::riot::types::*;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const MIN: f64 = 60_000.0;
fn pu(i: i64) -> String {
    format!("puuid-{i}")
}
const POS: [&str; 5] = ["TOP", "JUNGLE", "MIDDLE", "BOTTOM", "UTILITY"];

#[derive(Clone)]
struct MatchOpts {
    id: String,
    pid: i64,
    puuid: Option<String>,
    champion: String,
    champion_id: i64,
    position: Option<String>,
    win: bool,
    kills: i64,
    deaths: i64,
    assists: i64,
    fb_kill: bool,
    end: f64,
    duration_sec: f64,
    queue_id: i64,
}

fn mo(id: &str) -> MatchOpts {
    MatchOpts { id: id.into(), pid: 2, puuid: None, champion: "Warwick".into(), champion_id: 19, position: None, win: true, kills: 5, deaths: 5, assists: 5, fb_kill: false, end: 1_700_000_000_000.0, duration_sec: 1800.0, queue_id: 420 }
}

fn make_match(o: &MatchOpts) -> MatchDto {
    let pid = o.pid;
    let participants: Vec<MatchParticipantDto> = (0..10)
        .map(|i| {
            let id = i as i64 + 1;
            let me = id == pid;
            let team_id = if id <= 5 { 100 } else { 200 };
            let my_team = if pid <= 5 { 100 } else { 200 };
            MatchParticipantDto {
                puuid: if me { o.puuid.clone().unwrap_or_else(|| pu(id)) } else { format!("other-{}-{id}", o.id) },
                participant_id: id,
                team_id,
                champion_id: if me { o.champion_id } else { 100 + id },
                champion_name: if me { o.champion.clone() } else { format!("Champ{id}") },
                team_position: Some(if me { o.position.clone().unwrap_or(POS[i % 5].into()) } else { POS[i % 5].into() }),
                win: (team_id == my_team) == o.win,
                kills: if me { o.kills } else { 3 },
                deaths: if me { o.deaths } else { 3 },
                assists: if me { o.assists } else { 3 },
                first_blood_kill: Some(me && o.fb_kill),
                first_blood_assist: Some(false),
                total_minions_killed: Some(180.0),
                neutral_minions_killed: Some(20.0),
                challenges: Some(Challenges { solo_kills: Some(1.0) }),
                ..Default::default()
            }
        })
        .collect();
    MatchDto {
        metadata: MatchMetadata { match_id: o.id.clone(), participants: participants.iter().map(|p| p.puuid.clone()).collect() },
        info: MatchInfo { game_creation: o.end - o.duration_sec * 1000.0 - 60_000.0, game_duration: o.duration_sec, game_end_timestamp: Some(o.end), queue_id: o.queue_id, map_id: Some(11), participants, ..Default::default() },
    }
}

type Frames = HashMap<i64, HashMap<i64, Position>>;

fn frames(list: &[(i64, i64, Position)]) -> Frames {
    let mut f: Frames = HashMap::new();
    for &(m, id, p) in list {
        f.entry(m).or_default().insert(id, p);
    }
    f
}

fn make_timeline(m: &MatchDto, fr: &Frames, evs: &[TimelineEventDto], minutes: i64) -> TimelineDto {
    let mut out = Vec::new();
    for mi in 0..=minutes {
        let mut pf = BTreeMap::new();
        for id in 1..=10 {
            let pos = fr.get(&mi).and_then(|x| x.get(&id)).copied().unwrap_or(if id <= 5 { p(554.0, 581.0) } else { p(14302.0, 14390.0) });
            pf.insert(id.to_string(), ParticipantFrameDto { participant_id: id, position: Some(pos), level: Some(1), ..Default::default() });
        }
        out.push(TimelineFrameDto {
            timestamp: if mi == 0 { 0.0 } else { mi as f64 * MIN + 37.0 },
            participant_frames: pf,
            events: evs.iter().filter(|e| (e.timestamp / MIN).floor() as i64 == mi).cloned().collect(),
        });
    }
    TimelineDto {
        metadata: m.metadata.clone(),
        info: TimelineInfo { frame_interval: Some(60000.0), frames: out, participants: Some(m.info.participants.iter().map(|x| TimelineParticipant { participant_id: x.participant_id, puuid: x.puuid.clone() }).collect()) },
    }
}

fn tl(m: &MatchDto, fr: &[(i64, i64, Position)], evs: &[TimelineEventDto]) -> TimelineDto {
    make_timeline(m, &frames(fr), evs, 16)
}

fn kill(t: f64, killer: i64, victim: i64, pos: Position, assists: Option<Vec<i64>>) -> TimelineEventDto {
    TimelineEventDto { kind: "CHAMPION_KILL".into(), timestamp: t, killer_id: Some(killer), victim_id: Some(victim), position: Some(pos), assisting_participant_ids: assists, ..Default::default() }
}
const fn p(x: f64, y: f64) -> Position {
    Position { x, y }
}
fn rot(q: Position) -> Position {
    p(MAP_SIZE - q.x, MAP_SIZE - q.y)
}

const BLUE_BLUEBUFF: Position = p(3800.0, 7900.0);
const BLUE_REDBUFF: Position = p(7800.0, 4000.0);
const RED_REDBUFF: Position = p(7100.0, 10900.0);
const RED_BLUEBUFF: Position = p(11000.0, 7000.0);
const BOT_LANE: Position = p(10500.0, 1200.0);
const TOP_LANE: Position = p(1200.0, 10500.0);
const MID_LANE: Position = p(7400.0, 7300.0);
const DRAGON: Position = p(9866.0, 4414.0);

fn close(a: f64, b: f64, digits: i32) {
    assert!((a - b).abs() < 0.5 * 10f64.powi(-digits), "{a} != {b}");
}

// ---------------- geometry ----------------

#[test]
fn map_geometry_lanes_and_camps() {
    assert_eq!(classify_lane(TOP_LANE, 0.0), Some(Lane::Top));
    assert_eq!(classify_lane(BOT_LANE, 0.0), Some(Lane::Bottom));
    assert_eq!(classify_lane(MID_LANE, 0.0), Some(Lane::Middle));
    assert_eq!(classify_lane(p(2090.0, 8430.0), 0.0), None); // blue gromp
    assert_eq!(classify_lane(p(8400.0, 2650.0), 0.0), None); // blue krugs
    assert_eq!(classify_lane(p(12700.0, 6400.0), 0.0), None); // red gromp
    assert_eq!(classify_lane(p(500.0, 500.0), 0.0), None); // fountain
    assert_eq!(jungle_owner(DRAGON), None); // river
}

#[test]
fn map_geometry_jungle_sides() {
    assert_eq!(classify_jungle_side(BLUE_BLUEBUFF, 100), Some("top"));
    assert_eq!(classify_jungle_side(BLUE_REDBUFF, 100), Some("bot"));
    assert_eq!(classify_jungle_side(RED_REDBUFF, 200), Some("top"));
    assert_eq!(classify_jungle_side(RED_BLUEBUFF, 200), Some("bot"));
    assert_eq!(classify_jungle_side(RED_BLUEBUFF, 100), None);
    assert_eq!(classify_jungle_side(BLUE_BLUEBUFF, 200), None);
    assert_eq!(classify_jungle_side(rot(RED_BLUEBUFF), 100), Some("top"));
    assert_eq!(classify_jungle_side(rot(BLUE_REDBUFF), 200), Some("top"));
}

// ---------------- jungle analysis ----------------

#[test]
fn jungle_red_side_start_and_gank() {
    let m = make_match(&MatchOpts { pid: 7, position: Some("JUNGLE".into()), ..mo("EUN1_1") });
    let t = tl(
        &m,
        &[(1, 7, p(11300.0, 7200.0)), (2, 7, RED_BLUEBUFF), (3, 7, p(10800.0, 5600.0)), (4, 7, p(11500.0, 3000.0))],
        &[kill(2.0 * MIN + 10_000.0, 4, 1, BOT_LANE, None), kill(3.0 * MIN + 20_000.0, 7, 4, BOT_LANE, Some(vec![9])), kill(5.0 * MIN, 7, 3, MID_LANE, None)],
    );
    let j = analyze_jungle_game(&t, 7, 200);
    assert_eq!(j.start_side, Some("bot"));
    assert_eq!(j.first_gank.unwrap().lane, Lane::Bottom);
    close(j.first_gank.unwrap().minute, 3.0 + 20.0 / 60.0, 5);
    assert!(!j.invaded);
    assert_eq!(j.dragon_present, None);
}

#[test]
fn jungle_mirrored_blue_side() {
    let m = make_match(&MatchOpts { pid: 2, position: Some("JUNGLE".into()), ..mo("EUN1_2") });
    let t = tl(&m, &[(1, 2, rot(p(11300.0, 7200.0))), (2, 2, rot(RED_BLUEBUFF)), (3, 2, rot(p(10800.0, 5600.0))), (4, 2, rot(p(11500.0, 3000.0)))], &[kill(3.0 * MIN + 20_000.0, 2, 9, rot(BOT_LANE), None)]);
    let j = analyze_jungle_game(&t, 2, 100);
    assert_eq!(j.start_side, Some("top"));
    assert_eq!(j.first_gank.unwrap().lane, Lane::Top);
    close(j.first_gank.unwrap().minute, 3.333, 2);
}

#[test]
fn jungle_minute1_fallback_and_position_gank() {
    let m = make_match(&MatchOpts { pid: 2, position: Some("JUNGLE".into()), ..mo("EUN1_3") });
    let t = tl(&m, &[(1, 2, BLUE_REDBUFF), (2, 2, p(554.0, 581.0)), (3, 2, p(6000.0, 3000.0)), (4, 2, MID_LANE)], &[]);
    let j = analyze_jungle_game(&t, 2, 100);
    assert_eq!(j.start_side, Some("bot"));
    assert_eq!(j.first_gank.unwrap().lane, Lane::Middle);
    close(j.first_gank.unwrap().minute, 4.0, 2);
}

#[test]
fn jungle_invades_and_dragon_presence() {
    let m = make_match(&MatchOpts { pid: 2, position: Some("JUNGLE".into()), ..mo("EUN1_4") });
    let dragon_kill = TimelineEventDto { kind: "ELITE_MONSTER_KILL".into(), timestamp: 6.0 * MIN + 30_000.0, killer_id: Some(8), killer_team_id: Some(200), monster_type: Some("DRAGON".into()), monster_sub_type: Some("FIRE_DRAGON".into()), position: Some(DRAGON), ..Default::default() };
    let near = tl(&m, &[(1, 2, RED_BLUEBUFF), (6, 2, p(9000.0, 4000.0)), (7, 2, p(9500.0, 5000.0))], &[dragon_kill.clone()]);
    let a = analyze_jungle_game(&near, 2, 100);
    assert!(a.invaded);
    assert_eq!(a.dragon_present, Some(true));

    let far = tl(&m, &[(6, 2, TOP_LANE), (7, 2, TOP_LANE)], &[dragon_kill.clone()]);
    let b = analyze_jungle_game(&far, 2, 100);
    assert!(!b.invaded);
    assert_eq!(b.dragon_present, Some(false));

    let assisted = tl(&m, &[(6, 2, TOP_LANE)], &[TimelineEventDto { killer_id: Some(3), killer_team_id: Some(100), assisting_participant_ids: Some(vec![2]), ..dragon_kill }]);
    assert_eq!(analyze_jungle_game(&assisted, 2, 100).dragon_present, Some(true));
}

#[test]
fn jungle_aggregation() {
    let g = |side: &'static str, minute: f64, lane: Lane, invaded: bool| JungleGameStats { start_side: Some(side), first_gank: Some(FirstGank { minute, lane }), invaded, dragon_present: Some(true) };
    let gs = [g("bot", 3.2, Lane::Bottom, false), g("bot", 3.5, Lane::Bottom, true), g("top", 4.0, Lane::Top, false), g("bot", 3.0, Lane::Bottom, false)];
    let j = aggregate_jungle(&gs.iter().collect::<Vec<_>>()).unwrap();
    assert_eq!(j.start_bot_side, 0.75);
    assert_eq!(j.start_top_side, 0.25);
    close(j.first_gank_minute.unwrap(), 3.35, 5);
    assert_eq!(serde_json::to_value(&j.first_gank_lane).unwrap(), json!({ "BOTTOM": 0.75, "TOP": 0.25 }));
    assert_eq!(j.early_invade_rate, 0.25);
    assert_eq!(j.sample_games, 4);
}

// ---------------- laner / early stats ----------------

fn laner_fixture() -> (MatchDto, TimelineDto) {
    let m = make_match(&MatchOpts { pid: 3, position: Some("MIDDLE".into()), ..mo("EUN1_10") });
    let t = tl(
        &m,
        &[],
        &[
            kill(3.0 * MIN, 3, 8, MID_LANE, None),
            kill(4.0 * MIN, 3, 8, MID_LANE, Some(vec![])),
            kill(6.0 * MIN, 3, 9, MID_LANE, Some(vec![2])),
            kill(8.0 * MIN, 8, 3, MID_LANE, None),
            kill(12.0 * MIN, 8, 3, MID_LANE, Some(vec![7])),
            kill(16.0 * MIN, 3, 8, MID_LANE, None),
        ],
    );
    (m, t)
}

#[test]
fn early_stats_counts() {
    let (_, t) = laner_fixture();
    assert_eq!(early_stats(&t, 3), EarlyStats { kills15: 3, deaths15: 2, deaths10: 1, solo_kills15: 2 });
}

#[test]
fn roams_only_outside_own_lane_before_14() {
    let (m, t) = laner_fixture();
    assert!(!roamed_in_game(&t, 3, Lane::Middle, 100));
    let roam = tl(&m, &[], &[kill(6.0 * MIN, 2, 6, TOP_LANE, Some(vec![3]))]);
    assert!(roamed_in_game(&roam, 3, Lane::Middle, 100));
    let late = tl(&m, &[], &[kill(14.0 * MIN + 5000.0, 2, 6, TOP_LANE, Some(vec![3]))]);
    assert!(!roamed_in_game(&late, 3, Lane::Middle, 100));
    let lvl1 = tl(&m, &[], &[kill(70_000.0, 3, 6, RED_BLUEBUFF, None)]);
    assert!(!roamed_in_game(&lvl1, 3, Lane::Middle, 100));
}

#[test]
fn analyze_game_wires_and_skips_remakes() {
    let (m, t) = laner_fixture();
    let g = analyze_game(&m, Some(&t), &pu(3)).unwrap();
    assert_eq!(g.team_id, 100);
    assert_eq!(g.early.unwrap().solo_kills15, 2);
    assert_eq!(g.roamed, Some(false));
    assert!(g.jungle.is_none());
    close(g.cs_per_min, 200.0 / 30.0, 5);
    assert!(analyze_game(&make_match(&MatchOpts { pid: 3, duration_sec: 200.0, ..mo("EUN1_11") }), None, &pu(3)).is_none());
    assert!(analyze_game(&m, None, "nobody").is_none());
}

// ---------------- aggregation & summary ----------------

#[test]
fn recent_form_from_most_recent() {
    assert_eq!(recent_form(&[(true, 1.0), (false, 4.0), (false, 3.0), (false, 2.0)]), RecentForm { wins: 1, losses: 3, streak: -3 });
    assert_eq!(recent_form(&[]).streak, 0);
}

#[test]
fn aggressive_bot_side_jungler_profile() {
    let mut games = Vec::new();
    for i in 0..8 {
        let m = make_match(&MatchOpts { pid: 7, position: Some("JUNGLE".into()), win: i % 2 == 0, fb_kill: i < 4, end: 1e12 + i as f64 * 1e7, ..mo(&format!("EUN1_{}", 100 + i)) });
        let t = tl(&m, &[(2, 7, RED_BLUEBUFF)], &[kill(3.0 * MIN + 20_000.0, 7, 4, BOT_LANE, None), kill(5.0 * MIN, 7, 3, BOT_LANE, None), kill(9.0 * MIN, 2, 7, MID_LANE, None), kill(11.0 * MIN, 7, 5, BOT_LANE, Some(vec![8]))]);
        games.push(analyze_game(&m, Some(&t), &pu(7)).unwrap());
    }
    let pr = build_profile(&ProfileInput { puuid: &pu(7), riot_id: "WW#EUNE", champion: "Warwick", role: "JUNGLE", rank: Some("GOLD II 45 LP".into()), games: &games, mastery_points: None, now: 5.0 });
    assert_eq!(pr.games_analyzed, 8);
    assert_eq!(pr.champion_games, 8);
    assert_eq!(pr.champion_win_rate, Some(0.5));
    assert_eq!(pr.first_blood_rate, 0.5);
    assert_eq!(pr.kills_before15, 3.0);
    assert_eq!(pr.deaths_before10, 1.0);
    close(pr.aggression, 0.85, 3);
    assert_eq!(pr.jungle.as_ref().unwrap().start_bot_side, 1.0);
    assert_eq!(serde_json::to_value(&pr.jungle.as_ref().unwrap().first_gank_lane).unwrap(), json!({ "BOTTOM": 1.0 }));
    assert_eq!(pr.roam_rate, 0.0);
    assert_eq!(pr.summary, "agresywny, start dół, gank bot ~3:20");
    assert!(pr.summary.split(' ').count() <= 6);
    // Same JSON keys as the TS OpponentProfile.
    let v = serde_json::to_value(&pr).unwrap();
    for k in ["puuid", "riotId", "champion", "role", "rank", "gamesAnalyzed", "aggression", "firstBloodRate", "deathsBefore10", "killsBefore15", "roamRate", "championGames", "championWinRate", "recentForm", "jungle", "summary", "fetchedAt"] {
        assert!(v.get(k).is_some(), "missing {k}");
    }
    assert!(v["jungle"].get("firstGankLane").is_some() && v["jungle"].get("sampleGames").is_some());
}

#[test]
fn passive_farmer_on_new_champion() {
    let mut games = Vec::new();
    for i in 0..6 {
        let mut m = make_match(&MatchOpts { pid: 4, position: Some("BOTTOM".into()), champion: if i < 2 { "Jinx".into() } else { "Caitlyn".into() }, kills: 4, deaths: 2, ..mo(&format!("EUN1_{}", 200 + i)) });
        m.info.participants[3].total_minions_killed = Some(230.0);
        let t = tl(&m, &[], &[]);
        games.push(analyze_game(&m, Some(&t), &pu(4)).unwrap());
    }
    let pr = build_profile(&ProfileInput { puuid: &pu(4), riot_id: "x#y", champion: "Jinx", role: "BOTTOM", rank: None, games: &games, mastery_points: None, now: 0.0 });
    assert_eq!(pr.aggression, 0.0);
    assert_eq!(pr.champion_games, 2);
    assert_eq!(pr.summary, "pasywny farmer, 2 gry na postaci");
}

#[test]
fn empty_samples_plurals_and_time() {
    let pr = build_profile(&ProfileInput { puuid: "", riot_id: "a#b", champion: "Ahri", role: "MIDDLE", rank: None, games: &[], mastery_points: None, now: 0.0 });
    assert_eq!(pr.summary, "brak danych");
    assert_eq!(pr.champion_win_rate, None);
    assert_eq!(pl_plural(1, "gra", "gry", "gier"), "gra");
    assert_eq!(pl_plural(3, "gra", "gry", "gier"), "gry");
    assert_eq!(pl_plural(12, "gra", "gry", "gier"), "gier");
    assert_eq!(pl_plural(22, "gra", "gry", "gier"), "gry");
    assert_eq!(fmt_minute(3.0 + 20.0 / 60.0), "3:20");
    let q = OpponentProfile { games_analyzed: 5, aggression: 0.5, recent_form: RecentForm { wins: 0, losses: 5, streak: -5 }, champion_games: 3, ..pr };
    assert_eq!(summarize(&q, &SummaryExtras::default()), "5 porażek z rzędu");
    // Never played (mastery fetched, none) → "nowa postać".
    let n = OpponentProfile { champion_games: 0, recent_form: RecentForm::default(), ..q };
    assert_eq!(summarize(&n, &SummaryExtras { mastery_points: Some(None), ..Default::default() }), "nowa postać");
}

#[test]
fn picks_same_role_same_champion_timelines_first() {
    let ms = [
        make_match(&MatchOpts { pid: 2, position: Some("JUNGLE".into()), champion: "LeeSin".into(), end: 3e12, ..mo("EUN1_1") }),
        make_match(&MatchOpts { pid: 2, position: Some("JUNGLE".into()), champion: "Warwick".into(), end: 2e12, ..mo("EUN1_2") }),
        make_match(&MatchOpts { pid: 2, position: Some("TOP".into()), champion: "Warwick".into(), end: 4e12, ..mo("EUN1_3") }),
        make_match(&MatchOpts { pid: 2, position: Some("JUNGLE".into()), champion: "Warwick".into(), end: 1e12, ..mo("EUN1_4") }),
    ];
    let ids: Vec<&str> = pick_timeline_games(&ms, &pu(2), "JUNGLE", "Warwick").iter().map(|m| m.metadata.match_id.as_str()).collect();
    assert_eq!(ids, ["EUN1_2", "EUN1_4", "EUN1_1"]);
}

// ---------------- client: cache, retries, key hygiene ----------------

const KEY: &str = "RGAPI-test-key-0000";

type Handler = dyn Fn(&str) -> HttpResponse + Send + Sync;

/// Fake transport: records (url, headers) and answers through `handler`.
struct FakeHttp {
    handler: Box<Handler>,
    calls: Mutex<Vec<(String, Vec<(String, String)>)>>,
    times: Mutex<Vec<tokio::time::Instant>>,
}

impl FakeHttp {
    fn new(h: impl Fn(&str) -> HttpResponse + Send + Sync + 'static) -> Arc<FakeHttp> {
        Arc::new(FakeHttp { handler: Box::new(h), calls: Default::default(), times: Default::default() })
    }
    fn urls(&self) -> Vec<String> {
        self.calls.lock().unwrap().iter().map(|c| c.0.clone()).collect()
    }
    fn count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

impl RiotHttp for FakeHttp {
    fn get<'a>(&'a self, url: &'a str, headers: &'a [(&'a str, &'a str)], _t: Duration) -> BoxFuture<'a, Result<HttpResponse, String>> {
        self.calls.lock().unwrap().push((url.to_string(), headers.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()));
        self.times.lock().unwrap().push(tokio::time::Instant::now());
        let r = (self.handler)(url);
        Box::pin(async move {
            tokio::task::yield_now().await;
            Ok(r)
        })
    }
}

fn json_res(body: &Value) -> HttpResponse {
    HttpResponse { status: 200, body: body.to_string(), ..Default::default() }
}
fn status(s: u16) -> HttpResponse {
    HttpResponse { status: s, body: "{}".into(), ..Default::default() }
}

fn client_with(http: Arc<FakeHttp>, f: impl FnOnce(&mut RiotClientOptions)) -> RiotClient {
    let mut o = RiotClientOptions::new(KEY);
    o.http = Some(http);
    f(&mut o);
    RiotClient::new(o).unwrap()
}

#[tokio::test]
async fn serves_immutable_matches_from_disk_cache_across_instances() {
    let dir = tempfile::tempdir().unwrap();
    let m = serde_json::to_value(make_match(&mo("EUN1_42"))).unwrap();
    let http = FakeHttp::new(move |_| json_res(&m));
    let c1 = client_with(http.clone(), |o| o.cache_dir = Some(dir.path().to_path_buf()));
    let o = CallOptions::default();
    assert_eq!(c1.get_match("EUN1_42", &o).await.unwrap().unwrap().metadata.match_id, "EUN1_42");
    c1.get_match("EUN1_42", &o).await.unwrap();
    assert_eq!(http.urls(), ["https://europe.api.riotgames.com/lol/match/v5/matches/EUN1_42"]);
    let hdr = http.calls.lock().unwrap()[0].1.clone();
    assert!(hdr.contains(&("X-Riot-Token".into(), KEY.into())));
    let c2 = client_with(http.clone(), |o| o.cache_dir = Some(dir.path().to_path_buf()));
    c2.get_match("EUN1_42", &o).await.unwrap();
    assert_eq!(http.count(), 1);
    assert_eq!(c2.request_count(), 0);
    // The key is never in a URL nor in a cache file.
    assert!(http.urls().iter().all(|u| !u.contains(KEY)));
    let cached = std::fs::read_to_string(dir.path().join("match").join("EUN1_42.json")).unwrap();
    assert!(!cached.contains(KEY));
}

#[tokio::test]
async fn ttl_expiry_and_routing() {
    let now = Arc::new(Mutex::new(1_000_000.0));
    let n2 = now.clone();
    let http = FakeHttp::new(|_| json_res(&json!(["EUN1_1"])));
    let dir = tempfile::tempdir().unwrap();
    let c = client_with(http.clone(), |o| {
        o.cache_dir = Some(dir.path().to_path_buf());
        o.now = Some(Arc::new(move || *n2.lock().unwrap()));
    });
    let o = CallOptions::default();
    let q = MatchIdsQuery { queue: Some(420), count: Some(20), ..Default::default() };
    c.get_match_ids("abc", &q, &o).await.unwrap();
    c.get_match_ids("abc", &q, &o).await.unwrap();
    assert_eq!(http.count(), 1);
    assert_eq!(http.urls()[0], "https://europe.api.riotgames.com/lol/match/v5/matches/by-puuid/abc/ids?queue=420&start=0&count=20");
    *now.lock().unwrap() += 11.0 * 60_000.0;
    c.get_match_ids("abc", &q, &o).await.unwrap();
    assert_eq!(http.count(), 2);
    c.get_league_entries("abc", &o).await.ok(); // fake answers an array: payload error is fine here
    assert_eq!(http.urls()[2], "https://eun1.api.riotgames.com/lol/league/v4/entries/by-puuid/abc");
    let na = client_with(http.clone(), |o| o.platform = Some("na1".into()));
    assert_eq!(na.region, "americas");
    // account-v1 for "sea" platforms goes to asia
    let oc = client_with(http.clone(), |o| o.platform = Some("oc1".into()));
    oc.get_account_by_riot_id("Gracz", "OCE", &o).await.ok();
    assert!(http.urls().last().unwrap().starts_with("https://asia.api.riotgames.com/riot/account/v1/accounts/by-riot-id/Gracz/OCE"));
}

#[tokio::test]
async fn null_on_404_not_cached_and_inflight_dedup() {
    let http = FakeHttp::new(|_| status(404));
    let c = client_with(http.clone(), |_| {});
    let o = CallOptions::default();
    let (a, b) = tokio::join!(c.get_timeline("EUN1_9", &o), c.get_timeline("EUN1_9", &o));
    assert!(a.unwrap().is_none());
    assert!(b.unwrap().is_none());
    assert_eq!(http.count(), 1);
    c.get_timeline("EUN1_9", &o).await.unwrap();
    assert_eq!(http.count(), 2);
}

#[tokio::test(start_paused = true)]
async fn retries_429_and_5xx_errors_never_contain_the_key() {
    let seq = Arc::new(Mutex::new(vec![HttpResponse { status: 429, retry_after: Some("2".into()), body: "{}".into(), ..Default::default() }, status(503), json_res(&json!({ "puuid": "p1" }))]));
    let s2 = seq.clone();
    let http = FakeHttp::new(move |_| s2.lock().unwrap().remove(0));
    let c = client_with(http.clone(), |_| {});
    let acc = c.get_account_by_riot_id("Gracz", "EUNE", &CallOptions::default()).await.unwrap();
    assert_eq!(acc.unwrap().puuid, "p1");
    let t = http.times.lock().unwrap().clone();
    assert!(t[1] - t[0] >= Duration::from_millis(2000));
    assert!(t[2] - t[1] >= Duration::from_millis(500));

    let bad = client_with(FakeHttp::new(|_| status(403)), |o| o.max_retries = 0);
    let err = bad.get_match("EUN1_1", &CallOptions::default()).await.unwrap_err();
    assert!(!err.to_string().contains(KEY));
    assert!(!format!("{err:?}").contains(KEY));
    assert_eq!(err.status(), Some(403));
    assert!(RiotClient::new(RiotClientOptions::new("  ")).err().unwrap().to_string().contains("missing"));
    assert!(RiotClient::new(RiotClientOptions { platform: Some("xx".into()), ..RiotClientOptions::new(KEY) }).is_err());

    // Network errors after retries name the endpoint (puuid redacted), never the key.
    struct Down;
    impl RiotHttp for Down {
        fn get<'a>(&'a self, _u: &'a str, _h: &'a [(&'a str, &'a str)], _t: Duration) -> BoxFuture<'a, Result<HttpResponse, String>> {
            Box::pin(async { Err("ConnectError".to_string()) })
        }
    }
    let mut o = RiotClientOptions::new(KEY);
    o.http = Some(Arc::new(Down));
    o.max_retries = 1;
    let down = RiotClient::new(o).unwrap();
    let e = down.get_league_entries(&"p".repeat(78), &CallOptions::default()).await.unwrap_err();
    assert_eq!(e.to_string(), "Riot request failed on /lol/league/v4/entries/by-puuid/{id}: ConnectError");
    assert_eq!(down.request_count(), 2);
}

#[tokio::test(start_paused = true)]
async fn deadline_stops_backoff() {
    let http = FakeHttp::new(|_| HttpResponse { status: 429, retry_after: Some("30".into()), body: "{}".into(), ..Default::default() });
    let c = client_with(http.clone(), |_| {});
    let o = CallOptions { deadline: Some(tokio::time::Instant::now() + Duration::from_secs(5)), budget: None };
    assert_eq!(c.get_match("EUN1_1", &o).await.unwrap_err(), RiotError::Deadline);
    assert_eq!(http.count(), 1);
}

#[tokio::test]
async fn budgets_but_cache_hits_still_served() {
    let http = FakeHttp::new(|url| json_res(&serde_json::to_value(make_match(&mo(url.rsplit('/').next().unwrap()))).unwrap()));
    let c = client_with(http.clone(), |_| {});
    let b = RequestBudget::new(1, None);
    let o = CallOptions { budget: Some(b), deadline: None };
    c.get_match("EUN1_1", &o).await.unwrap();
    assert_eq!(c.get_match("EUN1_2", &o).await.unwrap_err(), RiotError::Budget);
    assert_eq!(c.get_match("EUN1_1", &o).await.unwrap().unwrap().metadata.match_id, "EUN1_1");
    let parent = RequestBudget::new(1, None);
    let child = RequestBudget::new(5, Some(parent));
    assert!(child.take());
    assert!(!child.take());
}

// ---------------- orchestration ----------------

fn match_for(players: &[(String, String, String)], pi: usize, k: usize) -> MatchDto {
    let pid = (if k % 2 == 1 { 6 } else { 1 }) + pi as i64;
    make_match(&MatchOpts {
        pid,
        puuid: Some(format!("pu-{pi}")),
        position: Some(players[pi].2.clone()),
        champion: players[pi].1.clone(),
        win: k % 3 != 0,
        end: 2e12 + k as f64 * 1e7,
        queue_id: if k == 15 { 450 } else { 420 },
        ..mo(&format!("EUN1_{pi}{k:03}"))
    })
}

fn orchestration_http(players: Vec<(String, String, String)>) -> Arc<FakeHttp> {
    let re_acc = regex::Regex::new(r"by-riot-id/Enemy(\d)/").unwrap();
    let re_ids = regex::Regex::new(r"by-puuid/pu-(\d)/ids").unwrap();
    let re_match = regex::Regex::new(r"matches/EUN1_(\d)(\d{3})(/timeline)?$").unwrap();
    FakeHttp::new(move |url| {
        let (path, query) = url.split_once('?').unwrap_or((url, ""));
        if let Some(m) = re_acc.captures(path) {
            return json_res(&json!({ "puuid": format!("pu-{}", &m[1]) }));
        }
        if path.contains("/league/v4/") {
            return json_res(&json!([{ "queueType": "RANKED_SOLO_5x5", "tier": "GOLD", "rank": "II", "leaguePoints": 45 }]));
        }
        if let Some(m) = re_ids.captures(path) {
            if !query.split('&').any(|kv| kv == "type=ranked") {
                return json_res(&json!([]));
            }
            return json_res(&json!((0..20).map(|k| format!("EUN1_{}{k:03}", &m[1])).collect::<Vec<_>>()));
        }
        if let Some(m) = re_match.captures(path) {
            let pi: usize = m[1].parse().unwrap();
            let k: usize = m[2].parse().unwrap();
            let mt = match_for(&players, pi, k);
            if m.get(3).is_none() {
                return json_res(&serde_json::to_value(&mt).unwrap());
            }
            let pid = mt.info.participants.iter().find(|x| x.puuid == format!("pu-{pi}")).unwrap().participant_id;
            let team = if pid <= 5 { 100 } else { 200 };
            let t = tl(&mt, &[(2, pid, if team == 100 { BLUE_REDBUFF } else { RED_BLUEBUFF })], &[kill(3.0 * MIN + 20_000.0, pid, if team == 100 { 9 } else { 4 }, BOT_LANE, None)]);
            return json_res(&serde_json::to_value(&t).unwrap());
        }
        status(404)
    })
}

#[tokio::test]
async fn build_profiles_cold_warm_and_extended() {
    let roles = ["TOP", "JUNGLE", "MIDDLE", "BOTTOM", "UTILITY"];
    let tuples: Vec<(String, String, String)> = roles.iter().enumerate().map(|(i, r)| (format!("Enemy{i}#EUNE"), if i == 1 { "Warwick".into() } else { format!("Champ{i}") }, r.to_string())).collect();
    let players: Vec<ProfilePlayer> = tuples.iter().map(|(id, c, r)| ProfilePlayer { riot_id: id.clone(), champion: c.clone(), role: r.clone(), ..Default::default() }).collect();
    let http = orchestration_http(tuples.clone());
    let client = client_with(http.clone(), |o| o.limits = vec![rc_core::riot::limiter::RateLimit { count: 10_000, window_ms: 1000.0 }]);
    let progress = Arc::new(Mutex::new(Vec::<String>::new()));
    let pg = progress.clone();
    let opts = BuildProfilesOptions { on_progress: Some(Box::new(move |p: &ProfileProgress| pg.lock().unwrap().push(format!("{}:{}", p.riot_id, p.stage)))), ..Default::default() };
    let profiles = build_profiles(&client, &players, &opts).await;

    assert_eq!(profiles.len(), 5);
    let requests = http.count();
    assert!(requests <= 95, "{requests}");
    assert_eq!(client.request_count() as usize, requests);
    let ww = &profiles[1];
    assert_eq!(ww.puuid, "pu-1");
    assert_eq!(ww.rank.as_deref(), Some("GOLD II 45 LP"));
    assert_eq!(ww.games_analyzed, 11); // 12 fetched, one ARAM (queue 450) dropped
    let j = ww.jungle.as_ref().unwrap();
    assert_eq!(j.sample_games, 10);
    assert_eq!(j.start_bot_side, 1.0);
    assert_eq!(serde_json::to_value(&j.first_gank_lane).unwrap(), json!({ "BOTTOM": 1.0 }));
    assert!(ww.summary.contains("start dół"), "{}", ww.summary);
    assert_eq!(profiles[0].games_analyzed, 6); // 7 newest fetched, ARAM dropped
    assert_eq!(progress.lock().unwrap().iter().filter(|s| s.ends_with(":done")).count(), 5);

    // Warm run with no network allowance: everything comes from cache.
    let before = http.count();
    let warm = build_profiles(&client, &players, &BuildProfilesOptions { max_match_fetch: RoleCaps { jungle: 0, laner: 0 }, max_timelines: RoleCaps { jungle: 0, laner: 0 }, ..Default::default() }).await;
    assert_eq!(http.count(), before);
    assert_eq!(warm.iter().map(|p| p.games_analyzed).collect::<Vec<_>>(), profiles.iter().map(|p| p.games_analyzed).collect::<Vec<_>>());
    assert_eq!(warm[1].jungle.as_ref().unwrap().sample_games, 10);
    // A normal warm run extends the sample with new downloads instead of re-fetching.
    let extended = build_profiles(&client, &players, &BuildProfilesOptions::default()).await;
    assert!(extended[1].games_analyzed > 11);
}

#[tokio::test]
async fn build_profiles_shrinks_plan_when_limiter_is_busy() {
    let n = Arc::new(AtomicUsize::new(0));
    let n2 = n.clone();
    let http = FakeHttp::new(move |_| {
        n2.fetch_add(1, Ordering::SeqCst);
        json_res(&json!({ "puuid": "p" }))
    });
    let client = client_with(http, |_| {});
    client.limiter_for("europe").record_grants(95); // 95 of 100 tokens used just now
    build_profiles(&client, &[ProfilePlayer { riot_id: "A#EUNE".into(), champion: "Ahri".into(), role: "MIDDLE".into(), ..Default::default() }], &BuildProfilesOptions { timeout_ms: 5000, ..Default::default() }).await;
    assert!(n.load(Ordering::SeqCst) <= 2, "{}", n.load(Ordering::SeqCst));
}

#[tokio::test]
async fn build_profiles_partial_when_unresolved() {
    let client = client_with(FakeHttp::new(|_| status(404)), |_| {});
    let r = build_profiles(&client, &[ProfilePlayer { riot_id: "Ghost#EUNE".into(), champion: "Ahri".into(), role: "MIDDLE".into(), ..Default::default() }], &BuildProfilesOptions::default()).await;
    assert_eq!(r[0].summary, "brak danych");
    assert_eq!(r[0].games_analyzed, 0);
}
