//! What the coach knows beyond the Live API (index.ts): enemy profiles from the Riot API
//! (once per game), the patch brief (once per patch), the lane matchup (once per pairing),
//! the build panel, and the situational calls (fed / danger / objective) for the HUD.

use crate::game::Game;
use crate::state::AppState;
use crate::vision::SeenUnit;
use rc_core::build_advisor::{advise_build, build_context};
use rc_core::intel::ddragon::patch_of;
use rc_core::intel::game_intel::GameIntel;
use rc_core::intel::matchup::{fetch_matchup, lane_opponent, load_cached_matchup, power_score, MatchupQuery};
use rc_core::intel::situational::{self, fed_champions, FogEstimate, FogTrack, Pos};
use rc_core::llm::caller::CliCaller;
use rc_core::llm::profiles::{pick, Task};
use rc_core::patch_brief::{brief_for_game, build_patch_brief, load_cached_brief, BriefRequest};
use rc_core::riot::client::is_platform;
use rc_core::riot::{build_profiles, BuildProfilesOptions, ProfilePlayer, RiotClient, RiotClientOptions};
use rc_core::sources::live::AllGameData;
use rc_core::types::ProfilesStatus;
use std::sync::Arc;
use std::time::Duration;

/// Models used for research (patch brief, matchup) and in champion select — `applyModels`.
pub fn apply_models(st: &AppState) {
    let coach = st.settings.lock().unwrap().value().coach.clone();
    rc_core::llm::codex::set_fast_mode(coach.fast != Some(false));
    let b = st.game.lock().unwrap().backends;
    let r = pick(&coach.research_model, Task::Research, b);
    let d = pick(&coach.draft_model, Task::Draft, b);
    rc_core::research::set_research_model(&r.model, Some(r.effort));
    rc_core::research::set_fast_research_model(&d.model, Some(d.effort));
}

/// Riot platform from the settings (default eun1).
pub fn platform(st: &AppState) -> String {
    let p = st.settings.lock().unwrap().value().riot.platform.clone();
    if is_platform(&p) {
        p
    } else {
        "eun1".into()
    }
}

/// Riot Web API client with the key from the settings (None without a key). The key stays
/// inside the client (X-Riot-Token header) and is never logged.
pub fn riot_client(st: &AppState) -> Option<RiotClient> {
    let key = st.settings.lock().unwrap().value().riot.api_key.clone();
    if key.is_empty() {
        return None;
    }
    let mut o = RiotClientOptions::new(&key);
    o.cache_dir = Some(st.data_dir.join("riot-cache"));
    o.platform = Some(platform(st));
    match RiotClient::new(o) {
        Ok(c) => Some(c),
        Err(e) => {
            log::warn!("riot client: {e}");
            None
        }
    }
}

fn coach_usable(g: &Game) -> bool {
    !matches!(g.coach.status.state.as_str(), "not-logged-in" | "off")
}

fn role_of(pos: &str) -> String {
    if ["TOP", "JUNGLE", "MIDDLE", "BOTTOM", "UTILITY"].contains(&pos) {
        pos.to_string()
    } else {
        "UNKNOWN".into()
    }
}

/// Fetch enemy profiles once per game (needs a Riot API key) — `maybeLoadProfiles`.
pub fn maybe_load_profiles(st: &Arc<AppState>, g: &mut Game, d: &AllGameData, api_key_set: bool) {
    let key = d.all_players.iter().map(|p| p.champion_name.as_str()).collect::<Vec<_>>().join("|");
    if key == g.profiles_key {
        return;
    }
    let set = |s: ProfilesStatus| st.ui.lock().unwrap().profiles = s;
    if !api_key_set {
        // Not remembered as done: when a key is added during the game, the next tick fetches.
        set(ProfilesStatus { state: "no-key".into(), count: 0, error: None });
        return;
    }
    g.profiles_key = key.clone();
    g.profiles.clear();
    let my_team = g.intel.as_ref().and_then(|i| i.me.as_ref()).map(|m| m.team.clone()).unwrap_or_else(|| "ORDER".into());
    let players: Vec<ProfilePlayer> = d
        .all_players
        .iter()
        .filter(|p| p.team != my_team && !p.is_bot.unwrap_or(false) && p.riot_id.as_deref().is_some_and(|r| r.contains('#')))
        .map(|p| ProfilePlayer { riot_id: p.riot_id.clone().unwrap_or_default(), champion: p.champion_name.clone(), role: role_of(&p.position), ..Default::default() })
        .collect();
    if players.is_empty() {
        set(ProfilesStatus { state: "idle".into(), count: 0, error: Some("brak graczy z Riot ID (boty?)".into()) });
        return;
    }
    set(ProfilesStatus { state: "loading".into(), count: 0, error: None });
    let st = st.clone();
    st.rt.clone().spawn(async move {
        let Some(client) = riot_client(&st) else {
            st.ui.lock().unwrap().profiles = ProfilesStatus { state: "error".into(), count: 0, error: Some("Nieprawidłowy klucz Riot API".into()) };
            return;
        };
        let st2 = st.clone();
        let opts = BuildProfilesOptions {
            on_progress: Some(Box::new(move |pr| {
                // Late progress reports must not turn "ready" back into "loading".
                let mut ui = st2.ui.lock().unwrap();
                if ui.profiles.state == "loading" {
                    ui.profiles = ProfilesStatus { state: "loading".into(), count: pr.players_done as u32, error: None };
                }
            })),
            ..Default::default()
        };
        let list = build_profiles(&client, &players, &opts).await;
        let count = list.iter().filter(|p| p.games_analyzed > 0).count() as u32;
        {
            let mut g = st.game.lock().unwrap();
            if g.profiles_key != key {
                return;
            }
            g.profiles = list;
        }
        log::info!("profiles ready: {count} of {} with games", players.len());
        st.ui.lock().unwrap().profiles = ProfilesStatus { state: "ready".into(), count, error: None };
    });
}

/// Lane matchup brief (curve per level, spikes, tips), once per patch & pairing — `maybeLoadMatchup`.
pub fn maybe_load_matchup(st: &Arc<AppState>, g: &mut Game) {
    let (Some(intel), Some(sd)) = (g.intel.as_ref(), g.static_data.as_ref()) else { return };
    let Some(me) = intel.me.as_ref() else { return };
    let Some(opp) = lane_opponent(intel) else { return };
    let q = MatchupQuery { patch: patch_of(&sd.version), me: me.champion.clone(), enemy: opp.champion.clone(), role: me.role.clone() };
    let k = format!("{}|{}|{}|{}", q.patch, q.me, q.enemy, q.role);
    if k == g.matchup_key {
        return;
    }
    g.matchup_key = k.clone();
    let dir = st.data_dir.join("matchups");
    g.matchup = load_cached_matchup(&dir, &q);
    if g.matchup.is_some() {
        return;
    }
    if !coach_usable(g) {
        // Retry once the coach is usable (auth comes after the first intel tick).
        g.matchup_key.clear();
        return;
    }
    let st = st.clone();
    st.rt.clone().spawn(async move {
        match fetch_matchup(&CliCaller, &dir, &q, None, None, rc_core::jsutil::now_ms()).await {
            Ok(b) => {
                let mut g = st.game.lock().unwrap();
                if g.matchup_key == k {
                    g.matchup = Some(b);
                }
            }
            Err(e) => {
                log::warn!("matchup {e}");
                st.game.lock().unwrap().matchup_key.clear();
            }
        }
    });
}

/// Patch brief: cached file, else one research call per patch (10 min back-off on failure).
pub fn maybe_patch_brief(st: &Arc<AppState>, g: &mut Game) {
    let Some(sd) = g.static_data.clone() else { return };
    let dir = st.data_dir.join("patch");
    let patch = patch_of(&sd.version);
    if g.patch_brief.is_none() {
        g.patch_brief = load_cached_brief(&dir, &patch);
    }
    if g.patch_brief.is_some() || g.patch_brief_requested || g.coach.status.state != "idle" {
        return;
    }
    g.patch_brief_requested = true;
    let mut focus: Vec<String> = vec!["Rumble".into(), "Akali".into(), "Ambessa".into()];
    for p in g.intel.iter().flat_map(|i| i.players.iter()) {
        if !focus.contains(&p.champion) {
            focus.push(p.champion.clone());
        }
    }
    let st = st.clone();
    st.rt.clone().spawn(async move {
        let req = BriefRequest { patch, focus_champions: focus, ..Default::default() };
        match build_patch_brief(&CliCaller, &dir, &req, rc_core::jsutil::now_ms()).await {
            Ok(b) => st.game.lock().unwrap().patch_brief = Some(b),
            Err(e) => {
                log::warn!("patch brief failed {e}");
                tokio::time::sleep(Duration::from_secs(600)).await;
                st.game.lock().unwrap().patch_brief_requested = false;
            }
        }
    });
}

/// Patch brief text limited to this game's champions.
pub fn brief_text(g: &Game, intel: &GameIntel) -> Option<String> {
    let champions: Vec<String> = intel.players.iter().map(|p| p.champion.clone()).collect();
    g.patch_brief.as_ref().map(|b| brief_for_game(b, &champions))
}

/// Enemies clearly ahead of the lobby.
pub fn fed_names(intel: &GameIntel) -> Vec<String> {
    fed_champions(intel).into_iter().filter(|p| !p.is_ally).map(|p| p.champion.clone()).collect()
}

/// Game + my champion: a build answer is only valid for the session it was asked for.
fn build_session(intel: &GameIntel) -> String {
    let me = intel.me.as_ref().map(|m| m.champion.as_str()).unwrap_or("");
    format!("{me}|{}", intel.players.iter().map(|p| p.champion.as_str()).collect::<Vec<_>>().join(","))
}

/// Build panel: re-ask the build model on item / enemy-item / death changes or every 3 min.
pub fn maybe_build(st: &Arc<AppState>, g: &mut Game, live_model: &str, now: f64) {
    let (Some(sd), Some(_)) = (g.static_data.clone(), g.intel.as_ref()) else { return };
    // Just bought something from the plan: drop it from the panel now, not after the next answer.
    if let (Some(b), Some(me)) = (g.build.as_mut(), g.intel.as_ref().and_then(|i| i.me.as_ref())) {
        let owned: Vec<String> = me.items.iter().map(|i| i.name.to_lowercase()).collect();
        b.items.retain(|it| !owned.contains(&it.name.to_lowercase()));
        if b.items.is_empty() {
            g.build = None;
        }
    }
    let intel = g.intel.as_ref().unwrap();
    if intel.me.is_none() || g.build_busy || !coach_usable(g) || !g.build_trigger.should_refresh(intel, now) {
        return;
    }
    let brief = brief_text(g, intel);
    let Some(mut ctx) = build_context(intel, &sd.items, &g.profiles, brief.as_deref(), &fed_names(intel)) else { return };
    // Plan before this answer: an item from it I already started stays first until finished.
    let prev: Vec<String> = g.build.as_ref().map(|b| b.items.iter().map(|i| i.name.clone()).collect()).unwrap_or_default();
    let owned_ids: Vec<i64> = intel.me.as_ref().map(|m| m.items.iter().map(|i| i.id).collect()).unwrap_or_default();
    // Champion classes (Mage / Assassin / Tank ...) for me and every enemy: models that reason from item
    // stats alone put Liandry's on a burst Ahri; with the class they build for the champion.
    if let Some(o) = ctx.as_object_mut() {
        let class = |c: &str| sd.champion_tags.get(c).cloned().unwrap_or_default();
        if let (Some(me), Some(mo)) = (intel.me.as_ref(), o.get_mut("me").and_then(serde_json::Value::as_object_mut)) {
            mo.insert("class".into(), serde_json::json!(class(&me.champion)));
        }
        if let Some(es) = o.get_mut("enemies").and_then(serde_json::Value::as_array_mut) {
            for e in es {
                let c = e.get("champion").and_then(serde_json::Value::as_str).unwrap_or("").to_string();
                if let Some(eo) = e.as_object_mut() {
                    eo.insert("class".into(), serde_json::json!(class(&c)));
                }
            }
        }
        // The whole enemy team at a glance: what damage it deals and what it already builds against
        // (models miscount "4 of 5 AD" or "3 have MR" from raw item lists).
        let tagged = |p: &rc_core::intel::game_intel::PlayerIntel, tag: &str| p.items.iter().filter(|i| i.price >= 400.0 && sd.items.get(&i.id).is_some_and(|d| d.tags.iter().any(|t| t == tag))).count();
        let (mut ad, mut ap, mut mixed) = (0, 0, 0);
        let (mut armor, mut mr) = (Vec::new(), Vec::new());
        let mut per: std::collections::HashMap<String, serde_json::Value> = std::collections::HashMap::new();
        for p in intel.players.iter().filter(|p| !p.is_ally) {
            let (ad_i, ap_i) = (tagged(p, "Damage"), tagged(p, "SpellDamage"));
            // Items decide once built (an AP Kai'Sa / an AD Kayle); otherwise the champion's ratings.
            let dmg = if ap_i >= ad_i + 2 { "AP" } else if ad_i >= ap_i + 2 { "AD" } else { sd.champion_damage.get(&p.champion).copied().unwrap_or("mixed") };
            match dmg { "AD" => ad += 1, "AP" => ap += 1, _ => mixed += 1 }
            let (a, m) = (tagged(p, "Armor"), tagged(p, "SpellBlock"));
            if a > 0 { armor.push(format!("{} ({a})", p.champion)); }
            if m > 0 { mr.push(format!("{} ({m})", p.champion)); }
            per.insert(p.champion.clone(), serde_json::json!({ "damage": dmg, "armorItems": a, "mrItems": m }));
        }
        if let Some(es) = o.get_mut("enemies").and_then(serde_json::Value::as_array_mut) {
            for e in es {
                let c = e.get("champion").and_then(serde_json::Value::as_str).unwrap_or("").to_string();
                if let (Some(eo), Some(x)) = (e.as_object_mut(), per.get(&c)) {
                    eo.insert("damage".into(), x["damage"].clone());
                }
            }
        }
        o.insert("enemyTeam".into(), serde_json::json!({ "damage": { "AD": ad, "AP": ap, "mixed": mixed }, "withArmorItems": armor, "withMagicResistItems": mr }));
        if let Some(opp) = intel.me.as_ref().filter(|m| !m.role.is_empty()).and_then(|m| intel.players.iter().find(|p| !p.is_ally && p.role == m.role)) {
            o.insert("laneOpponent".into(), serde_json::json!(opp.champion));
        }
    }
    if let (false, Some(o)) = (prev.is_empty(), ctx.as_object_mut()) {
        o.insert("currentPlan".into(), serde_json::json!(prev));
    }
    // Free item slots (6 minus everything but the trinket): with none, a new item replaces one.
    let slots_used = intel.me.as_ref().map_or(0, |m| m.items.iter().filter(|i| sd.items.get(&i.id).is_none_or(|d| !d.tags.iter().any(|t| t == "Trinket"))).count());
    if let Some(mo) = ctx.get_mut("me").and_then(serde_json::Value::as_object_mut) {
        mo.insert("freeSlots".into(), serde_json::json!(6usize.saturating_sub(slots_used)));
    }
    let locked = rc_core::build_advisor::committed(&sd.items, &owned_ids).map(|t| (t.name.clone(), t.id, t.total));
    if let (Some(t), Some(o)) = (rc_core::build_advisor::in_progress(&prev, &sd.items, &owned_ids).map(|t| t.name.clone()).or_else(|| locked.as_ref().map(|l| l.0.clone())), ctx.as_object_mut()) {
        o.insert("finishFirst".into(), serde_json::json!(t));
    }
    g.build_busy = true;
    // Who asked: same game + same champion when the answer comes back, or it is dropped.
    let asked_for = build_session(intel);
    // Build plan: the profile's model (Opus low when Claude is there), not the live-call model.
    let p = rc_core::llm::profiles::profile_for(g.backends).map(|n| rc_core::llm::profiles::profile(n, Task::Build)).unwrap_or_else(|| pick(live_model, Task::Build, g.backends));
    let st = st.clone();
    st.rt.clone().spawn(async move {
        let version = sd.version.clone();
        let icon = move |id: i64| format!("https://ddragon.leagueoflegends.com/cdn/{version}/img/item/{id}.png");
        let r = advise_build(&CliCaller, &ctx, &sd.items, &icon, &p.model, p.effort, &[], rc_core::jsutil::now_ms()).await;
        let mut g = st.game.lock().unwrap();
        g.build_busy = false;
        match r {
            Ok(mut b) => {
                let Some(intel) = g.intel.as_ref() else { return };
                if build_session(intel) != asked_for {
                    log::info!("build: answer for another game / champion dropped");
                    return;
                }
                // Bought while the model was thinking: never recommend what I already have.
                // By id (the model answers in English, the inventory names may be Polish).
                let owned_ids: Vec<i64> = intel.me.as_ref().map(|m| m.items.iter().map(|i| i.id).collect()).unwrap_or_default();
                b.items.retain(|it| !sd.items.values().any(|d| owned_ids.contains(&d.id) && (d.name.eq_ignore_ascii_case(&it.name) || d.name_en.as_deref().is_some_and(|e| e.eq_ignore_ascii_case(&it.name)))));
                // Only basic Boots: the upgraded boots stay in the plan (the model dropped Plated Steelcaps right
                // after the basic Boots were bought, as if boots were done).
                if let Some(me) = intel.me.as_ref() {
                    // Model names are English, the item data may be localised: match either.
                    let same = |d: &rc_core::intel::ddragon::ItemData, name: &str| d.name.eq_ignore_ascii_case(name) || d.name_en.as_deref().is_some_and(|e| e.eq_ignore_ascii_case(name));
                    let is_boots = |name: &str| sd.items.values().any(|d| same(d, name) && d.tags.iter().any(|t| t == "Boots") && d.total > 300.0);
                    let only_basic = me.items.iter().any(|i| i.id == 1001) && !me.items.iter().any(|i| i.id != 1001 && sd.items.get(&i.id).is_some_and(|d| d.tags.iter().any(|t| t == "Boots")));
                    if only_basic && !b.items.iter().any(|it| is_boots(&it.name)) {
                        if let Some(prev_boots) = prev.iter().find(|n| is_boots(n)) {
                            if let Some(d) = sd.items.values().find(|d| same(d, prev_boots)) {
                                b.items.insert(0, rc_core::build_advisor::BuildItem { name: d.name.clone(), icon_url: format!("https://ddragon.leagueoflegends.com/cdn/{}/img/item/{}.png", sd.version, d.id), why: String::new(), cost: Some(d.total) });
                                b.items.truncate(3);
                            }
                        }
                    }
                }
                // Half an item already bought in components (2x Large Rod -> Rabadon's): it stays first (the
                // model switched to Zhonya's with the rods in the inventory).
                if let Some((name, id, total)) = locked.as_ref() {
                    match b.items.iter().position(|it| it.name.eq_ignore_ascii_case(name)) {
                        Some(0) => {}
                        Some(p) => {
                            let it = b.items.remove(p);
                            b.items.insert(0, it);
                        }
                        None => {
                            b.items.insert(0, rc_core::build_advisor::BuildItem { name: name.clone(), icon_url: format!("https://ddragon.leagueoflegends.com/cdn/{}/img/item/{}.png", sd.version, id), why: String::new(), cost: Some(*total) });
                            b.items.truncate(3);
                        }
                    }
                }
                // Full inventory: the plan is a swap. Say what goes (the cheapest finished item, never boots).
                if let Some(me) = intel.me.as_ref() {
                    let used = me.items.iter().filter(|i| sd.items.get(&i.id).is_none_or(|d| !d.tags.iter().any(|t| t == "Trinket"))).count();
                    let has_component = me.items.iter().any(|i| sd.items.get(&i.id).is_some_and(|d| !d.completed && d.total >= 300.0 && !d.tags.iter().any(|t| t == "Trinket" || t == "Consumable")));
                    if used >= 6 && !has_component {
                        let weakest = me.items.iter().filter_map(|i| sd.items.get(&i.id)).filter(|d| d.completed && !d.tags.iter().any(|t| t == "Boots" || t == "Trinket")).min_by(|a, b| a.total.total_cmp(&b.total)).map(|d| d.name.clone());
                        if let Some(w) = weakest {
                            b.phase = format!("pełny eq · zamień {w}");
                        }
                    }
                }
                // An answer with nothing left replaces the old plan too (it may hold bought items).
                log::info!(target: "build", "plan {:?} (prev {:?})", b.items.iter().map(|i| &i.name).collect::<Vec<_>>(), prev);
                g.build = if b.items.is_empty() { None } else { Some(b) };
            }
            Err(e) => log::warn!(target: "build", "error {e}"),
        }
    });
}

// ---------- situational (HUD) ----------

pub fn pos(p: Option<(f64, f64)>) -> Option<Pos> {
    p.map(|(x, y)| Pos { x, y })
}

pub fn seen(units: &[SeenUnit]) -> Vec<situational::SeenUnit> {
    units.iter().map(|u| situational::SeenUnit { champion: u.champion.clone(), x: u.x, y: u.y, vx: Some(u.vx), vy: Some(u.vy) }).collect()
}

/// Fog tracks as the situational calls see them.
pub fn fog_tracks(g: &Game) -> Vec<FogTrack> {
    if g.field.is_stale(crate::vision::now_ms()) {
        return Vec::new();
    }
    g.field
        .tracker
        .as_ref()
        .map(|t| {
            let gt = g.intel.as_ref().map(|i| i.game_time).unwrap_or(0.0);
            let my_role = g.intel.as_ref().and_then(|i| i.me.as_ref()).map(|m| rc_vision::tracker::Role::parse(&m.role));
            t.all()
                .into_iter()
                .map(|tr| FogTrack {
                    near_me: g.field.my_pos.and_then(|p| t.mass_near(&tr.champion, p, 0.12)),
                    gank_ready: rc_vision::tracker::alerts::gank_ready(&tr, my_role, gt),
                    champion: tr.champion,
                    in_fog: tr.state == rc_vision::tracker::TrackState::Fog,
                    estimate: tr.estimate.map(|e| FogEstimate { x: e.x, y: e.y, radius: e.radius }),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// P(I win the lane fight now), from the matchup curve and the live state.
pub fn current_p_me(g: &Game) -> Option<f64> {
    let intel = g.intel.as_ref()?;
    intel.me.as_ref()?;
    let opp = lane_opponent(intel)?;
    Some(power_score(intel, opp, g.matchup.as_ref()).0)
}

/// Who is around me on the minimap, for the power bar's fight estimate (fast, local): allies and
/// visible enemies within ~1/7 of the map, plus enemies whose fresh fog estimate is that close.
/// Lane towers (outer, inner, inhibitor) per lane, ORDER side, minimap fractions (y down). CHAOS
/// towers are the point mirror. Towers die outer first, so "lost N" = the first N are gone.
const TOWERS: [(&str, [(f64, f64); 3]); 3] = [
    ("top", [(0.066, 0.298), (0.102, 0.549), (0.079, 0.712)]),
    ("mid", [(0.393, 0.570), (0.339, 0.676), (0.246, 0.751)]),
    ("bot", [(0.706, 0.931), (0.465, 0.900), (0.288, 0.916)]),
];
const NEXUS_TOWERS: [(f64, f64); 2] = [(0.118, 0.847), (0.146, 0.878)];
/// Tower shot range (~750 units + champion size) as a fraction of the map.
const TOWER_RANGE: f64 = 0.055;

/// Standing towers: (mine, ours) for every tower still up.
pub fn standing_towers(g: &Game) -> Vec<(f64, f64, bool)> {
    let Some(i) = g.intel.as_ref() else { return Vec::new() };
    let my_chaos = i.me.as_ref().is_some_and(|m| m.team == "CHAOS");
    let mut out = Vec::new();
    for (lane, pos) in TOWERS {
        let st = i.lanes.iter().find(|l| l.lane == lane);
        for order_side in [true, false] {
            let ours = order_side != my_chaos;
            let lost = st.map_or(0, |s| if ours { s.ours_lost } else { s.theirs_lost }) as usize;
            for &(x, y) in pos.iter().skip(lost) {
                // Red towers: top / bot = reflection (y, x) of the same blue lane, mid = 180° turn.
                out.push(if order_side { (x, y, ours) } else if lane == "mid" { (1.0 - x, 1.0 - y, ours) } else { (y, x, ours) });
            }
        }
    }
    for &(x, y) in &NEXUS_TOWERS {
        out.push((x, y, !my_chaos));
        out.push((1.0 - x, 1.0 - y, my_chaos));
    }
    out
}

pub fn nearby(g: &Game) -> rc_core::intel::matchup::Nearby {
    // ~1/7 of the map: a whole dragon / baron pit fight with the bushes around it.
    const R: f64 = 0.15;
    let mut n = rc_core::intel::matchup::Nearby::default();
    // Health bar on my screen in the last 1.5 s = right next to me, whatever the minimap shows (portraits
    // hide under mine when we run together).
    // ...but only when the screen is around me: camera locked on me, or my own bar seen on it (a panned
    // camera looking at a far fight must not fill my fight bar).
    let screen_is_mine = g.field.cam_locked_at >= g.field.cam_unlocked_at || g.field.pos_src == "screen";
    if let Some(i) = g.intel.as_ref().filter(|_| screen_is_mine) {
        for p in i.players.iter().filter(|p| !p.is_me && !p.is_dead && p.onscreen_at.is_some_and(|t| (-1.0..=4.0).contains(&(i.game_time - t)))) {
            if p.is_ally {
                n.allies.push(p.champion.clone());
            } else {
                n.enemies.push((p.champion.clone(), true));
            }
        }
    }
    // Enemy health bars on screen right now that got no name: someone is there all the same.
    if let Some(i) = g.intel.as_ref() {
        let (bars, at) = g.field.enemy_bars;
        if i.game_time - at <= 1.5 {
            let named = i.players.iter().filter(|p| !p.is_ally && !p.is_dead && p.onscreen_at.is_some_and(|t| (-1.0..=4.0).contains(&(i.game_time - t)))).count();
            n.unknown_enemies = bars.saturating_sub(named);
        }
    }
    let Some((mx, my)) = g.field.my_pos else { return n };
    let me = g.intel.as_ref().and_then(|i| i.me.as_ref()).map(|m| m.champion.clone()).unwrap_or_default();
    for a in &g.field.allies {
        if a.champion != me && (a.x - mx).hypot(a.y - my) < R && !n.allies.contains(&a.champion) {
            n.allies.push(a.champion.clone());
        }
    }
    for e in &g.field.visible_enemies {
        if (e.x - mx).hypot(e.y - my) < R && !n.enemies.iter().any(|(c, _)| *c == e.champion) {
            n.enemies.push((e.champion.clone(), true));
        }
    }
    // Heading to me from further away (velocity, per minimap frame, points at me): gank / collapse incoming.
    let coming = |u: &crate::vision::SeenUnit| {
        let (dx, dy) = (mx - u.x, my - u.y);
        let d = dx.hypot(dy);
        let v = u.vx.hypot(u.vy);
        (R..0.3).contains(&d) && v > 0.001 && (u.vx * dx + u.vy * dy) / (d * v) > 0.7
    };
    for a in g.field.allies.iter().filter(|a| a.champion != me && coming(a)) {
        n.allies_coming.push(a.champion.clone());
    }
    for e in g.field.visible_enemies.iter().filter(|e| coming(e)) {
        n.enemies_coming.push(e.champion.clone());
    }
    let towers = standing_towers(g);
    let in_range = |x: f64, y: f64, ours: bool| towers.iter().any(|t| t.2 == ours && (t.0 - x).hypot(t.1 - y) < TOWER_RANGE);
    n.me_under_enemy_tower = in_range(mx, my, false);
    n.me_under_my_tower = in_range(mx, my, true);
    n.enemies_under_tower = g.field.visible_enemies.iter().filter(|e| (e.x - mx).hypot(e.y - my) < R && in_range(e.x, e.y, false)).map(|e| e.champion.clone()).collect();
    // Near me a moment ago (icon flickered off the minimap): still in the fight.
    if let Some(i) = g.intel.as_ref() {
        for (c, _) in g.field.near_seen.iter() {
            let alive = i.players.iter().any(|p| p.champion == *c && !p.is_ally && !p.is_dead);
            if alive && !n.enemies.iter().any(|(e, _)| e == c) {
                n.enemies.push((c.clone(), true));
            }
        }
    }
    // A live gank / invade warning about X: X counts as next to me (in fog) for the fight bar too.
    let now = crate::vision::now_ms();
    if let Some(i) = g.intel.as_ref() {
        for a in g.field.alerts.iter().filter(|a| (a.kind == "gank" || a.kind == "invade") && a.expires_at > now) {
            let Some(c) = i.players.iter().find(|p| !p.is_ally && !p.is_dead && a.text.starts_with(p.champion.as_str())).map(|p| p.champion.clone()) else { continue };
            if !n.enemies.iter().any(|(e, _)| *e == c) {
                n.enemies.push((c, false));
            }
        }
    }
    for t in fog_tracks(g) {
        let Some(est) = t.estimate.filter(|_| t.in_fog) else { continue };
        if est.radius < 0.2 && (est.x - mx).hypot(est.y - my) < R + est.radius * 0.5 && !n.enemies.iter().any(|(c, _)| *c == t.champion) {
            n.enemies.push((t.champion, false));
        }
    }
    n
}
