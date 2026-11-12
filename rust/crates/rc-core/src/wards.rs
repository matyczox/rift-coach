//! Ward spots (`src/shared/wards.ts`): blue-side minimap coordinates, value 1–5,
//! Polish label. Same ids and order (the coach schema enum and prompt use them).

pub struct WardSpot {
    pub id: &'static str,
    pub x: f64,
    pub y: f64,
    pub value: u8,
    /// Neutral spots (river/pits/lanes) are not mirrored for red side.
    pub neutral: bool,
    pub label: &'static str,
}

const fn w(id: &'static str, x: f64, y: f64, value: u8, neutral: bool, label: &'static str) -> WardSpot {
    WardSpot { id, x, y, value, neutral, label }
}

pub const WARD_SPOTS: &[WardSpot] = &[
    w("faelight-river-top", 0.24, 0.30, 5, true, "Faelight (wyspa w rzece, top)"),
    w("faelight-river-bot", 0.76, 0.70, 5, true, "Faelight (wyspa w rzece, bot)"),
    w("faelight-my-base", 0.22, 0.78, 3, false, "Faelight przy naszej bazie"),
    w("faelight-enemy-base", 0.78, 0.22, 4, false, "Faelight przy bazie wroga"),
    w("faelight-mid-top", 0.44, 0.40, 4, true, "Faelight (środek, top)"),
    w("faelight-mid-bot", 0.56, 0.60, 4, true, "Faelight (środek, bot)"),
    w("dragon-pit-entrance", 0.62, 0.66, 4, true, "wejście do smoka"),
    w("river-bush-bot", 0.70, 0.60, 5, true, "krzak w rzece (bot)"),
    w("baron-pit-entrance", 0.38, 0.34, 4, true, "wejście do barona"),
    w("river-bush-top", 0.30, 0.40, 5, true, "krzak w rzece (top)"),
    w("mid-river-bot", 0.58, 0.56, 3, true, "rzeka przy midzie (bot)"),
    w("mid-river-top", 0.42, 0.44, 3, true, "rzeka przy midzie (top)"),
    w("my-tri-bush-bot", 0.60, 0.86, 3, false, "tri-bush bot (nasz)"),
    w("my-tri-bush-top", 0.14, 0.40, 3, false, "tri-bush top (nasz)"),
    w("my-raptors", 0.47, 0.60, 3, false, "raptory (nasze)"),
    w("my-blue-buff", 0.25, 0.47, 2, false, "niebieski buff (nasz)"),
    w("my-red-buff", 0.52, 0.74, 2, false, "czerwony buff (nasz)"),
    w("enemy-tri-bush-bot", 0.86, 0.60, 4, false, "tri-bush bot (wroga)"),
    w("enemy-tri-bush-top", 0.40, 0.14, 4, false, "tri-bush top (wroga)"),
    w("enemy-raptors", 0.53, 0.40, 4, false, "raptory (wroga)"),
    w("enemy-blue-buff", 0.75, 0.53, 3, false, "niebieski buff (wroga)"),
    w("enemy-red-buff", 0.48, 0.26, 3, false, "czerwony buff (wroga)"),
    w("top-lane-bush", 0.12, 0.22, 2, true, "krzak na topie"),
    w("bot-lane-bush", 0.78, 0.88, 2, true, "krzak na bocie"),
    w("plant-dragon", 0.66, 0.62, 5, true, "roślina wizji przy smoku"),
    w("plant-baron", 0.34, 0.38, 5, true, "roślina wizji przy baronie"),
    w("plant-my-jungle-bot", 0.56, 0.70, 3, false, "roślina wizji (nasza dżungla, bot)"),
    w("plant-my-jungle-top", 0.30, 0.52, 3, false, "roślina wizji (nasza dżungla, top)"),
];

pub fn ward_point(spot: &str, red_side: bool) -> Option<(f64, f64)> {
    let p = WARD_SPOTS.iter().find(|s| s.id == spot)?;
    Some(if red_side && !p.neutral { (1.0 - p.x, 1.0 - p.y) } else { (p.x, p.y) })
}

#[cfg(test)]
mod tests {
    #[test]
    fn mirror() {
        assert_eq!(super::ward_point("my-red-buff", true), Some((0.48, 0.26)));
        assert_eq!(super::ward_point("plant-dragon", true), Some((0.66, 0.62)));
        assert_eq!(super::WARD_SPOTS.len(), 28);
    }
}
