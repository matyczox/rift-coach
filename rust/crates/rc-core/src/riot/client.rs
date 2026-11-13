//! Riot Web API client for the user's personal development key
//! (`src/main/riot/client.ts`).
//! - The key is only ever sent in the `X-Riot-Token` header; it is never
//!   logged, never put in URLs and never included in error messages
//!   (`RiotClient` has no `Debug` that could print it).
//! - Rate limits are per routing value (host), so each host has its own limiter.
//! - 404 → `None`. 429 → honour Retry-After (pauses the whole host), retry.
//!   5xx / network errors → exponential backoff retry. 401/403 → `RiotError::Api`.

use super::cache::{epoch_ms, DiskCache, NowFn};
use super::limiter::{RateLimit, RateLimiter, DEV_KEY_LIMITS};
use super::types::*;
use crate::llm::caller::BoxFuture;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::OnceCell;
use tokio::time::Instant;

pub const PLATFORMS: [(&str, &str, &str); 15] = [
    // platform, region, default tag line
    ("eun1", "europe", "EUNE"),
    ("euw1", "europe", "EUW"),
    ("tr1", "europe", "TR1"),
    ("ru", "europe", "RU"),
    ("me1", "europe", "ME1"),
    ("na1", "americas", "NA1"),
    ("br1", "americas", "BR1"),
    ("la1", "americas", "LAN"),
    ("la2", "americas", "LAS"),
    ("kr", "asia", "KR1"),
    ("jp1", "asia", "JP1"),
    ("oc1", "sea", "OCE"),
    ("sg2", "sea", "SG2"),
    ("tw2", "sea", "TW2"),
    ("vn2", "sea", "VN2"),
];

pub fn is_platform(s: &str) -> bool {
    PLATFORMS.iter().any(|p| p.0 == s)
}
/// PLATFORM_REGION
pub fn platform_region(p: &str) -> Option<&'static str> {
    PLATFORMS.iter().find(|x| x.0 == p).map(|x| x.1)
}
/// PLATFORM_DEFAULT_TAG: tag line guess when a Riot ID comes without "#TAG".
pub fn platform_default_tag(p: &str) -> Option<&'static str> {
    PLATFORMS.iter().find(|x| x.0 == p).map(|x| x.2)
}

pub mod queues {
    pub const RANKED_SOLO: i64 = 420;
    pub const RANKED_FLEX: i64 = 440;
    pub const NORMAL_DRAFT: i64 = 400;
    pub const SWIFTPLAY: i64 = 480;
}

/// Errors never contain the API key (nor puuids: long path segments become `{id}`).
#[derive(Clone, Debug, PartialEq)]
pub enum RiotError {
    /// HTTP error status (401/403 = key rejected, 429 after retries, other 4xx/5xx).
    Api { status: u16, endpoint: String, message: String },
    /// Request budget exhausted (`BudgetExceededError`).
    Budget,
    /// The request (incl. limiter waits / retries) would run past the deadline.
    Deadline,
    /// Network / timeout error after retries.
    Network { endpoint: String, kind: String },
    /// Unexpected payload.
    Payload { endpoint: String },
}

impl RiotError {
    pub fn status(&self) -> Option<u16> {
        match self {
            RiotError::Api { status, .. } => Some(*status),
            _ => None,
        }
    }
}

impl std::fmt::Display for RiotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RiotError::Api { message, .. } => f.write_str(message),
            RiotError::Budget => f.write_str("Riot request budget exhausted"),
            RiotError::Deadline => f.write_str("Riot request would exceed the deadline"),
            RiotError::Network { endpoint, kind } => write!(f, "Riot request failed on {endpoint}: {kind}"),
            RiotError::Payload { endpoint } => write!(f, "Riot API: unexpected payload on {endpoint}"),
        }
    }
}
impl std::error::Error for RiotError {}

/// Counts network requests (cache hits are free). Budgets can be nested: a
/// per-player budget with a global parent only grants when both have room.
pub struct RequestBudget {
    pub max: usize,
    used: Mutex<usize>,
    parent: Option<Arc<RequestBudget>>,
}

impl RequestBudget {
    pub fn new(max: usize, parent: Option<Arc<RequestBudget>>) -> Arc<RequestBudget> {
        Arc::new(RequestBudget { max, used: Mutex::new(0), parent })
    }
    pub fn used(&self) -> usize {
        *self.used.lock().unwrap()
    }
    pub fn remaining(&self) -> usize {
        let own = self.max.saturating_sub(self.used());
        match &self.parent {
            Some(p) => own.min(p.remaining()),
            None => own,
        }
    }
    pub fn take(&self) -> bool {
        let mut u = self.used.lock().unwrap();
        if *u >= self.max {
            return false;
        }
        if let Some(p) = &self.parent {
            if !p.take() {
                return false;
            }
        }
        *u += 1;
        true
    }
}

#[derive(Clone, Default)]
pub struct CallOptions {
    /// Network requests must fit this budget, else `RiotError::Budget` (cache still served).
    pub budget: Option<Arc<RequestBudget>>,
    /// No request (incl. limiter waits / retries) runs past it → `RiotError::Deadline`.
    pub deadline: Option<Instant>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CacheTtls {
    pub account: f64,
    pub league: f64,
    pub match_ids: f64,
    pub mastery: f64,
}

pub const DEFAULT_TTLS: CacheTtls = CacheTtls {
    account: 24.0 * 3600_000.0, // Riot ID → puuid rarely changes
    league: 10.0 * 60_000.0,
    match_ids: 10.0 * 60_000.0,
    mastery: 60.0 * 60_000.0,
};

/// One HTTP response as the client needs it.
#[derive(Clone, Debug, Default)]
pub struct HttpResponse {
    pub status: u16,
    pub retry_after: Option<String>,
    pub rate_limit_type: Option<String>,
    pub body: String,
}

/// HTTP transport (injectable: tests use a fake). `headers` carries the token.
/// An `Err` is a network-level failure; its text must not contain the URL's secrets
/// (the URL has none: the key travels in the header).
pub trait RiotHttp: Send + Sync {
    fn get<'a>(&'a self, url: &'a str, headers: &'a [(&'a str, &'a str)], timeout: Duration) -> BoxFuture<'a, Result<HttpResponse, String>>;
}

/// reqwest transport (native-tls / SChannel).
pub struct ReqwestHttp(reqwest::Client);

impl Default for ReqwestHttp {
    fn default() -> Self {
        // No redirects: reqwest would carry the X-Riot-Token header to whatever host it is sent to.
        ReqwestHttp(reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().expect("http client"))
    }
}

impl RiotHttp for ReqwestHttp {
    fn get<'a>(&'a self, url: &'a str, headers: &'a [(&'a str, &'a str)], timeout: Duration) -> BoxFuture<'a, Result<HttpResponse, String>> {
        Box::pin(async move {
            let mut rq = self.0.get(url).timeout(timeout);
            for (k, v) in headers {
                rq = rq.header(*k, *v);
            }
            // Only the error kind leaves this function (reqwest errors embed the URL).
            let kind = |e: &reqwest::Error| if e.is_timeout() { "TimeoutError" } else if e.is_connect() { "ConnectError" } else { "NetworkError" }.to_string();
            let res = rq.send().await.map_err(|e| kind(&e))?;
            let h = |n: &str| res.headers().get(n).and_then(|v| v.to_str().ok()).map(str::to_string);
            let (retry_after, rate_limit_type) = (h("retry-after"), h("x-rate-limit-type"));
            let status = res.status().as_u16();
            let body = res.text().await.map_err(|e| kind(&e))?;
            Ok(HttpResponse { status, retry_after, rate_limit_type, body })
        })
    }
}

pub struct RiotClientOptions {
    /// Personal development key (RGAPI-...). Required.
    pub api_key: String,
    /// Cache directory (e.g. `<userData>/riot-cache`). None = memory only.
    pub cache_dir: Option<PathBuf>,
    /// Default "eun1".
    pub platform: Option<String>,
    /// Override the regional routing; default from the platform.
    pub region: Option<String>,
    pub request_timeout_ms: u64,
    /// Per request, for 429/5xx/network.
    pub max_retries: u32,
    pub limits: Vec<RateLimit>,
    pub ttl: CacheTtls,
    pub http: Option<Arc<dyn RiotHttp>>,
    /// Epoch ms for cache TTLs (default: system clock).
    pub now: Option<NowFn>,
}

impl RiotClientOptions {
    pub fn new(api_key: &str) -> Self {
        RiotClientOptions {
            api_key: api_key.to_string(),
            cache_dir: None,
            platform: None,
            region: None,
            request_timeout_ms: 8000,
            max_retries: 3,
            limits: DEV_KEY_LIMITS.to_vec(),
            ttl: DEFAULT_TTLS,
            http: None,
            now: None,
        }
    }
}

type Flight = Arc<OnceCell<Result<Option<Value>, RiotError>>>;

pub struct RiotClient {
    pub platform: String,
    pub region: String,
    request_count: AtomicU64,
    api_key: String,
    cache: DiskCache,
    limiters: Mutex<HashMap<String, Arc<RateLimiter>>>,
    inflight: Mutex<HashMap<String, Flight>>,
    ttl: CacheTtls,
    http: Arc<dyn RiotHttp>,
    timeout_ms: u64,
    max_retries: u32,
    limits: Vec<RateLimit>,
}

/// `encodeURIComponent`.
pub fn encode_uri_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        let c = b as char;
        if c.is_ascii_alphanumeric() || "-_.!~*'()".contains(c) {
            out.push(c);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Path without query, long segments (puuids) replaced by `{id}`.
fn endpoint_of(path_and_query: &str) -> String {
    let p = path_and_query.split('?').next().unwrap_or("");
    p.split('/').map(|seg| if seg.encode_utf16().count() >= 30 { "{id}" } else { seg }).collect::<Vec<_>>().join("/")
}

impl RiotClient {
    pub fn new(opts: RiotClientOptions) -> anyhow::Result<RiotClient> {
        let key = opts.api_key.trim().to_string();
        if key.is_empty() {
            anyhow::bail!("Riot API key missing");
        }
        let platform = opts.platform.unwrap_or_else(|| "eun1".into());
        let Some(default_region) = platform_region(&platform) else { anyhow::bail!("unknown platform {platform}") };
        let region = opts.region.unwrap_or_else(|| default_region.to_string());
        let now = opts.now.unwrap_or_else(|| Arc::new(epoch_ms));
        Ok(RiotClient {
            platform,
            region,
            request_count: AtomicU64::new(0),
            api_key: key,
            cache: DiskCache::new(opts.cache_dir, now),
            limiters: Mutex::new(HashMap::new()),
            inflight: Mutex::new(HashMap::new()),
            ttl: opts.ttl,
            http: opts.http.unwrap_or_else(|| Arc::new(ReqwestHttp::default())),
            timeout_ms: opts.request_timeout_ms,
            max_retries: opts.max_retries,
            limits: opts.limits,
        })
    }

    /// Network requests actually sent (including retries).
    pub fn request_count(&self) -> u64 {
        self.request_count.load(Ordering::SeqCst)
    }

    /// account-v1 supports americas/asia/europe only.
    fn account_region(&self) -> String {
        if self.region == "sea" {
            "asia".into()
        } else {
            self.region.clone()
        }
    }

    pub fn limiter_for(&self, host: &str) -> Arc<RateLimiter> {
        let mut m = self.limiters.lock().unwrap();
        m.entry(host.to_string()).or_insert_with(|| Arc::new(RateLimiter::new(&self.limits, 50.0))).clone()
    }

    // ---------------- endpoints ----------------

    pub async fn get_account_by_riot_id(&self, game_name: &str, tag_line: &str, o: &CallOptions) -> Result<Option<AccountDto>, RiotError> {
        let key = format!("{}#{}", game_name.to_lowercase(), tag_line.to_lowercase());
        let path = format!("/riot/account/v1/accounts/by-riot-id/{}/{}", encode_uri_component(game_name), encode_uri_component(tag_line));
        let host = self.account_region();
        typed(self.cached("account", &key, Some(self.ttl.account), o, &host, &path).await?, &path)
    }

    /// Resolve "Name#TAG" (tag defaults to the platform's usual tag).
    pub async fn resolve_riot_id(&self, riot_id: &str, o: &CallOptions) -> Result<Option<AccountDto>, RiotError> {
        let (game_name, tag_line) = parse_riot_id(riot_id, platform_default_tag(&self.platform).unwrap_or(""));
        if game_name.is_empty() {
            return Ok(None);
        }
        self.get_account_by_riot_id(&game_name, &tag_line, o).await
    }

    pub async fn get_league_entries(&self, puuid: &str, o: &CallOptions) -> Result<Vec<LeagueEntryDto>, RiotError> {
        let path = format!("/lol/league/v4/entries/by-puuid/{}", encode_uri_component(puuid));
        let platform = self.platform.clone();
        Ok(typed(self.cached("league", puuid, Some(self.ttl.league), o, &platform, &path).await?, &path)?.unwrap_or_default())
    }

    pub async fn get_match_ids(&self, puuid: &str, q: &MatchIdsQuery, o: &CallOptions) -> Result<Vec<String>, RiotError> {
        let mut params: Vec<(String, String)> = Vec::new();
        if let Some(qu) = q.queue {
            params.push(("queue".into(), qu.to_string()));
        }
        if let Some(t) = &q.kind {
            params.push(("type".into(), t.clone()));
        }
        if let Some(st) = q.start_time {
            params.push(("startTime".into(), (st.floor() as i64).to_string()));
        }
        params.push(("start".into(), q.start.unwrap_or(0).to_string()));
        params.push(("count".into(), q.count.unwrap_or(20).clamp(1, 100).to_string()));
        let qs = params.iter().map(|(k, v)| format!("{k}={}", encode_uri_component(v))).collect::<Vec<_>>().join("&");
        let path = format!("/lol/match/v5/matches/by-puuid/{}/ids?{qs}", encode_uri_component(puuid));
        let region = self.region.clone();
        Ok(typed(self.cached("matchids", &format!("{puuid}?{qs}"), Some(self.ttl.match_ids), o, &region, &path).await?, &path)?.unwrap_or_default())
    }

    pub async fn get_match(&self, match_id: &str, o: &CallOptions) -> Result<Option<MatchDto>, RiotError> {
        let path = format!("/lol/match/v5/matches/{}", encode_uri_component(match_id));
        let region = self.region.clone();
        typed(self.cached("match", match_id, None, o, &region, &path).await?, &path)
    }

    pub async fn get_timeline(&self, match_id: &str, o: &CallOptions) -> Result<Option<TimelineDto>, RiotError> {
        let path = format!("/lol/match/v5/matches/{}/timeline", encode_uri_component(match_id));
        let region = self.region.clone();
        typed(self.cached("timeline", match_id, None, o, &region, &path).await?, &path)
    }

    pub async fn get_champion_mastery(&self, puuid: &str, champion_id: i64, o: &CallOptions) -> Result<Option<ChampionMasteryDto>, RiotError> {
        let path = format!("/lol/champion-mastery/v4/champion-masteries/by-puuid/{}/by-champion/{champion_id}", encode_uri_component(puuid));
        let platform = self.platform.clone();
        typed(self.cached("mastery", &format!("{puuid}_{champion_id}"), Some(self.ttl.mastery), o, &platform, &path).await?, &path)
    }

    /// spectator-v5: the live game or None when not in game. Not cached.
    pub async fn get_active_game(&self, puuid: &str, o: &CallOptions) -> Result<Option<CurrentGameInfoDto>, RiotError> {
        if let Some(b) = &o.budget {
            if !b.take() {
                return Err(RiotError::Budget);
            }
        }
        let path = format!("/lol/spectator/v5/active-games/by-summoner/{}", encode_uri_component(puuid));
        let platform = self.platform.clone();
        typed(self.request(&platform, &path, o).await?, &path)
    }

    // ---------------- plumbing ----------------

    /// Cache-first fetch with in-flight de-duplication. Only non-null results are
    /// stored (a 404 may be temporary, e.g. a match still being processed).
    async fn cached(&self, ns: &str, key: &str, ttl: Option<f64>, o: &CallOptions, host: &str, path: &str) -> Result<Option<Value>, RiotError> {
        if let Some(hit) = self.cache.get(ns, key, ttl) {
            return Ok(Some(hit));
        }
        let id = format!("{ns}/{key}");
        let (cell, owner) = {
            let mut m = self.inflight.lock().unwrap();
            match m.get(&id) {
                Some(c) => (c.clone(), false),
                None => {
                    if let Some(b) = &o.budget {
                        if !b.take() {
                            return Err(RiotError::Budget);
                        }
                    }
                    let c: Flight = Arc::new(OnceCell::new());
                    m.insert(id.clone(), c.clone());
                    (c, true)
                }
            }
        };
        struct Cleanup<'a>(&'a Mutex<HashMap<String, Flight>>, String, bool);
        impl Drop for Cleanup<'_> {
            fn drop(&mut self) {
                if self.2 {
                    self.0.lock().unwrap().remove(&self.1);
                }
            }
        }
        let _cleanup = Cleanup(&self.inflight, id, owner);
        // Waiters share the owner's result; if the owner was cancelled, a waiter loads instead.
        cell.get_or_init(|| async {
            let v = self.request(host, path, o).await?;
            if let Some(v) = &v {
                self.cache.set(ns, key, v);
            }
            Ok(v)
        })
        .await
        .clone()
    }

    async fn request(&self, host: &str, path_and_query: &str, o: &CallOptions) -> Result<Option<Value>, RiotError> {
        let limiter = self.limiter_for(host);
        let url = format!("https://{host}.api.riotgames.com{path_and_query}");
        let endpoint = endpoint_of(path_and_query);
        let headers = [("X-Riot-Token", self.api_key.as_str()), ("Accept", "application/json")];
        let mut attempt: u32 = 0;
        loop {
            limiter.acquire(o.deadline).await.map_err(|_| RiotError::Deadline)?;
            let remaining = match o.deadline {
                Some(d) => {
                    let now = Instant::now();
                    if now >= d {
                        return Err(RiotError::Deadline);
                    }
                    d.duration_since(now)
                }
                None => Duration::MAX,
            };
            let timeout = Duration::from_millis(self.timeout_ms).min(remaining).max(Duration::from_millis(1));
            self.request_count.fetch_add(1, Ordering::SeqCst);
            let (res, net_err) = match tokio::time::timeout(timeout, self.http.get(&url, &headers, timeout)).await {
                Ok(Ok(r)) => (Some(r), None),
                Ok(Err(e)) => (None, Some(e)),
                Err(_) => (None, Some("TimeoutError".to_string())),
            };
            if let Some(res) = &res {
                match res.status {
                    200 => return serde_json::from_str(&res.body).map(Some).map_err(|_| RiotError::Payload { endpoint }),
                    404 => return Ok(None),
                    401 | 403 => {
                        return Err(RiotError::Api { status: res.status, endpoint, message: format!("Riot API key rejected ({}) — expired or invalid dev key?", res.status) });
                    }
                    429 => {
                        let ra = res.retry_after.as_deref().and_then(|s| s.trim().parse::<f64>().ok()).filter(|x| x.is_finite() && *x > 0.0);
                        let wait_ms = ra.map(|x| x * 1000.0).unwrap_or(1000.0 * 2f64.powi(attempt as i32));
                        // application/method limits block the whole host; "service" limits are per request
                        if res.rate_limit_type.as_deref() != Some("service") {
                            limiter.pause(wait_ms);
                        }
                        if attempt >= self.max_retries {
                            return Err(api_err(429, &endpoint));
                        }
                        attempt += 1;
                        self.backoff(wait_ms, o).await?;
                        continue;
                    }
                    s if s < 500 => return Err(api_err(s, &endpoint)),
                    _ => {}
                }
            }
            // 5xx or network/timeout error
            if attempt >= self.max_retries {
                if let Some(res) = res {
                    return Err(api_err(res.status, &endpoint));
                }
                return Err(RiotError::Network { endpoint, kind: net_err.unwrap_or_else(|| "Error".into()) });
            }
            self.backoff(500.0 * 2f64.powi(attempt as i32), o).await?;
            attempt += 1;
        }
    }

    async fn backoff(&self, ms: f64, o: &CallOptions) -> Result<(), RiotError> {
        let d = Duration::from_secs_f64(ms / 1000.0);
        if let Some(dl) = o.deadline {
            if Instant::now() + d > dl {
                return Err(RiotError::Deadline);
            }
        }
        tokio::time::sleep(d).await;
        Ok(())
    }
}

fn api_err(status: u16, endpoint: &str) -> RiotError {
    RiotError::Api { status, endpoint: endpoint.to_string(), message: format!("Riot API {status} on {endpoint}") }
}

fn typed<T: DeserializeOwned>(v: Option<Value>, path: &str) -> Result<Option<T>, RiotError> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(v) => serde_json::from_value(v).map(Some).map_err(|_| RiotError::Payload { endpoint: endpoint_of(path) }),
    }
}

/// Query of match-v5 `ids`.
#[derive(Clone, Debug, Default)]
pub struct MatchIdsQuery {
    pub queue: Option<i64>,
    /// ranked | normal | tourney | tutorial
    pub kind: Option<String>,
    pub start: Option<u32>,
    pub count: Option<u32>,
    pub start_time: Option<f64>,
}

/// "Name#TAG" → (gameName, tagLine); a missing/empty tag gives `default_tag`.
pub fn parse_riot_id(riot_id: &str, default_tag: &str) -> (String, String) {
    let s = riot_id.trim();
    match s.rfind('#') {
        None => (s.to_string(), default_tag.to_string()),
        Some(i) => {
            let tag = s[i + 1..].trim();
            (s[..i].trim().to_string(), if tag.is_empty() { default_tag.to_string() } else { tag.to_string() })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn riot_ids_and_endpoints() {
        assert_eq!(parse_riot_id("Zażółć Gęślą#EUNE", "X"), ("Zażółć Gęślą".into(), "EUNE".into()));
        assert_eq!(parse_riot_id("noTag", "EUNE"), ("noTag".into(), "EUNE".into()));
        assert_eq!(parse_riot_id("a#", "EUNE"), ("a".into(), "EUNE".into()));
        assert_eq!(encode_uri_component("Zaż ó#1"), "Za%C5%BC%20%C3%B3%231");
        let p = "x".repeat(78);
        assert_eq!(endpoint_of(&format!("/lol/league/v4/entries/by-puuid/{p}?a=1")), "/lol/league/v4/entries/by-puuid/{id}");
        assert_eq!(platform_region("na1"), Some("americas"));
        assert_eq!(platform_default_tag("la1"), Some("LAN"));
        assert!(!is_platform("xx"));
    }

    #[test]
    fn budgets_nest() {
        let parent = RequestBudget::new(1, None);
        let child = RequestBudget::new(5, Some(parent.clone()));
        assert!(child.take());
        assert!(!child.take());
        assert_eq!(child.remaining(), 0);
        assert_eq!(parent.used(), 1);
    }
}
