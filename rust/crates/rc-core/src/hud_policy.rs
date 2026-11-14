//! Presentation rules shared by the HUD and regression tests.
use serde_json::{json, Value};

/// Recheck model answers against live objective state, including while their TTL runs.
pub fn block_unavailable_objective(advice: &mut Value, objectives: &[crate::intel::game_intel::ObjectiveTimer], game_time: f64) -> bool {
    let call = advice["call"].as_str().unwrap_or("").to_lowercase();
    let target = advice.pointer("/mapHint/target").and_then(Value::as_str).unwrap_or("").to_lowercase();
    let objective_hint = advice.pointer("/mapHint/type").and_then(Value::as_str) == Some("objective");
    let negative = call.split_whitespace().any(|w| matches!(w,"nie"|"oddaj"|"odpuść"|"wycofaj"|"uciekaj"|"wróć"));
    let mut blocked = false;
    for (kind, aliases) in [("baron", &["baron"][..]), ("dragon", &["dragon","smok","elder"][..]), ("herald", &["herald"][..]), ("grubs", &["grubs","voidgrub"][..])] {
        let hinted = objective_hint && aliases.iter().any(|a| target.contains(a));
        let named = aliases.iter().any(|a| call.contains(a));
        if !hinted && !named { continue; }
        let ready = objectives.iter().find(|o| o.kind==kind).is_some_and(|o| o.alive || o.spawns_at.is_some_and(|at| at-game_time<=90.0));
        if !ready {
            if hinted { advice["mapHint"] = Value::Null; }
            if !negative { advice["call"] = Value::Null; blocked = true; }
        }
    }
    blocked
}

/// Danger takes effect immediately; optimistic changes need sustained evidence.
pub fn stabilize_power(current: &mut Value, previous: Option<(&Value, f64)>) {
    let Some(raw) = current["pMe"].as_f64() else { return };
    if let Some((old, elapsed)) = previous {
        if let Some(p) = old["pMe"].as_f64() {
            let ceiling = if old["enemy"]["champion"] == current["enemy"]["champion"] {
                p + 0.15 * elapsed.clamp(0.0, 1.0)
            } else { 0.6 };
            current["pMe"] = json!(raw.min(ceiling));
        }
    }
    let p = current["pMe"].as_f64().unwrap_or(raw);
    let text = current["advice"].as_str().unwrap_or("").to_lowercase();
    if positive(&text) && (p < 0.6 || current["local"] == false) {
        current["advice"] = json!(if current["local"] == false { "Ocena 1v1 — brak potwierdzonej walki obok" } else { "Ocena zmienia się — zachowaj ostrożność" });
    }
}

fn positive(s: &str) -> bool {
    ["wchodź", "all-in", "trade teraz", "trade'uj"].iter().any(|w| s.contains(w))
}

/// Resolve opposing actionable messages after all HUD holds and smoothing.
pub fn reconcile(state: &mut Value) {
    let call = state["topBar"]["call"].as_str().unwrap_or("").to_string();
    let lower = call.to_lowercase();
    let advice = state["power"]["advice"].as_str().unwrap_or("").to_lowercase();
    let retreat = ["cofnij", "wycofaj", "nie wchodź", "nie dive", "back", "odpuść", "oddaj"].iter().any(|w| lower.contains(w));
    if retreat && positive(&advice) {
        state["power"]["advice"] = json!(call);
    }
    let engage = !retreat && ["wejdź", "wchodź", "all-in", "atakuj", "zabij"].iter().any(|w| lower.contains(w));
    let unsafe_now = state["power"]["pMe"].as_f64().is_some_and(|p| p <= 0.4)
        || advice.contains("nie nurkuj") || advice.contains("wycofaj")
        || state["danger"]["level"].as_f64().is_some_and(|p| p >= 0.7);
    if engage && unsafe_now { state["topBar"] = Value::Null; }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn baron_killed_while_answer_is_in_flight_or_visible_invalidates_call() {
        use crate::intel::game_intel::ObjectiveTimer;
        let mut objectives=vec![ObjectiveTimer{kind:"baron",label:"Baron".into(),spawns_at:None,alive:true}];
        let a=json!({"call":"Przygotuj Baron z Caitlyn","mapHint":{"type":"objective","target":"baron"},"item":"Kolczasta Kolczuga"});
        let mut shown=a.clone();
        assert!(!block_unavailable_objective(&mut shown,&objectives,1650.0));
        objectives[0].alive=false;objectives[0].spawns_at=Some(2010.0);
        assert!(block_unavailable_objective(&mut shown,&objectives,1653.0));
        assert!(shown["call"].is_null() && shown["mapHint"].is_null());
        assert_eq!(shown["item"],a["item"]);
        let mut near=a.clone();
        assert!(!block_unavailable_objective(&mut near,&objectives,1920.0));
        let mut retreat=json!({"call":"Nie idź na Barona","mapHint":{"type":"objective","target":"baron"}});
        assert!(!block_unavailable_objective(&mut retreat,&objectives,1653.0));
        assert_eq!(retreat["call"],"Nie idź na Barona");
        assert!(retreat["mapHint"].is_null());
    }
    #[test]
    fn one_frame_low_hp_does_not_flip_to_all_in() {
        let old = json!({"pMe":0.23,"enemy":{"champion":"KaiSa"}});
        let mut now = json!({"pMe":0.92,"enemy":{"champion":"KaiSa"},"advice":"KaiSa 4% HP → wchodź","local":true});
        stabilize_power(&mut now, Some((&old,0.1)));
        assert!(now["pMe"].as_f64().unwrap() < 0.25);
        assert!(!now["advice"].as_str().unwrap().contains("wchodź"));
        now["pMe"] = json!(0.05);
        stabilize_power(&mut now, Some((&old,0.1)));
        assert_eq!(now["pMe"],0.05);
    }
    #[test]
    fn dive_and_retreat_conflicts_from_game_log() {
        let mut s = json!({"topBar":{"call":"Wejdź z Caitlyn na Sonę"},"power":{"pMe":0.47,"advice":"Sona pod wieżą → nie nurkuj"}});
        reconcile(&mut s);
        assert!(s["topBar"].is_null());
        s = json!({"topBar":{"call":"Wycofaj się do Amumu"},"power":{"pMe":0.83,"advice":"Przewaga tutaj → wchodź"}});
        reconcile(&mut s);
        assert_eq!(s["power"]["advice"],"Wycofaj się do Amumu");
    }
}
