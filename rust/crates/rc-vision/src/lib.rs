//! Rift Coach vision: minimap portrait detector, fog-of-war mask, jungle camps, health bars,
//! game-window capture and the fog tracker (particle filter). Port of the TypeScript code in
//! `src/main/vision` and `src/main/tracker`; the TS code is the spec (same parameters,
//! thresholds and algorithms).

pub mod background;
pub mod calibration;
pub mod camera;
#[cfg(all(windows, feature = "capture"))]
pub mod capture;
pub mod camps;
pub mod detect;
pub mod fog;
pub mod healthbars;
pub mod icons;
pub mod image;
pub mod pipeline;
pub mod teamframes;
pub mod terrain;
pub mod tracker;
pub mod waves;

pub use calibration::{Calibration, Rect};
pub use detect::{Detection, DetectParams, Hint, Team};
pub use image::Rgba;
pub use pipeline::{Roster, RosterEntry, Vision, VisionParams, VisionResult, VisionService};
pub use tracker::{EnemyInput, EnemyTrack, EnemyTracker, FrameInput, Ghost, GridTerrain, Learner};
