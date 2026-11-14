//! Rift Coach core logic (no UI): settings, local data sources (Live Client
//! API, LCU, Replay API), LLM calls (Claude Code / Codex CLI) and the shapes
//! shared with the React UI.

pub mod build_advisor;
pub mod coach;
pub mod files;
pub mod harvest;
pub mod hud_policy;
pub mod intel;
pub mod jsutil;
pub mod llm;
pub mod patch_brief;
pub mod recorder;
pub mod replayer;
pub mod research;
pub mod review;
pub mod riot;
pub mod self_train;
pub mod runes;
pub mod settings;
pub mod sources;
pub mod types;
pub mod teamplay;
pub mod wards;
