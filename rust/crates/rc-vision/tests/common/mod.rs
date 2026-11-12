#![allow(dead_code)]
use rc_vision::image::{load_png, Rgba};
use std::path::PathBuf;

/// The repo's `tests/fixtures` (shared with the TS tests). `RC_FIXTURES` overrides.
pub fn fixtures() -> PathBuf {
    if let Ok(p) = std::env::var("RC_FIXTURES") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures")
}

pub fn load(sub: &str, f: &str) -> Rgba {
    let p = fixtures().join(sub).join(f);
    load_png(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}
