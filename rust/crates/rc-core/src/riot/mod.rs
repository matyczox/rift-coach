//! Riot Web API (personal dev key) and enemy opponent profiles (`src/main/riot/*`).
//!
//! Wiring: `RiotClient::new(RiotClientOptions { cache_dir: Some(<userData>/riot-cache), platform, .. })`
//! then `profiles::build_profiles(&client, &players, &BuildProfilesOptions::default())`.
//! The API key travels only in the `X-Riot-Token` header and never appears in
//! errors, logs or `Debug` output.

pub mod analyze;
pub mod cache;
pub mod client;
pub mod limiter;
pub mod profiles;
pub mod types;

pub use client::{CallOptions, RequestBudget, RiotClient, RiotClientOptions, RiotError, RiotHttp};
pub use profiles::{build_profiles, BuildProfilesOptions, ProfilePlayer, ProfileProgress};
pub use types::OpponentProfile;
