//! Injectable model caller. The research / build / review modules take a
//! `&dyn ModelCaller` instead of calling [`call_model`] directly, so the app
//! passes [`CliCaller`] (Claude Code / Codex CLI) and tests pass a fake that
//! returns canned JSON and records the calls.

use super::{call_model, CallResult, ModelCall};
use anyhow::Result;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait ModelCaller: Send + Sync {
    /// Run one model call; the answer is the parsed JSON object.
    fn call<'a>(&'a self, c: &'a ModelCall) -> BoxFuture<'a, Result<CallResult<Value>>>;
}

/// The real thing: `claude -p` / `codex exec` picked by model name (`callModel`).
#[derive(Clone, Copy, Debug, Default)]
pub struct CliCaller;

impl ModelCaller for CliCaller {
    fn call<'a>(&'a self, c: &'a ModelCall) -> BoxFuture<'a, Result<CallResult<Value>>> {
        Box::pin(call_model::<Value>(c))
    }
}

/// Test helper: answers every call with `answer` (or an error) and keeps the calls.
pub struct FakeCaller {
    pub answer: std::sync::Mutex<Result<Value, String>>,
    pub calls: std::sync::Mutex<Vec<ModelCall>>,
}

impl FakeCaller {
    pub fn new(answer: Value) -> Self {
        FakeCaller { answer: std::sync::Mutex::new(Ok(answer)), calls: Default::default() }
    }
    pub fn failing(msg: &str) -> Self {
        FakeCaller { answer: std::sync::Mutex::new(Err(msg.to_string())), calls: Default::default() }
    }
    pub fn calls(&self) -> Vec<ModelCall> {
        self.calls.lock().unwrap().clone()
    }
}

impl ModelCaller for FakeCaller {
    fn call<'a>(&'a self, c: &'a ModelCall) -> BoxFuture<'a, Result<CallResult<Value>>> {
        self.calls.lock().unwrap().push(c.clone());
        let a = self.answer.lock().unwrap().clone();
        Box::pin(async move {
            match a {
                Ok(v) => Ok(CallResult { value: v.clone(), latency_ms: 1, raw: v }),
                Err(e) => Err(anyhow::anyhow!(e)),
            }
        })
    }
}
