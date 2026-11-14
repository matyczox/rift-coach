//! AI build advisor (`src/main/coach/build.ts`): the next 2–3 items for me given
//! the phase, my items, enemy damage types/items, who is fed and the patch's meta
//! build. Called on real changes only ([`BuildTrigger`]: start, enemy completes an
//! item, I die/recall, every ~3 min).

use crate::intel::ddragon::ItemData;
use crate::intel::game_intel::GameIntel;
use crate::intel::js_round;
use crate::llm::caller::ModelCaller;
use crate::llm::{Effort, ModelCall};
use crate::riot::types::OpponentProfile;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

pub fn schema() -> Value {
    json!({
        "type": "object", "additionalProperties": false, "required": ["phase", "items"],
        "properties": {
            "phase": { "type": "string", "maxLength": 20 },
            "items": { "type": "array", "minItems": 1, "maxItems": 3, "items": { "type": "object", "additionalProperties": false, "required": ["name"], "properties": { "name": { "type": "string" } } } }
        }
    })
}

pub const SYSTEM: &str = "Jesteś ekspertem od buildów League of Legends. Dobierz NASTĘPNE 2–3 przedmioty dla gracza (kolejność zakupu), uwzględniając:
- fazę gry, jego obecne przedmioty i złoto,
- typ obrażeń wrogów (AD/AP), ich przedmioty, leczenie, tarcze, CC, kto jest nafeedowany (najgroźniejszy),
- aktualny build z mety z briefu patcha (jeśli podany) — odchodź od niego tylko z konkretnego powodu.
Najpierw ustal, czym jest postać gracza (\"me.class\" z danych gry) i jak wygrywa walki, dopiero potem dobieraj przedmioty — nie licz samych statystyk. Burst (Mage + Assassin, np. Ahri, Syndra, LeBlanc, Zed): obrażenia na jedno combo i penetracja (Shadowflame, Rabadon's Deathcap, Stormsurge, Lich Bane, Void Staff gdy wróg ma odporność na magię) plus jeden przedmiot na przeżycie (Zhonya's albo Banshee's); Liandry's Torment i Rylai's to przedmioty dla magów zadających obrażenia długo (Brand, Cassiopeia, Vladimir, Swain) — burstowi daj je tylko przeciw 3+ wrogom, których GŁÓWNA klasa (pierwsza w \"class\") to Tank — Fighter z dopiskiem Tank (Urgot, Wukong, Darius) to nie tank. Przeciw zabójcom i nurkującym (Assassin, Fighter wchodzący w ciebie) — Zhonya's albo Plated Steelcaps; przeciw samym magom — Mercury's Treads / Banshee's Veil; przeciw tankom (\"enemies[].class\" Tank) — procentowa penetracja i obrażenia od maks. HP. Skład wroga: \"enemyTeam.damage\" = ilu wrogów zadaje głównie fizyczne (AD) / magiczne (AP) / mieszane obrażenia, \"withArmorItems\" / \"withMagicResistItems\" = kto już kupił pancerz / odporność na magię (liczba przedmiotów). Defensywę dobieraj do CAŁEGO składu, nie tylko do \"laneOpponent\" (rywal z linii liczy się mocniej na początku gry): 4+ AD → pancerz (Zhonya's, Plated Steelcaps; nie Banshee's ani Mercury's), 4+ AP → odporność na magię (Banshee's Veil, Mercury's Treads; nie Zhonya's jako jedyna defensywa), mieszany skład → przedmiot pod najgroźniejszego (nakarmionego) wroga. Penetracja: 2+ wrogów z \"withMagicResistItems\" (albo nakarmiony z MR) → Void Staff / Cryptbloom wcześniej (2.–3. przedmiot); 2+ wrogów z \"withArmorItems\" albo tanki → Serylda's Grudge / Lord Dominik's / Black Cleaver; bez odporności u wrogów penetracji procentowej nie dawaj jako pierwszej. Nazwy przedmiotów DOKŁADNIE z listy \"availableItems\" (polskie nazwy z gry). Jeśli jest \"finishFirst\" (zaczęty przedmiot z planu — gracz ma już jego składniki), zwykle daj go jako pierwszy; inny tylko, gdy wyraźnie bardziej się opłaca (np. te składniki lepiej pasują do innego przedmiotu, który teraz bardziej się przyda). Gdy \"me.boots\" to \"brak\" albo zwykłe Boots, a minęło ~8 min, w planie MUSZĄ być buty tier 2 (np. Plated Steelcaps na AD i auto ataki, Mercury's Treads na AP i CC) jako pierwszy albo drugi przedmiot. Jeśli jest \"currentPlan\" (plan, który gracz już widzi), zostaw jego pierwszy przedmiot, chyba że jest konkretny nowy powód (przedmioty lub feed wrogów) — nie zmieniaj planu w tę i z powrotem. Jeśli w \"me.componentsInto\" gracz ma składnik, zwykle pierwszy przedmiot to jeden z tych, w które się składa (i pasuje do postaci) — nie zostawiaj kupionego składnika bez dokończenia. Gdy \"me.freeSlots\" = 0, nowy przedmiot musi coś zastąpić: proponuj tylko 1–2 przedmioty wyraźnie lepsze od najsłabszego posiadanego (nie buty), nie kolejne pełne przedmioty jak przy wolnym miejscu. Leczenie wrogów: gdy w \"enemyTeam.healing\" są 2+ postacie (albo jedna nakarmiona), daj przedmiot z Grievous Wounds wcześnie, jako 1.–2. (AP: Morellonomicon; AD: Chempunk Chainsword lub Mortal Reminder; tank: Thornmail). Plan zmieniaj, gdy wrogowie kupili coś, co zmienia sytuację (odporności, leczenie, tankowość); bez nowego powodu zostaw pierwszy przedmiot z \"currentPlan\". Bez uzasadnień — tylko nazwy.
\"phase\": \"wczesna gra\" / \"środek gry\" / \"późna gra\".";

/// Items in Data Dragon order (JS iterates integer keys ascending).
fn sorted_items(items: &HashMap<i64, ItemData>) -> Vec<&ItemData> {
    let mut v: Vec<&ItemData> = items.values().collect();
    v.sort_by_key(|i| i.id);
    v
}

/// The JSON context sent as the prompt (same keys/order as TS `buildContext`).
/// `None` without `intel.me`.
pub fn build_context(intel: &GameIntel, items: &HashMap<i64, ItemData>, profiles: &[OpponentProfile], patch_brief: Option<&str>, fed: &[String]) -> Option<Value> {
    let me = intel.me.as_ref()?;
    let mut mo = Map::new();
    mo.insert("champion".into(), json!(me.champion));
    mo.insert("role".into(), json!(me.role));
    mo.insert("level".into(), json!(me.level));
    if let Some(s) = &intel.my_stats {
        mo.insert("goldNow".into(), json!(s.gold));
    }
    mo.insert("items".into(), json!(me.items.iter().map(|i| &i.name).collect::<Vec<_>>()));
    // Boots are easy to forget in a 3-item plan (lvl 15 Ambessa with 4 items and no boots).
    let boots = me.items.iter().find(|i| items.get(&i.id).is_some_and(|d| d.tags.iter().any(|t| t == "Boots"))).map(|i| i.name.clone());
    mo.insert("boots".into(), json!(boots.unwrap_or_else(|| "brak".into())));
    // My components (>= 400 g, not finished items) and the finished items each one builds into.
    let comps: Map<String, Value> = me
        .items
        .iter()
        .filter(|i| items.get(&i.id).is_some_and(|d| !d.completed && d.total >= 400.0 && !d.tags.iter().any(|t| t == "Boots")))
        .map(|i| {
            let into: Vec<&str> = sorted_items(items).into_iter().filter(|d| d.completed && d.from.contains(&i.id)).map(|d| d.name_en.as_deref().unwrap_or(&d.name)).take(6).collect();
            (i.name.clone(), json!(into))
        })
        .collect();
    if !comps.is_empty() {
        mo.insert("componentsInto".into(), Value::Object(comps));
    }
    if let Some(s) = &intel.my_stats {
        mo.insert("stats".into(), serde_json::to_value(s).unwrap_or(Value::Null));
    }
    let enemies: Vec<Value> = intel
        .players
        .iter()
        .filter(|p| !p.is_ally)
        .map(|p| {
            let mut e = Map::new();
            e.insert("champion".into(), json!(p.champion));
            e.insert("role".into(), json!(p.role));
            e.insert("kda".into(), json!(format!("{}/{}/{}", p.kills, p.deaths, p.assists)));
            // Components count too (a Null-Magic Mantle already changes what to build).
            e.insert("items".into(), json!(p.items.iter().filter(|i| i.price >= 400.0).map(|i| &i.name).collect::<Vec<_>>()));
            e.insert("fed".into(), json!(fed.contains(&p.champion)));
            if let Some(pr) = profiles.iter().find(|pr| pr.champion == p.champion) {
                e.insert("profile".into(), json!(pr.summary));
            }
            Value::Object(e)
        })
        .collect();
    let mut o = Map::new();
    o.insert("time".into(), json!(format!("{} min", js_round(intel.game_time / 60.0) as i64)));
    o.insert("me".into(), Value::Object(mo));
    o.insert("enemies".into(), json!(enemies));
    o.insert("allies".into(), json!(intel.players.iter().filter(|p| p.is_ally && !p.is_me).map(|p| &p.champion).collect::<Vec<_>>()));
    o.insert("goldDiff".into(), json!(intel.gold_diff));
    if let Some(b) = patch_brief {
        o.insert("patch".into(), json!(b));
    }
    o.insert("availableItems".into(), json!(sorted_items(items).into_iter().filter(|i| i.purchasable && (i.completed || i.tags.iter().any(|t| t == "Boots")) && i.total >= 900.0).map(|i| &i.name).collect::<Vec<_>>()));
    Some(crate::jsutil::js_numbers(Value::Object(o)))
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildItem {
    pub name: String,
    pub icon_url: String,
    pub why: String,
    pub cost: Option<f64>,
}

/// `BuildPanelState` from shared/intel.ts.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildPanelState {
    /// "wczesna gra" / "środek gry" / "późna gra"
    pub phase: String,
    pub updated_at: f64,
    pub items: Vec<BuildItem>,
}

#[derive(Deserialize)]
struct Answer {
    phase: String,
    items: Vec<NameOnly>,
}
#[derive(Deserialize)]
struct NameOnly {
    name: String,
}

/// Ask the build model. TS defaults: model "claude-sonnet-5-5", effort low, no tools;
/// the app passes the profile's build pick.
pub async fn advise_build(caller: &dyn ModelCaller, ctx: &Value, items: &HashMap<i64, ItemData>, icon_url: &(dyn Fn(i64) -> String + Sync), model: &str, effort: Effort, tools: &[String], now_ms: f64) -> Result<BuildPanelState> {
    let call = ModelCall { model: model.into(), effort: Some(effort), tools: tools.to_vec(), system: SYSTEM.into(), schema: schema(), prompt: serde_json::to_string(ctx)?, timeout_ms: Some(120_000), ..Default::default() };
    let r = caller.call(&call).await?;
    let a: Answer = serde_json::from_value(r.value).map_err(|e| anyhow::anyhow!("odpowiedź niezgodna ze schematem: {e}"))?;
    let all = sorted_items(items);
    let by_name: HashMap<String, &ItemData> = all.iter().flat_map(|i| {
        std::iter::once((i.name.to_lowercase(), *i)).chain(i.name_en.as_ref().map(|en| (en.to_lowercase(), *i)))
    }).collect();
    Ok(BuildPanelState {
        phase: a.phase,
        updated_at: now_ms,
        items: a
            .items
            .iter()
            .take(3)
            .filter_map(|it| {
                let lc = it.name.to_lowercase();
                let data = by_name.get(&lc).copied().or_else(|| all.iter().copied().find(|i| i.name.to_lowercase().contains(&lc)));
                if data.is_some_and(|d| !d.purchasable) { return None; }
                // Only the item: the user asked for no descriptions in the panel.
                Some(BuildItem { name: data.map(|d| d.name.clone()).unwrap_or_else(|| it.name.clone()), icon_url: data.map(|d| icon_url(d.id)).unwrap_or_default(), why: String::new(), cost: data.map(|d| d.total) })
            })
            .collect(),
    })
}

/// A planned item I already started (its own components I hold cover >= 35% of its price) and do not
/// own yet: finish it before anything else ("kupiłem składniki Shadowflame, a każe składać Stormsurge").
pub fn in_progress<'a>(names: &[String], items: &'a HashMap<i64, ItemData>, owned: &[i64]) -> Option<&'a ItemData> {
    names
        .iter()
        .filter_map(|n| {
            let lc = n.to_lowercase();
            items.values().find(|i| i.completed && (i.name.to_lowercase() == lc || i.name_en.as_ref().is_some_and(|e| e.to_lowercase() == lc)))
        })
        .filter(|t| !owned.contains(&t.id))
        .find(|t| {
            let mut own = owned.to_vec();
            let have: f64 = t
                .from
                .iter()
                .filter(|c| match own.iter().position(|o| o == *c) {
                    Some(p) => {
                        own.remove(p);
                        true
                    }
                    None => false,
                })
                .map(|c| items.get(c).map_or(0.0, |i| i.total))
                .sum();
            have >= 0.35 * t.total
        })
}

/// Gold of `t`'s own components that I hold (each owned item counted once).
fn owned_share(t: &ItemData, items: &HashMap<i64, ItemData>, owned: &[i64]) -> f64 {
    let mut own = owned.to_vec();
    t.from
        .iter()
        .filter(|c| match own.iter().position(|o| o == *c) {
            Some(p) => {
                own.remove(p);
                true
            }
            None => false,
        })
        .map(|c| items.get(c).map_or(0.0, |i| i.total))
        .sum::<f64>()
        / t.total.max(1.0)
}

/// The finished item I am clearly building (half of its price or more already in my components, e.g. two
/// Needlessly Large Rods -> Rabadon's Deathcap): it stays first in the plan whatever the model says.
pub fn committed<'a>(items: &'a HashMap<i64, ItemData>, owned: &[i64]) -> Option<&'a ItemData> {
    items
        .values()
        .filter(|t| t.completed && t.purchasable && !owned.contains(&t.id) && !t.from.is_empty())
        .map(|t| (owned_share(t, items, owned), t))
        .filter(|(s, _)| *s >= 0.5)
        .max_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total.total_cmp(&b.1.total)))
        .map(|(_, t)| t)
}

/// What to buy right now with `gold` towards `target` (item ids, shop order): the whole item if the
/// rest of its price fits, else the biggest missing components that fit (recursing into a component
/// that does not fit). Components I already own count towards the target. Boots are added when I have
/// none and gold is left.
pub fn buy_now(target: &ItemData, items: &std::collections::HashMap<i64, ItemData>, owned: &[i64], gold: f64, want_boots: bool) -> Vec<i64> {
    if !target.purchasable || owned.contains(&target.id) { return vec![]; }
    let price = |id: i64| items.get(&id).map_or(0.0, |i| i.total);
    let by_price_desc = |v: &mut Vec<i64>| v.sort_by(|a, b| price(*b).total_cmp(&price(*a)));
    // Recipe parts still missing (an owned component counts once).
    let mut owned = owned.to_vec();
    let mut missing: Vec<i64> = Vec::new();
    for &c in &target.from {
        match owned.iter().position(|&o| o == c) {
            Some(p) => {
                owned.remove(p);
            }
            None => missing.push(c),
        }
    }
    let have: f64 = target.from.iter().map(|&c| price(c)).sum::<f64>() - missing.iter().map(|&c| price(c)).sum::<f64>();
    let rest = target.total - have;
    let mut gold = gold;
    let mut out = Vec::new();
    if rest > 0.0 && gold >= rest {
        out.push(target.id);
        gold -= rest;
    } else {
        let mut todo = missing;
        by_price_desc(&mut todo);
        while !todo.is_empty() {
            let c = todo.remove(0);
            let Some(it) = items.get(&c) else { continue };
            if it.total <= 0.0 {
                continue;
            }
            // Its own parts I already hold ("Null-Magic Mantle" again while it sat in my inventory):
            // they count, each owned item once.
            let mut owned_subs = Vec::new();
            for &sc in &it.from {
                if let Some(p) = owned.iter().position(|&o| o == sc) {
                    owned.remove(p);
                    owned_subs.push(sc);
                }
            }
            let cost = it.total - owned_subs.iter().map(|&x| price(x)).sum::<f64>();
            if gold >= cost {
                out.push(c);
                gold -= cost;
            } else {
                // Too expensive as a whole: its missing components, biggest first.
                let mut sub: Vec<i64> = it.from.clone();
                for o in &owned_subs {
                    if let Some(p) = sub.iter().position(|x| x == o) {
                        sub.remove(p);
                    }
                }
                by_price_desc(&mut sub);
                for s in sub.into_iter().rev() {
                    todo.insert(0, s);
                }
            }
        }
    }
    if want_boots && gold >= price(1001) && price(1001) > 0.0 {
        out.push(1001);
    }
    out
}

/// First actionable purchase in plan order. An earned upgrade or finished item
/// must not hide affordable components of the next planned item.
pub fn purchase_cost(target: &ItemData, items: &HashMap<i64, ItemData>, owned: &[i64]) -> f64 {
    fn credit(id: i64, items: &HashMap<i64, ItemData>, owned: &mut Vec<i64>, depth: usize) -> f64 {
        if depth > 16 { return 0.0; }
        let Some(d) = items.get(&id) else { return 0.0 };
        if let Some(p) = owned.iter().position(|&o| o == id) { owned.remove(p); return d.total; }
        d.from.iter().map(|&c| credit(c, items, owned, depth + 1)).sum::<f64>().min(d.total)
    }
    let mut remaining = owned.to_vec();
    // This is the price of a NEW copy. Owning the same component does not make
    // a second copy free; only its recipe components can reduce the price.
    (target.total - target.from.iter().map(|&c|credit(c,items,&mut remaining,0)).sum::<f64>()).max(0.0)
}

pub fn buy_from_plan(names: &[String], items: &HashMap<i64, ItemData>, owned: &[i64], gold: f64, want_boots: bool) -> Vec<i64> {
    for name in names {
        let target = items.values().find(|d| d.name.eq_ignore_ascii_case(name) || d.name_en.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(name)));
        if let Some(target) = target {
            let buy = buy_now(target, items, owned, gold, false);
            if !buy.is_empty() { return buy; }
        }
    }
    if want_boots && !owned.contains(&1001) && items.get(&1001).is_some_and(|d| d.purchasable && d.total <= gold) { vec![1001] } else { vec![] }
}

/// Decides when the build should be re-evaluated.
#[derive(Default)]
pub struct BuildTrigger {
    last_key: String,
    last_at: f64,
}

impl BuildTrigger {
    /// `now` in ms; true on item/enemy-item/death changes or every 3 min.
    pub fn should_refresh(&mut self, intel: &GameIntel, now: f64) -> bool {
        let Some(me) = &intel.me else { return false };
        // Per enemy: items >= 400 g (resist components too, swaps with the same count) and kills in
        // steps of 3 (someone suddenly fed).
        let enemies: Vec<String> = intel
            .players
            .iter()
            .filter(|p| !p.is_ally)
            .map(|p| format!("{}:{}:{}", p.champion, p.items.iter().filter(|i| i.price >= 400.0).map(|i| i.id.to_string()).collect::<Vec<_>>().join("."), p.kills / 3))
            .collect();
        // My items >= 400 g only: potions / wards / starter changes asked again 3 times in 15 s at game
        // start (Malignance -> Luden's -> Malignance). And at most once per 45 s.
        let key = format!("{}|{}|{}", me.items.iter().filter(|i| i.price >= 400.0).map(|i| i.id.to_string()).collect::<Vec<_>>().join(","), enemies.join(","), me.is_dead);
        // My own purchase (items >= 400 g) asks right away; other changes at most every 45 s.
        let mine_changed = key.split('|').next() != self.last_key.split('|').next();
        let changed = key != self.last_key && (self.last_key.is_empty() || mine_changed || now - self.last_at > 45_000.0);
        if changed || now - self.last_at > 180_000.0 {
            self.last_key = key;
            self.last_at = now;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intel::game_intel::PlayerItem;
    use crate::intel::matchup::tests::{intel, player};
    use crate::llm::caller::FakeCaller;

    fn item(id: i64, name: &str, total: f64, completed: bool) -> ItemData {
        ItemData { id, name: name.into(), total, completed, tags: vec![], name_en: None, flat_ms: 0.0, pct_ms: 0.0, purchasable: true, from: vec![] }
    }

    fn db() -> HashMap<i64, ItemData> {
        [item(3157, "Klepsydra Zhonyi", 3250.0, true), item(1001, "Buty", 300.0, false), item(3089, "Zabójczy Kapelusz Rabadona", 3500.0, true), item(2003, "Mikstura", 50.0, true)].into_iter().map(|i| (i.id, i)).collect()
    }

    #[test]
    fn context_shape() {
        let mut me = player("Ahri", true, true, "MIDDLE", 9);
        me.items = vec![PlayerItem { id: 1001, name: "Buty".into(), price: 300.0 }];
        let mut zed = player("Zed", false, false, "MIDDLE", 10);
        zed.items = vec![PlayerItem { id: 3142, name: "Ostrze Duchów".into(), price: 2800.0 }, PlayerItem { id: 1036, name: "Długi Miecz".into(), price: 350.0 }];
        zed.kills = 5;
        let i = intel(vec![me, zed, player("Lux", true, false, "UTILITY", 8)]);
        let prof = OpponentProfile { champion: "Zed".into(), summary: "agresywny".into(), ..Default::default() };
        let c = build_context(&i, &db(), &[prof], Some("Patch 16.22: x"), &["Zed".into()]).unwrap();
        let keys: Vec<&String> = c.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["time", "me", "enemies", "allies", "goldDiff", "patch", "availableItems"]);
        assert_eq!(c["time"], "10 min");
        assert_eq!(c["me"]["goldNow"], 500);
        assert_eq!(c["me"]["stats"]["maxHp"], 1000);
        assert_eq!(c["enemies"][0], json!({ "champion": "Zed", "role": "MIDDLE", "kda": "5/0/0", "items": ["Ostrze Duchów"], "fed": true, "profile": "agresywny" }));
        assert_eq!(c["allies"], json!(["Lux"]));
        assert_eq!(c["availableItems"], json!(["Zabójczy Kapelusz Rabadona", "Klepsydra Zhonyi"]));
        assert!(build_context(&intel(vec![]), &db(), &[], None, &[]).is_none());
    }

    #[tokio::test]
    async fn english_model_answer_is_displayed_in_polish() {
        let mut items = db();
        items.get_mut(&3157).unwrap().name_en = Some("Zhonya's Hourglass".into());
        let fake = FakeCaller::new(json!({"phase":"środek gry", "items":[{"name":"Zhonya's Hourglass"}]}));
        let b = advise_build(&fake, &json!({}), &items, &|id|format!("img/{id}.png"), "test", Effort::Low, &[], 0.0).await.unwrap();
        assert_eq!(b.items[0].name, "Klepsydra Zhonyi");
        assert_eq!(b.items[0].icon_url, "img/3157.png");
        assert_eq!(b.items[0].cost, Some(3250.0));
    }

    #[tokio::test]
    async fn advise_matches_names() {
        let fake = FakeCaller::new(json!({ "phase": "środek gry", "items": [{ "name": "klepsydra zhonyi" }, { "name": "Rabadona" }, { "name": "Nieznany" }, { "name": "x" }] }));
        let ctx = json!({ "time": "10 min" });
        let b = advise_build(&fake, &ctx, &db(), &|id| format!("img/{id}.png"), "claude-sonnet-5-5", Effort::Low, &[], 42.0).await.unwrap();
        assert_eq!(b.phase, "środek gry");
        assert_eq!(b.items.len(), 3);
        assert_eq!(b.items[0], BuildItem { name: "Klepsydra Zhonyi".into(), icon_url: "img/3157.png".into(), why: String::new(), cost: Some(3250.0) });
        assert_eq!(b.items[1].name, "Zabójczy Kapelusz Rabadona");
        assert_eq!(b.items[2], BuildItem { name: "Nieznany".into(), icon_url: String::new(), why: String::new(), cost: None });
        let call = &fake.calls()[0];
        assert_eq!(call.prompt, "{\"time\":\"10 min\"}");
        assert_eq!(call.timeout_ms, Some(120_000));
        assert_eq!(call.system, SYSTEM);
        let v = serde_json::to_value(&b).unwrap();
        assert!(v.get("updatedAt").is_some() && v["items"][0].get("iconUrl").is_some());
    }

    #[test]
    fn purchases_continue_after_owned_or_earned_boots() {
        let mut items = db();
        let mut upgrade = item(3173, "Chainlaced Crushers", 1250.0, false);
        upgrade.purchasable = false;
        upgrade.from = vec![3111];
        items.insert(3173, upgrade);
        items.insert(3111, item(3111, "Obuwie Merkurego", 1250.0, true));
        items.get_mut(&3111).unwrap().name_en = Some("Mercury's Treads".into());
        items.insert(1033, item(1033, "Opończa", 450.0, false));
        items.insert(3000, item(3000, "Next item", 2400.0, true));
        items.get_mut(&3000).unwrap().from = vec![1033];
        for first in ["Chainlaced Crushers", "Mercury's Treads"] {
            let plan = vec![first.into(), "Next item".into()];
            assert_eq!(buy_from_plan(&plan, &items, &[3111], 1870.0, false), vec![1033]);
        }
        assert_eq!(buy_from_plan(&["Next item".into()], &items, &[3111,1033], 500.0, false), Vec::<i64>::new());
    }

    #[test]
    fn buy_now_whole_item_or_components() {
        let mut items = std::collections::HashMap::new();
        let mk = |id: i64, total: f64, from: Vec<i64>| ItemData { id, name: format!("i{id}"), total, completed: from.len() > 1, tags: vec![], name_en: None, flat_ms: 0.0, pct_ms: 0.0, purchasable: true, from };
        items.insert(1058, mk(1058, 1200.0, vec![]));
        items.insert(1026, mk(1026, 850.0, vec![]));
        items.insert(1052, mk(1052, 400.0, vec![]));
        items.insert(1001, mk(1001, 300.0, vec![]));
        items.insert(3089, mk(3089, 3500.0, vec![1058, 1026, 1052]));
        let rab = items[&3089].clone();
        assert_eq!(purchase_cost(&rab, &items, &[1058,1026]),1450.0);
        assert_eq!(purchase_cost(&items[&1058], &items, &[1058]),1200.0);
        // 1300 g: the biggest component that fits.
        assert_eq!(buy_now(&rab, &items, &[], 1300.0, false), vec![1058]);
        // 900 g: the rod does not fit, the wand does.
        assert_eq!(buy_now(&rab, &items, &[], 900.0, false), vec![1026]);
        // Owning the rod and the wand: the rest (3500 - 2050 = 1450) fits -> the whole item.
        assert_eq!(buy_now(&rab, &items, &[1058, 1026], 1500.0, false), vec![3089]);
        // Gold left after the biggest part buys the next one too (rod + tome).
        assert_eq!(buy_now(&rab, &items, &[], 1600.0, true), vec![1058, 1052]);
        // Boots when gold is left over and I have none.
        assert_eq!(buy_now(&rab, &items, &[1026, 1052], 1550.0, true), vec![1058, 1001]);
        // Nested recipe with a part already owned (Maw: Hexdrinker = Long Sword + Null-Magic Mantle).
        items.insert(1036, mk(1036, 350.0, vec![]));
        items.insert(1033, mk(1033, 450.0, vec![]));
        items.insert(3155, mk(3155, 1300.0, vec![1036, 1033]));
        items.insert(1037, mk(1037, 875.0, vec![]));
        items.insert(3156, mk(3156, 3100.0, vec![3155, 1037]));
        let maw = items[&3156].clone();
        // Mantle owned, 400 g: the Long Sword, never a second Mantle.
        assert_eq!(buy_now(&maw, &items, &[1033], 400.0, false), vec![1036]);
        // Mantle owned, 900 g: Hexdrinker costs only 850 more -> buy it.
        assert_eq!(buy_now(&maw, &items, &[1033], 900.0, false), vec![3155]);
    }

    /// The TS build prompt (src/main/coach/build.ts) must stay the same text as SYSTEM.
    #[test]
    fn prompt_matches_ts() {
        let ts = include_str!("../../../../src/main/coach/build.ts");
        let start = ts.find("const SYSTEM = `").expect("SYSTEM in build.ts") + "const SYSTEM = `".len();
        let end = start + ts[start..].find('`').expect("end of SYSTEM");
        assert_eq!(ts[start..end].replace("\r\n", "\n"), SYSTEM);
    }

    #[test]
    fn trigger() {
        let mut t = BuildTrigger::default();
        let mut i = intel(vec![player("Ahri", true, true, "MIDDLE", 9), player("Zed", false, false, "MIDDLE", 10)]);
        assert!(t.should_refresh(&i, 0.0));
        assert!(!t.should_refresh(&i, 1000.0));
        // Audit V3 F05: a cheap resist component counts, and so does a sudden kill streak.
        i.players[1].items.push(crate::intel::game_intel::PlayerItem { id: 1033, name: "Null-Magic Mantle".into(), price: 400.0 });
        // ...but not within 45 s of the last ask (no flip-flop at game start).
        assert!(!t.should_refresh(&i, 2000.0));
        assert!(t.should_refresh(&i, 50_000.0));
        assert!(!t.should_refresh(&i, 51_000.0));
        i.players[1].kills = 6;
        assert!(t.should_refresh(&i, 100_000.0));
        assert!(!t.should_refresh(&i, 279_000.0));
        assert!(t.should_refresh(&i, 280_001.0));
        assert!(!t.should_refresh(&intel(vec![]), 0.0));
    }
}
