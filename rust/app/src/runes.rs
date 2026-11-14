//! Automatic runes in champion select (onChampSelect / setRunes in index.ts).

use crate::state::AppState;
use rc_core::intel::ddragon::patch_of;
use rc_core::llm::profiles::{pick, Task};
use rc_core::runes::{advise_runes, apply_rune_page, assign_roles, lane_opponent_for, load_rune_trees, role_rates, ApplyResult};
use rc_core::types::RunesStatus;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default)]
pub struct RunesState {
    key: String,
    timer: Option<tokio::task::JoinHandle<()>>,
    base_key: String,
    base_timer: Option<tokio::task::JoinHandle<()>>,
    refined_applied: bool,
    apply_lock: Arc<tokio::sync::Mutex<()>>,
    /// Map of the current champion select (11 = Summoner's Rift), from the gameflow session.
    map_id: Option<i64>,
}

fn role_name(pos: &str) -> Option<&'static str> {
    Some(match pos {
        "top" => "TOP",
        "jungle" => "JUNGLE",
        "middle" => "MIDDLE",
        "bottom" => "BOTTOM",
        "utility" => "UTILITY",
        _ => return None,
    })
}

pub fn on_phase(rt: &tokio::runtime::Handle, st: &Arc<AppState>, rs: &Arc<Mutex<RunesState>>, phase: &str) {
    if phase != "ChampSelect" {
        let mut r = rs.lock().unwrap();
        r.key.clear();
        r.base_key.clear();
        if let Some(t) = r.base_timer.take() { t.abort(); }
        if let Some(t) = r.timer.take() { t.abort(); }
        return;
    }
    // Runes only on Summoner's Rift: remember this lobby's map (gameflow session).
    rs.lock().unwrap().map_id = None;
    let (st, rs) = (st.clone(), rs.clone());
    rt.spawn(async move {
        if let Ok(g) = st.lcu_client.get::<Value>("/lol-gameflow/v1/session").await {
            let map = g.pointer("/map/id").and_then(Value::as_i64).or_else(|| g.pointer("/gameData/queue/mapId").and_then(Value::as_i64));
            rs.lock().unwrap().map_id = map;
        }
    });
}

pub fn on_champ_select(rt: &tokio::runtime::Handle, st: &Arc<AppState>, rs: &Arc<Mutex<RunesState>>, session: &Value) {
    if !st.settings.lock().unwrap().value().auto_runes || rs.lock().unwrap().map_id != Some(11) {
        return;
    }
    let (sd, coach_state) = {
        let g = st.game.lock().unwrap();
        (g.static_data.clone(), g.coach.status.state.clone())
    };
    let Some(sd) = sd else { return };
    if coach_state == "not-logged-in" {
        return;
    }
    let local = session.get("localPlayerCellId").and_then(Value::as_i64);
    let my_team = session.get("myTeam").and_then(Value::as_array).cloned().unwrap_or_default();
    let Some(me) = my_team.iter().find(|p| p.get("cellId").and_then(Value::as_i64) == local && local.is_some()) else { return };
    let me_cell = me.get("cellId").and_then(Value::as_i64);
    let champ_id = me.get("championId").and_then(Value::as_i64).unwrap_or(0);
    if champ_id == 0 {
        return;
    }
    let locked = session
        .get("actions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_array)
        .flatten()
        .any(|a| a.get("actorCellId").and_then(Value::as_i64) == me_cell && a.get("type").and_then(Value::as_str) == Some("pick") && a.get("completed").and_then(Value::as_bool) == Some(true));
    if !locked {
        return;
    }
    let Some(champion) = sd.champion_by_key.get(&champ_id).cloned() else { return };
    // Empty = not assigned (Swiftplay / blind): resolved in set_runes. It was "ARAM/BRAK ROLI", and the
    // model then looked up ARAM pages for Summoner's Rift games.
    let role = role_name(me.get("assignedPosition").and_then(Value::as_str).unwrap_or("")).unwrap_or("").to_string();
    let by_key = |p: &Value| p.get("championId").and_then(Value::as_i64).and_then(|k| sd.champion_by_key.get(&k).cloned());
    let enemies: Vec<String> = session.get("theirTeam").and_then(Value::as_array).into_iter().flatten().filter_map(by_key).collect();
    let mut sorted = enemies.clone();
    sorted.sort();
    let key = format!("{champion}|{role}|{}", sorted.join(","));
    let mut r = rs.lock().unwrap();
    let base_key = format!("{champion}|{role}");
    if r.base_key != base_key {
        if let Some(t) = r.base_timer.take() { t.abort(); }
        r.base_key = base_key.clone();
        r.refined_applied = false;
        let (st, rs2, version, champ, role) = (st.clone(), rs.clone(), sd.version.clone(), champion.clone(), role.clone());
        r.base_timer = Some(rt.spawn(async move {
            set_base_runes(&st, &rs2, &base_key, &champ, &role, &version).await;
        }));
    }
    if key == r.key {
        return;
    }
    r.key = key.clone();
    // Debounce: enemies often lock in quick succession.
    if let Some(t) = r.timer.take() {
        t.abort();
    }
    if enemies.is_empty() {
        return; // The base request already handles this case.
    }
    let (st, rs2, version) = (st.clone(), rs.clone(), sd.version.clone());
    r.timer = Some(rt.spawn(async move {
        tokio::time::sleep(Duration::from_millis(2500)).await;
        set_runes(&st, &rs2, &key, &champion, &role, &enemies, &version).await;
    }));
}

/// Prepare a basic page immediately, independently of changing enemy picks.
async fn set_base_runes(st: &Arc<AppState>, rs: &Arc<Mutex<RunesState>>, key: &str, champion: &str, role: &str, version: &str) {
    let result: anyhow::Result<()> = async {
        let dir = st.data_dir.join("runes");
        let trees = load_rune_trees(&st.data_dir.join("ddragon"), version).await?;
        let draft = {
            let cfg = st.settings.lock().unwrap().value().coach.draft_model.clone();
            pick(&cfg, Task::Draft, st.game.lock().unwrap().backends)
        };
        let patch = patch_of(version);
        let resolved;
        let role = if role.is_empty() {
            resolved = resolve_role(st, &dir, &patch, champion, &draft.model, draft.effort).await;
            resolved.as_str()
        } else { role };
        let plan = advise_runes(&dir, &trees, &patch, champion, role, &[], None, &draft.model, draft.effort).await?;
        let apply_lock = rs.lock().unwrap().apply_lock.clone();
        let _guard = apply_lock.lock().await;
        { let r = rs.lock().unwrap(); if r.base_key != key || r.refined_applied { return Ok(()); } }
        let result = apply_rune_page(&st.lcu_client, &plan).await?;
        if result == ApplyResult::NoSlot {
            st.ui.lock().unwrap().runes = RunesStatus { state: "no-slot".into(), label: Some(plan.label), error: Some("Brak wolnej strony run — usuń jedną albo nazwij ją „Rift Coach”.".into()) };
        } else {
            st.ui.lock().unwrap().runes = RunesStatus { state: "set".into(), label: Some(plan.label), error: None };
        }
        Ok(())
    }.await;
    if let Err(e) = result { log::warn!("base runes: {e}"); }
}

/// No assigned position: the role picked in the lobby (Swiftplay / blind: first position preference),
/// else the champion's most played role on this patch.
async fn resolve_role(st: &Arc<AppState>, dir: &std::path::Path, patch: &str, champion: &str, model: &str, effort: rc_core::llm::Effort) -> String {
    if let Ok(l) = st.lcu_client.get::<Value>("/lol-lobby/v2/lobby").await {
        let pref = l.pointer("/localMember/firstPositionPreference").and_then(Value::as_str).unwrap_or("").to_lowercase();
        if let Some(r) = role_name(&pref) {
            return r.into();
        }
    }
    if let Ok(rates) = role_rates(dir, patch, &[champion.to_string()], model, effort).await {
        if let Some((r, _)) = rates.get(champion).and_then(|m| m.iter().max_by(|a, b| a.1.total_cmp(b.1))) {
            return r.clone();
        }
    }
    "MIDDLE".into()
}

async fn set_runes(st: &Arc<AppState>, rs: &Arc<Mutex<RunesState>>, key: &str, champion: &str, role: &str, enemies: &[String], version: &str) {
    st.ui.lock().unwrap().runes = RunesStatus { state: "working".into(), label: Some(format!("{champion} {}", role.to_lowercase())), error: None };
    let res: anyhow::Result<()> = async {
        let dir = st.data_dir.join("runes");
        let trees = load_rune_trees(&st.data_dir.join("ddragon"), version).await?;
        let draft = {
            let cfg = st.settings.lock().unwrap().value().coach.draft_model.clone();
            pick(&cfg, Task::Draft, st.game.lock().unwrap().backends)
        };
        let patch = patch_of(version);
        let resolved;
        let role = if role.is_empty() {
            resolved = resolve_role(st, &dir, &patch, champion, &draft.model, draft.effort).await;
            st.ui.lock().unwrap().runes.label = Some(format!("{champion} {}", resolved.to_lowercase()));
            resolved.as_str()
        } else {
            role
        };
        // Who is my lane opponent? Joint role assignment of enemy picks on this patch's play rates.
        let rates = if enemies.is_empty() { Default::default() } else { role_rates(&dir, &patch, enemies, &draft.model, draft.effort).await? };
        let opponent = lane_opponent_for(role, &assign_roles(enemies, &rates)).filter(|o| o.1 >= 0.6);
        let plan = advise_runes(&dir, &trees, &patch, champion, role, enemies, opponent, &draft.model, draft.effort).await?;
        let apply_lock = rs.lock().unwrap().apply_lock.clone();
        let _guard = apply_lock.lock().await;
        if rs.lock().unwrap().key != key {
            return Ok(()); // draft moved on; a newer request is coming
        }
        let r = apply_rune_page(&st.lcu_client, &plan).await?;
        if r != ApplyResult::NoSlot { rs.lock().unwrap().refined_applied = true; }
        st.ui.lock().unwrap().runes = if r == ApplyResult::NoSlot {
            RunesStatus { state: "no-slot".into(), label: Some(plan.label.clone()), error: Some("Brak wolnej strony run — usuń jedną albo nazwij ją „Rift Coach”.".into()) }
        } else {
            RunesStatus { state: "set".into(), label: Some(plan.label.clone()), error: None }
        };
        Ok(())
    }
    .await;
    if let Err(e) = res {
        let msg: String = e.to_string().chars().take(160).collect();
        st.ui.lock().unwrap().runes = RunesStatus { state: "error".into(), label: None, error: Some(msg) };
        rs.lock().unwrap().key.clear(); // allow a retry on the next session update
    }
}
