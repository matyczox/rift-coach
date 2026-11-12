//! Post-game review (`src/main/coach/review.ts`): [`MatchLog`] collects the key
//! moments of a game while it is played, then [`build_review`] asks a strong model
//! for a review (what went well, what to improve). Saved as `<dir>/<id>.json`
//! (dir = `<userData>/reviews`) in the `MatchReview` format the UI lists
//! (`crate::files::list_reviews`). Arena (CHERRY) is skipped.

use crate::intel::game_intel::GameIntel;
use crate::intel::js_round;
use crate::jsutil::{iso_id, json_pretty};
use crate::llm::caller::ModelCaller;
use crate::llm::{Effort, ModelCall};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;

pub use crate::files::list_reviews;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Moment {
    pub t: i64,
    /// death | kill | objective | advice
    pub kind: String,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GoldPoint {
    pub t: i64,
    pub diff: i64,
}

/// `MatchReview` from shared/types.ts (same JSON).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchReview {
    /// e.g. "2026-11-12T03-10-00"
    pub id: String,
    pub created_at: f64,
    pub champion: String,
    pub role: String,
    pub win: Option<bool>,
    pub duration_sec: i64,
    /// "5/3/7"
    pub kda: String,
    pub cs_per_min: f64,
    pub summary: String,
    pub good: Vec<String>,
    pub improve: Vec<String>,
    pub moments: Vec<Moment>,
    pub gold_curve: Vec<GoldPoint>,
    pub model: String,
}

#[derive(Default)]
pub struct MatchLog {
    pub game_key: String,
    /// Epoch ms when this game was first seen (review id).
    pub started_at: f64,
    pub champion: String,
    pub role: String,
    pub moments: Vec<Moment>,
    pub gold_curve: Vec<GoldPoint>,
    pub last: Option<GameIntel>,
    pub game_mode: String,
    seen_deaths: usize,
    seen_kills: i64,
    seen_recent: HashSet<String>,
    last_curve_at: f64,
}

fn rnd(x: f64) -> i64 {
    js_round(x) as i64
}

impl MatchLog {
    pub fn new() -> Self {
        MatchLog { last_curve_at: -999.0, ..Default::default() }
    }

    /// Feed every intel update; `now_ms` = epoch ms (used when a new game starts).
    pub fn update(&mut self, i: &GameIntel, now_ms: f64) {
        let key = i.players.iter().map(|p| p.champion.as_str()).collect::<Vec<_>>().join("|");
        if key != self.game_key {
            self.reset(key, i, now_ms);
        }
        self.last = Some(i.clone());
        self.game_mode = i.game_mode.clone();
        let Some(me) = &i.me else { return };
        if i.game_time - self.last_curve_at >= 30.0 {
            self.gold_curve.push(GoldPoint { t: rnd(i.game_time), diff: rnd(i.gold_diff) });
            self.last_curve_at = i.game_time;
        }
        if i.my_deaths.len() > self.seen_deaths {
            // myDeaths is newest first: push the new ones oldest first.
            for d in i.my_deaths[..i.my_deaths.len() - self.seen_deaths].iter().rev() {
                let helpers = if d.assisters.is_empty() { String::new() } else { format!(" + {}", d.assisters.join(", ")) };
                self.moments.push(Moment { t: rnd(d.at), kind: "death".into(), text: format!("Śmierć: {}{}{}", d.killer, helpers, if d.was_gank { " (gank)" } else { "" }) });
            }
            self.seen_deaths = i.my_deaths.len();
        }
        if me.kills > self.seen_kills {
            self.moments.push(Moment { t: rnd(i.game_time), kind: "kill".into(), text: format!("Kill ({}/{}/{})", me.kills, me.deaths, me.assists) });
            self.seen_kills = me.kills;
        }
        let re = regex::Regex::new("Smok|Baron|Herald|Herold|Wieża|Inhib|Starszy").unwrap();
        for r in &i.recent {
            let k = format!("{}|{}", rnd(r.at), r.text);
            if !self.seen_recent.insert(k) {
                continue;
            }
            if re.is_match(&r.text) {
                self.moments.push(Moment { t: rnd(r.at), kind: "objective".into(), text: r.text.clone() });
            }
        }
    }

    /// Record a coach call (death reviews are already covered by the death moment).
    /// `a` is the CoachAdvice JSON (`call`, `gameTime`, `deathReview`).
    pub fn advice(&mut self, a: &Value) {
        if a.get("deathReview").is_some_and(|d| !d.is_null()) {
            return;
        }
        self.moments.push(Moment { t: rnd(a.get("gameTime").and_then(Value::as_f64).unwrap_or(0.0)), kind: "advice".into(), text: a.get("call").and_then(Value::as_str).unwrap_or("").to_string() });
    }

    fn reset(&mut self, key: String, i: &GameIntel, now_ms: f64) {
        self.game_key = key;
        self.started_at = now_ms;
        self.moments.clear();
        self.gold_curve.clear();
        self.seen_deaths = i.my_deaths.len();
        self.seen_kills = i.me.as_ref().map(|m| m.kills).unwrap_or(0);
        self.seen_recent = i.recent.iter().map(|r| format!("{}|{}", rnd(r.at), r.text)).collect();
        self.last_curve_at = -999.0;
        self.champion = i.me.as_ref().map(|m| m.champion.clone()).unwrap_or_default();
        self.role = i.me.as_ref().map(|m| m.role.clone()).unwrap_or_default();
    }
}

pub fn schema() -> Value {
    json!({
        "type": "object", "additionalProperties": false, "required": ["summary", "good", "improve"],
        "properties": {
            "summary": { "type": "string", "maxLength": 400 },
            "good": { "type": "array", "items": { "type": "string", "maxLength": 140 }, "minItems": 1, "maxItems": 4 },
            "improve": { "type": "array", "items": { "type": "string", "maxLength": 160 }, "minItems": 2, "maxItems": 5 }
        }
    })
}

pub fn system_prompt(patch_brief: Option<&str>) -> String {
    let mut s = String::from("Jesteś coachem League of Legends. Na podstawie danych z meczu napisz krótką, konkretną analizę dla gracza (po polsku, bez lania wody):
- summary: max 3 zdania — jak przebiegł mecz i co zdecydowało.
- good: 1–4 rzeczy zrobione dobrze (konkretnie, z odniesieniem do danych).
- improve: 2–5 najważniejszych rzeczy do poprawy, NAJWAŻNIEJSZA PIERWSZA; każda jako praktyczna wskazówka (np. \"3 z 6 śmierci to ganki przed 10 min na wysoko popchniętej fali — wardeuj tri-bush przy 2:45 i 6:00\").
Opieraj się tylko na danych; nie wymyślaj zdarzeń.");
    if let Some(b) = patch_brief.filter(|b| !b.is_empty()) {
        s.push_str(&format!("\n\nKontekst patcha:\n{b}"));
    }
    s
}

/// JS `+x.toFixed(1)`.
fn fixed1(x: f64) -> f64 {
    js_round(x * 10.0) / 10.0
}

/// The facts object sent as the prompt; None for remakes (< 5 min), no `me`, or Arena.
pub fn review_facts(log: &MatchLog, win: Option<bool>) -> Option<Value> {
    let i = log.last.as_ref()?;
    let me = i.me.as_ref()?;
    if i.game_time < 300.0 || log.game_mode == "CHERRY" {
        return None;
    }
    let lane = i.players.iter().find(|p| !p.is_ally && p.role == me.role && p.role != "UNKNOWN");
    let mode = match log.game_mode.as_str() {
        "PRACTICETOOL" => "Practice Tool (trening — NIE oceniaj farmy/CS ani złota, skup się na walkach i decyzjach)".to_string(),
        "ARAM" => "ARAM (jedna linia — nie oceniaj farmy ani wizji)".to_string(),
        m => m.to_string(),
    };
    let names = |items: &[crate::intel::game_intel::PlayerItem]| items.iter().map(|x| x.name.clone()).collect::<Vec<_>>();
    let deaths: Vec<Value> = i
        .my_deaths
        .iter()
        .rev()
        .map(|d| json!({ "at": format!("{}:{:02}", (d.at / 60.0).floor() as i64, (d.at % 60.0).floor() as i64), "killer": d.killer, "helpers": d.assisters, "gank": d.was_gank }))
        .collect();
    let facts = json!({
        "mode": mode,
        "champion": me.champion, "role": me.role,
        "result": match win { None => "nieznany", Some(true) => "wygrana", Some(false) => "przegrana" },
        "durationMin": fixed1(i.game_time / 60.0),
        "kda": format!("{}/{}/{}", me.kills, me.deaths, me.assists),
        "cs": me.cs,
        "csPerMin": fixed1(me.cs as f64 / (i.game_time / 60.0)),
        "items": names(&me.items),
        "laneOpponent": lane.map(|l| json!({ "champion": l.champion, "kda": format!("{}/{}/{}", l.kills, l.deaths, l.assists), "cs": l.cs, "items": names(&l.items) })),
        "deaths": deaths,
        "goldCurve": log.gold_curve.iter().enumerate().filter(|(k, _)| k % 2 == 0).map(|(_, g)| g).collect::<Vec<_>>(),
        "moments": log.moments[log.moments.len().saturating_sub(60)..],
    });
    Some(crate::jsutil::js_numbers(facts))
}

#[derive(Clone, Debug, Default)]
pub struct ReviewOptions {
    pub model: String,
    pub effort: Option<Effort>,
    pub win: Option<bool>,
    pub patch_brief: Option<String>,
}

#[derive(Deserialize)]
struct Answer {
    summary: String,
    good: Vec<String>,
    improve: Vec<String>,
}

/// Ask the review model and save the review to `dir`. `Ok(None)` when the game
/// does not qualify (remake / practice blip < 5 min, Arena).
pub async fn build_review(caller: &dyn ModelCaller, log: &MatchLog, opts: &ReviewOptions, dir: &Path, now_ms: f64) -> Result<Option<MatchReview>> {
    let Some(facts) = review_facts(log, opts.win) else { return Ok(None) };
    let i = log.last.as_ref().expect("facts imply last");
    let me = i.me.as_ref().expect("facts imply me");
    let call = ModelCall { model: opts.model.clone(), effort: opts.effort, schema: schema(), timeout_ms: Some(180_000), system: system_prompt(opts.patch_brief.as_deref()), prompt: serde_json::to_string(&facts)?, ..Default::default() };
    let r = caller.call(&call).await?;
    let a: Answer = serde_json::from_value(r.value).map_err(|e| anyhow::anyhow!("odpowiedź niezgodna ze schematem: {e}"))?;
    let review = MatchReview {
        id: iso_id(log.started_at),
        created_at: now_ms,
        champion: me.champion.clone(),
        role: me.role.clone(),
        win: opts.win,
        duration_sec: rnd(i.game_time),
        kda: facts["kda"].as_str().unwrap_or("").to_string(),
        cs_per_min: facts["csPerMin"].as_f64().unwrap_or(0.0),
        summary: a.summary,
        good: a.good,
        improve: a.improve,
        moments: log.moments.clone(),
        gold_curve: log.gold_curve.clone(),
        model: opts.model.clone(),
    };
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(format!("{}.json", review.id)), json_pretty(&review))?;
    Ok(Some(review))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intel::game_intel::{MyDeath, Recent};
    use crate::intel::matchup::tests::{intel, player};
    use crate::llm::caller::FakeCaller;

    fn game() -> GameIntel {
        let mut zed = player("Zed", false, false, "MIDDLE", 10);
        zed.kills = 4;
        intel(vec![player("Ahri", true, true, "MIDDLE", 9), zed])
    }

    #[tokio::test]
    async fn log_moments_and_review_file() {
        let mut log = MatchLog::new();
        let mut i = game();
        i.game_time = 100.0;
        i.recent = vec![Recent { at: 90.0, text: "Smok przed startem".into() }];
        log.update(&i, 1_794_454_200_000.0);
        assert_eq!(log.gold_curve.len(), 1);
        i.game_time = 400.0;
        i.gold_diff = 512.4;
        i.my_deaths = vec![
            MyDeath { at: 380.0, killer: "Zed".into(), assisters: vec!["Vi".into()], enemies_involved: 2, was_gank: true },
            MyDeath { at: 200.0, killer: "Zed".into(), assisters: vec![], enemies_involved: 1, was_gank: false },
        ];
        i.me.as_mut().unwrap().kills = 1;
        i.me.as_mut().unwrap().cs = 60;
        i.recent.push(Recent { at: 390.0, text: "Wieża bot zniszczona".into() });
        i.recent.push(Recent { at: 391.0, text: "Zed kupił item".into() });
        log.update(&i, 0.0);
        log.advice(&json!({ "call": "Idź na smoka", "gameTime": 395.4, "deathReview": null }));
        log.advice(&json!({ "call": "Zginąłeś", "gameTime": 396.0, "deathReview": ["a"] }));
        let kinds: Vec<(&str, &str)> = log.moments.iter().map(|m| (m.kind.as_str(), m.text.as_str())).collect();
        assert_eq!(
            kinds,
            [("death", "Śmierć: Zed"), ("death", "Śmierć: Zed + Vi (gank)"), ("kill", "Kill (1/0/0)"), ("objective", "Wieża bot zniszczona"), ("advice", "Idź na smoka")]
        );
        assert_eq!(log.gold_curve[1], GoldPoint { t: 400, diff: 512 });

        let f = review_facts(&log, Some(false)).unwrap();
        assert_eq!(f["result"], "przegrana");
        assert_eq!(f["durationMin"], 6.7);
        assert_eq!(f["csPerMin"], 9);
        assert_eq!(f["deaths"][0], json!({ "at": "3:20", "killer": "Zed", "helpers": [], "gank": false }));
        assert_eq!(f["laneOpponent"]["kda"], "4/0/0");

        let d = tempfile::tempdir().unwrap();
        let fake = FakeCaller::new(json!({ "summary": "Mecz przegrany przez ganki.", "good": ["farma"], "improve": ["ward", "mapa"] }));
        let opts = ReviewOptions { model: "claude-opus-5-5".into(), effort: Some(Effort::Medium), win: Some(false), patch_brief: Some("Patch 16.22: x".into()) };
        let r = build_review(&fake, &log, &opts, d.path(), 1_794_456_000_000.0).await.unwrap().unwrap();
        assert_eq!(r.id, "2026-11-12T03-30-00");
        assert_eq!(r.duration_sec, 400);
        let call = &fake.calls()[0];
        assert!(call.system.ends_with("\n\nKontekst patcha:\nPatch 16.22: x"));
        assert_eq!(call.timeout_ms, Some(180_000));
        let text = std::fs::read_to_string(d.path().join("2026-11-12T03-30-00.json")).unwrap();
        assert!(text.starts_with("{\n  \"id\": \"2026-11-12T03-30-00\",\n  \"createdAt\": 1794456000000,\n  \"champion\": \"Ahri\""), "{text}");
        // The existing UI listing reads it back.
        let listed = list_reviews(d.path());
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["csPerMin"], 9);
        assert_eq!(serde_json::from_value::<MatchReview>(listed[0].clone()).unwrap(), r);
    }

    #[tokio::test]
    async fn skips_short_games_and_arena() {
        let fake = FakeCaller::new(json!({}));
        let d = tempfile::tempdir().unwrap();
        let mut log = MatchLog::new();
        let mut i = game();
        i.game_time = 200.0;
        log.update(&i, 0.0);
        assert!(build_review(&fake, &log, &ReviewOptions::default(), d.path(), 0.0).await.unwrap().is_none());
        i.game_time = 900.0;
        i.game_mode = "CHERRY".into();
        log.update(&i, 0.0);
        assert!(build_review(&fake, &log, &ReviewOptions::default(), d.path(), 0.0).await.unwrap().is_none());
        assert!(fake.calls().is_empty());
        // New champions → new game: moments reset.
        let mut j = game();
        j.players[1].champion = "Yasuo".into();
        log.update(&j, 5.0);
        assert_eq!(log.started_at, 5.0);
        assert!(log.moments.is_empty());
    }
}
