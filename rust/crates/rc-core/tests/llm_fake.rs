//! callModel against a fake claude/codex executable (tests/fake_llm.rs).

use rc_core::llm::{claude, codex, Effort, ModelCall};
use serde_json::{json, Value};
use std::path::Path;

fn fake() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_rc-fake-llm"))
}

fn call(model: &str, prompt: &str) -> ModelCall {
    ModelCall {
        model: model.into(),
        system: "SYSTEM".into(),
        prompt: prompt.into(),
        schema: json!({ "type": "object" }),
        timeout_ms: Some(5000),
        thinking_tokens: Some(0),
        effort: Some(Effort::Medium),
        ..Default::default()
    }
}

#[tokio::test]
async fn claude_round_trip() {
    let r = claude::call_claude_with(fake(), &call("claude-sonnet-5-5", "Stan meczu: {}")).await.unwrap();
    let v = &r.value;
    assert_eq!(v["stdin"], "Stan meczu: {}");
    assert_eq!(v["thinking"], "0");
    let args: Vec<&str> = v["args"].as_array().unwrap().iter().map(|a| a.as_str().unwrap()).collect();
    assert_eq!(&args[..7], ["-p", "--model", "claude-sonnet-5-5", "--output-format", "json", "--effort", "medium"]);
    assert!(args.contains(&"--no-session-persistence"));
    assert_eq!(r.raw["usage"]["input_tokens"], 1);
}

#[tokio::test]
async fn claude_errors() {
    let e = claude::call_claude_with(fake(), &call("claude-x", "FAIL")).await.unwrap_err();
    assert_eq!(e.to_string(), "Claude: limit wyczerpany");
    let e = claude::call_claude_with(fake(), &call("claude-x", "GARBAGE")).await.unwrap_err();
    assert!(e.to_string().starts_with("Claude: nieczytelna odpowiedź"), "{e}");
    let mut c = call("claude-x", "SLEEP");
    c.timeout_ms = Some(500);
    let t = std::time::Instant::now();
    let e = claude::call_claude_with(fake(), &c).await.unwrap_err();
    assert!(e.to_string().starts_with("Claude: przekroczony czas"), "{e}");
    assert!(t.elapsed().as_secs() < 5, "child killed on timeout");
    let e = claude::call_claude_with(Path::new("Z:/nope/claude.exe"), &call("claude-x", "x")).await;
    assert!(e.is_err());
}

#[tokio::test]
async fn codex_round_trip_and_errors() {
    let r = codex::call_codex_with(fake(), &call("codex:gpt-6.1-terra", "Stan")).await.unwrap();
    let v: &Value = &r.value;
    let args: Vec<&str> = v["args"].as_array().unwrap().iter().map(|a| a.as_str().unwrap()).collect();
    assert_eq!(&args[..5], ["exec", "-m", "gpt-6.1-terra", "-c", "model_reasoning_effort=\"medium\""]);
    assert_eq!(args.last(), Some(&"-"));
    let stdin = v["stdin"].as_str().unwrap();
    assert!(stdin.starts_with("SYSTEM\n\nNie uruchamiaj poleceń powłoki"));
    assert!(stdin.ends_with("---\nStan"));
    // Temp dir removed afterwards.
    let dir = args[args.iter().position(|a| *a == "-C").unwrap() + 1];
    assert!(!Path::new(dir).exists());

    let e = codex::call_codex_with(fake(), &call("gpt-6.1-terra", "CODEFAIL")).await.unwrap_err();
    assert!(e.to_string().starts_with("Codex: kod 3 boom"), "{e}");
    let mut c = call("gpt-6.1-terra", "SLEEP");
    c.timeout_ms = Some(500);
    assert_eq!(codex::call_codex_with(fake(), &c).await.unwrap_err().to_string(), "Codex: przekroczony czas");
}

#[tokio::test]
async fn auth_status() {
    let a = claude::auth_status_with(fake()).await;
    assert!(a.logged_in);
    let a = claude::auth_status_with(Path::new("Z:/nope/claude.exe")).await;
    assert!(!a.logged_in);
    assert_eq!(a.error.as_deref(), Some("Claude Code nie jest zainstalowany"));
}
