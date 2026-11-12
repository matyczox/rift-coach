//! HTTP to Riot's local endpoints (game :2999, LCU). They use a self-signed
//! Riot root CA; certificate checks are skipped, so this client refuses any
//! host other than 127.0.0.1 (same rule as `src/main/sources/http.ts`).

use anyhow::{anyhow, bail, Result};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::sync::OnceLock;
use std::time::Duration;

fn client() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .danger_accept_invalid_hostnames(true)
            .no_proxy()
            .pool_idle_timeout(Duration::from_secs(30))
            .build()
            .expect("local http client")
    })
}

pub fn check_local(url: &str) -> Result<reqwest::Url> {
    let u = reqwest::Url::parse(url)?;
    if u.host_str() != Some("127.0.0.1") {
        bail!("local endpoints only");
    }
    Ok(u)
}

/// GET JSON (default timeout 1500 ms like getJson).
pub async fn get_json<T: DeserializeOwned>(url: &str, auth: Option<&str>, timeout_ms: u64) -> Result<T> {
    let u = check_local(url)?;
    let mut req = client().get(u).timeout(Duration::from_millis(timeout_ms));
    if let Some(a) = auth {
        req = req.header("Authorization", a);
    }
    let res = req.send().await?;
    let status = res.status();
    if status.as_u16() >= 400 {
        bail!("HTTP {}", status.as_u16());
    }
    let text = res.text().await?;
    serde_json::from_str(&text).map_err(|e| anyhow!("bad JSON: {e}"))
}

/// POST/PUT/PATCH/DELETE JSON; `None` on an empty body (postJson, timeout 3000 ms default).
pub async fn send_json(method: &str, url: &str, body: &Value, auth: Option<&str>, timeout_ms: u64) -> Result<Option<Value>> {
    let u = check_local(url)?;
    let m = reqwest::Method::from_bytes(method.as_bytes())?;
    let mut req = client().request(m, u).timeout(Duration::from_millis(timeout_ms)).json(body);
    if let Some(a) = auth {
        req = req.header("Authorization", a);
    }
    let res = req.send().await?;
    let status = res.status();
    if status.as_u16() >= 400 {
        bail!("HTTP {}", status.as_u16());
    }
    let text = res.text().await?;
    if text.is_empty() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_str(&text)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_non_local_hosts() {
        assert!(check_local("https://127.0.0.1:2999/x").is_ok());
        assert!(check_local("https://localhost:2999/x").is_err());
        assert!(check_local("https://example.com/").is_err());
    }
}
