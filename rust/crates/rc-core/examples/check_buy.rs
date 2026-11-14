//! Buy list check on real item data: `cargo run --release -p rc-core --example check_buy`
use rc_core::build_advisor::buy_from_plan;
use rc_core::intel::ddragon::from_raw;
fn main() {
    let p = std::path::PathBuf::from(std::env::var("APPDATA").unwrap()).join("Rift Coach/ddragon/16.22.1-pl_PL.json");
    let raw: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
    let sd = from_raw("16.22.1", &raw);
    let name = |id: i64| sd.items.get(&id).map(|d| d.name.clone()).unwrap_or_default();
    let plan: Vec<String> = ["Abyssal Mask", "Kaenic Rookern", "Zeke's Convergence"].iter().map(|s| s.to_string()).collect();
    for (owned, gold) in [(vec![1033i64], 900.0), (vec![1033, 1028], 700.0), (vec![3067, 1057], 900.0), (vec![1057], 1200.0)] {
        let b = buy_from_plan(&plan, &sd.items, &owned, gold, false);
        println!("owned {:?} gold {gold} -> {:?}", owned.iter().map(|i| name(*i)).collect::<Vec<_>>(), b.iter().map(|i| name(*i)).collect::<Vec<_>>());
    }
}
