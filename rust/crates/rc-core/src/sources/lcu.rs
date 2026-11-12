//! League client (LCU) connection (`src/main/sources/lcu.ts`): lockfile auth,
//! gameflow phase, champion-select session; plus the player profile from
//! `refreshMe` in index.ts. Writes are only used by the rune page writer.

use crate::sources::http::{get_json, send_json};
use crate::types::{PlayerProfile, RankEntry, SourceState};
use anyhow::{anyhow, Result};
use base64::Engine;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

pub const LOCKFILES: [&str; 2] = [r"C:\Riot Games\League of Legends\lockfile", r"D:\Riot Games\League of Legends\lockfile"];
pub const POLL_MS: u64 = 2000;

#[derive(Clone, PartialEq, Eq)]
pub struct LcuAuth {
    pub port: u16,
    pub password: String,
}

impl std::fmt::Debug for LcuAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LcuAuth").field("port", &self.port).field("password", &"<redacted>").finish()
    }
}

/// `name:pid:port:password:protocol`
pub fn parse_lockfile(text: &str) -> Option<LcuAuth> {
    let parts: Vec<&str> = text.trim().split(':').collect();
    let port = parts.get(2)?.parse().ok()?;
    let password = parts.get(3)?.to_string();
    (!password.is_empty()).then_some(LcuAuth { port, password })
}

fn from_lockfile() -> Option<LcuAuth> {
    LOCKFILES.iter().find_map(|p| std::fs::read_to_string(p).ok().and_then(|t| parse_lockfile(&t)))
}

/// `--app-port` / `--remoting-auth-token` from the client command line.
pub fn parse_command_line(out: &str) -> Option<LcuAuth> {
    let port = regex::Regex::new(r"--app-port=(\d+)").unwrap().captures(out)?.get(1)?.as_str().parse().ok()?;
    let password = regex::Regex::new(r"--remoting-auth-token=([\w-]+)").unwrap().captures(out)?.get(1)?.as_str().to_string();
    Some(LcuAuth { port, password })
}

/// Fallback: read the LeagueClientUx.exe command line through PowerShell (hidden window, 5 s).
async fn from_process() -> Option<LcuAuth> {
    let cmd = "Get-CimInstance Win32_Process -Filter \"Name='LeagueClientUx.exe'\" | Select-Object -ExpandProperty CommandLine";
    let mut c = tokio::process::Command::new("powershell.exe");
    c.args(["-NoProfile", "-Command", cmd]).kill_on_drop(true);
    crate::llm::hide_window(&mut c);
    let out = tokio::time::timeout(Duration::from_secs(5), c.output()).await.ok()?.ok()?;
    parse_command_line(&String::from_utf8_lossy(&out.stdout))
}

#[derive(Clone)]
pub struct LcuClient {
    auth: Arc<Mutex<Option<LcuAuth>>>,
}

impl LcuClient {
    fn base(&self) -> Result<(String, String)> {
        let a = self.auth.lock().unwrap().clone().ok_or_else(|| anyhow!("LCU not connected"))?;
        let token = base64::engine::general_purpose::STANDARD.encode(format!("riot:{}", a.password));
        Ok((format!("https://127.0.0.1:{}", a.port), format!("Basic {token}")))
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let (base, auth) = self.base()?;
        get_json(&format!("{base}{path}"), Some(&auth), 1500).await
    }

    pub async fn post(&self, path: &str, body: &Value) -> Result<Option<Value>> {
        let (base, auth) = self.base()?;
        send_json("POST", &format!("{base}{path}"), body, Some(&auth), 3000).await
    }

    /// Write request (PUT/POST/PATCH/DELETE), 5 s timeout.
    pub async fn request(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Option<Value>> {
        let (base, auth) = self.base()?;
        let empty = serde_json::json!({});
        send_json(method, &format!("{base}{path}"), body.unwrap_or(&empty), Some(&auth), 5000).await
    }
}

#[derive(Default)]
pub struct LcuShared {
    pub state: SourceState,
    pub phase: Option<String>,
    pub me: Option<PlayerProfile>,
}

pub enum LcuEvent {
    State(SourceState),
    Phase(String),
    ChampSelect(Value),
}

/// `/lol-ranked/v1/current-ranked-stats` queues -> RankEntry (solo/flex with a tier only).
pub fn parse_ranks(v: &Value) -> Vec<RankEntry> {
    let mut out = Vec::new();
    for q in v.get("queues").and_then(Value::as_array).into_iter().flatten() {
        let qt = q.get("queueType").and_then(Value::as_str).unwrap_or("");
        let tier = q.get("tier").and_then(Value::as_str).unwrap_or("");
        if !(qt == "RANKED_SOLO_5x5" || qt == "RANKED_FLEX_SR") || tier.is_empty() || tier == "NONE" {
            continue;
        }
        let n = |k: &str| q.get(k).and_then(Value::as_i64).unwrap_or(0);
        out.push(RankEntry {
            queue: if qt == "RANKED_SOLO_5x5" { "solo" } else { "flex" }.into(),
            tier: tier.into(),
            division: q.get("division").and_then(Value::as_str).unwrap_or("").into(),
            lp: n("leaguePoints"),
            wins: n("wins"),
            losses: n("losses"),
        });
    }
    out
}

pub fn parse_profile(summoner: &Value, ranked: Option<&Value>) -> Option<PlayerProfile> {
    let name = summoner.get("gameName").and_then(Value::as_str).filter(|s| !s.is_empty())?;
    Some(PlayerProfile {
        game_name: name.into(),
        tag_line: summoner.get("tagLine").and_then(Value::as_str).unwrap_or("").into(),
        level: summoner.get("summonerLevel").and_then(Value::as_i64).unwrap_or(0),
        icon_id: summoner.get("profileIconId").and_then(Value::as_i64).unwrap_or(0),
        ranks: ranked.map(parse_ranks).unwrap_or_default(),
        top_champion: None,
        top_champion_key: None,
    })
}

/// Name, level, icon and ranks of the logged-in player (refreshMe).
pub async fn refresh_me(client: &LcuClient, shared: &Arc<Mutex<LcuShared>>) {
    if shared.lock().unwrap().state != SourceState::Connected {
        return;
    }
    let Ok(s) = client.get::<Value>("/lol-summoner/v1/current-summoner").await else { return };
    let rk = client.get::<Value>("/lol-ranked/v1/current-ranked-stats").await.ok();
    let mastery = client.get::<Value>("/lol-champion-mastery/v1/local-player/champion-mastery").await.ok();
    if let Some(mut p) = parse_profile(&s, rk.as_ref()) {
        p.top_champion_key = mastery.as_ref().and_then(Value::as_array).and_then(|a| {
            a.iter().max_by_key(|m| m.get("championPoints").and_then(Value::as_i64).unwrap_or(0))
                .and_then(|m| m.get("championId").and_then(Value::as_i64))
        });
        shared.lock().unwrap().me = Some(p);
    }
}

/// Poll every 2 s like Lcu.tick; also refreshes the player profile (1.5 s after connecting,
/// every 2 min, 8 s after EndOfGame), as index.ts does.
pub fn spawn(rt: &tokio::runtime::Handle) -> (LcuClient, Arc<Mutex<LcuShared>>, mpsc::UnboundedReceiver<LcuEvent>) {
    let client = LcuClient { auth: Arc::new(Mutex::new(None)) };
    let shared = Arc::new(Mutex::new(LcuShared::default()));
    let (tx, rx) = mpsc::unbounded_channel();
    let (c, sh) = (client.clone(), shared.clone());
    let h = rt.clone();
    rt.spawn(async move {
        let mut last_me = tokio::time::Instant::now();
        loop {
            let started = tokio::time::Instant::now();
            let have = c.auth.lock().unwrap().is_some();
            if !have {
                let a = match from_lockfile() {
                    Some(a) => Some(a),
                    None => from_process().await,
                };
                *c.auth.lock().unwrap() = a;
            }
            let set_state = |st: SourceState| {
                let mut s = sh.lock().unwrap();
                if s.state != st {
                    s.state = st;
                    let _ = tx.send(LcuEvent::State(st));
                    return true;
                }
                false
            };
            if c.auth.lock().unwrap().is_none() {
                set_state(SourceState::Waiting);
            } else {
                match c.get::<String>("/lol-gameflow/v1/gameflow-phase").await {
                    Ok(phase) => {
                        if set_state(SourceState::Connected) {
                            let (c2, s2) = (c.clone(), sh.clone());
                            h.spawn(async move {
                                tokio::time::sleep(Duration::from_millis(1500)).await;
                                refresh_me(&c2, &s2).await;
                            });
                        }
                        let changed = {
                            let mut s = sh.lock().unwrap();
                            let ch = s.phase.as_deref() != Some(phase.as_str());
                            if ch {
                                s.phase = Some(phase.clone());
                            }
                            ch
                        };
                        if changed {
                            if phase == "EndOfGame" {
                                let (c2, s2) = (c.clone(), sh.clone());
                                h.spawn(async move {
                                    tokio::time::sleep(Duration::from_millis(8000)).await;
                                    refresh_me(&c2, &s2).await;
                                });
                            }
                            let _ = tx.send(LcuEvent::Phase(phase.clone()));
                        }
                        if phase == "ChampSelect" {
                            match c.get::<Value>("/lol-champ-select/v1/session").await {
                                Ok(sess) => {
                                    let _ = tx.send(LcuEvent::ChampSelect(sess));
                                }
                                Err(_) => reset(&c, &sh, &set_state),
                            }
                        }
                    }
                    Err(_) => reset(&c, &sh, &set_state),
                }
            }
            if last_me.elapsed() >= Duration::from_secs(120) {
                last_me = tokio::time::Instant::now();
                refresh_me(&c, &sh).await;
            }
            if tx.is_closed() {
                break;
            }
            tokio::time::sleep_until(started + Duration::from_millis(POLL_MS)).await;
        }
    });
    (client, shared, rx)
}

/// Client restarted -> new port/password.
fn reset(c: &LcuClient, sh: &Arc<Mutex<LcuShared>>, set_state: &dyn Fn(SourceState) -> bool) {
    *c.auth.lock().unwrap() = None;
    sh.lock().unwrap().phase = None;
    set_state(SourceState::Waiting);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lockfile() {
        let a = parse_lockfile("LeagueClient:1234:54321:s3cr3t-pw:https").unwrap();
        assert_eq!(a.port, 54321);
        assert_eq!(a.password, "s3cr3t-pw");
        assert!(!format!("{a:?}").contains("s3cr3t"));
        assert!(parse_lockfile("garbage").is_none());
    }

    #[test]
    fn command_line() {
        let a = parse_command_line("\"LeagueClientUx.exe\" --riotclient-app-port=1 --app-port=60123 --remoting-auth-token=abc_DEF-1 --x").unwrap();
        assert_eq!((a.port, a.password.as_str()), (60123, "abc_DEF-1"));
    }

    #[test]
    fn profile_and_ranks() {
        let s = json!({ "gameName": "Gracz", "tagLine": "EUNE", "summonerLevel": 312, "profileIconId": 29 });
        let rk = json!({ "queues": [
            { "queueType": "RANKED_SOLO_5x5", "tier": "GOLD", "division": "II", "leaguePoints": 45, "wins": 10, "losses": 8 },
            { "queueType": "RANKED_FLEX_SR", "tier": "NONE", "division": "NA", "leaguePoints": 0, "wins": 0, "losses": 0 },
            { "queueType": "CHERRY", "tier": "", "division": "", "leaguePoints": 0, "wins": 0, "losses": 0 }
        ]});
        let p = parse_profile(&s, Some(&rk)).unwrap();
        assert_eq!(p.level, 312);
        assert_eq!(p.ranks.len(), 1);
        assert_eq!(p.ranks[0].queue, "solo");
        assert_eq!(p.ranks[0].lp, 45);
        let js = serde_json::to_value(&p).unwrap();
        assert_eq!(js["gameName"], "Gracz");
        assert_eq!(js["iconId"], 29);
        assert!(parse_profile(&json!({}), None).is_none());
    }
}
