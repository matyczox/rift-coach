//! Live coach on ward / roam situations (real CLIs, uses quota).
//! `cargo run --release -p rc-core --example bench_coach_wards`
use rc_core::coach::{schema, SYSTEM};
use rc_core::llm::{call_model, Effort, ModelCall};
use rc_core::wards::WARD_SPOTS;
use serde_json::{json, Value};

fn snap(name: &str) -> Value {
    let me = json!({ "champion": "Ahri", "role": "MIDDLE", "level": 7, "hp": 0.8, "kda": "2/0/1", "flash": true });
    let enemies = json!([{ "champion": "Xerath", "role": "MIDDLE", "level": 6, "kda": "0/2/0" }, { "champion": "Zaahen", "role": "JUNGLE", "level": 6, "kda": "1/1/0" }, { "champion": "Yunara", "role": "BOTTOM", "level": 6, "kda": "1/0/0" }, { "champion": "Sona", "role": "UTILITY", "level": 5, "kda": "0/1/1" }, { "champion": "Darius", "role": "TOP", "level": 7, "kda": "1/1/0" }]);
    let wards = json!({ "trinket": "Stealth Ward", "trinketReady": true, "controlWard": false, "lastWardPlacedSecAgo": 150 });
    let habits = json!({ "Zaahen": "gankuje: bot 2×, mid 1×; najczęściej strona bot" });
    match name {
        // Walking from mid down the river toward bot; Yunara low on bot.
        "roam-bot" => json!({ "reason": "periodic", "time": "8:40", "me": me, "enemies": enemies, "goldDiff": 600,
            "waves": { "mid": { "state": "crashing", "front": 0.66 }, "bot": { "state": "our_side", "front": 0.42 } },
            "field": { "me": { "x": 0.58, "y": 0.62, "area": "rzeka bot" }, "allies": [{ "champ": "Miss Fortune", "x": 0.80, "y": 0.90, "area": "linia bot" }, { "champ": "Sona", "x": 0.79, "y": 0.92, "area": "linia bot" }],
              "visibleEnemies": [{ "champ": "Yunara", "x": 0.86, "y": 0.88, "area": "linia bot" }, { "champ": "Xerath", "x": 0.62, "y": 0.38, "area": "linia mid" }],
              "roam": { "from": "mid", "at": "rzeka bot", "headingTo": "linia bot", "towardLane": "bot", "offLaneSec": 6 },
              "rivalBackOnLaneInSec": [], "enemyHabits": habits, "myWards": wards, "fogZones": [{ "champ": "Zaahen", "best": [0.40, 0.25, 0.12], "unseenSec": 20 }] } }),
        // Wave crashed into their tower, ahead, Xerath recalled: on lane.
        "ahead-pushed" => json!({ "reason": "periodic", "time": "9:30", "me": me, "enemies": enemies, "goldDiff": 1100,
            "waves": { "mid": { "state": "crashing", "front": 0.68 } },
            "field": { "me": { "x": 0.60, "y": 0.40, "area": "linia mid" }, "allies": [], "visibleEnemies": [],
              "rivalBackOnLaneInSec": [{ "champ": "Xerath", "sec": 25 }], "enemyHabits": habits, "myWards": wards, "fogZones": [{ "champ": "Zaahen", "best": [0.80, 0.60, 0.10], "unseenSec": 12 }] },
            "alreadyWarded": ["mid-river-bot"] }),
        // Losing mid, jungler unknown.
        _ => json!({ "reason": "periodic", "time": "7:10", "me": { "champion": "Ahri", "role": "MIDDLE", "level": 6, "hp": 0.55, "kda": "0/2/0", "flash": false }, "enemies": enemies, "goldDiff": -900,
            "waves": { "mid": { "state": "our_side", "front": 0.45 } },
            "field": { "me": { "x": 0.44, "y": 0.58, "area": "linia mid" }, "allies": [], "visibleEnemies": [{ "champ": "Xerath", "x": 0.52, "y": 0.49, "area": "linia mid" }],
              "rivalBackOnLaneInSec": [], "enemyHabits": habits, "myWards": wards, "fogZones": [{ "champ": "Zaahen", "best": [0.50, 0.42, 0.15], "unseenSec": 35 }] } }),
    }
}

#[tokio::main]
async fn main() {
    let system = format!(
        "{SYSTEM}\n\nStrona gracza: niebieska (lewy dolny róg).\n\nMiejsca na wardy (id: opis, wartość wizji):\n{}",
        WARD_SPOTS.iter().map(|s| format!("{}: {}, {}/5", s.id, s.label, s.value)).collect::<Vec<_>>().join("\n")
    );
    for game in ["roam-bot", "ahead-pushed", "losing"] {
        for (model, effort) in [("gpt-6.1-terra", Effort::High), ("claude-sonnet-5-5", Effort::Medium)] {
            let call = ModelCall { model: model.into(), effort: Some(effort), system: system.clone(), schema: schema(), prompt: snap(game).to_string(), timeout_ms: Some(180_000), ..Default::default() };
            match call_model::<Value>(&call).await {
                Ok(r) => println!("{game} | {model} {:.1} s: call={} ward={}", r.latency_ms as f64 / 1000.0, r.value["call"], r.value["ward"]),
                Err(e) => println!("{game} | {model}: ERROR {e}"),
            }
        }
    }
}
