//! Live API snapshots -> GameIntel (`src/main/intel/gameIntel.ts`).

use super::ddragon::{estimate_move_speed, ItemData};
use super::js_round;
use crate::sources::live::{AllGameData, LivePlayer};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};

/// Objective timings (seconds). Season 2026: Atakhan removed, Baron back at 20:00.
pub mod obj {
    pub const DRAGON_FIRST: f64 = 5.0 * 60.0;
    pub const DRAGON_RESPAWN: f64 = 5.0 * 60.0;
    pub const ELDER_RESPAWN: f64 = 6.0 * 60.0;
    pub const GRUBS_FIRST: f64 = 8.0 * 60.0;
    pub const HERALD_FIRST: f64 = 15.0 * 60.0;
    pub const BARON_FIRST: f64 = 20.0 * 60.0;
    pub const BARON_RESPAWN: f64 = 6.0 * 60.0;
    pub const ATAKHAN_FIRST: f64 = f64::INFINITY; // removed in 2026
    pub const INHIB_RESPAWN: f64 = 5.0 * 60.0;
}

fn dragon_pl(t: &str) -> Option<&'static str> {
    Some(match t {
        "Fire" => "Infernal",
        "Earth" => "Mountain",
        "Water" => "Ocean",
        "Air" => "Cloud",
        "Chemtech" => "Chemtech",
        "Hextech" => "Hextech",
        "Elder" => "Elder",
        _ => return None,
    })
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Spike {
    pub kind: &'static str, // level | item
    pub label: String,
    pub at: f64,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct PlayerItem {
    pub id: i64,
    pub name: String,
    pub price: f64,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlayerIntel {
    pub champion: String,
    pub team: String,
    pub is_ally: bool,
    pub is_me: bool,
    pub role: String,
    pub level: i64,
    pub kills: i64,
    pub deaths: i64,
    pub assists: i64,
    pub cs: i64,
    pub ward_score: i64,
    pub is_dead: bool,
    pub respawn_in: f64,
    pub item_gold: f64,
    /// Gold of shop-only items (recall proof); evolving items excluded.
    pub shop_gold: f64,
    pub est_gold: f64,
    pub items: Vec<PlayerItem>,
    pub completed_items: usize,
    pub spikes: Vec<Spike>,
    pub summoner_spells: Vec<String>,
    pub hp_pct: Option<f64>,
    pub hp_seen_at: Option<f64>,
    pub move_speed: f64,
    pub flash_in: Option<f64>,
    /// Mana fraction for mana champions (me: Live API; allies: portraits above the minimap). None = no mana / unknown.
    pub mana_pct: Option<f64>,
    /// Shield as a fraction of max HP, seen in the last 3 s (shields are short-lived).
    pub shield_pct: Option<f64>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ObjectiveTimer {
    pub kind: &'static str,
    pub label: String,
    pub spawns_at: Option<f64>,
    pub alive: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct TeamIntel {
    pub kills: i64,
    pub dragons: Vec<String>,
    pub towers: i64,
    pub inhibitors: i64,
    pub barons: i64,
    pub heralds: i64,
    pub est_gold: f64,
    pub alive_count: usize,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Recent {
    pub at: f64,
    pub text: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MyDeath {
    pub at: f64,
    pub killer: String,
    pub assisters: Vec<String>,
    pub enemies_involved: usize,
    pub was_gank: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Buff {
    pub kind: &'static str, // baron | elder | herald (held Eye of the Herald, ~240 s)
    pub team: &'static str, // ally | enemy
    pub until: f64,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Ace {
    pub team: &'static str,
    pub at: f64,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MyStats {
    pub gold: f64,
    pub hp: f64,
    pub max_hp: f64,
    pub ad: f64,
    pub ap: f64,
    pub armor: f64,
    pub mr: f64,
    pub move_speed: f64,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GameIntel {
    pub game_time: f64,
    pub game_mode: String,
    pub me: Option<PlayerIntel>,
    pub players: Vec<PlayerIntel>,
    pub ally: TeamIntel,
    pub enemy: TeamIntel,
    pub gold_diff: f64,
    pub objectives: Vec<ObjectiveTimer>,
    pub recent: Vec<Recent>,
    pub my_deaths: Vec<MyDeath>,
    pub buffs: Vec<Buff>,
    pub streaks: BTreeMap<String, i64>,
    pub ace: Option<Ace>,
    pub my_stats: Option<MyStats>,
    /// Per lane: towers lost by each side and inhibitors down (with respawn time).
    pub lanes: Vec<LaneState>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LaneState {
    /// top | mid | bot
    pub lane: &'static str,
    pub ours_lost: u8,
    pub theirs_lost: u8,
    /// Game time when our / their inhibitor in this lane respawns (None = standing).
    pub our_inhib_back: Option<f64>,
    pub their_inhib_back: Option<f64>,
}

/// Passive gold + farm + takedowns: a rough "gold earned so far".
pub fn estimate_gold_earned(game_time: f64, cs: f64, kills: f64, assists: f64, ward_score: f64) -> f64 {
    let passive = (game_time - 65.0).max(0.0) * 2.04; // ~20.4 gold / 10 s after 1:05
    500.0 + passive + cs * 21.0 + kills * 300.0 + assists * 120.0 + ward_score * 2.0
}

fn name_of(p: &LivePlayer) -> String {
    p.riot_id_game_name
        .clone()
        .or_else(|| p.riot_id.as_ref().map(|r| r.split('#').next().unwrap_or("").to_string()))
        .or_else(|| p.summoner_name.clone())
        .unwrap_or_default()
}

fn role_of(p: &LivePlayer) -> String {
    match p.position.as_str() {
        "TOP" | "JUNGLE" | "MIDDLE" | "BOTTOM" | "UTILITY" => return p.position.clone(),
        _ => {}
    }
    let smite = p.spell_names().iter().map(|s| s.to_lowercase()).any(|s| s.contains("smite") || s.contains("porażenie"));
    if smite { "JUNGLE" } else { "UNKNOWN" }.into()
}

/// JS `String(x ?? '')` for event fields.
fn s(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(x)) => x.clone(),
        Some(o) => o.to_string(),
    }
}

/// Stateful: keeps history needed for spikes and manual Flash timers.
#[derive(Default)]
pub struct IntelTracker {
    prev: HashMap<String, (i64, usize)>,
    spikes: HashMap<String, Vec<Spike>>,
    flash_used_at: HashMap<String, f64>,
    game_id: String,
    hp: HashMap<String, (f64, f64)>,
    mana: HashMap<String, (Option<f64>, f64)>,
    shield: HashMap<String, (f64, f64)>,
}

impl IntelTracker {
    /// HP fraction of a champion read from the screen, at game time t.
    pub fn set_hp(&mut self, champion: &str, pct: f64, t: f64) {
        self.hp.insert(champion.into(), (pct, t));
    }

    /// Shield (fraction of max HP) from a health bar / ally portrait.
    pub fn set_shield(&mut self, champion: &str, pct: f64, t: f64) {
        self.shield.insert(champion.into(), (pct, t));
    }

    /// Mana fraction of an ally (portraits above the minimap); None = not a mana champion.
    pub fn set_mana(&mut self, champion: &str, pct: Option<f64>, t: f64) {
        self.mana.insert(champion.into(), (pct, t));
    }

    /// Manual hotkey: "enemy X used Flash".
    pub fn mark_flash(&mut self, champion: &str, game_time: f64) {
        self.flash_used_at.insert(champion.into(), game_time);
    }

    pub fn update(&mut self, d: &AllGameData, item_db: Option<&HashMap<i64, ItemData>>, champ_stats: Option<&HashMap<String, (f64, f64)>>) -> GameIntel {
        let t = d.game_data.game_time;
        let key = d.all_players.iter().map(|p| p.champion_name.as_str()).collect::<Vec<_>>().join("|");
        if key != self.game_id {
            self.game_id = key;
            self.prev.clear();
            self.spikes.clear();
            self.flash_used_at.clear();
            self.hp.clear();
        }
        let ap = &d.active_player;
        let me_name = ap.riot_id.as_ref().map(|r| r.split('#').next().unwrap_or("").to_string()).or_else(|| ap.summoner_name.clone()).unwrap_or_default();
        let me_idx = d.all_players.iter().position(|p| name_of(p) == me_name || p.riot_id == ap.riot_id);
        let my_team = me_idx.map(|i| d.all_players[i].team.clone()).unwrap_or_else(|| "ORDER".into());

        let mut players = Vec::new();
        for (pi, p) in d.all_players.iter().enumerate() {
            let is_me = Some(pi) == me_idx;
            let items: Vec<(PlayerItem, bool, bool)> = p
                .items
                .iter()
                .map(|i| {
                    let db = item_db.and_then(|m| m.get(&i.item_id));
                    (
                        PlayerItem { id: i.item_id, name: db.map(|x| x.name.clone()).unwrap_or_else(|| i.display_name.clone()), price: db.map(|x| x.total).unwrap_or(i.price) },
                        db.map(|x| x.completed).unwrap_or(false),
                        db.is_some_and(|x| x.purchasable),
                    )
                })
                .collect();
            let item_gold: f64 = items.iter().map(|i| i.0.price).sum();
            // Gold of items that can only come from the shop: proof of a base visit. Items that evolve on
            // their own (jungle pet, support quest, transforms) are not in the shop and must not count.
            let shop_gold: f64 = items.iter().filter(|i| i.2).map(|i| i.0.price).sum();
            let completed = items.iter().filter(|i| i.1).count();
            let sc = &p.scores;
            let mut spikes = self.spikes.get(&p.champion_name).cloned().unwrap_or_default();
            if let Some(&(plevel, pcompleted)) = self.prev.get(&p.champion_name) {
                for lvl in [6, 11, 16] {
                    if plevel < lvl && p.level >= lvl {
                        spikes.insert(0, Spike { kind: "level", label: format!("poziom {lvl}"), at: t });
                    }
                }
                if completed > pcompleted {
                    let newest = items.iter().filter(|i| i.1).last().map(|i| i.0.name.clone()).unwrap_or_else(|| "nowy przedmiot".into());
                    spikes.insert(0, Spike { kind: "item", label: newest, at: t });
                }
            }
            self.prev.insert(p.champion_name.clone(), (p.level, completed));
            spikes.truncate(6);
            self.spikes.insert(p.champion_name.clone(), spikes.clone());
            let flash_in = self.flash_used_at.get(&p.champion_name).map(|at| (at + 300.0 - t).max(0.0));
            let max_hp = ap.stat("maxHealth").filter(|v| *v != 0.0);
            let hp_pct = if is_me && max_hp.is_some() {
                Some(ap.stat("currentHealth").unwrap_or(0.0) / max_hp.unwrap())
            } else if p.is_dead {
                self.hp.remove(&p.champion_name);
                Some(0.0)
            } else {
                self.hp.get(&p.champion_name).map(|h| h.0)
            };
            let ms = ap.stat("moveSpeed").filter(|v| *v != 0.0);
            let move_speed = if is_me && ms.is_some() {
                js_round(ms.unwrap())
            } else {
                let base = champ_stats.and_then(|m| m.get(&p.champion_name)).map(|s| s.0).unwrap_or(340.0);
                let bonuses: Vec<(f64, f64)> = p.items.iter().map(|i| item_db.and_then(|m| m.get(&i.item_id)).map(|x| (x.flat_ms, x.pct_ms)).unwrap_or((0.0, 0.0))).collect();
                estimate_move_speed(base, &bonuses)
            };
            players.push(PlayerIntel {
                champion: p.champion_name.clone(),
                team: p.team.clone(),
                is_ally: p.team == my_team,
                is_me,
                role: role_of(p),
                level: p.level,
                kills: sc.kills,
                deaths: sc.deaths,
                assists: sc.assists,
                cs: sc.creep_score,
                ward_score: js_round(sc.ward_score) as i64,
                is_dead: p.is_dead,
                respawn_in: if p.is_dead { js_round(p.respawn_timer).max(0.0) } else { 0.0 },
                item_gold,
                shop_gold,
                est_gold: js_round(item_gold.max(estimate_gold_earned(t, sc.creep_score as f64, sc.kills as f64, sc.assists as f64, sc.ward_score))),
                items: items.into_iter().map(|i| i.0).collect(),
                completed_items: completed,
                spikes,
                summoner_spells: p.spell_names(),
                hp_pct,
                hp_seen_at: if is_me { Some(t) } else { self.hp.get(&p.champion_name).map(|h| h.1) },
                move_speed,
                flash_in,
                shield_pct: self.shield.get(&p.champion_name).filter(|s| t - s.1 <= 3.0 && s.0 > 0.01).map(|s| s.0),
                mana_pct: if is_me {
                    let mana = ap.champion_stats.get("resourceType").and_then(|v| v.as_str()) == Some("MANA");
                    match (mana, ap.stat("resourceValue"), ap.stat("resourceMax")) {
                        (true, Some(v), Some(m)) if m > 0.0 => Some((v / m).clamp(0.0, 1.0)),
                        _ => None,
                    }
                } else {
                    self.mana.get(&p.champion_name).filter(|m| t - m.1 <= 20.0).and_then(|m| m.0)
                },
            });
        }

        let mut name_team: HashMap<String, String> = HashMap::new();
        for p in &d.all_players {
            name_team.insert(name_of(p), p.team.clone());
        }
        let re_order = regex::Regex::new(r"(?i)T(Order|100)").unwrap();
        let re_chaos = regex::Regex::new(r"(?i)T(Chaos|200)").unwrap();
        let team_of = |n: &str| -> Option<String> {
            if let Some(t) = name_team.get(n) {
                return Some(t.clone());
            }
            if let Some(b) = d.all_players.iter().find(|p| format!("Bot {}", p.champion_name) == n) {
                return Some(b.team.clone()); // practice tool bots
            }
            if re_order.is_match(n) || n.contains("_T1_") {
                return Some("ORDER".into()); // minions/turrets
            }
            if re_chaos.is_match(n) || n.contains("_T2_") {
                return Some("CHAOS".into());
            }
            None
        };
        let player_champ: HashMap<String, String> = d.all_players.iter().map(|p| (name_of(p), p.champion_name.clone())).collect();
        let me_champ = me_idx.map(|i| d.all_players[i].champion_name.clone());
        let swift = d.game_data.game_mode == "SWIFTPLAY";
        let mut ev = events(&d.events.events, t, &my_team, &team_of, &players, &me_name, me_champ.as_deref(), &player_champ, swift);
        ev.ally.est_gold = players.iter().filter(|p| p.is_ally).map(|p| p.est_gold).sum();
        ev.enemy.est_gold = players.iter().filter(|p| !p.is_ally).map(|p| p.est_gold).sum();
        ev.ally.alive_count = players.iter().filter(|p| p.is_ally && !p.is_dead).count();
        ev.enemy.alive_count = players.iter().filter(|p| !p.is_ally && !p.is_dead).count();
        let st = |k: &str| js_round(ap.stat(k).unwrap_or(0.0));
        GameIntel {
            game_time: t,
            game_mode: d.game_data.game_mode.clone(),
            me: players.iter().find(|p| p.is_me).cloned(),
            gold_diff: ev.ally.est_gold - ev.enemy.est_gold,
            players,
            ally: ev.ally,
            enemy: ev.enemy,
            objectives: ev.objectives,
            recent: ev.recent,
            my_deaths: ev.my_deaths,
            buffs: ev.buffs,
            streaks: ev.streaks,
            ace: ev.ace,
            lanes: ev.lanes,
            my_stats: me_idx.map(|_| MyStats {
                gold: js_round(ap.current_gold),
                hp: st("currentHealth"),
                max_hp: st("maxHealth"),
                ad: st("attackDamage"),
                ap: st("abilityPower"),
                armor: st("armor"),
                mr: st("magicResist"),
                move_speed: st("moveSpeed"),
            }),
        }
    }
}

struct EventsOut {
    ally: TeamIntel,
    enemy: TeamIntel,
    objectives: Vec<ObjectiveTimer>,
    recent: Vec<Recent>,
    my_deaths: Vec<MyDeath>,
    buffs: Vec<Buff>,
    streaks: BTreeMap<String, i64>,
    ace: Option<Ace>,
    lanes: Vec<LaneState>,
}

#[allow(clippy::too_many_arguments)]
fn events(
    evs: &[Value],
    t: f64,
    my_team: &str,
    team_of: &dyn Fn(&str) -> Option<String>,
    players: &[PlayerIntel],
    me_name: &str,
    me_champ: Option<&str>,
    player_champ: &HashMap<String, String>,
    // Swiftplay (26.1): no Voidgrubs / Herald, Baron at 12:00.
    swift: bool,
) -> EventsOut {
    let blank = || TeamIntel { alive_count: 5, ..Default::default() };
    let (mut ally, mut enemy) = (blank(), blank());
    let mut recent: Vec<Recent> = Vec::new();
    let (mut last_dragon, mut last_baron): (Option<f64>, Option<f64>) = (None, None);
    let (mut herald_taken, mut grubs_taken, mut atakhan_taken, mut elder_unlocked) = (false, 0, false, false);
    let mut inhibs: Vec<(f64, bool, &'static str)> = Vec::new();
    // (lane, our tower) for every destroyed lane tower.
    let mut towers_down: Vec<(&'static str, bool)> = Vec::new();
    let mut my_deaths: Vec<MyDeath> = Vec::new();
    let mut buffs: Vec<Buff> = Vec::new();
    let mut streaks: BTreeMap<String, i64> = BTreeMap::new();
    let mut ace: Option<Ace> = None;
    let is_me = |n: &str| n == me_name || me_champ.is_some_and(|c| n.strip_prefix("Bot ").unwrap_or(n) == c);
    // Real games name players (Riot ID game name) in events; practice tool uses 'Bot <Champion>'.
    let champ_of = |raw: &str| -> String {
        if let Some(c) = player_champ.get(raw) {
            return c.clone();
        }
        let s = raw.strip_prefix("Bot ").unwrap_or(raw);
        players.iter().find(|p| p.champion == s).map(|p| p.champion.clone()).unwrap_or_else(|| s.to_string())
    };
    let time = |e: &Value| e.get("EventTime").and_then(Value::as_f64).unwrap_or(0.0);

    // Robustness: ignore events from the future and duplicates (replays re-emit events after a seek).
    let mut seen = HashSet::new();
    let mut clean: Vec<&Value> = evs
        .iter()
        .filter(|e| {
            if time(e) > t + 1.0 {
                return false;
            }
            let k = format!("{}|{}|{}|{}", s(e.get("EventName")), js_round(time(e)), s(e.get("KillerName")), s(e.get("VictimName")));
            seen.insert(k)
        })
        .collect();
    clean.sort_by(|a, b| time(a).total_cmp(&time(b)));

    for e in clean {
        let at = time(e);
        let killer_name = s(e.get("KillerName"));
        let killer_team = team_of(&killer_name);
        let side_is_ally = killer_team.as_deref().map(|k| k == my_team);
        let who = if killer_team.as_deref() == Some(my_team) { "my" } else { "wróg" };
        macro_rules! sd {
            () => {
                match side_is_ally {
                    Some(true) => Some(&mut ally),
                    Some(false) => Some(&mut enemy),
                    None => None,
                }
            };
        }
        let rel = |team: &Option<String>| if team.as_deref() == Some(my_team) { "ally" } else { "enemy" };
        match s(e.get("EventName")).as_str() {
            "ChampionKill" => {
                let victim_name = s(e.get("VictimName"));
                let (killer, victim) = (champ_of(&killer_name), champ_of(&victim_name));
                *streaks.entry(killer.clone()).or_insert(0) += 1;
                streaks.insert(victim.clone(), 0);
                if is_me(&victim_name) {
                    let assisters: Vec<String> = e.get("Assisters").and_then(Value::as_array).map(|a| a.iter().map(|x| champ_of(&s(Some(x)))).collect()).unwrap_or_default();
                    // A gank = the kill involved an enemy who isn't my lane opponent (jungler/roamer) or ≥2 enemies.
                    let my_role = players.iter().find(|x| x.is_me).map(|x| x.role.clone());
                    let lane_opp = players.iter().find(|p| !p.is_ally && Some(&p.role) == my_role.as_ref()).map(|p| p.champion.clone());
                    let involved: Vec<String> = std::iter::once(killer.clone()).chain(assisters.iter().cloned()).filter(|c| players.iter().any(|p| &p.champion == c && !p.is_ally)).collect();
                    let was_gank = involved.len() >= 2 || lane_opp.as_ref().is_some_and(|o| !involved.contains(o));
                    my_deaths.insert(0, MyDeath { at, killer: killer.clone(), assisters, enemies_involved: involved.len(), was_gank });
                }
                if let Some(x) = sd!() {
                    x.kills += 1;
                }
                recent.insert(0, Recent { at, text: format!("{killer} zabił {victim}") });
            }
            "DragonKill" => {
                let ty = s(e.get("DragonType"));
                if ty == "Elder" {
                    if killer_team.is_some() {
                        buffs.push(Buff { kind: "elder", team: rel(&killer_team), until: at + 150.0 });
                    }
                    last_dragon = Some(at);
                    recent.insert(0, Recent { at, text: format!("Elder Dragon: {who}") });
                    continue;
                }
                if let Some(x) = sd!() {
                    x.dragons.push(ty.clone());
                    if x.dragons.len() >= 4 {
                        elder_unlocked = true;
                    }
                }
                last_dragon = Some(at);
                recent.insert(0, Recent { at, text: format!("{} Dragon: {who}", dragon_pl(&ty).map(String::from).unwrap_or(ty)) });
            }
            "BaronKill" => {
                if let Some(x) = sd!() {
                    x.barons += 1;
                }
                last_baron = Some(at);
                if killer_team.is_some() {
                    buffs.push(Buff { kind: "baron", team: rel(&killer_team), until: at + 180.0 });
                }
                recent.insert(0, Recent { at, text: format!("Baron: {who}") });
            }
            "HeraldKill" => {
                if let Some(x) = sd!() {
                    x.heralds += 1;
                }
                herald_taken = true;
                if killer_team.is_some() {
                    buffs.push(Buff { kind: "herald", team: rel(&killer_team), until: at + 240.0 });
                }
                recent.insert(0, Recent { at, text: format!("Herald: {who}") });
            }
            "HordeKill" => grubs_taken += 1,
            "Ace" => {
                let acing = s(e.get("AcingTeam"));
                if acing == "ORDER" || acing == "CHAOS" {
                    ace = Some(Ace { team: if acing == my_team { "ally" } else { "enemy" }, at });
                }
            }
            "AtakhanKill" => {
                atakhan_taken = true;
                recent.insert(0, Recent { at, text: format!("Atakhan: {who}") });
            }
            "TurretKilled" => {
                if let Some(x) = sd!() {
                    x.towers += 1;
                }
                // "Turret_T1_L_03_A": T1 = ORDER's tower, T2 = CHAOS's; L = top, C = mid, R = bot.
                let name = s(e.get("TurretKilled"));
                let owner = if name.contains("_T1_") { Some("ORDER") } else if name.contains("_T2_") { Some("CHAOS") } else { None };
                let lane = if name.contains("_L_") { Some("top") } else if name.contains("_R_") { Some("bot") } else if name.contains("_C_") { Some("mid") } else { None };
                let nexus = name.contains("_C_01") || name.contains("_C_02");
                if let (Some(o), Some(l)) = (owner, lane) {
                    if !nexus {
                        towers_down.push((l, o == my_team));
                    }
                }
                recent.insert(0, Recent { at, text: format!("Wieża: {who}") });
            }
            "InhibKilled" => {
                if let Some(x) = sd!() {
                    x.inhibitors += 1;
                }
                let name = s(e.get("InhibKilled"));
                // First lane marker in the name, like /_L0_|_L1_|_L2_/.exec.
                let lane = ["_L0_", "_L1_", "_L2_"].iter().filter_map(|m| name.find(m).map(|i| (i, *m))).min().map(|x| x.1);
                let lane_name = match lane {
                    Some("_L0_") => "bot",
                    Some("_L1_") => "mid",
                    Some("_L2_") => "top",
                    _ => "",
                };
                inhibs.push((at, killer_team.as_deref() != Some(my_team), lane_name));
            }
            _ => {}
        }
    }

    let mut objectives = Vec::new();
    let dragon_at = match last_dragon {
        None => obj::DRAGON_FIRST,
        Some(l) => l + if elder_unlocked { obj::ELDER_RESPAWN } else { obj::DRAGON_RESPAWN },
    };
    objectives.push(ObjectiveTimer {
        kind: "dragon",
        label: if elder_unlocked { "Elder Dragon" } else { "Dragon" }.into(),
        spawns_at: (dragon_at > t).then_some(dragon_at),
        alive: dragon_at <= t,
    });
    // One encounter of 3 Voidgrubs (the second spawn was removed in 25.09).
    if !swift && t < obj::HERALD_FIRST + 5.0 * 60.0 && grubs_taken < 3 {
        objectives.push(ObjectiveTimer { kind: "grubs", label: "Voidgrubs".into(), spawns_at: (obj::GRUBS_FIRST > t).then_some(obj::GRUBS_FIRST), alive: obj::GRUBS_FIRST <= t });
    }
    if !swift && !herald_taken && t < obj::BARON_FIRST - 10.0 {
        objectives.push(ObjectiveTimer { kind: "herald", label: "Herald".into(), spawns_at: (obj::HERALD_FIRST > t).then_some(obj::HERALD_FIRST), alive: obj::HERALD_FIRST <= t });
    }
    if obj::ATAKHAN_FIRST.is_finite() && !atakhan_taken && t < obj::BARON_FIRST + 60.0 {
        objectives.push(ObjectiveTimer { kind: "atakhan", label: "Atakhan".into(), spawns_at: (obj::ATAKHAN_FIRST > t).then_some(obj::ATAKHAN_FIRST), alive: obj::ATAKHAN_FIRST <= t });
    }
    let baron_first = if swift { 12.0 * 60.0 } else { obj::BARON_FIRST };
    let baron_at = last_baron.map(|l| l + obj::BARON_RESPAWN).unwrap_or(baron_first);
    objectives.push(ObjectiveTimer { kind: "baron", label: "Baron".into(), spawns_at: (baron_at > t).then_some(baron_at), alive: baron_at <= t });
    for &(at, mine, name) in &inhibs {
        let back = at + obj::INHIB_RESPAWN;
        if back > t {
            objectives.push(ObjectiveTimer { kind: "inhibitor", label: format!("Inhib {name} ({})", if mine { "nasz" } else { "wroga" }), spawns_at: Some(back), alive: false });
        }
    }
    let lanes = ["top", "mid", "bot"]
        .into_iter()
        .map(|lane| {
            let back = |mine: bool| inhibs.iter().filter(|(_, m, n)| *m == mine && *n == lane).map(|(at, _, _)| at + obj::INHIB_RESPAWN).filter(|b| *b > t).fold(None, |a: Option<f64>, b| Some(a.map_or(b, |a| a.max(b))));
            LaneState {
                lane,
                ours_lost: towers_down.iter().filter(|(l, mine)| *l == lane && *mine).count() as u8,
                theirs_lost: towers_down.iter().filter(|(l, mine)| *l == lane && !*mine).count() as u8,
                our_inhib_back: back(true),
                their_inhib_back: back(false),
            }
        })
        .collect();
    recent.truncate(10);
    EventsOut {
        lanes,
        ally,
        enemy,
        objectives,
        recent,
        my_deaths,
        buffs: buffs.into_iter().filter(|b| b.until > t).collect(),
        streaks,
        ace: ace.filter(|a| t - a.at < 60.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intel::ddragon::tests::test_items;
    use serde_json::json;

    fn load() -> AllGameData {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/allgamedata.json")).unwrap();
        crate::sources::live::parse(serde_json::from_str(&text).unwrap()).unwrap()
    }

    #[test]
    fn real_practice_tool_snapshot() {
        let items = test_items();
        let intel = IntelTracker::default().update(&load(), Some(&items), None);
        assert_eq!(intel.me.as_ref().unwrap().champion, "Ambessa");
        assert_eq!(intel.players.iter().filter(|p| p.is_ally).count(), 5);
        assert!(!intel.players.iter().find(|p| p.champion == "Warwick").unwrap().is_ally);
        let me = intel.me.as_ref().unwrap();
        assert_eq!(me.items.iter().find(|i| i.id == 6333).unwrap().price, 3300.0);
        assert!(me.completed_items >= 1);
        assert!(intel.enemy.dragons.len() + intel.ally.dragons.len() > 0);
        assert!(intel.ally.kills + intel.enemy.kills > 0);
        let dragon = intel.objectives.iter().find(|o| o.kind == "dragon").unwrap();
        assert!(dragon.alive || dragon.spawns_at.unwrap() > intel.game_time);
        assert_eq!(intel.recent.len(), 10);
        let js = serde_json::to_value(&intel).unwrap();
        assert!(js.get("goldDiff").is_some() && js["players"][0].get("isAlly").is_some());
    }

    /// d07ffee: recall proof counts only shop-purchasable items (an evolving jungle pet / support
    /// quest item is not bought in the shop); items unknown to Data Dragon do not count either.
    #[test]
    fn shop_gold_counts_purchasable_items_only() {
        let mut items = test_items();
        let base = IntelTracker::default().update(&load(), Some(&items), None);
        let me = base.me.as_ref().unwrap();
        let known: f64 = me.items.iter().filter(|i| items.contains_key(&i.id)).map(|i| i.price).sum();
        assert_eq!(me.shop_gold, known);
        assert!(me.shop_gold <= me.item_gold);
        items.get_mut(&6333).unwrap().purchasable = false;
        let i = IntelTracker::default().update(&load(), Some(&items), None);
        let me2 = i.me.as_ref().unwrap();
        assert_eq!(me2.item_gold, me.item_gold);
        assert_eq!(me2.shop_gold, me.shop_gold - 3300.0);
        assert!(serde_json::to_value(&i).unwrap()["players"][0].get("shopGold").is_some());
    }

    #[test]
    fn level_six_spike_between_snapshots() {
        let mut d = load();
        let mut tr = IntelTracker::default();
        let i = d.all_players.iter().position(|x| x.champion_name == "Warwick").unwrap();
        d.all_players[i].level = 5;
        tr.update(&d, None, None);
        d.all_players[i].level = 6;
        d.game_data.game_time += 2.0;
        let intel = tr.update(&d, None, None);
        assert_eq!(intel.players.iter().find(|x| x.champion == "Warwick").unwrap().spikes[0].label, "poziom 6");
    }

    #[test]
    fn dragon_respawns_five_minutes_after_kill() {
        let mut d = load();
        d.events.events = vec![json!({ "EventID": 1, "EventName": "DragonKill", "EventTime": 600, "DragonType": "Fire", "KillerName": "Bot Warwick" })];
        d.game_data.game_time = 700.0;
        let intel = IntelTracker::default().update(&d, None, None);
        let dragon = intel.objectives.iter().find(|o| o.kind == "dragon").unwrap();
        assert_eq!(dragon.spawns_at, Some(600.0 + obj::DRAGON_RESPAWN));
        assert_eq!(intel.enemy.dragons, ["Fire"]);
        assert_eq!(intel.recent[0].text, "Infernal Dragon: wróg");
    }

    #[test]
    fn my_death_gank_and_flash_timer() {
        let mut d = load();
        d.events.events = vec![
            json!({ "EventName": "ChampionKill", "EventTime": 500, "KillerName": "Bot Warwick", "VictimName": "Bot Ambessa", "Assisters": ["Bot Lulu"] }),
            json!({ "EventName": "ChampionKill", "EventTime": 500.2, "KillerName": "Bot Warwick", "VictimName": "Bot Ambessa", "Assisters": [] }), // duplicate
            json!({ "EventName": "ChampionKill", "EventTime": 9999, "KillerName": "Bot Fizz", "VictimName": "Bot Lulu" }), // future
        ];
        d.game_data.game_time = 520.0;
        let mut tr = IntelTracker::default();
        tr.update(&d, None, None); // a new game clears manual timers, like the TS tracker
        tr.mark_flash("Warwick", 510.0);
        let intel = tr.update(&d, None, None);
        assert_eq!(intel.my_deaths.len(), 1);
        assert_eq!(intel.my_deaths[0].killer, "Warwick");
        assert_eq!(intel.my_deaths[0].enemies_involved, 2);
        assert!(intel.my_deaths[0].was_gank);
        assert_eq!(intel.streaks["Warwick"], 1);
        assert_eq!(intel.enemy.kills, 1);
        assert_eq!(intel.players.iter().find(|p| p.champion == "Warwick").unwrap().flash_in, Some(290.0));
    }

    #[test]
    fn gold_estimate() {
        assert_eq!(estimate_gold_earned(65.0, 0.0, 0.0, 0.0, 0.0), 500.0);
        assert!((estimate_gold_earned(165.0, 10.0, 1.0, 1.0, 5.0) - (500.0 + 204.0 + 210.0 + 300.0 + 120.0 + 10.0)).abs() < 1e-9);
    }
}
