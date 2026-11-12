//! Game intel (System 1 derived state) and Data Dragon static data.
pub mod ddragon;
pub mod game_intel;

/// JS `Math.round` (halves towards +infinity).
pub fn js_round(v: f64) -> f64 {
    (v + 0.5).floor()
}
