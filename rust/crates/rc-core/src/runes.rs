//! Automatic rune pages in champion select (`src/main/runes/runes.ts`, `roles.ts`).
//! Only ever touches ONE page named [`PAGE_NAME`]; never overwrites or deletes
//! the user's own pages.

use crate::llm::{call_model, Effort, ModelCall};
use crate::sources::lcu::LcuClient;
use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

pub const PAGE_NAME: &str = "Rift Coach";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunePlan {
    pub primary_style_id: i64,
    pub sub_style_id: i64,
    /// 4 primary, 2 secondary, 3 shards
    pub selected_perk_ids: Vec<i64>,
    pub label: String,
    pub lane_opponent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub why: Option<String>,
}

fn shard(name: &str) -> Option<i64> {
    Some(match name.to_lowercase().trim() {
        "adaptive force" => 5008,
        "attack speed" => 5005,
        "ability haste" => 5007,
        "move speed" | "movement speed" => 5010,
        "health scaling" | "scaling health" => 5001,
        "health" => 5011,
        "tenacity and slow resist" | "tenacity" => 5013,
        _ => return None,
    })
}

#[derive(Clone, Debug, Deserialize)]
pub struct Rune {
    pub id: i64,
    pub name: String,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Slot {
    pub runes: Vec<Rune>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct RuneTree {
    pub id: i64,
    pub name: String,
    pub slots: Vec<Slot>,
}

pub async fn load_rune_trees(cache_dir: &Path, version: &str) -> Result<Vec<RuneTree>> {
    std::fs::create_dir_all(cache_dir)?;
    let f = cache_dir.join(format!("{version}-runes-en.json"));
    if !f.exists() {
        let res = crate::intel::ddragon::web_client().get(format!("https://ddragon.leagueoflegends.com/cdn/{version}/data/en_US/runesReforged.json")).send().await?;
        if !res.status().is_success() {
            bail!("runes: HTTP {}", res.status().as_u16());
        }
        std::fs::write(&f, res.text().await?)?;
    }
    Ok(serde_json::from_str(&std::fs::read_to_string(&f)?)?)
}

fn norm(s: &str) -> String {
    s.to_lowercase().chars().filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit()).collect()
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuneAnswer {
    pub lane_opponent: Option<String>,
    pub primary_tree: String,
    pub keystone: String,
    pub primary: Vec<String>,
    pub secondary_tree: String,
    pub secondary: Vec<String>,
    pub shards: Vec<String>,
    #[serde(default)]
    pub why: Option<String>,
}

/// Map the model's English rune names to a valid page; errors if something breaks the rules.
pub fn plan_from_names(trees: &[RuneTree], a: &RuneAnswer, label: &str, lane_opponent: Option<String>) -> Result<RunePlan> {
    let tree = |n: &str| trees.iter().find(|t| norm(&t.name) == norm(n)).ok_or_else(|| anyhow!("tree {n}"));
    let (p, s) = (tree(&a.primary_tree)?, tree(&a.secondary_tree)?);
    if p.id == s.id {
        bail!("secondary tree equals primary");
    }
    let in_tree = |t: &RuneTree, n: &str, slots: &[usize]| -> Result<(i64, usize)> {
        for &i in slots {
            if let Some(r) = t.slots.get(i).and_then(|sl| sl.runes.iter().find(|x| norm(&x.name) == norm(n))) {
                return Ok((r.id, i));
            }
        }
        Err(anyhow!("rune {n} not in {}", t.name))
    };
    let ks = in_tree(p, &a.keystone, &[0])?;
    let mut prim = a.primary.iter().take(3).map(|n| in_tree(p, n, &[1, 2, 3])).collect::<Result<Vec<_>>>()?;
    let mut rows: Vec<usize> = prim.iter().map(|x| x.1).collect();
    rows.sort();
    rows.dedup();
    if rows.len() != 3 {
        bail!("primary runes must be one per row");
    }
    let mut sec = a.secondary.iter().take(2).map(|n| in_tree(s, n, &[1, 2, 3])).collect::<Result<Vec<_>>>()?;
    if sec.len() != 2 || sec[0].1 == sec[1].1 {
        bail!("secondary runes must be from two different rows");
    }
    let shards = a.shards.iter().take(3).map(|n| shard(n).ok_or_else(|| anyhow!("shard {n}"))).collect::<Result<Vec<_>>>()?;
    if shards.len() != 3 {
        bail!("need 3 shards");
    }
    prim.sort_by_key(|x| x.1);
    sec.sort_by_key(|x| x.1);
    let mut ids = vec![ks.0];
    ids.extend(prim.iter().map(|x| x.0));
    ids.extend(sec.iter().map(|x| x.0));
    ids.extend(shards);
    Ok(RunePlan { primary_style_id: p.id, sub_style_id: s.id, selected_perk_ids: ids, label: label.into(), lane_opponent, why: None })
}

pub fn schema() -> Value {
    json!({
        "type": "object", "additionalProperties": false,
        "required": ["laneOpponent", "primaryTree", "keystone", "primary", "secondaryTree", "secondary", "shards"],
        "properties": {
            "laneOpponent": { "anyOf": [{ "type": "null" }, { "type": "string" }] },
            "primaryTree": { "type": "string" }, "keystone": { "type": "string" },
            "primary": { "type": "array", "items": { "type": "string" }, "minItems": 3, "maxItems": 3 },
            "secondaryTree": { "type": "string" },
            "secondary": { "type": "array", "items": { "type": "string" }, "minItems": 2, "maxItems": 2 },
            "shards": { "type": "array", "items": { "type": "string" }, "minItems": 3, "maxItems": 3 },
            "why": { "type": "string" }
        }
    })
}

/// Rows of every tree, so the model can't put two runes from one row into the secondary tree.
pub fn tree_rows(trees: &[RuneTree]) -> String {
    trees
        .iter()
        .map(|t| {
            let rows: Vec<String> = t
                .slots
                .iter()
                .enumerate()
                .map(|(i, sl)| format!("{} [{}]", if i == 0 { "keystone".to_string() } else { format!("rząd {i}") }, sl.runes.iter().map(|r| r.name.as_str()).collect::<Vec<_>>().join(" | ")))
                .collect();
            format!("{}: {}", t.name, rows.join("; "))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub struct RuneAsk<'a> {
    pub patch: &'a str,
    pub champion: &'a str,
    pub role: &'a str,
    pub enemies: &'a [String],
    pub opponent: Option<(String, f64)>,
    pub trees: Option<&'a [RuneTree]>,
}

fn pct(p: f64) -> i64 {
    crate::intel::js_round(p * 100.0) as i64
}

pub fn rune_system_prompt(o: &RuneAsk) -> String {
    let enemies = if o.enemies.is_empty() { "jeszcze nikt".to_string() } else { o.enemies.join(", ") };
    let (champion, role, patch) = (o.champion, o.role, o.patch);
    let opp_line = match &o.opponent {
        Some((c, p)) => format!("Przeciwnik na linii (pewność {}%): {c}. Ustaw laneOpponent = \"{c}\".", pct(*p)),
        None => "Przeciwnik na linii niepewny: laneOpponent = null.".into(),
    };
    let steps = match &o.opponent {
        Some((opp, _)) => format!(
            "2. Matchup: sprawdź stronę {champion} vs {opp} (np. lolalytics \"vs\", u.gg matchups). Keystone i główne drzewo zmieniaj względem bazy TYLKO, gdy dane matchupu mają dużą próbkę (≥ 300 gier dla tej runy) i wyraźnie wyższy winrate (≥ 2 pkt proc.). Małe próbki (Master+, kilkadziesiąt gier) i mało znane agregatory IGNORUJ.
3. Drobne zmiany pod przeciwnika są zawsze w porządku, jeśli mają jasny powód:
   - drugie drzewo i drobne runy: Second Wind / Doran's start przeciw poke, Bone Plating przeciw burstowi, Unflinching lub Tenacity przeciw dużej ilości CC;
   - odłamki: Health Scaling, Health, Move Speed.
   Nie wymyślaj egzotycznego keystone'a, którego nikt na tej postaci nie gra."
        ),
        None => "2. Możesz lekko dopasować drugie drzewo i odłamki do składu wrogów (np. dużo AP albo dużo CC), ale keystone zostaw z bazy.".into(),
    };
    let trees = match o.trees {
        Some(t) => format!("Drzewa i rzędy run na tym patchu (secondary: 2 runy z DWÓCH RÓŻNYCH rzędów, bez keystone'a):\n{}\n", tree_rows(t)),
        None => String::new(),
    };
    format!(
        "Jesteś ekspertem od run w League of Legends. Patch {patch}. Gracz: {champion}, rola {role}. Wrogowie: {enemies}.
{opp_line}

MUSISZ sprawdzić dane w internecie. Nie odpowiadaj z pamięci, bo runy zmieniają się co patch. Postępuj tak:
1. Strona bazowa: najlepsza strona {champion} {role} na tym patchu (albo poprzednim, jeśli ten jest świeży) z dużej próbki: lolalytics.com, u.gg, op.gg albo mobalytics.gg, ranga Emerald+ lub Platinum+, tysiące gier. Keystone wybierz po winrate przy sensownej popularności (pickrate ≥ 5%), a nie tylko po popularności.
{steps}

Podaj ANGIELSKIE nazwy dokładnie jak w grze:
- primaryTree (Precision/Domination/Sorcery/Resolve/Inspiration), keystone, primary (3 runy z kolejnych rzędów);
- secondaryTree, secondary (2 runy z różnych rzędów);
- shards (3 odłamki, wybierz z: \"Adaptive Force\", \"Attack Speed\", \"Ability Haste\", \"Move Speed\", \"Health Scaling\", \"Health\", \"Tenacity and Slow Resist\").
{trees}why: jedno-dwa zdania po polsku: źródło i próbka strony bazowej, co zmieniłeś pod przeciwnika i dlaczego."
    )
}

/// Cache file key like the TS (`[^\w.-]` removed).
pub fn cache_key(patch: &str, champion: &str, role: &str, enemies: &[String]) -> String {
    let mut e = enemies.to_vec();
    e.sort();
    let joined = if e.is_empty() { "none".to_string() } else { e.join("_") };
    format!("{patch}-{champion}-{role}-{joined}").chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.' || *c == '-').collect()
}

/// Ask the draft model (web) for the page; cached per patch/champion/role/enemies.
#[allow(clippy::too_many_arguments)]
pub async fn advise_runes(cache_dir: &Path, trees: &[RuneTree], patch: &str, champion: &str, role: &str, enemies: &[String], opponent: Option<(String, f64)>, model: &str, effort: Effort) -> Result<RunePlan> {
    let f = cache_dir.join(format!("runes-{}.json", cache_key(patch, champion, role, enemies)));
    if let Some(p) = std::fs::read_to_string(&f).ok().and_then(|t| serde_json::from_str(&t).ok()) {
        return Ok(p);
    }
    let system = rune_system_prompt(&RuneAsk { patch, champion, role, enemies, opponent, trees: Some(trees) });
    let mut last_err = String::new();
    for attempt in 0..2 {
        let call = ModelCall {
            model: model.into(),
            effort: Some(effort),
            system: system.clone(),
            prompt: if attempt > 0 { format!("Poprzednia odpowiedź była niepoprawna ({last_err}). Popraw nazwy/rzędy.") } else { "Dobierz runy.".into() },
            schema: schema(),
            tools: vec!["WebSearch".into(), "WebFetch".into()],
            timeout_ms: Some(120000),
            thinking_tokens: None,
        };
        let r = match call_model::<RuneAnswer>(&call).await {
            Ok(r) => r,
            Err(e) => {
                last_err = format!("Error: {e}");
                continue;
            }
        };
        let label = format!("{champion} {}{}", role.to_lowercase(), r.value.lane_opponent.as_ref().map(|o| format!(" vs {o}")).unwrap_or_default());
        match plan_from_names(trees, &r.value, &label, r.value.lane_opponent.clone()) {
            Ok(mut plan) => {
                plan.why = r.value.why.clone();
                std::fs::create_dir_all(cache_dir)?;
                std::fs::write(&f, serde_json::to_string_pretty(&plan)?)?;
                return Ok(plan);
            }
            Err(e) => last_err = format!("Error: {e}"),
        }
    }
    bail!("Runy: {last_err}")
}

#[derive(Debug, PartialEq, Eq)]
pub enum ApplyResult {
    Updated,
    Created,
    NoSlot,
}

/// Write the plan into the "Rift Coach" page (create it if there's a free slot) and select it.
/// Pure decision part, so it can be tested without a client.
pub enum PageAction {
    Update { id: i64, body: Value },
    Create { body: Value },
}

pub fn page_body(plan: &RunePlan) -> Value {
    json!({ "name": PAGE_NAME, "primaryStyleId": plan.primary_style_id, "subStyleId": plan.sub_style_id, "selectedPerkIds": plan.selected_perk_ids, "current": true })
}

/// Our editable page among the user's pages, if any (matched by exact name only).
pub fn own_page(pages: &Value) -> Option<i64> {
    pages
        .as_array()?
        .iter()
        .find(|p| p.get("name").and_then(Value::as_str) == Some(PAGE_NAME) && p.get("isEditable").and_then(Value::as_bool) == Some(true))
        .and_then(|p| p.get("id").and_then(Value::as_i64))
}

pub async fn apply_rune_page(lcu: &LcuClient, plan: &RunePlan) -> Result<ApplyResult> {
    let pages: Value = lcu.get("/lol-perks/v1/pages").await?;
    let body = page_body(plan);
    if let Some(id) = own_page(&pages) {
        let mut b = body.clone();
        b["id"] = json!(id);
        lcu.request("PUT", &format!("/lol-perks/v1/pages/{id}"), Some(&b)).await?;
        lcu.request("PUT", "/lol-perks/v1/currentpage", Some(&json!(id))).await?;
        return Ok(ApplyResult::Updated);
    }
    let inv: Value = lcu.get("/lol-perks/v1/inventory").await?;
    if inv.get("canAddCustomPage").and_then(Value::as_bool) != Some(true) {
        return Ok(ApplyResult::NoSlot);
    }
    lcu.request("POST", "/lol-perks/v1/pages", Some(&body)).await?;
    Ok(ApplyResult::Created)
}

// ---------- draft roles (roles.ts) ----------

pub const ROLES: [&str; 5] = ["TOP", "JUNGLE", "MIDDLE", "BOTTOM", "UTILITY"];
pub type RoleRates = BTreeMap<String, f64>;

/// Play-rate share per role for each champion, from the web, cached per patch & champion.
pub async fn role_rates(dir: &Path, patch: &str, champions: &[String], model: &str, effort: Effort) -> Result<HashMap<String, RoleRates>> {
    std::fs::create_dir_all(dir)?;
    let file = dir.join(format!("roles-{patch}.json"));
    let mut cache: BTreeMap<String, RoleRates> = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let missing: Vec<&String> = champions.iter().filter(|c| !cache.contains_key(*c)).collect();
    if !missing.is_empty() {
        let props: serde_json::Map<String, Value> = ROLES.iter().map(|r| (r.to_string(), json!({ "type": "number", "minimum": 0, "maximum": 1 }))).collect();
        let schema = json!({
            "type": "object", "additionalProperties": false, "required": ["rates"],
            "properties": { "rates": { "type": "object", "additionalProperties": { "type": "object", "additionalProperties": false, "required": ROLES, "properties": props } } }
        });
        #[derive(Deserialize)]
        struct R {
            rates: HashMap<String, RoleRates>,
        }
        let r = call_model::<R>(&ModelCall {
            model: model.into(),
            effort: Some(effort),
            schema,
            tools: vec!["WebSearch".into(), "WebFetch".into()],
            timeout_ms: Some(120000),
            system: format!("Podaj udział gier na każdej roli (TOP, JUNGLE, MIDDLE, BOTTOM, UTILITY) dla podanych bohaterów League of Legends na patchu {patch}, na podstawie stron ze statystykami (np. lolalytics, u.gg, op.gg). Wartości 0..1 sumujące się do ~1. Klucz = nazwa bohatera dokładnie jak w zapytaniu."),
            prompt: missing.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "),
            thinking_tokens: None,
        })
        .await?;
        for c in missing {
            if let Some(v) = r.value.rates.get(c) {
                cache.insert(c.clone(), v.clone());
            }
        }
        let mut buf = Vec::new();
        let mut ser = serde_json::Serializer::with_formatter(&mut buf, serde_json::ser::PrettyFormatter::with_indent(b" "));
        cache.serialize(&mut ser)?;
        std::fs::write(&file, buf)?;
    }
    Ok(champions.iter().filter_map(|c| cache.get(c).map(|r| (c.clone(), r.clone()))).collect())
}

/// P(champion plays role) for every enemy pick, marginalised over all injective assignments.
pub fn assign_roles(picks: &[String], rates: &HashMap<String, RoleRates>) -> Vec<(String, [f64; 5])> {
    let n = picks.len();
    let mut result: Vec<(String, [f64; 5])> = picks.iter().map(|c| (c.clone(), [0.0; 5])).collect();
    if n == 0 {
        return result;
    }
    fn rec(i: usize, w: f64, picks: &[String], rates: &HashMap<String, RoleRates>, used: &mut [bool; 5], choice: &mut Vec<usize>, total: &mut f64, result: &mut [(String, [f64; 5])]) {
        if w == 0.0 {
            return;
        }
        if i == picks.len() {
            *total += w;
            for k in 0..picks.len() {
                result[k].1[choice[k]] += w;
            }
            return;
        }
        let r = rates.get(&picks[i]);
        for j in 0..5 {
            if used[j] {
                continue;
            }
            // Unknown champion: uniform; tiny floor so off-role picks are possible but unlikely.
            let p = r.map(|r| r.get(ROLES[j]).copied().unwrap_or(0.0).max(0.005)).unwrap_or(0.2);
            used[j] = true;
            choice[i] = j;
            rec(i + 1, w * p, picks, rates, used, choice, total, result);
            used[j] = false;
        }
    }
    let mut total = 0.0;
    rec(0, 1.0, picks, rates, &mut [false; 5], &mut vec![0; n], &mut total, &mut result);
    if total > 0.0 {
        for (_, v) in result.iter_mut() {
            for x in v.iter_mut() {
                *x /= total;
            }
        }
    }
    result
}

/// Most likely lane opponent for my role, with its probability.
pub fn lane_opponent_for(my_role: &str, assigned: &[(String, [f64; 5])]) -> Option<(String, f64)> {
    let j = ROLES.iter().position(|r| *r == my_role)?;
    let mut best: Option<(String, f64)> = None;
    for (c, r) in assigned {
        if best.as_ref().is_none_or(|b| r[j] > b.1) {
            best = Some((c.clone(), r[j]));
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trees() -> Vec<RuneTree> {
        serde_json::from_value(json!([
            { "id": 8000, "name": "Precision", "slots": [
                { "runes": [{ "id": 8010, "name": "Conqueror" }, { "id": 8005, "name": "Press the Attack" }] },
                { "runes": [{ "id": 9111, "name": "Triumph" }, { "id": 8009, "name": "Presence of Mind" }] },
                { "runes": [{ "id": 9104, "name": "Legend: Alacrity" }, { "id": 9105, "name": "Legend: Haste" }] },
                { "runes": [{ "id": 8299, "name": "Last Stand" }, { "id": 8014, "name": "Coup de Grace" }] } ] },
            { "id": 8400, "name": "Resolve", "slots": [
                { "runes": [{ "id": 8437, "name": "Grasp of the Undying" }] },
                { "runes": [{ "id": 8446, "name": "Demolish" }, { "id": 8473, "name": "Bone Plating" }] },
                { "runes": [{ "id": 8429, "name": "Conditioning" }, { "id": 8444, "name": "Second Wind" }] },
                { "runes": [{ "id": 8451, "name": "Overgrowth" }, { "id": 8242, "name": "Unflinching" }] } ] }
        ]))
        .unwrap()
    }

    fn answer() -> RuneAnswer {
        serde_json::from_value(json!({
            "laneOpponent": "Warwick", "primaryTree": "precision", "keystone": "Conqueror",
            "primary": ["Last Stand", "Triumph", "Legend: Haste"], "secondaryTree": "Resolve",
            "secondary": ["Unflinching", "Second Wind"], "shards": ["Attack Speed", "Adaptive Force", "Health Scaling"], "why": "x"
        }))
        .unwrap()
    }

    #[test]
    fn plan_orders_by_row_and_validates() {
        let p = plan_from_names(&trees(), &answer(), "Ambessa top vs Warwick", Some("Warwick".into())).unwrap();
        assert_eq!(p.primary_style_id, 8000);
        assert_eq!(p.selected_perk_ids, [8010, 9111, 9105, 8299, 8444, 8242, 5005, 5008, 5001]);
        let mut bad = answer();
        bad.secondary = vec!["Second Wind".into(), "Conditioning".into()];
        assert!(plan_from_names(&trees(), &bad, "", None).unwrap_err().to_string().contains("two different rows"));
        let mut bad = answer();
        bad.secondary_tree = "Precision".into();
        assert!(plan_from_names(&trees(), &bad, "", None).is_err());
        let mut bad = answer();
        bad.shards[0] = "Lethality".into();
        assert_eq!(plan_from_names(&trees(), &bad, "", None).unwrap_err().to_string(), "shard Lethality");
    }

    #[test]
    fn only_our_page() {
        let pages = json!([
            { "id": 1, "name": "Moja strona", "isEditable": true },
            { "id": 2, "name": "Rift Coach", "isEditable": false },
            { "id": 3, "name": "Rift Coach", "isEditable": true }
        ]);
        assert_eq!(own_page(&pages), Some(3));
        assert_eq!(own_page(&json!([{ "id": 1, "name": "rift coach", "isEditable": true }])), None);
        let p = plan_from_names(&trees(), &answer(), "L", None).unwrap();
        assert_eq!(page_body(&p)["name"], "Rift Coach");
    }

    #[test]
    fn prompt_and_cache_key() {
        let enemies = vec!["Warwick".to_string(), "Lee Sin".to_string()];
        let t = trees();
        let s = rune_system_prompt(&RuneAsk { patch: "16.22", champion: "Ambessa", role: "TOP", enemies: &enemies, opponent: Some(("Warwick".into(), 0.83)), trees: Some(&t) });
        assert!(s.contains("Przeciwnik na linii (pewność 83%): Warwick. Ustaw laneOpponent = \"Warwick\"."));
        assert!(s.contains("Precision: keystone [Conqueror | Press the Attack]; rząd 1 [Triumph | Presence of Mind]"));
        assert!(s.ends_with("co zmieniłeś pod przeciwnika i dlaczego."));
        assert_eq!(cache_key("16.22", "Kai'Sa", "BOTTOM", &enemies), "16.22-KaiSa-BOTTOM-LeeSin_Warwick");
        assert_eq!(cache_key("16.22", "Ambessa", "TOP", &[]), "16.22-Ambessa-TOP-none");
    }

    #[test]
    fn joint_role_assignment() {
        let r = |v: [f64; 5]| -> RoleRates { ROLES.iter().zip(v).map(|(k, x)| (k.to_string(), x)).collect() };
        let mut rates = HashMap::new();
        rates.insert("Warwick".to_string(), r([0.3, 0.7, 0.0, 0.0, 0.0]));
        rates.insert("Lee Sin".to_string(), r([0.05, 0.95, 0.0, 0.0, 0.0]));
        let picks = vec!["Warwick".to_string(), "Lee Sin".to_string()];
        let a = assign_roles(&picks, &rates);
        let (opp, p) = lane_opponent_for("TOP", &a).unwrap();
        assert_eq!(opp, "Warwick");
        assert!(p > 0.8, "{p}");
        assert!(lane_opponent_for("ARAM/BRAK ROLI", &a).is_none());
        let sum: f64 = a[0].1.iter().sum();
        assert!((sum - 1.0).abs() < 1e-9);
    }
}
