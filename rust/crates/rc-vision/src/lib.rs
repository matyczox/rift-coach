//! Rift Coach vision: minimap portrait detector, fog-of-war mask, jungle camps, health bars,
//! game-window capture and the fog tracker (particle filter). Port of the TypeScript code in
//! `src/main/vision` and `src/main/tracker`; the TS code is the spec (same parameters,
//! thresholds and algorithms).

pub mod background;
pub mod calibration;
pub mod camps;
pub mod detect;
pub mod fog;
pub mod healthbars;
pub mod image;
pub mod terrain;
