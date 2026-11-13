//! Same build question to several models (real CLIs, uses the user's quota): which plans better?
//! `cargo run --release -p rc-core --example bench_build`
use rc_core::build_advisor::{schema, SYSTEM};
use rc_core::llm::{call_model, Effort, ModelCall};
use serde_json::{json, Value};

#[tokio::main]
async fn main() {
    // Ahri mid ~16 min of the 2026-11-13 game (from the coach log): Malignance + Plated Steelcaps.
    let ctx = json!({
        "time": "16 min",
        "me": { "champion": "Ahri", "role": "MIDDLE", "level": 11, "goldNow": 1450, "items": ["Malignance", "Plated Steelcaps"], "boots": "Plated Steelcaps" },
        "enemies": [
            { "champion": "Sion", "role": "MIDDLE", "kda": "1/3/2", "items": ["Hollow Radiance", "Plated Steelcaps"], "fed": false },
            { "champion": "Urgot", "role": "TOP", "kda": "7/2/1", "items": ["Black Cleaver", "Plated Steelcaps", "Long Sword"], "fed": true },
            { "champion": "Wukong", "role": "JUNGLE", "kda": "3/2/4", "items": ["Profane Hydra", "Boots of Swiftness"], "fed": false },
            { "champion": "Ashe", "role": "BOTTOM", "kda": "7/3/3", "items": ["Kraken Slayer", "Berserker's Greaves", "Pickaxe"], "fed": true },
            { "champion": "Milio", "role": "UTILITY", "kda": "0/2/9", "items": ["Moonstone Renewer", "Ionian Boots of Lucidity"], "fed": false }
        ],
        "allies": ["Zaahen", "Lux", "Kai'Sa", "Karma"],
        "goldDiff": -900,
        "availableItems": ["Rabadon's Deathcap", "Shadowflame", "Zhonya's Hourglass", "Banshee's Veil", "Void Staff", "Cryptbloom", "Morellonomicon", "Liandry's Torment", "Luden's Echo", "Stormsurge", "Lich Bane", "Horizon Focus", "Mejai's Soulstealer", "Seraph's Embrace", "Cosmic Drive", "Sorcerer's Shoes", "Mercury's Treads", "Plated Steelcaps", "Ionian Boots of Lucidity"]
    });
    let runs = [("claude-opus-5-5", Effort::Low), ("claude-opus-5-5", Effort::Medium), ("gpt-6.1-terra", Effort::Xhigh), ("gpt-6.1-sol", Effort::Xhigh)];
    for (model, effort) in runs {
        let call = ModelCall { model: model.into(), effort: Some(effort), system: SYSTEM.into(), schema: schema(), prompt: ctx.to_string(), timeout_ms: Some(240_000), ..Default::default() };
        match call_model::<Value>(&call).await {
            Ok(r) => println!("{model} {effort:?}: {:.1} s -> {}", r.latency_ms as f64 / 1000.0, r.value),
            Err(e) => println!("{model} {effort:?}: ERROR {e}"),
        }
    }
}
