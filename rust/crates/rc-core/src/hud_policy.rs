//! Presentation rules shared by the HUD and regression tests.
use serde_json::{json, Value};

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
