//! Lane matchup knowledge (`src/main/intel/matchup.ts`): fetched once per
//! (patch, my champion, enemy champion, role) by the research model with web
//! access, cached on disk (`<userData>/matchups/<key>.json`, same files as TS).
//! `curve[i]` = my advantage at level i+1, from -1 (he wins hard) to +1 (I win hard).

use super::game_intel::{GameIntel, PlayerIntel};
use super::js_round;
use crate::llm::caller::ModelCaller;
use crate::llm::{Effort, ModelCall};
use crate::research::{research_effort, research_model};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchupBrief {
    pub patch: String,
    pub me: String,
    pub enemy: String,
    pub role: String,
    /// My win rate in this matchup on the current patch (0..1), None if not found.
    pub win_rate: Option<f64>,
    /// 18 values
    pub curve: Vec<f64>,
    #[serde(default)]
    pub my_spikes: Vec<String>,
    #[serde(default)]
    pub enemy_spikes: Vec<String>,
    /// ≤ 3 short Polish tips
    #[serde(default)]
    pub tips: Vec<String>,
    pub created_at: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MatchupQuery {
    pub patch: String,
    pub me: String,
    pub enemy: String,
    pub role: String,
}

pub fn schema() -> Value {
    json!({
        "type": "object", "additionalProperties": false,
        "required": ["winRate", "curve", "mySpikes", "enemySpikes", "tips"],
        "properties": {
            "winRate": { "anyOf": [{ "type": "null" }, { "type": "number", "minimum": 0, "maximum": 1 }] },
            "curve": { "type": "array", "items": { "type": "number", "minimum": -1, "maximum": 1 }, "minItems": 18, "maxItems": 18 },
            "mySpikes": { "type": "array", "items": { "type": "string", "maxLength": 40 }, "maxItems": 5 },
            "enemySpikes": { "type": "array", "items": { "type": "string", "maxLength": 40 }, "maxItems": 5 },
            "tips": { "type": "array", "items": { "type": "string", "maxLength": 90 }, "maxItems": 3 }
        }
    })
}

/// `${patch}-${role}-${me}-vs-${enemy}` without characters outside `[\w.-]`.
pub fn cache_key(q: &MatchupQuery) -> String {
    format!("{}-{}-{}-vs-{}", q.patch, q.role, q.me, q.enemy).chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')).collect()
}

fn file(dir: &Path, q: &MatchupQuery) -> PathBuf {
    dir.join(format!("{}.json", cache_key(q)))
}

pub fn load_cached_matchup(dir: &Path, q: &MatchupQuery) -> Option<MatchupBrief> {
    serde_json::from_str(&std::fs::read_to_string(file(dir, q)).ok()?).ok()
}

pub fn system_prompt(q: &MatchupQuery) -> String {
    let (me, en, role, patch) = (&q.me, &q.enemy, &q.role, &q.patch);
    format!(
        "Jesteś analitykiem matchupów League of Legends. Masz wyszukiwarkę i pobieranie stron.
Sprawdź matchup {me} vs {en} na linii {role} na patchu {patch} (strony ze statystykami matchupów, np. lolalytics/u.gg/op.gg, oraz poradniki matchupów).
Zwróć:
- winRate: winrate {me} przeciw {en} (0..1) albo null, jeśli nie znalazłeś,
- curve: 18 liczb, przewaga {me} na poziomach 1..18 w walce 1v1 przy równych itemach (-1 = {en} wyraźnie silniejszy, 0 = równo, +1 = {me} wyraźnie silniejszy). Uwzględnij spike'i z umiejętności (np. 2 lvl, 3 lvl, 6 lvl ult, 11, 16).
- mySpikes / enemySpikes: kluczowe momenty siły (poziomy, przedmioty).
- tips: maks. 3 krótkie wskazówki po polsku, jak grać ten matchup (kiedy tradować, czego unikać).
Opieraj się na źródłach; gdzie brak danych, oceń ostrożnie na podstawie kitów."
    )
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// Cached brief or a new one from the research model (written to `dir`).
pub async fn fetch_matchup(caller: &dyn ModelCaller, dir: &Path, q: &MatchupQuery, model: Option<&str>, effort: Option<Effort>, now_ms: f64) -> Result<MatchupBrief> {
    if let Some(b) = load_cached_matchup(dir, q) {
        return Ok(b);
    }
    let call = ModelCall {
        model: model.map(str::to_string).unwrap_or_else(research_model),
        effort: Some(effort.unwrap_or_else(research_effort)),
        system: system_prompt(q),
        prompt: format!("Matchup: {} vs {}, {}, patch {}.", q.me, q.enemy, q.role, q.patch),
        schema: schema(),
        tools: vec!["WebSearch".into(), "WebFetch".into()],
        timeout_ms: Some(180_000),
        ..Default::default()
    };
    let r = caller.call(&call).await?;
    let v = r.value.as_object().cloned().unwrap_or_else(Map::new);
    let raw_curve = v.get("curve").and_then(Value::as_array).cloned().unwrap_or_default();
    let curve = (0..18).map(|i| raw_curve.get(i).and_then(Value::as_f64).unwrap_or(0.0).clamp(-1.0, 1.0)).collect();
    let brief = MatchupBrief {
        patch: q.patch.clone(),
        me: q.me.clone(),
        enemy: q.enemy.clone(),
        role: q.role.clone(),
        win_rate: v.get("winRate").and_then(Value::as_f64),
        curve,
        my_spikes: strings(v.get("mySpikes")),
        enemy_spikes: strings(v.get("enemySpikes")),
        tips: strings(v.get("tips")),
        created_at: now_ms,
    };
    std::fs::create_dir_all(dir)?;
    std::fs::write(file(dir, q), crate::jsutil::json_pretty(&brief))?;
    Ok(brief)
}

/// Enemy playing my role (lane opponent); None if roles unknown.
pub fn lane_opponent(intel: &GameIntel) -> Option<&PlayerIntel> {
    let me = intel.me.as_ref()?;
    if me.role == "UNKNOWN" {
        return None;
    }
    intel.players.iter().find(|p| !p.is_ally && p.role == me.role)
}

fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}

/// Last seen HP (health bar on screen), remembered after the bar leaves the screen: it recovers
/// ~0.6%/s (regen) and is forgotten after 90 s or a death. (Was a 4 s cutoff: someone seen at 30%
/// counted as full HP 5 s later.)
/// HP plus a fresh shield (shields only count while just seen: they last seconds).
fn hp_eff(intel: &GameIntel, p: &PlayerIntel) -> Option<f64> {
    match (hp_fresh(intel, p), p.shield_pct) {
        (Some(h), Some(s)) => Some((h + s).min(1.6)),
        (None, Some(s)) => Some((1.0 + s).min(1.6)),
        (h, None) => h,
    }
}

fn hp_fresh(intel: &GameIntel, p: &PlayerIntel) -> Option<f64> {
    if p.is_dead {
        return None;
    }
    match (p.hp_pct, p.hp_seen_at) {
        (Some(h), Some(at)) => {
            let age = (intel.game_time - at).max(0.0);
            (age <= 90.0).then(|| (h + 0.006 * age).min(1.0))
        }
        _ => None,
    }
}

/// Live all-in chance: matchup curve at the current levels (main term) + level
/// difference + item gold / completed-item difference + my HP (and enemy HP from
/// his health bar, remembered with regen) + enemy Flash down + kill-threat spells
/// Returns (pMe, score). (Meta win rate is not used: not actionable in game.)
pub fn power_score(intel: &GameIntel, opp: &PlayerIntel, brief: Option<&MatchupBrief>) -> (f64, f64) {
    let me = intel.me.as_ref().expect("me");
    let lvl_me = me.level.clamp(1, 18);
    let lvl_opp = opp.level.clamp(1, 18);
    let curve_at = |l: i64| brief.and_then(|b| b.curve.get(l as usize - 1).copied()).unwrap_or(0.0);
    let curve = (curve_at(lvl_me) + curve_at(lvl_opp)) / 2.0;
    let level_diff = (lvl_me - lvl_opp) as f64;
    let gold_diff = (me.item_gold - opp.item_gold) / 1000.0;
    let items_diff = me.completed_items as f64 - opp.completed_items as f64;
    let hp = match &intel.my_stats {
        Some(s) if s.max_hp > 0.0 => s.hp / s.max_hp,
        _ => 1.0,
    };
    let opp_hp = hp_eff(intel, opp).unwrap_or(1.0);
    let flash = if opp.flash_in.is_some_and(|f| f > 0.0) { 0.25 } else { 0.0 };
    let threat_re = regex::Regex::new(r"(?i)Ignite|Podpalenie|Exhaust|Wyczerpanie").unwrap();
    let threat = if opp.summoner_spells.iter().any(|s| threat_re.is_match(s)) { -0.15 } else { 0.0 };
    // My mana (mana champions): below ~40% the all-in gets weaker, at 0 about -0.6.
    let my_mana = me.mana_pct.map_or(0.0, |m| -0.6 * (1.0 - (m / 0.4).min(1.0)));
    let score = my_mana + 1.6 * curve + 0.45 * level_diff + 0.35 * gold_diff + 0.35 * items_diff + 1.5 * (hp - 1.0) - 1.5 * (opp_hp - 1.0) + flash + threat;
    (sigmoid(score), score)
}

/// Next level (above the current one) where the matchup curve turns in my favour.
pub fn next_good_level(brief: Option<&MatchupBrief>, level: i64) -> Option<i64> {
    let b = brief?;
    ((level + 1)..=18).find(|&l| b.curve.get(l as usize - 1).copied().unwrap_or(0.0) > 0.15)
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PowerSide {
    pub champion: String,
    pub icon_url: String,
    pub level: i64,
}

/// `PowerBarState` from shared/intel.ts.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PowerBarState {
    pub me: PowerSide,
    pub enemy: PowerSide,
    /// Estimated chance I win an all-in right now, 0..1.
    pub p_me: f64,
    /// e.g. "lvl 5 vs 6 · +450 g · CS -12"
    pub detail: String,
    pub advice: String,
    pub meta_win_rate: Option<f64>,
    /// Lead over the lane opponent (me minus them): item gold, CS, level.
    pub lane: LaneLead,
    /// Allies next to me (minimap), counted in pMe.
    pub allies: Vec<NearSide>,
    /// Other enemies next to me: seen now (sure) or a fresh fog estimate close by (not sure).
    pub enemies: Vec<NearSide>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NearSide {
    pub champion: String,
    pub icon_url: String,
    pub level: i64,
    pub sure: bool,
}

/// Who is around me right now (from the minimap), for the fight estimate. Champion names;
/// the lane opponent and I are left out by the caller or ignored here.
#[derive(Clone, Debug, Default)]
pub struct Nearby {
    pub allies: Vec<String>,
    /// (champion, seen now)
    pub enemies: Vec<(String, bool)>,
    /// A bit further away but heading to me (gank / collapse incoming): counted at half weight.
    pub allies_coming: Vec<String>,
    pub enemies_coming: Vec<String>,
}

/// Weight of one extra champion in a fight: ~1, more for higher level / more item gold than the
/// game average, scaled by known HP. Fast and local (no model).
fn unit_strength(p: &PlayerIntel, avg_lvl: f64, avg_gold: f64, hp: Option<f64>) -> f64 {
    let s = 1.0 + 0.1 * (p.level as f64 - avg_lvl) + 0.3 * (p.item_gold - avg_gold) / 1000.0;
    // Out of mana = few spells: down to 70% at 0 mana (mana champions only).
    let mana = p.mana_pct.map_or(1.0, |m| 0.7 + 0.3 * m.clamp(0.0, 1.0));
    s.max(0.3) * hp.unwrap_or(1.0).clamp(0.2, 1.6) * mana
}

/// Fight score delta from the champions around me: allies add, extra enemies subtract
/// (unsure fog ones at 60%). Each extra body is worth about one 1v1 "lane" unit.
pub fn nearby_delta(intel: &GameIntel, opp: &PlayerIntel, near: &Nearby) -> f64 {
    let alive: Vec<&PlayerIntel> = intel.players.iter().filter(|p| !p.is_dead).collect();
    if alive.is_empty() {
        return 0.0;
    }
    let n = alive.len() as f64;
    let avg_lvl = alive.iter().map(|p| p.level as f64).sum::<f64>() / n;
    let avg_gold = alive.iter().map(|p| p.item_gold).sum::<f64>() / n;
    let find = |c: &str| alive.iter().copied().find(|p| p.champion == c);
    let mut d = 0.0;
    for a in &near.allies {
        if let Some(p) = find(a).filter(|p| p.is_ally && !p.is_me) {
            d += unit_strength(p, avg_lvl, avg_gold, hp_eff(intel, p));
        }
    }
    for (e, sure) in &near.enemies {
        if let Some(p) = find(e).filter(|p| !p.is_ally && p.champion != opp.champion) {
            d -= unit_strength(p, avg_lvl, avg_gold, hp_eff(intel, p)) * if *sure { 1.0 } else { 0.6 };
        }
    }
    for a in &near.allies_coming {
        if let Some(p) = find(a).filter(|p| p.is_ally && !p.is_me) {
            d += 0.5 * unit_strength(p, avg_lvl, avg_gold, hp_eff(intel, p));
        }
    }
    for e in &near.enemies_coming {
        if let Some(p) = find(e).filter(|p| !p.is_ally && p.champion != opp.champion) {
            d -= 0.5 * unit_strength(p, avg_lvl, avg_gold, hp_eff(intel, p));
        }
    }
    1.3 * d
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct LaneLead {
    pub gold: i64,
    pub cs: i64,
    pub level: i64,
}

fn signed(n: i64) -> String {
    if n >= 0 {
        format!("+{n}")
    } else {
        n.to_string()
    }
}

pub fn power_bar(intel: &GameIntel, brief: Option<&MatchupBrief>, icon: &dyn Fn(&str) -> String) -> Option<PowerBarState> {
    power_bar_near(intel, brief, icon, &Nearby::default())
}

/// Power bar for a fight right here: the 1v1 lane estimate plus everyone around me.
pub fn power_bar_near(intel: &GameIntel, brief: Option<&MatchupBrief>, icon: &dyn Fn(&str) -> String, near: &Nearby) -> Option<PowerBarState> {
    let me = intel.me.as_ref()?;
    let opp = lane_opponent(intel)?;
    if me.is_dead {
        return None;
    }
    let (_, base) = power_score(intel, opp, brief);
    let p_me = sigmoid(base + nearby_delta(intel, opp, near));
    let side = |c: &str, sure: bool| intel.players.iter().find(|p| p.champion == c && !p.is_dead).map(|p| NearSide { champion: p.champion.clone(), icon_url: icon(&p.champion), level: p.level, sure });
    let allies: Vec<NearSide> = near.allies.iter().filter(|c| intel.players.iter().any(|p| &p.champion == *c && p.is_ally && !p.is_me)).filter_map(|c| side(c, true)).collect();
    let enemies: Vec<NearSide> = near.enemies.iter().filter(|(c, _)| *c != opp.champion && intel.players.iter().any(|p| &p.champion == c && !p.is_ally)).filter_map(|(c, s)| side(c, *s)).collect();
    let gold = (js_round((me.item_gold - opp.item_gold) / 50.0) * 50.0) as i64;
    let cs = me.cs - opp.cs;
    let detail = format!("lvl {} vs {} · {} g · CS {}", me.level, opp.level, signed(gold), signed(cs));
    let advice = if opp.is_dead {
        format!("{} nie żyje → push / płytki", opp.champion)
    } else if let Some(hp) = hp_fresh(intel, opp).filter(|&h| h < 0.35 && p_me >= 0.55) {
        format!("{} {}% HP → all-in", opp.champion, js_round(hp * 100.0) as i64)
    } else if p_me >= 0.6 {
        if me.level > opp.level {
            format!("Masz {}, on {} → trade teraz", me.level, opp.level)
        } else {
            "Silniejszy teraz → wymieniaj".into()
        }
    } else if p_me <= 0.4 {
        match next_good_level(brief, me.level) {
            Some(nl) => format!("Czekaj na {nl} lvl, farm bezpiecznie"),
            None => "Słabszy → farm pod wieżą, nie wchodź".into(),
        }
    } else {
        "Wyrównane → trade tylko z przewagą fali".into()
    };
    Some(PowerBarState {
        me: PowerSide { champion: me.champion.clone(), icon_url: icon(&me.champion), level: me.level },
        enemy: PowerSide { champion: opp.champion.clone(), icon_url: icon(&opp.champion), level: opp.level },
        p_me,
        detail,
        advice,
        meta_win_rate: brief.and_then(|b| b.win_rate),
        lane: LaneLead { gold: gold as i64, cs: cs as i64, level: me.level as i64 - opp.level as i64 },
        allies,
        enemies,
    })
}

/// Compact object for the coach prompt (`CoachContext::matchup`).
pub fn matchup_for_coach(intel: &GameIntel, brief: Option<&MatchupBrief>) -> Option<Value> {
    let me = intel.me.as_ref()?;
    let opp = lane_opponent(intel)?;
    let (p_me, _) = power_score(intel, opp, brief);
    let mut o = Map::new();
    o.insert("vs".into(), json!(opp.champion));
    o.insert("pWinAllIn".into(), crate::jsutil::js_numbers(json!(js_round(p_me * 100.0) / 100.0)));
    o.insert("metaWinRate".into(), json!(brief.and_then(|b| b.win_rate)));
    o.insert("myNextGoodLevel".into(), json!(next_good_level(brief, me.level)));
    if let Some(b) = brief {
        o.insert("mySpikes".into(), json!(b.my_spikes));
        o.insert("enemySpikes".into(), json!(b.enemy_spikes));
        o.insert("tips".into(), json!(b.tips));
    }
    Some(Value::Object(o))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::intel::game_intel::{MyStats, TeamIntel};
    use crate::llm::caller::FakeCaller;
    use std::collections::BTreeMap;

    #[test]
    fn nearby_jungler_lowers_the_odds() {
        let mut me = player("Jinx", true, true, "BOTTOM", 9);
        me.item_gold = 4000.0;
        let mut sup = player("Lulu", true, false, "UTILITY", 8);
        sup.item_gold = 2000.0;
        let mut opp = player("Caitlyn", false, false, "BOTTOM", 9);
        opp.item_gold = 4000.0;
        let mut jg = player("Briar", false, false, "JUNGLE", 12);
        jg.item_gold = 7000.0;
        let i = intel(vec![me, sup, opp, jg]);
        let icon = |c: &str| c.to_string();
        let solo = power_bar(&i, None, &icon).unwrap().p_me;
        let with_sup = power_bar_near(&i, None, &icon, &Nearby { allies: vec!["Lulu".into()], ..Default::default() }).unwrap().p_me;
        let fog_jg = power_bar_near(&i, None, &icon, &Nearby { allies: vec!["Lulu".into()], enemies: vec![("Briar".into(), false)], ..Default::default() }).unwrap();
        let jg_coming = power_bar_near(&i, None, &icon, &Nearby { allies: vec!["Lulu".into()], enemies_coming: vec!["Briar".into()], ..Default::default() }).unwrap().p_me;
        assert!(jg_coming < with_sup && jg_coming > fog_jg.p_me - 0.2);
        let seen_jg = power_bar_near(&i, None, &icon, &Nearby { allies: vec!["Lulu".into()], enemies: vec![("Briar".into(), true)], ..Default::default() }).unwrap().p_me;
        assert!(with_sup > solo);
        assert!(fog_jg.p_me < with_sup && seen_jg < fog_jg.p_me);
        assert_eq!(fog_jg.allies.len(), 1);
        assert!(!fog_jg.enemies[0].sure);
    }


    pub fn player(champion: &str, ally: bool, me: bool, role: &str, level: i64) -> PlayerIntel {
        PlayerIntel {
            champion: champion.into(),
            team: if ally { "ORDER".into() } else { "CHAOS".into() },
            is_ally: ally,
            is_me: me,
            role: role.into(),
            level,
            kills: 0,
            deaths: 0,
            assists: 0,
            cs: 0,
            ward_score: 0,
            is_dead: false,
            respawn_in: 0.0,
            item_gold: 0.0,
            shop_gold: 0.0,
            est_gold: 0.0,
            items: vec![],
            completed_items: 0,
            spikes: vec![],
            summoner_spells: vec![],
            hp_pct: None,
            hp_seen_at: None,
            move_speed: 340.0,
            flash_in: None,
            mana_pct: None,
            shield_pct: None,
        }
    }

    pub fn intel(players: Vec<PlayerIntel>) -> GameIntel {
        GameIntel {
            game_time: 600.0,
            game_mode: "CLASSIC".into(),
            me: players.iter().find(|p| p.is_me).cloned(),
            players,
            ally: TeamIntel { alive_count: 5, ..Default::default() },
            enemy: TeamIntel { alive_count: 5, ..Default::default() },
            gold_diff: 0.0,
            objectives: vec![],
            recent: vec![],
            my_deaths: vec![],
            buffs: vec![],
            lanes: vec![],
            streaks: BTreeMap::new(),
            ace: None,
            my_stats: Some(MyStats { gold: 500.0, hp: 1000.0, max_hp: 1000.0, ad: 60.0, ap: 0.0, armor: 30.0, mr: 30.0, move_speed: 345.0 }),
        }
    }

    fn brief(curve6: f64) -> MatchupBrief {
        let mut curve = vec![-0.3; 18];
        for c in curve.iter_mut().skip(5) {
            *c = curve6;
        }
        MatchupBrief { patch: "16.22".into(), me: "Ahri".into(), enemy: "Zed".into(), role: "MIDDLE".into(), win_rate: Some(0.5), curve, my_spikes: vec!["lvl 6".into()], enemy_spikes: vec![], tips: vec!["farm".into()], created_at: 0.0 }
    }

    #[test]
    fn power_score_terms() {
        let i = intel(vec![player("Ahri", true, true, "MIDDLE", 5), player("Zed", false, false, "MIDDLE", 5)]);
        let opp = lane_opponent(&i).unwrap();
        assert_eq!(opp.champion, "Zed");
        let (p, s) = power_score(&i, opp, None);
        assert_eq!((p, s), (0.5, 0.0));
        let b = brief(0.6);
        let (_, s) = power_score(&i, opp, Some(&b));
        assert!((s - 1.6 * -0.3).abs() < 1e-9);
        let mut opp2 = opp.clone();
        opp2.flash_in = Some(100.0);
        opp2.summoner_spells = vec!["Podpalenie".into()];
        opp2.level = 4;
        let (_, s2) = power_score(&i, &opp2, None);
        assert!((s2 - (0.45 + 0.25 - 0.15)).abs() < 1e-9);
        assert_eq!(next_good_level(Some(&b), 3), Some(6));
        assert_eq!(next_good_level(Some(&b), 18), None);
    }

    #[test]
    fn power_bar_advice() {
        let mut i = intel(vec![player("Ahri", true, true, "MIDDLE", 5), player("Zed", false, false, "MIDDLE", 5)]);
        let b = brief(0.6);
        let icon = |c: &str| format!("icon:{c}");
        let pb = power_bar(&i, Some(&b), &icon).unwrap();
        assert_eq!(pb.advice, "Czekaj na 6 lvl, farm bezpiecznie");
        assert_eq!(pb.detail, "lvl 5 vs 5 · +0 g · CS +0");
        assert_eq!(pb.enemy.icon_url, "icon:Zed");
        i.players[1].hp_pct = Some(0.2);
        i.players[1].hp_seen_at = Some(598.0);
        let pb = power_bar(&i, None, &icon).unwrap();
        // Seen 2 s ago at 20%, remembered with regen (+0.6%/s) -> 21%.
        assert_eq!(pb.advice, "Zed 21% HP → all-in");
        i.players[1].is_dead = true;
        assert_eq!(power_bar(&i, None, &icon).unwrap().advice, "Zed nie żyje → push / płytki");
        let v = serde_json::to_value(power_bar(&i, None, &icon).unwrap()).unwrap();
        assert!(v.get("pMe").is_some() && v.get("metaWinRate").is_some() && v["me"].get("iconUrl").is_some());
        let c = matchup_for_coach(&i, Some(&b)).unwrap();
        assert_eq!(c["vs"], "Zed");
        assert_eq!(c["myNextGoodLevel"], 6);
        assert!(matchup_for_coach(&i, None).unwrap().get("tips").is_none());
    }

    #[tokio::test]
    async fn fetch_and_cache() {
        let d = tempfile::tempdir().unwrap();
        let fake = FakeCaller::new(json!({ "winRate": 0.52, "curve": [0.1, 2.0, -5], "mySpikes": ["lvl 6"], "enemySpikes": [], "tips": ["a"] }));
        let q = MatchupQuery { patch: "16.22".into(), me: "Kai'Sa".into(), enemy: "Lee Sin".into(), role: "BOTTOM".into() };
        assert_eq!(cache_key(&q), "16.22-BOTTOM-KaiSa-vs-LeeSin");
        let b = fetch_matchup(&fake, d.path(), &q, None, None, 7.0).await.unwrap();
        assert_eq!(b.curve.len(), 18);
        assert_eq!(&b.curve[..4], &[0.1, 1.0, -1.0, 0.0]);
        let call = &fake.calls()[0];
        assert_eq!(call.prompt, "Matchup: Kai'Sa vs Lee Sin, BOTTOM, patch 16.22.");
        assert_eq!(call.timeout_ms, Some(180_000));
        let text = std::fs::read_to_string(d.path().join("16.22-BOTTOM-KaiSa-vs-LeeSin.json")).unwrap();
        assert!(text.starts_with("{\n  \"patch\": \"16.22\",\n  \"me\": \"Kai'Sa\",\n  \"enemy\": \"Lee Sin\",\n  \"role\": \"BOTTOM\",\n  \"winRate\": 0.52,"), "{text}");
        assert_eq!(load_cached_matchup(d.path(), &q).unwrap(), b);
        fetch_matchup(&fake, d.path(), &q, Some("x"), None, 8.0).await.unwrap();
        assert_eq!(fake.calls().len(), 1);
    }
}
