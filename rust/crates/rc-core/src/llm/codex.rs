//! Codex non-interactive call (`callCodex` in llm.ts). No tools: read-only
//! sandbox, ephemeral session, user config ignored (no MCP/notify hooks),
//! answer written to a temp file by `-o`.

use super::claude::{run, RunError};
use super::{extract_json, hide_window, tail, CallResult, ModelCall};
use anyhow::{anyhow, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::sync::OnceCell;

static CODEX: OnceCell<Option<PathBuf>> = OnceCell::const_new();

/// codex.exe ships inside the Codex desktop app (Microsoft Store package); find it once.
pub async fn resolve_codex() -> Option<PathBuf> {
    CODEX
        .get_or_init(|| async {
            if let Some(p) = std::env::var_os("CODEX_EXE").map(PathBuf::from) {
                if p.exists() {
                    return Some(p);
                }
            }
            let mut c = tokio::process::Command::new("powershell.exe");
            c.args(["-NoProfile", "-Command", "(Get-AppxPackage OpenAI.Codex | Sort-Object Version -Descending | Select-Object -First 1).InstallLocation"])
                .kill_on_drop(true);
            hide_window(&mut c);
            let out = tokio::time::timeout(Duration::from_secs(15), c.output()).await.ok()?.ok()?;
            let dir = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if dir.is_empty() || !out.status.success() {
                return None;
            }
            let exe = PathBuf::from(dir).join("app").join("resources").join("codex.exe");
            exe.exists().then_some(exe)
        })
        .await
        .clone()
}

/// Models called in Codex fast mode by default (`service_tier="fast"`; Terra advertises support for it).
pub const FAST_TIER_MODELS: &[&str] = &["gpt-6.1-terra"];

static FAST_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
/// Settings → Coach → Tryb fast (`coach.fast`, default on).
pub fn set_fast_mode(on: bool) {
    FAST_MODE.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn build_args(opts: &ModelCall, dir: &Path, out: &Path) -> Vec<String> {
    let model = opts.model.strip_prefix("codex:").unwrap_or(&opts.model).to_string();
    let effort = opts.effort.map(|e| e.as_str()).unwrap_or("low");
    let mut a = vec!["exec".into(), "-m".into(), model, "-c".into(), format!("model_reasoning_effort=\"{effort}\"")];
    // Web research (patch notes, matchup stats): Codex's native web_search tool.
    if opts.tools.iter().any(|t| t.to_lowercase().contains("web")) {
        a.push("-c".into());
        a.push("tools.web_search=true".into());
    }
    // Codex fast mode (service tier) for the in-game model: lower latency, uses more of the plan's quota.
    if FAST_MODE.load(std::sync::atomic::Ordering::Relaxed) && FAST_TIER_MODELS.contains(&a[2].as_str()) {
        a.push("-c".into());
        a.push("service_tier=\"fast\"".into());
    }
    for s in ["-s", "read-only", "--skip-git-repo-check", "--ephemeral", "--ignore-user-config", "--color", "never", "-C"] {
        a.push(s.into());
    }
    a.push(dir.display().to_string());
    a.push("-o".into());
    a.push(out.display().to_string());
    a.push("-".into());
    a
}

pub fn build_prompt(opts: &ModelCall) -> String {
    format!(
        "{}\n\nNie uruchamiaj poleceń powłoki ani nie zmieniaj plików{}. Odpowiedz WYŁĄCZNIE jednym obiektem JSON zgodnym ze schematem:\n{}\n\n---\n{}",
        opts.system,
        if opts.tools.is_empty() { " ani narzędzi" } else { "; możesz korzystać z wyszukiwania w internecie" },
        serde_json::to_string(&opts.schema).unwrap(),
        opts.prompt
    )
}

pub async fn call_codex(opts: &ModelCall) -> Result<CallResult<Value>> {
    let exe = resolve_codex().await.ok_or_else(|| anyhow!("Codex nie jest zainstalowany (aplikacja Codex z Microsoft Store)"))?;
    call_codex_with(&exe, opts).await
}

pub async fn call_codex_with(exe: &Path, opts: &ModelCall) -> Result<CallResult<Value>> {
    let dir = tempfile::Builder::new().prefix("rc-codex-").tempdir()?; // removed on drop (rmSync)
    let out = dir.path().join("answer.txt");
    let started = Instant::now();
    let args = build_args(opts, dir.path(), &out);
    match run(exe, &args, &build_prompt(opts), &[], Duration::from_millis(opts.timeout_ms.unwrap_or(60000))).await {
        Err(RunError::Spawn(e)) => return Err(anyhow!("{e}")),
        Err(RunError::Timeout { .. }) => return Err(anyhow!("Codex: przekroczony czas")),
        Ok(o) if o.code != Some(0) => {
            let code = o.code.map(|c| c.to_string()).unwrap_or_else(|| "null".into());
            return Err(anyhow!("Codex: kod {code} {}", tail(&o.stderr, 200)));
        }
        Ok(_) => {}
    }
    let text = std::fs::read_to_string(&out)?;
    let value: Value = serde_json::from_str(extract_json(&text)?)?;
    Ok(CallResult { value, latency_ms: started.elapsed().as_millis() as u64, raw: Value::String(text) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::Effort;
    use serde_json::json;

    #[test]
    fn args_and_prompt_match_ts() {
        let o = ModelCall { model: "codex:gpt-6.1-sol".into(), system: "SYS".into(), prompt: "P".into(), schema: json!({"a":1}), effort: Some(Effort::High), ..Default::default() };
        let a = build_args(&o, Path::new("D"), Path::new("D/answer.txt"));
        assert_eq!(
            a,
            ["exec", "-m", "gpt-6.1-sol", "-c", "model_reasoning_effort=\"high\"", "-s", "read-only", "--skip-git-repo-check", "--ephemeral", "--ignore-user-config", "--color", "never", "-C", "D", "-o", "D/answer.txt", "-"]
        );
        assert_eq!(build_prompt(&o), "SYS\n\nNie uruchamiaj poleceń powłoki ani nie zmieniaj plików ani narzędzi. Odpowiedz WYŁĄCZNIE jednym obiektem JSON zgodnym ze schematem:\n{\"a\":1}\n\n---\nP");
        let w = ModelCall { tools: vec!["WebSearch".into()], effort: None, ..o };
        let a = build_args(&w, Path::new("D"), Path::new("o"));
        assert_eq!(&a[3..7], ["-c", "model_reasoning_effort=\"low\"", "-c", "tools.web_search=true"]);
        assert!(build_prompt(&w).contains("; możesz korzystać z wyszukiwania w internecie."));
    }
}
