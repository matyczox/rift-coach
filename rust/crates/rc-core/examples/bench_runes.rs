//! Rune pages for several champions / matchups (real CLIs with web search, uses quota).
//! `cargo run --release -p rc-core --example bench_runes`
use rc_core::llm::Effort;
use rc_core::runes::{advise_runes, load_rune_trees, RuneTree};

fn names(trees: &[RuneTree], ids: &[i64]) -> String {
    ids.iter()
        .map(|id| trees.iter().flat_map(|t| t.slots.iter().flat_map(|s| s.runes.iter())).find(|r| r.id == *id).map(|r| r.name.clone()).unwrap_or_else(|| id.to_string()))
        .collect::<Vec<_>>()
        .join(", ")
}

#[tokio::main]
async fn main() {
    let dd = std::path::PathBuf::from(std::env::var("APPDATA").unwrap()).join("Rift Coach").join("ddragon");
    let trees = load_rune_trees(&dd, "16.22.1").await.expect("trees");
    let tmp = std::env::temp_dir().join(format!("rc-runes-bench-{}", std::process::id()));
    let cases: Vec<(&str, &str, Vec<&str>, Option<(&str, f64)>)> = vec![
        ("Ahri", "MIDDLE", vec!["Xerath", "Darius", "Zaahen", "Yunara", "Sona"], Some(("Xerath", 0.95))),
        ("Akali", "MIDDLE", vec!["Lissandra", "Yasuo", "Briar", "Jhin", "Talon"], Some(("Lissandra", 0.9))),
        ("Ambessa", "TOP", vec!["Darius", "Lux", "Kha'Zix", "Kai'Sa", "Leona"], Some(("Darius", 0.9))),
        ("Ahri", "MIDDLE", vec![], None),
        ("Ambessa", "TOP", vec![], None),
    ];
    let jobs = cases.into_iter().map(|(c, r, e, o)| {
        let (trees, tmp) = (trees.clone(), tmp.clone());
        tokio::spawn(async move {
            let enemies: Vec<String> = e.iter().map(|s| s.to_string()).collect();
            let t0 = std::time::Instant::now();
            let res = advise_runes(&tmp.join(format!("{c}-{}", enemies.len())), &trees, "16.22", c, r, &enemies, o.map(|(n, p)| (n.to_string(), p)), "gpt-6.1-terra", Effort::High).await;
            match res {
                Ok(p) => format!("{c} {r} vs {:?} ({:.0} s)\n  {}\n  why: {}", o.map(|x| x.0), t0.elapsed().as_secs_f64(), names(&trees, &p.selected_perk_ids), p.why.unwrap_or_default()),
                Err(err) => format!("{c} {r}: ERROR {err}"),
            }
        })
    });
    for j in jobs.collect::<Vec<_>>() {
        println!("{}\n", j.await.unwrap());
    }
}
