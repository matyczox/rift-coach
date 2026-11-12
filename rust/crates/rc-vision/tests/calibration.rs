// Port of the minimap-calibration part of tests/core.test.ts.
mod common;
use common::fixtures;
use flate2::read::GzDecoder;
use rc_vision::calibration::{find_minimap, scale_rect, Rect};
use std::io::Read;

#[test]
fn finds_the_minimap_on_a_real_1600x900_frame() {
    let gz = std::fs::read(fixtures().join("scene-1600x900-br.rgba.gz")).unwrap();
    let mut raw = Vec::new();
    GzDecoder::new(&gz[..]).read_to_end(&mut raw).unwrap();
    let nl = raw.iter().position(|&b| b == b'\n').unwrap();
    let h: serde_json::Value = serde_json::from_slice(&raw[..nl]).unwrap();
    let g = |k: &str| h[k].as_u64().unwrap() as usize;
    let (fw, fh, w, hh, ox, oy) = (g("fullWidth"), g("fullHeight"), g("width"), g("height"), g("offsetX"), g("offsetY"));
    let crop = &raw[nl + 1..];
    let mut full = vec![0u8; fw * fh * 4];
    for y in 0..hh {
        full[((y + oy) * fw + ox) * 4..((y + oy) * fw + ox + w) * 4].copy_from_slice(&crop[y * w * 4..(y + 1) * w * 4]);
    }
    let r = find_minimap(&full, fw, fh).expect("minimap found");
    assert!((r.rect.x - 1370).abs() <= 4, "{:?}", r);
    assert!((r.rect.y - 669).abs() <= 4, "{:?}", r);
    assert!((r.rect.width - 221).abs() <= 6, "{:?}", r);
}

#[test]
fn null_on_featureless_frame() {
    assert!(find_minimap(&vec![90u8; 640 * 360 * 4], 640, 360).is_none());
}

#[test]
fn scales_calibration_anchored_bottom_right() {
    let r = scale_rect(Rect { x: 1379, y: 678, width: 211, height: 211 }, 1600, 900, 1920, 1080);
    assert_eq!(r.width, 253);
    assert_eq!(r.x + r.width, 1920 - 12);
    assert_eq!(r.y + r.height, 1080 - 13);
}
