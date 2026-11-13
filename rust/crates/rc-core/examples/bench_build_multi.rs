//! Sol vs Opus on several games from the logs (real CLIs, uses quota).
//! `cargo run --release -p rc-core --example bench_build_multi`
use rc_core::build_advisor::{schema, SYSTEM};
use rc_core::llm::{call_model, Effort, ModelCall};
use serde_json::{json, Value};

fn e(c: &str, class: &[&str], role: &str, kda: &str, items: &[&str], fed: bool) -> Value {
    json!({ "champion": c, "class": class, "role": role, "kda": kda, "items": items, "fed": fed })
}

fn main_ctx(name: &str) -> Value {
    let avail_ap = ["Rabadon's Deathcap", "Shadowflame", "Zhonya's Hourglass", "Banshee's Veil", "Void Staff", "Cryptbloom", "Morellonomicon", "Liandry's Torment", "Luden's Echo", "Stormsurge", "Lich Bane", "Horizon Focus", "Malignance", "Sorcerer's Shoes", "Mercury's Treads", "Plated Steelcaps", "Riftmaker", "Hextech Rocketbelt", "Cosmic Drive"];
    let avail_ad = ["Eclipse", "Voltaic Cyclosword", "Death's Dance", "Sterak's Gage", "Black Cleaver", "Maw of Malmortius", "Guardian Angel", "Serylda's Grudge", "Profane Hydra", "Ravenous Hydra", "Chempunk Chainsword", "Spear of Shojin", "Hexdrinker", "Plated Steelcaps", "Mercury's Treads", "Sundered Sky", "Edge of Night", "Youmuu's Ghostblade"];
    match name {
        // Akali mid, Swiftplay 2026-11-12: vs Yasuo / Briar / Lissandra / Jhin / Talon
        "akali" => json!({ "time": "13 min", "me": { "champion": "Akali", "class": ["Assassin"], "role": "MIDDLE", "level": 10, "goldNow": 1400, "items": ["Hextech Alternator", "Boots"], "boots": "Boots", "componentsInto": { "Hextech Alternator": ["Hextech Rocketbelt", "Riftmaker", "Stormsurge", "Cosmic Drive"] } },
            "enemies": [e("Lissandra", &["Mage"], "MIDDLE", "2/3/3", &["Luden's Echo"], false), e("Yasuo", &["Fighter", "Assassin"], "TOP", "6/2/1", &["Kraken Slayer", "Berserker's Greaves"], true), e("Briar", &["Fighter", "Assassin"], "JUNGLE", "4/4/3", &["Eclipse"], false), e("Jhin", &["Marksman", "Mage"], "BOTTOM", "3/2/2", &["Collector"], false), e("Talon", &["Assassin"], "UTILITY", "2/3/4", &["Youmuu's Ghostblade"], false)],
            "allies": ["Teemo", "Rengar", "Caitlyn", "Lux"], "goldDiff": -1200, "availableItems": avail_ap }),
        // Ambessa top early vs Warwick
        "ambessa" => json!({ "time": "11 min", "me": { "champion": "Ambessa", "class": ["Fighter", "Assassin"], "role": "TOP", "level": 9, "goldNow": 1300, "items": ["Long Sword", "Boots"], "boots": "Boots" },
            "enemies": [e("Warwick", &["Fighter", "Tank"], "TOP", "2/1/0", &["Bami's Cinder"], false), e("Lux", &["Mage", "Support"], "MIDDLE", "3/1/2", &["Luden's Echo"], false), e("Kha'Zix", &["Assassin"], "JUNGLE", "4/2/2", &["Profane Hydra"], true), e("Kai'Sa", &["Marksman"], "BOTTOM", "2/2/3", &["Kraken Slayer"], false), e("Leona", &["Tank", "Support"], "UTILITY", "0/3/5", &["Locket of the Iron Solari"], false)],
            "allies": ["Lee Sin", "Ahri", "Ezreal", "Karma"], "goldDiff": 300, "availableItems": avail_ad }),
        // Ahri vs fed Yunara / Zaahen / Darius / Xerath / Sona (late)
        "ahri-late" => json!({ "time": "30 min", "me": { "champion": "Ahri", "class": ["Mage", "Assassin"], "role": "MIDDLE", "level": 16, "goldNow": 2800, "items": ["Luden's Echo", "Sorcerer's Shoes", "Shadowflame", "Zhonya's Hourglass"], "boots": "Sorcerer's Shoes" },
            "enemies": [e("Xerath", &["Mage"], "MIDDLE", "3/9/6", &["Luden's Echo", "Sorcerer's Shoes", "Shadowflame"], false), e("Darius", &["Fighter", "Tank"], "TOP", "6/6/3", &["Stridebreaker", "Sterak's Gage", "Plated Steelcaps"], false), e("Zaahen", &["Fighter"], "JUNGLE", "5/7/8", &["Black Cleaver", "Death's Dance"], false), e("Yunara", &["Marksman"], "BOTTOM", "25/3/4", &["Immortal Shieldbow", "Infinity Edge", "Kraken Slayer", "Lord Dominik's Regards"], true), e("Sona", &["Support", "Mage"], "UTILITY", "1/8/14", &["Moonstone Renewer"], false)],
            "allies": ["Fizz", "Teemo", "Miss Fortune", "Sona"], "goldDiff": -3500, "availableItems": avail_ap }),
        // Ambessa mid mid-game vs Xerath / Darius / Zaahen / Yunara / Swain (AP + AD mix)
        _ => json!({ "time": "18 min", "me": { "champion": "Ambessa", "class": ["Fighter", "Assassin"], "role": "MIDDLE", "level": 13, "goldNow": 1500, "items": ["Eclipse", "Voltaic Cyclosword", "Plated Steelcaps"], "boots": "Plated Steelcaps" },
            "enemies": [e("Xerath", &["Mage"], "MIDDLE", "1/5/3", &["Luden's Echo", "Sorcerer's Shoes"], false), e("Darius", &["Fighter", "Tank"], "TOP", "4/3/2", &["Stridebreaker", "Plated Steelcaps"], false), e("Zaahen", &["Fighter"], "JUNGLE", "2/4/5", &["Black Cleaver"], false), e("Yunara", &["Marksman"], "BOTTOM", "8/2/3", &["Kraken Slayer", "Infinity Edge"], true), e("Swain", &["Mage", "Fighter"], "UTILITY", "2/3/6", &["Liandry's Torment"], false)],
            "allies": ["Teemo", "Fizz", "Miss Fortune", "Sona"], "goldDiff": 1250, "availableItems": avail_ad }),
    }
}

#[tokio::main]
async fn main() {
    let only = std::env::var("GAMES").unwrap_or_default();
    for game in ["akali", "ambessa", "ahri-late", "ambessa-mid"] {
        if !only.is_empty() && !only.split(',').any(|g| g == game) {
            continue;
        }
        let ctx = main_ctx(game);
        for (model, effort) in [("gpt-6.1-sol", Effort::High), ("claude-opus-5-5", Effort::Low)] {
            let call = ModelCall { model: model.into(), effort: Some(effort), system: SYSTEM.into(), schema: schema(), prompt: ctx.to_string(), timeout_ms: Some(240_000), ..Default::default() };
            match call_model::<Value>(&call).await {
                Ok(r) => println!("{game} | {model} {effort:?} {:.1} s: {}", r.latency_ms as f64 / 1000.0, r.value["items"].as_array().map(|a| a.iter().filter_map(|i| i["name"].as_str()).collect::<Vec<_>>().join(" -> ")).unwrap_or_default()),
                Err(e) => println!("{game} | {model}: ERROR {e}"),
            }
        }
    }
}
