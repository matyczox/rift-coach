//! Fog tracker: where are the enemies we can't see (port of `src/main/tracker`).

pub mod alerts;
pub mod enemy;
pub mod learning;
pub mod predictor;
pub mod terrain;

pub use enemy::{EnemyInput, EnemyTrack, EnemyTracker, FrameInput, Ghost, Sighting, TrackState};
pub use learning::Learner;
pub use predictor::{Estimate, GoalKind, ParticleFilter, PriorContext, Rng, Role, Zone, MAP_UNITS};
pub use terrain::GridTerrain;
