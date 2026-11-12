//! One entry point for every coach model (`src/main/coach/llm.ts`):
//!   claude-*          -> Claude Code (`claude -p`, Claude subscription)
//!   gpt-* / codex:*   -> Codex CLI (`codex exec`, ChatGPT/Codex subscription)

pub mod caller;
pub mod claude;
pub mod codex;
pub mod profiles;

use anyhow::{anyhow, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl Effort {
    pub fn as_str(self) -> &'static str {
        match self {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
            Effort::Xhigh => "xhigh",
            Effort::Max => "max",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ModelCall {
    pub model: String,
    pub system: String,
    pub prompt: String,
    pub schema: Value,
    pub timeout_ms: Option<u64>,
    /// Built-in tools to allow (e.g. WebSearch, WebFetch). Default: none.
    pub tools: Vec<String>,
    /// Extended-thinking budget (Claude). None = CLI default.
    pub thinking_tokens: Option<u32>,
    pub effort: Option<Effort>,
}

#[derive(Clone, Debug)]
pub struct CallResult<T> {
    pub value: T,
    pub latency_ms: u64,
    pub raw: Value,
}

pub fn is_codex_model(m: &str) -> bool {
    m.starts_with("gpt-") || m.starts_with("codex:")
}

/// callModel: route to Codex or Claude by model name.
pub async fn call_model<T: DeserializeOwned>(opts: &ModelCall) -> Result<CallResult<T>> {
    let r = if is_codex_model(&opts.model) { codex::call_codex(opts).await? } else { claude::call_claude(opts).await? };
    let value = serde_json::from_value(r.value.clone()).map_err(|e| anyhow!("odpowiedź niezgodna ze schematem: {e}"))?;
    Ok(CallResult { value, latency_ms: r.latency_ms, raw: r.raw })
}

/// First top-level JSON object in the text (tolerates code fences or stray words): extractJson.
pub fn extract_json(s: &str) -> Result<&str> {
    let start = s.find('{');
    let end = s.rfind('}');
    match (start, end) {
        (Some(a), Some(b)) if b > a => Ok(&s[a..=b]),
        _ => Err(anyhow!("no JSON object")),
    }
}

/// `windowsHide: true`.
pub fn hide_window(c: &mut tokio::process::Command) {
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = c;
}

/// JS-like `s.slice(-n)` / `s.slice(0, n)` on characters.
pub(crate) fn tail(s: &str, n: usize) -> String {
    let c: Vec<char> = s.chars().collect();
    c[c.len().saturating_sub(n)..].iter().collect()
}
pub(crate) fn head(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_json_like_ts() {
        assert_eq!(extract_json("```json\n{\"a\":{\"b\":1}}\n```").unwrap(), "{\"a\":{\"b\":1}}");
        assert_eq!(extract_json("Sure! {\"x\":1} done").unwrap(), "{\"x\":1}");
        assert!(extract_json("no json").is_err());
        assert!(extract_json("} {").is_err());
    }

    #[test]
    fn routing() {
        assert!(is_codex_model("gpt-6.1-terra"));
        assert!(is_codex_model("codex:gpt-6.1-sol"));
        assert!(!is_codex_model("claude-opus-5-5"));
    }

    #[test]
    fn js_slices() {
        assert_eq!(tail("zażółć", 3), "ółć");
        assert_eq!(head("zażółć", 3), "zaż");
    }
}
