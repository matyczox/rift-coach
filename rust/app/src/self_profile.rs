//! Player profile for the coach (`refreshSelfProfile` in index.ts): habits from the player's
//! recent Summoner's Rift games (Riot API) + lessons from the post-game reviews. Rebuilt every
//! 6 h and 4 min after each game; the lessons are re-read from the reviews on every refresh.

use crate::state::AppState;
use rc_core::riot::self_profile::{build_for_riot_id, load_self_profile, needs_rebuild, save_self_profile, lessons_from, FILE_NAME};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

fn file(st: &AppState) -> PathBuf {
    st.data_dir.join(FILE_NAME)
}

/// Riot ID of the logged-in player from the LCU current summoner. The LCU puuid is not the
/// one the (dev-key) Riot API expects, so the profile is built from the Riot ID instead.
fn my_riot_id(st: &AppState) -> Option<String> {
    let l = st.lcu.lock().unwrap();
    let me = l.me.as_ref()?;
    (!me.game_name.is_empty() && !me.tag_line.is_empty()).then(|| format!("{}#{}", me.game_name, me.tag_line))
}

pub async fn refresh(st: Arc<AppState>, force: bool) {
    let reviews_dir = st.reviews_dir();
    let lessons = tokio::task::spawn_blocking(move || lessons_from(&rc_core::files::list_reviews(&reviews_dir))).await.unwrap_or_default();
    let now = rc_core::jsutil::now_ms();
    let riot=st.settings.lock().unwrap().value().riot.clone();
    let key_set = !riot.api_key.is_empty();
    let riot_id = my_riot_id(&st);
    let need_load = st.ui.lock().unwrap().player_profile.is_none();
    let loaded = if need_load { load_self_profile(&file(&st)) } else { None };
    let start = {
        let mut ui = st.ui.lock().unwrap();
        if ui.player_profile.is_none() {
            ui.player_profile = loaded;
        }
        if let Some(p) = ui.player_profile.as_mut() {
            p.lessons = lessons;
        }
        // --smoke never spends Riot API requests.
        let go = !crate::smoke() && !ui.self_profile_busy && key_set && riot_id.is_some() && needs_rebuild(ui.player_profile.as_ref(), now, force);
        if go {
            ui.self_profile_busy = true;
        }
        go
    };
    let (true, Some(id)) = (start, riot_id) else { return };
    let result = match crate::knowledge::riot_client(&st) {
        Some(client) => {
            let reviews_dir = st.reviews_dir();
            let reviews = tokio::task::spawn_blocking(move || rc_core::files::list_reviews(&reviews_dir)).await.unwrap_or_default();
            build_for_riot_id(&client, &id, &reviews, now).await.map_err(|e| e.to_string())
        }
        None => Err("no Riot client".into()),
    };
    let stale=st.settings.lock().unwrap().value().riot!=riot;
    let mut ui = st.ui.lock().unwrap();
    ui.self_profile_busy = false;
    if stale {return;}
    match result {
        Ok(Some(p)) => {
            if let Err(e) = save_self_profile(&file(&st), &p) {
                log::warn!("player profile save: {e}");
            }
            log::info!("player profile: {} games, {} weak spots, {} lessons", p.games, p.weaknesses.len(), p.lessons.len());
            ui.player_profile = Some(p);
        }
        // RiotError's text never contains the key.
        Ok(None) => log::warn!("player profile failed: account not found"),
        Err(e) => log::warn!("player profile failed: {e}"),
    }
}

/// A newly pasted key should take effect now, including after an old request fails.
pub fn settings_changed(st: &Arc<AppState>) {
    if crate::smoke() {return;}
    let wanted=st.settings.lock().unwrap().value().riot.clone();
    if wanted.api_key.is_empty() {return;}
    let st=st.clone();
    st.rt.clone().spawn(async move {
        for _ in 0..120 {
            if st.settings.lock().unwrap().value().riot!=wanted {return;}
            if !st.ui.lock().unwrap().self_profile_busy {
                refresh(st.clone(),true).await;
                return;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
}

/// Background loop: load the saved profile at once, then check every 2 min (the LCU refreshes
/// the current summoner about as often); a rebuild happens only when the 6 h window passed.
pub fn spawn(st: &Arc<AppState>) {
    let st = st.clone();
    st.rt.clone().spawn(async move {
        tokio::time::sleep(Duration::from_secs(2)).await;
        loop {
            refresh(st.clone(), false).await;
            tokio::time::sleep(Duration::from_secs(120)).await;
        }
    });
}

/// New game in the history + a new review: rebuild once Riot has the match.
pub fn after_game(st: &Arc<AppState>) {
    let st = st.clone();
    st.rt.clone().spawn(async move {
        tokio::time::sleep(Duration::from_secs(4 * 60)).await;
        refresh(st, true).await;
    });
}

/// `--smoke`: counts only (no names, champions or stats).
pub fn smoke_summary(st: &AppState) -> serde_json::Value {
    let ui = st.ui.lock().unwrap();
    match ui.player_profile.as_ref() {
        Some(p) => serde_json::json!({ "games": p.games, "weaknesses": p.weaknesses.len(), "lessons": p.lessons.len(), "coachText": rc_core::riot::self_profile::self_profile_for_coach(Some(p)).map(|t| t.len()) }),
        None => serde_json::Value::Null,
    }
}
