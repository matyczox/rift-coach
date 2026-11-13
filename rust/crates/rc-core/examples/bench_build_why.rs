//! Why do the models pick what they pick, and do they adapt to full-AP / full-AD / all-tank enemies?
//! Same build prompt with a "why" per item. Real CLIs (uses quota).
//! `cargo run --release -p rc-core --example bench_build_why`
use rc_core::build_advisor::SYSTEM;
use rc_core::llm::{call_model, Effort, ModelCall};
use serde_json::{json, Value};

fn enemies(kind: &str) -> Value {
    let e = |c: &str, r: &str, items: &[&str]| json!({ "champion": c, "role": r, "kda": "3/3/3", "items": items, "fed": false });
    match kind {
        "full AP" => json!([e("Syndra", "MIDDLE", &["Luden's Echo"]), e("Kennen", "TOP", &["Stormsurge"]), e("Elise", "JUNGLE", &["Stormsurge"]), e("Ziggs", "BOTTOM", &["Luden's Echo"]), e("Karma", "UTILITY", &["Imperial Mandate"])]),
        "full AD" => json!([e("Zed", "MIDDLE", &["Youmuu's Ghostblade"]), e("Darius", "TOP", &["Stridebreaker"]), e("Kha'Zix", "JUNGLE", &["Profane Hydra"]), e("Jinx", "BOTTOM", &["Kraken Slayer"]), e("Pyke", "UTILITY", &["Umbral Glaive"])]),
        _ => json!([e("Sion", "MIDDLE", &["Heartsteel"]), e("Ornn", "TOP", &["Sunfire Aegis"]), e("Zac", "JUNGLE", &["Unending Despair"]), e("Tahm Kench", "BOTTOM", &["Heartsteel"]), e("Alistar", "UTILITY", &["Knight's Vow"])]),
    }
}

#[tokio::main]
async fn main() {
    let system = SYSTEM.replace("Bez uzasadnień — tylko nazwy.", "Przy każdym przedmiocie krótkie \"why\" po polsku (1 zdanie): dlaczego ten przedmiot przeciw tym wrogom.");
    let schema = json!({ "type": "object", "additionalProperties": false, "required": ["phase", "items"], "properties": {
        "phase": { "type": "string" },
        "items": { "type": "array", "minItems": 2, "maxItems": 3, "items": { "type": "object", "additionalProperties": false, "required": ["name", "why"], "properties": { "name": { "type": "string" }, "why": { "type": "string" } } } }
    }});
    let avail = ["Rabadon's Deathcap", "Shadowflame", "Zhonya's Hourglass", "Banshee's Veil", "Void Staff", "Cryptbloom", "Morellonomicon", "Liandry's Torment", "Luden's Echo", "Stormsurge", "Lich Bane", "Horizon Focus", "Seraph's Embrace", "Cosmic Drive", "Sorcerer's Shoes", "Mercury's Treads", "Plated Steelcaps", "Malignance", "Blackfire Torch", "Riftmaker"];
    for scenario in ["full AP", "full AD", "same tanki"] {
        let ctx = json!({
            "time": "14 min",
            "me": { "champion": "Ahri", "role": "MIDDLE", "level": 10, "goldNow": 1300, "items": ["Malignance", "Boots"], "boots": "Boots" },
            "enemies": enemies(scenario), "allies": ["Lee Sin", "Garen", "Kai'Sa", "Lulu"], "goldDiff": 0, "availableItems": avail,
        });
        for (model, effort) in [("claude-opus-5-5", Effort::Low), ("gpt-6.1-terra", Effort::High)] {
            let call = ModelCall { model: model.into(), effort: Some(effort), system: system.clone(), schema: schema.clone(), prompt: ctx.to_string(), timeout_ms: Some(240_000), ..Default::default() };
            match call_model::<Value>(&call).await {
                Ok(r) => {
                    println!("== {scenario} | {model} {effort:?} ({:.1} s)", r.latency_ms as f64 / 1000.0);
                    for it in r.value["items"].as_array().into_iter().flatten() {
                        println!("   {} — {}", it["name"].as_str().unwrap_or(""), it["why"].as_str().unwrap_or(""));
                    }
                }
                Err(e) => println!("== {scenario} | {model}: ERROR {e}"),
            }
        }
    }
}
