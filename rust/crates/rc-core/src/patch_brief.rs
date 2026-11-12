//! Once per patch: the research model (with web search/fetch) reads the official
//! patch notes and current meta and writes a compact brief
//! (`src/main/coach/patchBrief.ts`). Every in-game coach call carries it, so
//! advice reflects the live patch rather than training-data knowledge.
//! Cached as `<dir>/patch-<patch>.json` (dir = `<userData>/patch`), same file as TS.

use crate::llm::caller::ModelCaller;
use crate::llm::{Effort, ModelCall};
use crate::research::{research_effort, research_model};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchBrief {
    /// e.g. "16.22"
    pub patch: String,
    pub created_at: f64,
    /// ≤ ~1200 chars: what changed and what is strong/weak right now.
    pub summary: String,
    /// Champion -> one-line note (buffs/nerfs, current build/role); insertion order kept.
    #[serde(default)]
    pub champions: Map<String, Value>,
    /// Notable item/system changes, one line each.
    #[serde(default)]
    pub items: Vec<String>,
    #[serde(default)]
    pub sources: Vec<String>,
}

pub fn schema() -> Value {
    json!({
        "type": "object", "additionalProperties": false,
        "required": ["summary", "champions", "items", "sources"],
        "properties": {
            "summary": { "type": "string", "maxLength": 1400 },
            "champions": { "type": "object", "additionalProperties": { "type": "string", "maxLength": 160 } },
            "items": { "type": "array", "items": { "type": "string", "maxLength": 160 }, "maxItems": 12 },
            "sources": { "type": "array", "items": { "type": "string" }, "maxItems": 8 }
        }
    })
}

/// "16.22.1" -> "16.22"
pub fn patch_of(ddragon_version: &str) -> String {
    ddragon_version.split('.').take(2).collect::<Vec<_>>().join(".")
}

pub fn brief_file(dir: &Path, patch: &str) -> std::path::PathBuf {
    dir.join(format!("patch-{patch}.json"))
}

pub fn load_cached_brief(dir: &Path, patch: &str) -> Option<PatchBrief> {
    serde_json::from_str(&std::fs::read_to_string(brief_file(dir, patch)).ok()?).ok()
}

pub fn system_prompt(patch: &str, focus_champions: &[String]) -> String {
    format!(
        "Jesteś analitykiem meta League of Legends. Masz dostęp do wyszukiwarki i pobierania stron.
Znajdź oficjalne notatki do patcha {patch} (leagueoflegends.com) oraz aktualne statystyki meta (np. strony ze statystykami winrate/pickrate) dla tego patcha.
Napisz po polsku zwięzły brief dla coacha, który doradza w trakcie gry:
- summary: najważniejsze zmiany i co jest teraz silne/słabe (role, junglerzy, itemy, obiektywy, zmiany mapy/timerów, jeśli są).
- champions: krótka notatka dla bohaterów zmienionych w patchu i dla: {} (aktualny build/runy/siła).
- items: istotne zmiany przedmiotów.
- sources: adresy, z których korzystałeś.
Tylko fakty z tego patcha; jeśli czegoś nie znalazłeś, pomiń zamiast zgadywać.",
        focus_champions.join(", ")
    )
}

#[derive(Clone, Debug, Default)]
pub struct BriefRequest {
    pub patch: String,
    pub focus_champions: Vec<String>,
    /// Default: `research_model()` / `research_effort()`.
    pub model: Option<String>,
    pub effort: Option<Effort>,
}

#[derive(Deserialize)]
struct Answer {
    summary: String,
    #[serde(default)]
    champions: Map<String, Value>,
    #[serde(default)]
    items: Vec<String>,
    #[serde(default)]
    sources: Vec<String>,
}

/// Cached brief or a new one (written to `dir`). `now_ms` = epoch ms for `createdAt`.
pub async fn build_patch_brief(caller: &dyn ModelCaller, dir: &Path, req: &BriefRequest, now_ms: f64) -> Result<PatchBrief> {
    if let Some(b) = load_cached_brief(dir, &req.patch) {
        return Ok(b);
    }
    let call = ModelCall {
        model: req.model.clone().unwrap_or_else(research_model),
        effort: Some(req.effort.unwrap_or_else(research_effort)),
        system: system_prompt(&req.patch, &req.focus_champions),
        prompt: format!("Przygotuj brief dla patcha {}.", req.patch),
        schema: schema(),
        timeout_ms: Some(240_000),
        tools: vec!["WebSearch".into(), "WebFetch".into()],
        ..Default::default()
    };
    let r = caller.call(&call).await?;
    let a: Answer = serde_json::from_value(r.value).map_err(|e| anyhow::anyhow!("odpowiedź niezgodna ze schematem: {e}"))?;
    let brief = PatchBrief { patch: req.patch.clone(), created_at: now_ms, summary: a.summary, champions: a.champions, items: a.items, sources: a.sources };
    std::fs::create_dir_all(dir)?;
    std::fs::write(brief_file(dir, &req.patch), crate::jsutil::json_pretty(&brief))?;
    Ok(brief)
}

/// Compact text for the coach prompt, limited to champions in this game.
pub fn brief_for_game(b: &PatchBrief, champions: &[String]) -> String {
    let mut lines = vec![format!("Patch {}: {}", b.patch, b.summary)];
    for c in champions {
        if let Some(n) = b.champions.get(c).and_then(Value::as_str).filter(|s| !s.is_empty()) {
            lines.push(format!("{c}: {n}"));
        }
    }
    lines.extend(b.items.iter().take(5).cloned());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::caller::FakeCaller;

    #[tokio::test]
    async fn builds_caches_and_formats() {
        let d = tempfile::tempdir().unwrap();
        let fake = FakeCaller::new(json!({ "summary": "Junglerzy silni.", "champions": { "Ahri": "buff Q", "Zed": "nerf R" }, "items": ["Zaćmienie tańsze"], "sources": ["https://x"] }));
        let req = BriefRequest { patch: "16.22".into(), focus_champions: vec!["Rumble".into(), "Ahri".into()], ..Default::default() };
        let b = build_patch_brief(&fake, d.path(), &req, 1234.0).await.unwrap();
        assert_eq!(b.patch, "16.22");
        let calls = fake.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].model, research_model());
        assert_eq!(calls[0].tools, ["WebSearch", "WebFetch"]);
        assert_eq!(calls[0].timeout_ms, Some(240_000));
        assert!(calls[0].system.contains("notatki do patcha 16.22") && calls[0].system.contains("dla: Rumble, Ahri (aktualny"));
        assert_eq!(calls[0].prompt, "Przygotuj brief dla patcha 16.22.");
        // Same file / key order as TS: patch, createdAt, summary, champions, items, sources.
        let text = std::fs::read_to_string(d.path().join("patch-16.22.json")).unwrap();
        assert!(text.starts_with("{\n  \"patch\": \"16.22\",\n  \"createdAt\": 1234,\n  \"summary\""), "{text}");
        // Second call: cache, no model call.
        build_patch_brief(&fake, d.path(), &req, 9.0).await.unwrap();
        assert_eq!(fake.calls().len(), 1);
        assert_eq!(brief_for_game(&b, &["Zed".into(), "Lux".into()]), "Patch 16.22: Junglerzy silni.\nZed: nerf R\nZaćmienie tańsze");
        assert_eq!(patch_of("16.22.1"), "16.22");
    }

    #[tokio::test]
    async fn model_error_writes_nothing() {
        let d = tempfile::tempdir().unwrap();
        let fake = FakeCaller::failing("Claude: limit");
        let r = build_patch_brief(&fake, d.path(), &BriefRequest { patch: "1.1".into(), ..Default::default() }, 0.0).await;
        assert!(r.is_err());
        assert!(load_cached_brief(d.path(), "1.1").is_none());
    }
}
