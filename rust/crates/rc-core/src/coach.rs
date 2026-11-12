//! System 2 coach (`src/main/coach/coach.ts`): decides WHEN to ask the model,
//! builds the snapshot and system prompt, keeps the answers. The async call
//! itself is run by the caller (`TickAction::Ask` -> `call_model` -> `complete`).

use crate::intel::game_intel::{GameIntel, PlayerIntel};
use crate::intel::js_round;
use crate::llm::{Effort, ModelCall};
use crate::wards::WARD_SPOTS;
use serde_json::{json, Map, Value};
use std::time::{Duration, Instant};

/// What System 1 and the rest of the app know right now.
#[derive(Default)]
pub struct CoachContext<'a> {
    /// Item plan shown in the HUD build panel; the coach's item call must match it.
    pub planned_build: Vec<String>,
    pub intel: Option<&'a GameIntel>,
    /// MinimapGhost JSON objects (x, y, radius, age, confidence, champion).
    pub ghosts: Vec<Value>,
    /// AlertState JSON objects (kind, text, confidence).
    pub alerts: Vec<Value>,
    /// OpponentProfile JSON objects.
    pub profiles: Vec<Value>,
    pub patch_brief: Option<String>,
    pub matchup: Option<Value>,
    pub camps: Vec<String>,
    pub covered_ward_spots: Vec<String>,
    /// Player profile text (habits, weak spots, lessons) for the per-game system prompt.
    pub self_profile: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CoachConfig {
    pub enabled: bool,
    pub live_model: String,
    pub review_model: String,
    pub period_sec: f64,
    pub min_gap_sec: f64,
    pub live_effort: Option<Effort>,
    pub review_effort: Option<Effort>,
}

impl Default for CoachConfig {
    /// DEFAULT_COACH
    fn default() -> Self {
        CoachConfig { enabled: true, live_model: "claude-sonnet-5-5".into(), review_model: "claude-sonnet-5-5".into(), period_sec: 25.0, min_gap_sec: 20.0, live_effort: None, review_effort: None }
    }
}

pub const SYSTEM: &str = r#"Jesteś coachem League of Legends na poziomie challengera. Gracz gra TERAZ i może tylko zerknąć na ekran.
Dostajesz stan meczu (JSON): jego postać, itemy, złoto, wszyscy gracze, timery celów, przewidywane pozycje wrogów we mgle (z % pewności) i profile przeciwników.
Zasady:
- "call": 2–4 słowa, po polsku, SAMA sugestia — co zrobić w ciągu najbliższych 5–20 sekund, bez uzasadnień i bez myślników (np. "Smok teraz", "Cofnij się", "Back po fali", "Nie pushuj"). Bez emotek.
- NIE opisuj tego, co gracz już widzi na ekranie (jego HP, złoto, timery, że ktoś zniknął) i NIE powtarzaj poprzednich porad (previousCalls) ani ich sensu innymi słowami. Ogólniki typu "farm pod wieżą", "uważaj na ganka" tylko gdy coś się zmieniło.
- Smoki, baron i herold są ważne — komentuj je (przed pojawieniem się i przy walce o nie).
- Bądź KONSEKWENTNY: nie zmieniaj decyzji z previousCalls (np. "Smok teraz" → "Odpuść smoka"), dopóki sytuacja wyraźnie się nie zmieni (ktoś zginął, wróg się pokazał, przegrana walka).
- Nie komentuj wszystkiego: jedna rzecz naraz, tylko to, co zmienia decyzję gracza.
- Wardy to częsta i ważna porada: gdy gracz nie ma wizji tam, skąd grozi gank, przed smokiem/baronem albo po pchnięciu fali, powiedz gdzie postawić ward (np. "Ward rzeka bot", "Ward krzaki tri") i ustaw pole "ward". Gdy ustawiasz "ward", "call" MUSI nazwać to miejsce słowami — gracz nie szuka znaczków na minimapie.
- Gdy nie masz NOWEJ, konkretnej decyzji: "call": null (gracz nic nie zobaczy — to lepsze niż szum).
- "tone": go = zrób to teraz, wait = czekaj/przygotuj się, danger = zagrożenie, farm = spokojnie farm.
- "mapHint": gdzie iść/co unikać, albo null. target to np. "dragon", "baron", "herald", "top", "mid", "bot", "my-jungle", "enemy-jungle", "base".
- "item": następny przedmiot do kupienia przy najbliższym powrocie. Jeśli w danych jest "plannedBuild" (plan z panelu buildu, który gracz widzi na ekranie), wybierz z niego pierwszy przedmiot, którego gracz jeszcze nie ma — nie proponuj innego. Odejdź od planu tylko, gdy sytuacja wyraźnie tego wymaga (np. wróg nagle nafeedowany AP). Gdy nie ma sensu nic mówić o przedmiotach: null.
- "ward": gdzie postawić ward TERAZ (spot z listy wardSpots, nazwy względem strony gracza: my-* = jego dżungla, enemy-* = wroga) i krótki powód, albo null. Wybieraj miejsca o wyższej wartości (więcej wizji); faelight-* to Faelighty (2026): ward postawiony NA nich daje dużo większy zasięg wizji — preferuj je, gdy dostępne (mid-* dopiero po przemianie mapy po 2. smoku); plant-* to rośliny wizji — uderzenie odsłania duży obszar za darmo. Proponuj ward przed objective (60–90 s przed smokiem/baronem), przy pushowaniu linii bez wizji, gdy jungler wroga zniknął.
- "deathReview": TYLKO gdy me.stan = "NIE ŻYJE" (jeśli "żyje", deathReview = null — także gdy zginął przed chwilą i już się odrodził): 3–5 krótkich punktów: dlaczego zginął (masz myDeath: kto zabił, kto asystował, czy to gank — używaj tego, nie zgaduj), co zrobić po respawnie (back/farm/gdzie iść), czy grać pasywnie czy agresywnie przez najbliższe minuty i dlaczego. Inaczej null.
- "ttlSec": jak długo porada jest aktualna (10–40).
Opieraj się na liczbach: przewaga złota, kto martwy i jak długo, timery, power spike'i, gdzie prawdopodobnie są wrogowie. Jeśli nic ważnego się nie dzieje — "call": null."#;

pub fn schema() -> Value {
    let spots: Vec<&str> = WARD_SPOTS.iter().map(|s| s.id).collect();
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["call", "tone", "mapHint", "item", "ward", "deathReview", "ttlSec"],
        "properties": {
            "call": { "anyOf": [{ "type": "null" }, { "type": "string", "maxLength": 32 }] },
            "tone": { "type": "string", "enum": ["go", "wait", "danger", "farm"] },
            "mapHint": {
                "anyOf": [{ "type": "null" }, {
                    "type": "object", "additionalProperties": false, "required": ["type", "target"],
                    "properties": { "type": { "type": "string", "enum": ["objective", "lane", "jungle", "base", "avoid"] }, "target": { "type": "string" } }
                }]
            },
            "item": { "anyOf": [{ "type": "null" }, { "type": "string" }] },
            "ward": { "anyOf": [{ "type": "null" }, { "type": "object", "additionalProperties": false, "required": ["spot", "reason"], "properties": { "spot": { "type": "string", "enum": spots }, "reason": { "type": "string", "maxLength": 60 } } }] },
            "deathReview": { "anyOf": [{ "type": "null" }, { "type": "array", "items": { "type": "string", "maxLength": 120 }, "minItems": 2, "maxItems": 5 }] },
            "ttlSec": { "type": "number", "minimum": 5, "maximum": 60 }
        }
    })
}

/// `m:ss`
pub fn fmt(s: f64) -> String {
    format!("{}:{:02}", (s / 60.0).floor() as i64, (s % 60.0).floor() as i64)
}

/// Number as JSON the way JSON.stringify writes it (integers without ".0").
pub fn num(v: f64) -> Value {
    if v.is_finite() && v.fract() == 0.0 && v.abs() < 9.0e15 {
        json!(v as i64)
    } else {
        json!(v)
    }
}

/// JS `+x.toFixed(2)`
fn fixed2(v: f64) -> Value {
    num((v * 100.0).round() / 100.0)
}

fn f(v: &Value, k: &str) -> f64 {
    v.get(k).and_then(Value::as_f64).unwrap_or(0.0)
}

/// Per-game context that doesn't change between calls -> system prompt (cached by Claude).
pub fn game_system_prompt(ctx: &CoachContext) -> String {
    let i = ctx.intel.expect("intel");
    let mut parts = vec![SYSTEM.to_string()];
    parts.push(format!(
        "Strona gracza: {}. Współrzędne fog x,y: 0..1, (0,0) = lewy górny róg minimapy.",
        if i.me.as_ref().map(|m| m.team.as_str()) == Some("CHAOS") { "czerwona (prawy górny róg)" } else { "niebieska (lewy dolny róg)" }
    ));
    if let Some(b) = &ctx.patch_brief {
        if !b.is_empty() {
            parts.push(format!("Brief patcha (aktualna meta):\n{b}"));
        }
    }
    let prof: Vec<String> = ctx
        .profiles
        .iter()
        .filter(|p| f(p, "gamesAnalyzed") > 0.0)
        .map(|p| {
            let rank = p.get("rank").and_then(Value::as_str).filter(|r| !r.is_empty()).map(|r| format!(" ({r})")).unwrap_or_default();
            format!("{}: {}{rank}", p.get("champion").and_then(Value::as_str).unwrap_or(""), p.get("summary").and_then(Value::as_str).unwrap_or(""))
        })
        .collect();
    if !prof.is_empty() {
        parts.push(format!("Profile wrogów z historii gier:\n{}", prof.join("\n")));
    }
    if let Some(s) = ctx.self_profile.as_deref().filter(|s| !s.is_empty()) {
        parts.push(s.to_string());
    }
    parts.push(format!(
        "Miejsca na wardy (id: opis, wartość wizji):\n{}",
        WARD_SPOTS.iter().map(|s| format!("{}: {}, {}/5", s.id, s.label, s.value)).collect::<Vec<_>>().join("\n")
    ));
    parts.join("\n\n")
}

fn player_json(i: &GameIntel, x: &PlayerIntel) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("champ".into(), json!(x.champion));
    m.insert("role".into(), json!(x.role));
    m.insert("lvl".into(), json!(x.level));
    m.insert("ms".into(), num(x.move_speed));
    m.insert("kda".into(), json!(format!("{}/{}/{}", x.kills, x.deaths, x.assists)));
    m.insert("cs".into(), json!(x.cs));
    m.insert("gold".into(), num(x.est_gold));
    m.insert("items".into(), json!(x.items.iter().filter(|it| it.price >= 900.0).map(|it| it.name.clone()).collect::<Vec<_>>()));
    if x.is_dead {
        m.insert("dead".into(), json!(format!("respawn {}s", x.respawn_in)));
    }
    if let Some(s) = x.spikes.first() {
        if i.game_time - s.at < 90.0 {
            m.insert("spike".into(), json!(s.label));
        }
    }
    if let Some(fl) = x.flash_in.filter(|v| *v != 0.0) {
        m.insert("flashCd".into(), num(js_round(fl)));
    }
    if !x.is_ally {
        m.insert("spells".into(), json!(x.summoner_spells));
    }
    if let (Some(hp), Some(seen)) = (x.hp_pct, x.hp_seen_at) {
        if i.game_time - seen <= 6.0 {
            m.insert("hp".into(), json!(format!("{}%", js_round(hp * 100.0))));
        }
    }
    let streak = i.streaks.get(&x.champion).copied().unwrap_or(0);
    if streak >= 3 {
        m.insert("killStreak".into(), json!(streak));
        m.insert("shutdown".into(), json!("duży shutdown za zabicie"));
    }
    m
}

/// Compact, model-friendly snapshot (keeps tokens and latency down).
pub fn build_snapshot(ctx: &CoachContext, previous: &[Value]) -> Value {
    let i = ctx.intel.expect("intel");
    let mut o = Map::new();
    o.insert("time".into(), json!(fmt(i.game_time)));
    o.insert("mode".into(), json!(i.game_mode));
    o.insert(
        "me".into(),
        match &i.me {
            None => Value::Null,
            Some(me) => {
                let mut m = Map::new();
                m.insert("stan".into(), json!(if me.is_dead { "NIE ŻYJE" } else { "żyje" }));
                m.extend(player_json(i, me));
                if let Some(s) = &i.my_stats {
                    m.insert("goldNow".into(), num(s.gold));
                    m.insert("hp".into(), json!(format!("{}/{}", s.hp, s.max_hp)));
                    m.insert("ad".into(), num(s.ad));
                    m.insert("ap".into(), num(s.ap));
                    m.insert("armor".into(), num(s.armor));
                    m.insert("mr".into(), num(s.mr));
                }
                Value::Object(m)
            }
        },
    );
    o.insert("allies".into(), Value::Array(i.players.iter().filter(|x| x.is_ally && !x.is_me).map(|x| Value::Object(player_json(i, x))).collect()));
    o.insert(
        "enemies".into(),
        Value::Array(
            i.players
                .iter()
                .filter(|x| !x.is_ally)
                .map(|x| {
                    let mut m = player_json(i, x);
                    if let Some(g) = ctx.ghosts.iter().find(|g| g.get("champion").and_then(Value::as_str) == Some(&x.champion)) {
                        m.insert("fog".into(), json!({ "x": fixed2(f(g, "x")), "y": fixed2(f(g, "y")), "r": fixed2(f(g, "radius")), "unseenSec": num(js_round(f(g, "age"))), "conf": fixed2(f(g, "confidence")) }));
                    }
                    Value::Object(m)
                })
                .collect(),
        ),
    );
    o.insert("goldDiff".into(), num(i.gold_diff));
    o.insert(
        "score".into(),
        json!({
            "kills": format!("{}-{}", i.ally.kills, i.enemy.kills),
            "dragons": format!("{}-{}", i.ally.dragons.len(), i.enemy.dragons.len()),
            "towers": format!("{}-{}", i.ally.towers, i.enemy.towers),
            "barons": format!("{}-{}", i.ally.barons, i.enemy.barons),
        }),
    );
    o.insert(
        "objectives".into(),
        Value::Array(
            // Only objectives that are up or spawn within 90 s: a far-off Baron made the model talk about it
            // ("przygotuj Barona") ten minutes before it exists.
            i.objectives
                .iter()
                .filter(|ob| ob.alive || ob.spawns_at.is_some_and(|s| s - i.game_time <= 90.0))
                .map(|ob| {
                    if ob.alive {
                        json!({ "what": ob.label, "status": "żyje" })
                    } else {
                        json!({ "what": ob.label, "in": num(js_round(ob.spawns_at.unwrap_or(0.0) - i.game_time)) })
                    }
                })
                .collect(),
        ),
    );
    if !i.buffs.is_empty() {
        o.insert(
            "buffs".into(),
            json!(i.buffs.iter().map(|b| format!("{} ({}) do {}", if b.kind == "baron" { "Baron" } else { "Starszy smok" }, if b.team == "ally" { "my" } else { "wróg" }, fmt(b.until))).collect::<Vec<_>>()),
        );
    }
    if let Some(a) = &i.ace {
        o.insert("ace".into(), json!(format!("{} {} s temu", if a.team == "ally" { "MY zrobiliśmy ace" } else { "WRÓG zrobił ace" }, js_round(i.game_time - a.at))));
    }
    if let (Some(true), Some(d)) = (i.me.as_ref().map(|m| m.is_dead), i.my_deaths.first()) {
        o.insert("myDeath".into(), json!({ "killer": d.killer, "assisters": d.assisters, "enemiesInvolved": d.enemies_involved, "gank": d.was_gank, "deathsSoFar": i.my_deaths.len() }));
    }
    let cs_diff = i.me.as_ref().and_then(|me| i.players.iter().find(|p| !p.is_ally && p.role == me.role && p.role != "UNKNOWN").map(|op| json!(me.cs - op.cs)));
    o.insert("csDiffVsLaneOpponent".into(), cs_diff.unwrap_or(Value::Null));
    o.insert(
        "alerts".into(),
        json!(ctx.alerts.iter().map(|a| format!("{} ({}%)", a.get("text").and_then(Value::as_str).unwrap_or(""), js_round(f(a, "confidence") * 100.0))).collect::<Vec<_>>()),
    );
    o.insert("recent".into(), json!(i.recent.iter().take(6).map(|r| format!("{} {}", fmt(r.at), r.text)).collect::<Vec<_>>()));
    o.insert(
        "previousCalls".into(),
        json!(previous.iter().take(3).map(|a| format!("{} {}", fmt(f(a, "gameTime")), a.get("call").and_then(Value::as_str).unwrap_or(""))).collect::<Vec<_>>()),
    );
    if let Some(m) = ctx.matchup.as_ref().filter(|m| !m.is_null()) {
        o.insert("laneMatchup".into(), m.clone());
    }
    if !ctx.planned_build.is_empty() {
        o.insert("plannedBuild".into(), json!(ctx.planned_build));
    }
    if !ctx.camps.is_empty() {
        o.insert("jungleCamps".into(), json!(ctx.camps));
    }
    if !ctx.covered_ward_spots.is_empty() {
        o.insert("alreadyWarded".into(), json!(ctx.covered_ward_spots));
    }
    Value::Object(o)
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoachStatus {
    pub state: String,
    pub model: String,
    pub calls: u64,
    pub last_error: Option<String>,
    pub last_latency_ms: Option<u64>,
}

pub struct AskRequest {
    pub call: ModelCall,
    pub reason: String,
    pub context_id: String,
    pub game_time: f64,
}

pub enum TickAction {
    None,
    /// Run `claude auth status`, then `auth_result`.
    CheckAuth,
    /// Run `call_model(&req.call)`, then `complete`.
    Ask(AskRequest),
}

struct LastSeen {
    dead: bool,
    events: f64,
    my_level: i64,
    game_key: String,
}

pub struct Coach {
    pub status: CoachStatus,
    pub config: CoachConfig,
    history: Vec<Value>,
    in_flight: bool,
    last_call_at: Option<Instant>,
    last_call_game_time: f64,
    last_seen: LastSeen,
    pending_reason: Option<String>,
    auth_checked_at: Option<Instant>,
    last_signature: String,
    /// Alert texts already reacted to (game time last seen); an alert triggers a call only when new.
    seen_alerts: Vec<(String, f64)>,
}

impl Default for Coach {
    fn default() -> Self {
        Coach {
            status: CoachStatus { state: "off".into(), model: CoachConfig::default().live_model, calls: 0, last_error: None, last_latency_ms: None },
            config: CoachConfig::default(),
            history: Vec::new(),
            in_flight: false,
            last_call_at: None,
            last_call_game_time: -999.0,
            last_seen: LastSeen { dead: false, events: 0.0, my_level: 0, game_key: String::new() },
            pending_reason: None,
            auth_checked_at: None,
            last_signature: String::new(),
            seen_alerts: Vec::new(),
        }
    }
}

fn since(t: Option<Instant>, now: Instant) -> Duration {
    t.map(|t| now.saturating_duration_since(t)).unwrap_or(Duration::MAX)
}

impl Coach {
    pub fn history(&self) -> &[Value] {
        &self.history
    }

    fn set_state(&mut self, state: &str, error: Option<Option<String>>) {
        let error = error.unwrap_or_else(|| self.status.last_error.clone());
        self.status.last_error = if state == "error" { error } else if state == "idle" { None } else { error };
        self.status.state = state.into();
    }

    /// Mark the start of an auth check (checkAuth sets authCheckedAt first).
    pub fn auth_result(&mut self, logged_in: bool, error: Option<String>) {
        self.set_state(if logged_in { "idle" } else { "not-logged-in" }, Some(error));
    }

    /// Call on every intel update (~2/s).
    pub fn tick(&mut self, ctx: &CoachContext, now: Instant) -> TickAction {
        if !self.config.enabled {
            self.set_state("off", None);
            return TickAction::None;
        }
        let st = self.status.state.clone();
        if st == "off" || (st == "not-logged-in" && since(self.auth_checked_at, now) > Duration::from_secs(60)) {
            if since(self.auth_checked_at, now) > Duration::from_secs(5) {
                self.auth_checked_at = Some(now);
                return TickAction::CheckAuth;
            }
            return TickAction::None;
        }
        if st == "not-logged-in" {
            return TickAction::None;
        }
        let i = ctx.intel.expect("intel");
        let game_key = i.players.iter().map(|p| p.champion.as_str()).collect::<Vec<_>>().join("|");
        if game_key != self.last_seen.game_key {
            self.history.clear();
            self.last_seen = LastSeen { dead: false, events: 0.0, my_level: 0, game_key };
        }
        if let Some(r) = self.trigger(ctx) {
            if self.pending_reason.is_none() {
                self.pending_reason = Some(r);
            }
        }
        let Some(pending) = self.pending_reason.clone() else { return TickAction::None };
        if self.in_flight {
            return TickAction::None;
        }
        if since(self.last_call_at, now) < Duration::from_secs_f64(self.config.min_gap_sec) && pending != "death" {
            return TickAction::None;
        }
        self.pending_reason = None;
        TickAction::Ask(self.ask(ctx, &pending, now))
    }

    fn trigger(&mut self, ctx: &CoachContext) -> Option<String> {
        let i = ctx.intel.unwrap();
        let me = i.me.as_ref();
        let mut reason: Option<&str> = None;
        let dead = me.is_some_and(|m| m.is_dead);
        if dead && !self.last_seen.dead {
            reason = Some("death");
        } else if !dead && self.last_seen.dead {
            reason = Some("respawn");
        }
        self.last_seen.dead = dead;

        let events_now = i.recent.first().map(|r| r.at).unwrap_or(0.0);
        let re = regex::Regex::new("Smok|Baron|Herald|Atakhan|Wieża|Inhibitor").unwrap();
        if reason.is_none() && events_now > self.last_seen.events && i.recent.first().is_some_and(|r| re.is_match(&r.text)) {
            reason = Some("event");
        }
        self.last_seen.events = self.last_seen.events.max(events_now);

        if reason.is_none() && me.is_some_and(|m| m.level >= 6) && self.last_seen.my_level < 6 && self.last_seen.my_level > 0 {
            reason = Some("level6");
        }
        if let Some(m) = me {
            self.last_seen.my_level = m.level;
        }
        let soon = i.objectives.iter().any(|o| o.spawns_at.is_some_and(|s| s != 0.0 && s - i.game_time < 60.0 && s - i.game_time > 45.0));
        if reason.is_none() && soon && i.game_time - self.last_call_game_time > 15.0 {
            reason = Some("objective-soon");
        }
        let text_of = |a: &Value| a.get("text").and_then(Value::as_str).unwrap_or("").to_string();
        let new_alert = ctx.alerts.iter().any(|a| f(a, "confidence") > 0.6 && !self.seen_alerts.iter().any(|(t, _)| *t == text_of(a)));
        for a in &ctx.alerts {
            let t = text_of(a);
            match self.seen_alerts.iter_mut().find(|(x, _)| *x == t) {
                Some(e) => e.1 = i.game_time,
                None => self.seen_alerts.push((t, i.game_time)),
            }
        }
        self.seen_alerts.retain(|(_, at)| i.game_time - at <= 60.0);
        if reason.is_none() && new_alert && i.game_time - self.last_call_game_time > 15.0 {
            reason = Some("alert");
        }
        if reason.is_none() && i.game_time - self.last_call_game_time > self.config.period_sec {
            // Quiet game: only ask again if something changed, or after 2× the period at most.
            let sig = self.signature(ctx);
            if sig != self.last_signature || i.game_time - self.last_call_game_time > self.config.period_sec * 2.0 {
                reason = Some("periodic");
            }
        }
        // While dead: one death review only. Other triggers (kills, alerts, periodic) would ask the live
        // model, which sees "NIE ŻYJE" and writes a second death review over the first.
        if dead && reason != Some("death") {
            return None;
        }
        reason.map(String::from)
    }

    /// Coarse game state: levels, kills, deaths, items, objective states, fog count, alerts.
    fn signature(&self, ctx: &CoachContext) -> String {
        let i = ctx.intel.unwrap();
        [
            i.players.iter().map(|p| format!("{}.{}.{}.{}.{}", p.level, p.kills, p.deaths, p.completed_items, if p.is_dead { 1 } else { 0 })).collect::<Vec<_>>().join(","),
            i.objectives
                .iter()
                .map(|o| {
                    let s = if o.alive {
                        "A".to_string()
                    } else if let Some(sp) = o.spawns_at.filter(|s| *s != 0.0) {
                        ((sp - i.game_time) / 30.0).floor().to_string()
                    } else {
                        "-".into()
                    };
                    format!("{}{s}", o.kind)
                })
                .collect::<Vec<_>>()
                .join(","),
            ctx.ghosts.len().to_string(),
            ctx.alerts.iter().map(|a| a.get("kind").and_then(Value::as_str).unwrap_or("").to_string()).collect::<Vec<_>>().join(","),
            js_round(i.my_stats.as_ref().map(|s| s.gold).unwrap_or(0.0) / 500.0).to_string(),
        ]
        .join("|")
    }

    fn ask(&mut self, ctx: &CoachContext, reason: &str, now: Instant) -> AskRequest {
        let i = ctx.intel.unwrap();
        self.last_signature = self.signature(ctx);
        self.in_flight = true;
        self.last_call_at = Some(now);
        self.last_call_game_time = i.game_time;
        let is_death = reason == "death";
        let model = if is_death { self.config.review_model.clone() } else { self.config.live_model.clone() };
        self.status.model = model.clone();
        self.set_state("thinking", None);
        let snapshot = build_snapshot(ctx, &self.history);
        let context_id = format!("{}-{reason}", js_round(i.game_time));
        // In-game: no extended thinking. 5.x models answer without a thinking phase (/-5-5|-5/ in the TS).
        let thinking = if model.contains("-5") { None } else if is_death { Some(2000) } else { Some(0) };
        AskRequest {
            call: ModelCall {
                model,
                system: game_system_prompt(ctx),
                schema: schema(),
                prompt: format!(
                    "Powód zapytania: {reason}{}\nStan meczu:\n{}",
                    if is_death { " (gracz właśnie zginął — wypełnij deathReview)" } else { "" },
                    serde_json::to_string(&snapshot).unwrap()
                ),
                thinking_tokens: thinking,
                effort: Some((if is_death { self.config.review_effort } else { self.config.live_effort }).unwrap_or(Effort::Low)),
                timeout_ms: Some(if is_death { 60000 } else { 25000 }),
                tools: vec![],
            },
            reason: reason.into(),
            context_id,
            game_time: i.game_time,
        }
    }

    /// Result of an `Ask`. Returns the advice (CoachAdvice JSON; None when the model had nothing new
    /// to say or on error) and whether to re-check auth.
    pub fn complete(&mut self, req: &AskRequest, result: anyhow::Result<(Value, u64)>, created_at_ms: f64) -> (Option<Value>, bool) {
        self.in_flight = false;
        match result {
            Ok((value, latency)) => {
                self.status.calls += 1;
                self.status.last_latency_ms = Some(latency);
                let call = value.get("call").and_then(Value::as_str).unwrap_or("").trim().to_string();
                // A ward spot without words is easy to miss on the minimap: say it on the top bar.
                let call = if call.is_empty() { ward_call(&value).unwrap_or(call) } else { call };
                // Silent (no new decision) or the same idea as a recent call: show nothing new. A death review still counts.
                let is_death = req.reason == "death";
                if !is_death && (call.is_empty() || self.history.iter().take(3).any(|h| similar_call(h.get("call").and_then(Value::as_str).unwrap_or(""), &call))) {
                    self.set_state("idle", None);
                    return (None, false);
                }
                let mut a = value.as_object().cloned().unwrap_or_default();
                a.insert("call".into(), json!(call));
                a.insert("contextId".into(), json!(req.context_id));
                a.insert("createdAt".into(), num(created_at_ms));
                a.insert("gameTime".into(), json!(req.game_time));
                a.insert("latencyMs".into(), json!(latency));
                let advice = Value::Object(a);
                self.history.insert(0, advice.clone());
                self.history.truncate(10);
                self.set_state("idle", None);
                (Some(advice), false)
            }
            Err(e) => {
                let msg = e.to_string();
                self.set_state("error", Some(Some(msg.clone())));
                let re = regex::Regex::new("(?i)auth|login|zalog").unwrap();
                (None, re.is_match(&msg))
            }
        }
    }

    /// Drop advice that no longer fits the game (too old or the player died/respawned since).
    pub fn is_fresh(a: &Value, intel: &GameIntel) -> bool {
        let age = intel.game_time - f(a, "gameTime");
        if age > f(a, "ttlSec") || age < -5.0 {
            return false;
        }
        let dead = intel.me.as_ref().is_some_and(|m| m.is_dead);
        let has_review = a.get("deathReview").is_some_and(|d| !d.is_null());
        !(has_review && !dead)
    }
}

/// index.ts: models sometimes write free text in 'item' — keep it only if it is a real shop item.
pub fn validate_item(advice: &mut Value, items: &std::collections::HashMap<i64, crate::intel::ddragon::ItemData>) {
    let Some(want) = advice.get("item").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_lowercase) else { return };
    let hit = items
        .values()
        .filter(|i| i.completed)
        .find(|i| [Some(&i.name), i.name_en.as_ref()].into_iter().flatten().any(|n| !n.is_empty() && (n.to_lowercase() == want || want.contains(&n.to_lowercase()))));
    advice["item"] = hit.map(|h| json!(h.name)).unwrap_or(Value::Null);
}

/// JS `x.normalize('NFD').replace(/[̀-ͯ]/g, '')` for an already lower-cased string, limited
/// to the letters that decompose in Latin-1 / Latin Extended-A (Polish ą ć ę ń ó ś ź ż; ł does not
/// decompose and stays, like in JS). Combining marks are dropped.
fn strip_diacritics(s: &str) -> String {
    s.chars()
        .filter(|c| !('\u{0300}'..='\u{036f}').contains(c))
        .map(|c| match c {
            'à'..='å' | 'ā' | 'ă' | 'ą' => 'a',
            'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => 'c',
            'ď' => 'd',
            'è'..='ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
            'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g',
            'ĥ' => 'h',
            'ì'..='ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' => 'i',
            'ĵ' => 'j',
            'ķ' => 'k',
            'ĺ' | 'ļ' | 'ľ' => 'l',
            'ñ' | 'ń' | 'ņ' | 'ň' => 'n',
            'ò'..='ö' | 'ō' | 'ŏ' | 'ő' => 'o',
            'ŕ' | 'ŗ' | 'ř' => 'r',
            'ś' | 'ŝ' | 'ş' | 'š' => 's',
            'ţ' | 'ť' => 't',
            'ù'..='ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => 'u',
            'ŵ' => 'w',
            'ý' | 'ÿ' | 'ŷ' => 'y',
            'ź' | 'ż' | 'ž' => 'z',
            c => c,
        })
        .collect()
}

/// Two calls say the same thing (shared content words), e.g. "Farm pod wieżą" vs "Farmuj pod wieżą — gank".
/// "Ward: <spot label>" for an advice whose only content is a ward spot.
fn ward_call(value: &Value) -> Option<String> {
    let id = value.get("ward")?.get("spot")?.as_str()?;
    WARD_SPOTS.iter().find(|s| s.id == id).map(|s| format!("Ward: {}", s.label))
}

pub fn similar_call(a: &str, b: &str) -> bool {
    let words = |x: &str| -> std::collections::HashSet<String> {
        let s: String = strip_diacritics(&x.to_lowercase()).chars().map(|c| if c.is_ascii_lowercase() || c.is_ascii_digit() || c == ' ' { c } else { ' ' }).collect();
        s.split_whitespace().filter(|w| w.len() > 3).map(|w| w[..w.len().min(5)].to_string()).collect()
    };
    let (wa, wb) = (words(a), words(b));
    if wa.is_empty() || wb.is_empty() {
        return false;
    }
    let common = wa.iter().filter(|w| wb.contains(*w)).count();
    common as f64 / wa.len().min(wb.len()) as f64 >= 0.6
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intel::ddragon::tests::test_items;
    use crate::intel::game_intel::{IntelTracker, ObjectiveTimer, Recent};

    fn intel() -> GameIntel {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/allgamedata.json")).unwrap();
        let d = crate::sources::live::parse(serde_json::from_str(&text).unwrap()).unwrap();
        IntelTracker::default().update(&d, Some(&test_items()), None)
    }

    #[test]
    fn compact_snapshot() {
        let i = intel();
        let ctx = CoachContext { intel: Some(&i), ..Default::default() };
        let s = build_snapshot(&ctx, &[]);
        assert_eq!(s["me"]["champ"], "Ambessa");
        assert_eq!(s["enemies"].as_array().unwrap().len(), 5);
        let text = serde_json::to_string(&s).unwrap();
        assert!(text.len() < 6000, "{}", text.len());
        assert!(text.starts_with("{\"time\":\"39:49\",\"mode\":\"PRACTICETOOL\",\"me\":{\"stan\":"));
        assert!(!text.contains(".0,"), "integers written like JSON.stringify: {text}");
        // me.hp from myStats replaces the per-player hp in place.
        assert!(s["me"]["hp"].as_str().unwrap().contains('/'));
    }

    #[test]
    fn system_prompt_and_schema() {
        let i = intel();
        let p = game_system_prompt(&CoachContext { intel: Some(&i), ..Default::default() });
        assert!(p.starts_with(SYSTEM));
        assert!(p.contains("Strona gracza: niebieska (lewy dolny róg)."));
        assert!(p.ends_with("plant-my-jungle-top: roślina wizji (nasza dżungla, top), 3/5"));
        assert_eq!(schema()["properties"]["ward"]["anyOf"][1]["properties"]["spot"]["enum"].as_array().unwrap().len(), 28);
        assert!(!p.contains("Profil gracza"));
        // 2c21335 / 9041f35: short calls, null when nothing new, item from the build panel.
        assert!(SYSTEM.contains("\n- \"call\": 2–4 słowa, po polsku, SAMA sugestia — co zrobić w ciągu najbliższych 5–20 sekund, bez uzasadnień i bez myślników"));
        assert!(SYSTEM.contains("\n- Gdy nie masz NOWEJ, konkretnej decyzji: \"call\": null (gracz nic nie zobaczy — to lepsze niż szum).\n- \"tone\":"));
        assert!(SYSTEM.contains("Jeśli w danych jest \"plannedBuild\" (plan z panelu buildu, który gracz widzi na ekranie)"));
        assert!(SYSTEM.ends_with("gdzie prawdopodobnie są wrogowie. Jeśli nic ważnego się nie dzieje — \"call\": null."));
        assert!(!SYSTEM.contains("8 słów") && !SYSTEM.contains("słowo w słowo"));
        assert_eq!(schema()["properties"]["call"], json!({ "anyOf": [{ "type": "null" }, { "type": "string", "maxLength": 32 }] }));
        assert_eq!(CoachConfig::default().min_gap_sec, 20.0);
        // 76599ed / 0c1ebb3: objectives are commented again, consistently, one thing at a time.
        assert!(SYSTEM.contains("\n- Smoki, baron i herold są ważne — komentuj je (przed pojawieniem się i przy walce o nie).\n- Bądź KONSEKWENTNY:"));
        assert!(!SYSTEM.contains("krótko napisz dlaczego"));
        // Same text as the TS app (when the TS sources are next to the Rust workspace).
        if let Ok(ts) = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../src/main/coach/coach.ts")) {
            let ts = ts.replace("\r\n", "\n");
            let body = ts.split("export const SYSTEM = `").nth(1).and_then(|s| s.split("`;").next()).expect("SYSTEM in coach.ts");
            assert_eq!(body, SYSTEM, "rc-core SYSTEM differs from src/main/coach/coach.ts");
        }
        // Player profile goes after the enemy profiles, before the ward spot list.
        let p = game_system_prompt(&CoachContext { intel: Some(&i), self_profile: Some("Profil gracza (Ty): test".into()), ..Default::default() });
        let at = p.find("\n\nProfil gracza (Ty): test\n\nMiejsca na wardy").expect("self profile block");
        assert!(at > SYSTEM.len());
    }

    #[test]
    fn auth_then_triggers_and_gap() {
        let mut i = intel();
        let mut c = Coach::default();
        c.config.period_sec = 25.0;
        let t0 = Instant::now();
        assert!(matches!(c.tick(&CoachContext { intel: Some(&i), ..Default::default() }, t0), TickAction::CheckAuth));
        c.auth_result(true, None);
        assert_eq!(c.status.state, "idle");
        // First tick: a new tower kill is a big event -> ask (a kill is not since 2c21335).
        i.recent.insert(0, Recent { at: i.game_time, text: "Wieża bot zniszczona".into() });
        let req = match c.tick(&CoachContext { intel: Some(&i), ..Default::default() }, t0) {
            TickAction::Ask(r) => r,
            _ => panic!("expected ask"),
        };
        assert_eq!(c.status.state, "thinking");
        assert!(req.call.prompt.starts_with("Powód zapytania: event\nStan meczu:\n{"));
        assert_eq!(req.call.timeout_ms, Some(25000));
        assert_eq!(req.call.thinking_tokens, None); // claude-sonnet-5-5
        // In flight: nothing else.
        assert!(matches!(c.tick(&CoachContext { intel: Some(&i), ..Default::default() }, t0), TickAction::None));
        let (adv, reauth) = c.complete(&req, Ok((json!({ "call": "Farm bot", "tone": "farm", "ttlSec": 20, "deathReview": null }), 1234)), 1.0e12);
        let adv = adv.unwrap();
        assert!(!reauth);
        assert_eq!(adv["latencyMs"], 1234);
        assert_eq!(c.status.calls, 1);
        assert!(Coach::is_fresh(&adv, &i));
        // Death bypasses the 20 s minimum gap.
        let me = i.players.iter_mut().find(|p| p.is_me).unwrap();
        me.is_dead = true;
        i.me = Some(me.clone());
        match c.tick(&CoachContext { intel: Some(&i), ..Default::default() }, t0 + Duration::from_secs(1)) {
            TickAction::Ask(r) => {
                assert_eq!(r.reason, "death");
                assert_eq!(r.call.timeout_ms, Some(60000));
                assert!(r.call.prompt.contains("(gracz właśnie zginął — wypełnij deathReview)"));
            }
            _ => panic!("death should ask immediately"),
        }
    }

    #[test]
    fn errors_and_item_validation() {
        let i = intel();
        let mut c = Coach::default();
        c.auth_result(true, None);
        let TickAction::Ask(req) = c.tick(&CoachContext { intel: Some(&i), ..Default::default() }, Instant::now()) else { panic!() };
        let (_, reauth) = c.complete(&req, Err(anyhow::anyhow!("Claude: Not logged in · Please run /login")), 0.0);
        assert!(reauth);
        assert_eq!(c.status.state, "error");
        let mut a = json!({ "item": "Buy Taniec Śmierci now" });
        validate_item(&mut a, &test_items());
        assert_eq!(a["item"], "Taniec Śmierci");
        let mut b = json!({ "item": "coś do obrony" });
        validate_item(&mut b, &test_items());
        assert!(b["item"].is_null());
    }

    fn ready_coach() -> Coach {
        let mut c = Coach::default();
        c.auth_result(true, None);
        c
    }

    fn ctx(i: &GameIntel) -> CoachContext<'_> {
        CoachContext { intel: Some(i), ..Default::default() }
    }

    fn set_dead(i: &mut GameIntel, dead: bool) {
        let me = i.players.iter_mut().find(|p| p.is_me).unwrap();
        me.is_dead = dead;
        i.me = Some(me.clone());
    }

    /// 58861b3: while dead, only the death review; kills / big events / periodic wait for the respawn.
    #[test]
    fn dead_player_gets_one_review_only() {
        let mut i = intel();
        let mut c = ready_coach();
        let t0 = Instant::now();
        set_dead(&mut i, true);
        let TickAction::Ask(req) = c.tick(&ctx(&i), t0) else { panic!("death should ask") };
        assert_eq!(req.reason, "death");
        let (adv, _) = c.complete(&req, Ok((json!({ "call": null, "tone": "wait", "ttlSec": 30, "deathReview": ["a", "b"] }), 10)), 1.0);
        // A death review is kept even with an empty call.
        assert_eq!(adv.unwrap()["call"], "");
        // Still dead: a Baron kill and a long quiet period trigger nothing.
        i.recent.insert(0, Recent { at: i.game_time + 1.0, text: "Baron zabity przez wroga".into() });
        i.game_time += 120.0;
        assert!(matches!(c.tick(&ctx(&i), t0 + Duration::from_secs(60)), TickAction::None));
        assert!(matches!(c.tick(&ctx(&i), t0 + Duration::from_secs(90)), TickAction::None));
        // Respawn: the live model again.
        set_dead(&mut i, false);
        let TickAction::Ask(req) = c.tick(&ctx(&i), t0 + Duration::from_secs(120)) else { panic!("respawn should ask") };
        assert_eq!(req.reason, "respawn");
    }

    /// 2c21335: empty call or one similar to the last 3 shows nothing; kills no longer trigger.
    #[test]
    fn quiet_coach_drops_silent_and_repeated_calls() {
        let mut i = intel();
        i.objectives.clear();
        i.recent.insert(0, Recent { at: i.game_time, text: "Zed zabił Ahri".into() });
        let mut c = ready_coach();
        let t0 = Instant::now();
        // A kill is not a big event any more (periodic still asks on the first tick).
        let TickAction::Ask(req) = c.tick(&ctx(&i), t0) else { panic!() };
        assert_eq!(req.reason, "periodic");
        let (adv, _) = c.complete(&req, Ok((json!({ "call": null, "tone": "farm", "ttlSec": 20, "deathReview": null }), 10)), 1.0);
        assert!(adv.is_none());
        assert_eq!(c.status.state, "idle");
        assert_eq!(c.status.calls, 1);
        let ok = |c: &mut Coach, call: &str| {
            let req = AskRequest { call: ModelCall::default(), reason: "periodic".into(), context_id: "x".into(), game_time: 0.0 };
            c.in_flight = true;
            c.complete(&req, Ok((json!({ "call": call, "tone": "farm", "ttlSec": 20, "deathReview": null }), 10)), 1.0).0
        };
        assert_eq!(ok(&mut c, "  Farmuj pod wieżą  ").unwrap()["call"], "Farmuj pod wieżą");
        assert!(ok(&mut c, "Farmuj pod wieżą — gank").is_none());
        assert!(ok(&mut c, "   ").is_none());
        assert!(ok(&mut c, "Smok teraz — 3 martwych").is_some());
        assert_eq!(c.history().len(), 2);
    }

    /// 2c21335: an alert triggers a call only when its text was not seen in the last 60 s.
    #[test]
    fn alert_triggers_only_when_new() {
        let mut i = intel();
        i.recent.clear();
        i.objectives.clear();
        let mut c = ready_coach();
        let t0 = Instant::now();
        let alert = json!({ "kind": "gank", "text": "Jungler wroga blisko", "confidence": 0.8 });
        let with = |i: &GameIntel, a: &[Value]| -> CoachContext<'static> {
            let i: &'static GameIntel = Box::leak(Box::new(i.clone()));
            CoachContext { intel: Some(i), alerts: a.to_vec(), ..Default::default() }
        };
        // A new alert asks.
        let TickAction::Ask(req) = c.tick(&with(&i, &[alert.clone()]), t0) else { panic!() };
        assert_eq!(req.reason, "alert");
        c.complete(&req, Ok((json!({ "call": "Back po tej fali", "tone": "farm", "ttlSec": 20 }), 1)), 1.0);
        // The same text 20 s later (still on screen) is not new: nothing.
        i.game_time += 20.0;
        assert!(matches!(c.tick(&with(&i, &[alert.clone()]), t0 + Duration::from_secs(25)), TickAction::None));
        // A different alert text is new.
        let other = json!({ "kind": "gank", "text": "Mid zniknął", "confidence": 0.8 });
        i.game_time += 1.0;
        let TickAction::Ask(req) = c.tick(&with(&i, &[alert.clone(), other]), t0 + Duration::from_secs(26)) else { panic!("new alert") };
        assert_eq!(req.reason, "alert");
    }

    /// 3c3ea75 + 9041f35: objectives only when up or within 90 s; plannedBuild in the snapshot.
    #[test]
    fn snapshot_objectives_and_planned_build() {
        let mut i = intel();
        let t = i.game_time;
        i.objectives = vec![
            ObjectiveTimer { kind: "dragon", label: "Smok".into(), spawns_at: Some(t + 80.4), alive: false },
            ObjectiveTimer { kind: "baron", label: "Baron".into(), spawns_at: Some(t + 600.0), alive: false },
            ObjectiveTimer { kind: "herald", label: "Herald".into(), spawns_at: None, alive: true },
            ObjectiveTimer { kind: "atakhan", label: "Atakhan".into(), spawns_at: None, alive: false },
        ];
        let mut c = ctx(&i);
        let s = build_snapshot(&c, &[]);
        assert_eq!(s["objectives"], json!([{ "what": "Smok", "in": 80 }, { "what": "Herald", "status": "żyje" }]));
        assert!(s.get("plannedBuild").is_none());
        c.planned_build = vec!["Taniec Śmierci".into(), "Opancerzone Natarcie".into()];
        let s = build_snapshot(&c, &[]);
        assert_eq!(s["plannedBuild"], json!(["Taniec Śmierci", "Opancerzone Natarcie"]));
    }

    /// Port of coach.ts similarCall.
    #[test]
    fn similar_call_matches_ts() {
        // As in TS: "farm" vs "farmu" differ, so the doc example of coach.ts is not similar (1 of 2 words).
        assert!(!similar_call("Farm pod wieżą", "Farmuj pod wieżą — gank"));
        assert!(similar_call("Farmuj pod wieżą", "Farmuj pod wieżą — gank"));
        assert!(similar_call("Nie pushuj — jungler top", "nie PUSHUJ, jungler top!"));
        assert!(similar_call("Zaczekaj na smoka", "Zaczekaj z grupą na smoka teraz"));
        assert!(!similar_call("Smok teraz — 3 martwych", "Back po tej fali"));
        assert!(!similar_call("Idź mid", "Idź top"), "only words longer than 3 letters count");
        assert!(!similar_call("", "Farm bot"));
        assert!(!similar_call("a b c", "a b c"));
        // Diacritics stripped like NFD (ś -> s), ł is not decomposed (becomes a separator).
        assert!(similar_call("Ściągnij falę", "sciagnij fale"));
        assert_eq!(strip_diacritics("ąćęńóśźżł"), "acenoszzł");
        // 0.6 of the smaller set: 1 of 2 words is not enough, 2 of 3 is.
        assert!(!similar_call("Farm bot lane", "Farm smoka"));
        assert!(similar_call("Farm wieża smoka", "Farm wieża teraz"));
    }

    #[test]
    fn helpers() {
        assert_eq!(fmt(125.9), "2:05");
        assert_eq!(num(3.0), json!(3));
        assert_eq!(fixed2(0.456), json!(0.46));
    }
}
