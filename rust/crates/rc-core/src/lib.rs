//! Rift Coach core logic (no UI): settings, local data sources (Live Client
//! API, LCU, Replay API), LLM calls (Claude Code / Codex CLI) and the shapes
//! shared with the React UI.

pub mod coach;
pub mod files;
pub mod intel;
pub mod llm;
pub mod settings;
pub mod sources;
pub mod types;
pub mod wards;
