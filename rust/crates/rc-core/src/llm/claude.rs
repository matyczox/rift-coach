//! Claude Code headless (`claude -p`) with the user's subscription login
//! (`src/main/coach/claude.ts`). No tools, no MCP, no project settings:
//! a pure prompt -> JSON call. The prompt goes through stdin.

use super::{extract_json, head, hide_window, tail, CallResult, ModelCall};
use anyhow::{anyhow, Result};
use serde_json::Value;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Native binary (no shell): npm global install, then the native installer, else PATH.
pub fn resolve_bin() -> PathBuf {
    let mut c = Vec::new();
    if let Some(a) = std::env::var_os("APPDATA") {
        c.push(PathBuf::from(a).join("npm").join("node_modules").join("@anthropic-ai").join("claude-code").join("bin").join("claude.exe"));
    }
    if let Some(u) = std::env::var_os("USERPROFILE") {
        c.push(PathBuf::from(u).join(".local").join("bin").join("claude.exe"));
    }
    c.into_iter().find(|p| p.exists()).unwrap_or_else(|| PathBuf::from("claude"))
}

/// The exact argument list claude.ts builds.
pub fn build_args(opts: &ModelCall) -> Vec<String> {
    let mut a = vec!["-p".to_string(), "--model".into(), opts.model.clone(), "--output-format".into(), "json".into()];
    if let Some(e) = opts.effort {
        a.push("--effort".into());
        a.push(e.as_str().into());
    }
    // Structured-output mode costs an extra round trip (~10 s); we ask for raw JSON and validate ourselves.
    a.push("--system-prompt".into());
    a.push(format!(
        "{}\n\nOdpowiedz WYŁĄCZNIE jednym obiektem JSON zgodnym z tym schematem, bez komentarzy i bez bloku kodu:\n{}",
        opts.system,
        serde_json::to_string(&opts.schema).unwrap()
    ));
    a.push("--tools".into());
    a.push(opts.tools.join(","));
    if !opts.tools.is_empty() {
        a.push("--allowedTools".into());
        a.extend(opts.tools.iter().cloned());
    }
    a.push("--setting-sources".into());
    a.push(String::new());
    a.push("--strict-mcp-config".into());
    a.push("--no-session-persistence".into());
    a
}

pub struct Output {
    pub stdout: String,
    pub stderr: String,
    pub code: Option<i32>,
}

/// Spawn `bin args`, write `stdin`, collect output; kill on timeout.
pub async fn run(bin: &std::path::Path, args: &[String], stdin: &str, env: &[(&str, String)], timeout: Duration) -> std::result::Result<Output, RunError> {
    let mut cmd = tokio::process::Command::new(bin);
    cmd.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    for (k, v) in env {
        cmd.env(k, v);
    }
    hide_window(&mut cmd);
    let mut child = cmd.spawn().map_err(RunError::Spawn)?;
    let mut sin = child.stdin.take().unwrap();
    let mut sout = child.stdout.take().unwrap();
    let mut serr = child.stderr.take().unwrap();
    let input = stdin.as_bytes().to_vec();
    let writer = tokio::spawn(async move {
        let _ = sin.write_all(&input).await;
        let _ = sin.shutdown().await;
    });
    let out_buf = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let err_buf = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let (ob, eb) = (out_buf.clone(), err_buf.clone());
    let r1 = tokio::spawn(async move {
        let mut b = [0u8; 8192];
        while let Ok(n) = sout.read(&mut b).await {
            if n == 0 {
                break;
            }
            ob.lock().await.extend_from_slice(&b[..n]);
        }
    });
    let r2 = tokio::spawn(async move {
        let mut b = [0u8; 8192];
        while let Ok(n) = serr.read(&mut b).await {
            if n == 0 {
                break;
            }
            eb.lock().await.extend_from_slice(&b[..n]);
        }
    });
    let waited = tokio::time::timeout(timeout, async {
        let st = child.wait().await;
        let _ = r1.await;
        let _ = r2.await;
        st
    })
    .await;
    match waited {
        Ok(st) => {
            let _ = writer.await;
            let stdout = String::from_utf8_lossy(&out_buf.lock().await).into_owned();
            let stderr = String::from_utf8_lossy(&err_buf.lock().await).into_owned();
            Ok(Output { stdout, stderr, code: st.ok().and_then(|s| s.code()) })
        }
        Err(_) => {
            // Kill first: a child that does not read stdin keeps the writer blocked on a full pipe, and
            // awaiting it here delayed the timeout until the child exited on its own.
            let _ = child.kill().await;
            writer.abort();
            let stdout = String::from_utf8_lossy(&out_buf.lock().await).into_owned();
            let stderr = String::from_utf8_lossy(&err_buf.lock().await).into_owned();
            Err(RunError::Timeout { stdout, stderr })
        }
    }
}

#[derive(Debug)]
pub enum RunError {
    Spawn(std::io::Error),
    Timeout { stdout: String, stderr: String },
}

pub async fn call_claude(opts: &ModelCall) -> Result<CallResult<Value>> {
    call_claude_with(&resolve_bin(), opts).await
}

pub async fn call_claude_with(bin: &std::path::Path, opts: &ModelCall) -> Result<CallResult<Value>> {
    let started = Instant::now();
    let args = build_args(opts);
    let env: Vec<(&str, String)> = opts.thinking_tokens.map(|t| vec![("MAX_THINKING_TOKENS", t.to_string())]).unwrap_or_default();
    let out = match run(bin, &args, &opts.prompt, &env, Duration::from_millis(opts.timeout_ms.unwrap_or(45000))).await {
        Ok(o) => o,
        Err(RunError::Spawn(e)) => return Err(anyhow!("{e}")),
        Err(RunError::Timeout { stdout, stderr }) => {
            return Err(anyhow!("Claude: przekroczony czas {} {}", tail(&stderr, 200), head(&stdout, 200)))
        }
    };
    let j: Value = match serde_json::from_str(&out.stdout) {
        Ok(j) => j,
        Err(e) => return Err(anyhow!("Claude: nieczytelna odpowiedź ({}) {}", head(&format!("SyntaxError: {e}"), 80), head(&out.stderr, 160))),
    };
    if j.get("is_error").and_then(Value::as_bool) == Some(true) {
        return Err(anyhow!("Claude: {}", j.get("result").and_then(Value::as_str).unwrap_or("błąd")));
    }
    let value = match j.get("structured_output").filter(|v| !v.is_null()) {
        Some(v) => v.clone(),
        None => {
            let res = j.get("result").and_then(Value::as_str).unwrap_or("");
            match extract_json(res).and_then(|s| serde_json::from_str::<Value>(s).map_err(Into::into)) {
                Ok(v) => v,
                Err(e) => return Err(anyhow!("Claude: nieczytelna odpowiedź ({}) {}", head(&format!("Error: {e}"), 80), head(&out.stderr, 160))),
            }
        }
    };
    if std::env::var_os("RC_USAGE").is_some() {
        let u = j.get("usage").cloned().unwrap_or(Value::Null);
        log::info!(
            "USAGE {}",
            serde_json::json!({ "in": u.get("input_tokens"), "cacheRead": u.get("cache_read_input_tokens"), "cacheWrite": u.get("cache_creation_input_tokens"), "out": u.get("output_tokens"), "ms": started.elapsed().as_millis() as u64 })
        );
    }
    Ok(CallResult { value, latency_ms: started.elapsed().as_millis() as u64, raw: j })
}

#[derive(Clone, Debug, PartialEq)]
pub struct AuthStatus {
    pub logged_in: bool,
    pub error: Option<String>,
}

/// `claude auth status` (15 s timeout).
pub async fn auth_status() -> AuthStatus {
    auth_status_with(&resolve_bin()).await
}

pub async fn auth_status_with(bin: &std::path::Path) -> AuthStatus {
    match run(bin, &["auth".into(), "status".into()], "", &[], Duration::from_secs(15)).await {
        Err(RunError::Spawn(e)) => AuthStatus {
            logged_in: false,
            error: Some(if e.kind() == std::io::ErrorKind::NotFound { "Claude Code nie jest zainstalowany".into() } else { e.to_string() }),
        },
        Err(RunError::Timeout { stdout, .. }) if stdout.is_empty() => AuthStatus { logged_in: false, error: Some("Claude: przekroczony czas".into()) },
        Err(RunError::Timeout { stdout, .. }) => parse_auth(&stdout),
        Ok(o) if o.stdout.is_empty() && o.code != Some(0) => {
            AuthStatus { logged_in: false, error: Some(format!("Command failed: {} {}", bin.display(), head(&o.stderr, 200))) }
        }
        Ok(o) => parse_auth(&o.stdout),
    }
}

fn parse_auth(out: &str) -> AuthStatus {
    match serde_json::from_str::<Value>(out) {
        Ok(v) => AuthStatus { logged_in: v.get("loggedIn").map(truthy).unwrap_or(false), error: None },
        Err(_) => AuthStatus { logged_in: false, error: Some("nieznana odpowiedź".into()) },
    }
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Null => false,
        Value::Number(n) => n.as_f64().is_some_and(|x| x != 0.0),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::Effort;
    use serde_json::json;

    #[test]
    fn args_match_ts() {
        let o = ModelCall { model: "claude-opus-5-5".into(), system: "SYS".into(), schema: json!({"type":"object"}), effort: Some(Effort::Low), ..Default::default() };
        let a = build_args(&o);
        assert_eq!(&a[..7], ["-p", "--model", "claude-opus-5-5", "--output-format", "json", "--effort", "low"]);
        assert_eq!(a[7], "--system-prompt");
        assert!(a[8].starts_with("SYS\n\nOdpowiedz WYŁĄCZNIE jednym obiektem JSON"));
        assert!(a[8].ends_with("\n{\"type\":\"object\"}"));
        assert_eq!(&a[9..], ["--tools", "", "--setting-sources", "", "--strict-mcp-config", "--no-session-persistence"]);
        let w = build_args(&ModelCall { tools: vec!["WebSearch".into(), "WebFetch".into()], ..o });
        let i = w.iter().position(|x| x == "--tools").unwrap();
        assert_eq!(&w[i..i + 5], ["--tools", "WebSearch,WebFetch", "--allowedTools", "WebSearch", "WebFetch"]);
    }
}
