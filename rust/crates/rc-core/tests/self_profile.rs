//! Player self profile (`src/main/riot/selfProfile.ts`) against a fake Riot HTTP:
//! Riot ID -> account puuid, match filtering, aggregates, weaknesses, lessons, file I/O.

use rc_core::llm::caller::BoxFuture;
use rc_core::riot::client::*;
use rc_core::riot::limiter::RateLimit;
use rc_core::riot::self_profile::*;
use rc_core::riot::types::*;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const KEY: &str = "RGAPI-test-key-0000";
const ME: &str = "account-puuid-from-riot-id-000000000000000";
const MIN: f64 = 60_000.0;

type Handler = dyn Fn(&str) -> HttpResponse + Send + Sync;

struct FakeHttp {
    handler: Box<Handler>,
    urls: Mutex<Vec<String>>,
}

impl FakeHttp {
    fn new(h: impl Fn(&str) -> HttpResponse + Send + Sync + 'static) -> Arc<FakeHttp> {
        Arc::new(FakeHttp { handler: Box::new(h), urls: Default::default() })
    }
    fn urls(&self) -> Vec<String> {
        self.urls.lock().unwrap().clone()
    }
}

impl RiotHttp for FakeHttp {
    fn get<'a>(&'a self, url: &'a str, _h: &'a [(&'a str, &'a str)], _t: Duration) -> BoxFuture<'a, Result<HttpResponse, String>> {
        self.urls.lock().unwrap().push(url.to_string());
        let r = (self.handler)(url);
        Box::pin(async move { Ok(r) })
    }
}

fn ok(v: &Value) -> HttpResponse {
    HttpResponse { status: 200, body: v.to_string(), ..Default::default() }
}
fn status(s: u16) -> HttpResponse {
    HttpResponse { status: s, body: "{}".into(), ..Default::default() }
}

fn client(http: Arc<FakeHttp>) -> RiotClient {
    let mut o = RiotClientOptions::new(KEY);
    o.http = Some(http);
    o.max_retries = 0;
    o.limits = vec![RateLimit { count: 10_000, window_ms: 1000.0 }];
    RiotClient::new(o).unwrap()
}

#[derive(Clone)]
struct G {
    id: &'static str,
    queue: i64,
    secs: f64,
    champ: &'static str,
    pos: &'static str,
    win: bool,
    kda: (i64, i64, i64),
    cs: f64,
    vision: f64,
}

fn g(id: &'static str, champ: &'static str, win: bool, kda: (i64, i64, i64)) -> G {
    G { id, queue: 420, secs: 1200.0, champ, pos: "MIDDLE", win, kda, cs: 100.0, vision: 10.0 }
}

/// Player is participant 3 (blue); the 4 teammates have 2 kills each.
fn make_match(o: &G) -> MatchDto {
    let participants: Vec<MatchParticipantDto> = (1..=10)
        .map(|id| {
            let me = id == 3;
            let team_id = if id <= 5 { 100 } else { 200 };
            MatchParticipantDto {
                puuid: if me { ME.into() } else { format!("other-{}-{id}", o.id) },
                participant_id: id,
                team_id,
                champion_name: if me { o.champ.into() } else { format!("Champ{id}") },
                team_position: Some(if me { o.pos.into() } else { "TOP".into() }),
                win: (team_id == 100) == o.win,
                kills: if me { o.kda.0 } else { 2 },
                deaths: if me { o.kda.1 } else { 2 },
                assists: if me { o.kda.2 } else { 2 },
                total_minions_killed: Some(if me { o.cs } else { 150.0 }),
                neutral_minions_killed: Some(0.0),
                vision_score: Some(if me { o.vision } else { 30.0 }),
                ..Default::default()
            }
        })
        .collect();
    MatchDto {
        metadata: MatchMetadata { match_id: o.id.into(), participants: participants.iter().map(|p| p.puuid.clone()).collect() },
        info: MatchInfo { game_duration: o.secs, game_end_timestamp: Some(1.7e12), queue_id: o.queue, map_id: Some(11), participants, ..Default::default() },
    }
}

/// Timeline with deaths of participant 3 at the given minutes (pid resolved from metadata order).
fn make_timeline(m: &MatchDto, death_minutes: &[f64]) -> TimelineDto {
    let events: Vec<TimelineEventDto> = death_minutes.iter().map(|&t| TimelineEventDto { kind: "CHAMPION_KILL".into(), timestamp: t * MIN, killer_id: Some(7), victim_id: Some(3), ..Default::default() }).collect();
    TimelineDto { metadata: m.metadata.clone(), info: TimelineInfo { frame_interval: Some(MIN), frames: vec![TimelineFrameDto { timestamp: 0.0, events, ..Default::default() }], participants: None } }
}

fn riot(ids: Vec<&'static str>, games: Vec<G>, timelines: HashMap<&'static str, Vec<f64>>) -> Arc<FakeHttp> {
    let matches: HashMap<String, MatchDto> = games.iter().map(|x| (x.id.to_string(), make_match(x))).collect();
    FakeHttp::new(move |url| {
        if url.contains("/riot/account/v1/accounts/by-riot-id/Gracz/EUNE") {
            return ok(&json!({ "puuid": ME, "gameName": "Gracz", "tagLine": "EUNE" }));
        }
        if url.contains("/riot/account/v1/") {
            return status(404);
        }
        if url.contains(&format!("/by-puuid/{ME}/ids")) {
            return ok(&json!(ids));
        }
        if url.contains("/by-puuid/") {
            return status(404);
        }
        let id = url.rsplit("/matches/").next().unwrap_or("");
        if let Some(mid) = id.strip_suffix("/timeline") {
            return match (matches.get(mid), timelines.get(mid)) {
                (Some(m), Some(d)) => ok(&serde_json::to_value(make_timeline(m, d)).unwrap()),
                _ => status(404),
            };
        }
        match matches.get(id) {
            Some(m) => ok(&serde_json::to_value(m).unwrap()),
            None => status(404),
        }
    })
}

fn reviews() -> Vec<Value> {
    vec![
        json!({ "createdAt": 1.0, "improve": ["Starsza lekcja"] }),
        json!({ "createdAt": 2.0, "improve": [" Ward przed smokiem ", "x"] }),
    ]
}

#[tokio::test]
async fn builds_profile_from_riot_id_resolved_puuid() {
    let mut aram = g("ARAM", "Ahri", true, (20, 0, 20));
    aram.queue = 450;
    let mut short = g("SHORT", "Ahri", true, (20, 0, 20));
    short.secs = 500.0;
    let mut g3 = g("M3", "Syndra", true, (6, 4, 6));
    g3.pos = ""; // no position: does not count for the main role
    let games = vec![g("M1", "Ahri", true, (4, 2, 4)), aram, g("M2", "Ahri", false, (2, 8, 2)), short, g3, g("M4", "Ahri", false, (0, 6, 0))];
    // M4 has no timeline (404): averaged over 3 timelines.
    let tls = HashMap::from([("M1", vec![3.0, 9.98, 12.0]), ("M2", vec![5.0]), ("M3", vec![1.0, 2.0])]);
    let http = riot(vec!["M1", "ARAM", "M2", "SHORT", "GONE", "M3", "M4"], games, tls);
    let c = client(http.clone());

    // The LCU puuid is never used: the profile is built for the account-v1 puuid of the Riot ID.
    let p = build_for_riot_id(&c, "Gracz#EUNE", &reviews(), 1234.0).await.unwrap().expect("account");
    assert!(http.urls().iter().any(|u| u.contains(&format!("/lol/match/v5/matches/by-puuid/{ME}/ids?start=0&count=30"))));
    assert!(build_for_riot_id(&c, "Nikt#EUNE", &[], 0.0).await.unwrap().is_none());

    assert_eq!((p.games, p.wins), (4, 2));
    assert_eq!(p.main_role.as_deref(), Some("MIDDLE"));
    assert_eq!(p.champions, vec![ChampionGames { name: "Ahri".into(), games: 3, wins: 1 }, ChampionGames { name: "Syndra".into(), games: 1, wins: 1 }]);
    assert_eq!(p.kda, 1.2); // (12 + 12) / 20
    assert_eq!(p.deaths_per_game, 5.0);
    assert_eq!(p.cs_per_min, 5.0); // 400 CS / 80 min
    assert_eq!(p.vision_per_min, 0.5);
    assert_eq!(p.kill_participation, 0.48); // (8/12 + 4/10 + 12/14 + 0) / 4
    assert_eq!(p.deaths_before10, Some(1.7)); // (2 + 1 + 2) / 3
    assert_eq!(p.weaknesses, vec!["niska farma (5 CS/min)", "dużo śmierci przed 10. minutą (1.7/mecz)", "mało wizji (0.5/min)"]);
    assert_eq!(p.lessons, vec!["Ward przed smokiem", "Starsza lekcja"]);
    assert_eq!(p.summary, "4 ostatnich gier na SR (2W 2L), głównie mid, postacie: Ahri, Syndra. KDA 1.2, 5 CS/min, 5 śmierci/mecz (1.7 przed 10. min), KP 48%.");
    assert_eq!(p.updated_at, 1234.0);

    let text = self_profile_for_coach(Some(&p)).unwrap();
    assert_eq!(
        text,
        "Profil gracza (Ty): 4 ostatnich gier na SR (2W 2L), głównie mid, postacie: Ahri, Syndra. KDA 1.2, 5 CS/min, 5 śmierci/mecz (1.7 przed 10. min), KP 48%.\n\
         Jego słabe strony: niska farma (5 CS/min); dużo śmierci przed 10. minutą (1.7/mecz); mało wizji (0.5/min).\n\
         Najważniejsze uwagi z jego ostatnich analiz:\n- Ward przed smokiem\n- Starsza lekcja\n\
         Gdy sytuacja dotyczy jego słabej strony (np. wychodzi daleko bez wizji, a często ginie od ganków), przypomnij mu o tym krótko w poradzie."
    );
}

#[tokio::test]
async fn caps_matches_and_timelines() {
    let names: Vec<&'static str> = (0..30).map(|i| &*Box::leak(format!("EUN1_{i}").into_boxed_str())).collect();
    let games: Vec<G> = names.iter().map(|&id| g(id, "Ahri", true, (5, 1, 5))).collect();
    let tls: HashMap<&'static str, Vec<f64>> = names.iter().map(|&id| (id, vec![])).collect();
    let http = riot(names, games, tls);
    let p = build_self_profile(&client(http.clone()), ME, &[], 0.0).await.unwrap();
    assert_eq!(p.games, MATCHES as u32);
    let urls = http.urls();
    assert_eq!(urls.iter().filter(|u| u.contains("/matches/EUN1_") && !u.ends_with("/timeline")).count(), MATCHES);
    assert_eq!(urls.iter().filter(|u| u.ends_with("/timeline")).count(), TIMELINES);
    assert_eq!(p.deaths_before10, Some(0.0));
}

#[tokio::test]
async fn bad_key_fails_and_never_leaks_it() {
    let c = client(FakeHttp::new(|_| status(403)));
    let e = build_self_profile(&c, ME, &[], 0.0).await.unwrap_err();
    assert_eq!(e.status(), Some(403));
    assert!(!format!("{e} {e:?}").contains(KEY));
    assert!(build_for_riot_id(&c, "Gracz#EUNE", &[], 0.0).await.is_err());
}

#[tokio::test]
async fn no_games_gives_empty_profile_with_lessons() {
    let http = riot(vec![], vec![], HashMap::new());
    let p = build_self_profile(&client(http), ME, &reviews(), 0.0).await.unwrap();
    assert_eq!((p.games, p.kda, p.deaths_before10, p.main_role.clone()), (0, 0.0, None, None));
    assert!(p.summary.starts_with("0 ostatnich gier na SR (0W 0L), głównie różne role. KDA 0"));
    // Coach still gets the review lessons.
    assert!(self_profile_for_coach(Some(&p)).unwrap().contains("- Ward przed smokiem"));
}

#[test]
fn file_roundtrip_and_ts_written_json() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("sub").join(FILE_NAME);
    assert!(load_self_profile(&file).is_none());
    let p = PlayerSelfProfile { games: 3, wins: 2, main_role: Some("JUNGLE".into()), deaths_before10: Some(0.7), lessons: vec!["x".into()], updated_at: 1.7e12, ..Default::default() };
    save_self_profile(&file, &p).unwrap();
    assert_eq!(load_self_profile(&file), Some(p));
    // camelCase JSON as the UI / Electron app writes it (integers, null).
    let v = serde_json::to_value(PlayerSelfProfile::default()).unwrap();
    assert!(v.get("deathsBefore10").is_some_and(Value::is_null) && v.get("mainRole").is_some_and(Value::is_null));
    assert!(v.get("killParticipation").is_some() && v.get("updatedAt").is_some() && v.get("csPerMin").is_some());
    std::fs::write(&file, r#"{"games":15,"wins":8,"mainRole":"MIDDLE","champions":[{"name":"Ahri","games":9,"wins":5}],"kda":2.81,"deathsPerGame":5.1,"csPerMin":7.2,"visionPerMin":0.71,"killParticipation":0.52,"deathsBefore10":null,"weaknesses":[],"lessons":[],"summary":"s","updatedAt":1762900000000}"#).unwrap();
    let t = load_self_profile(&file).unwrap();
    assert_eq!((t.games, t.champions[0].games, t.deaths_before10, t.updated_at), (15, 9, None, 1762900000000.0));
    std::fs::write(&file, "{broken").unwrap();
    assert!(load_self_profile(&file).is_none());
}
