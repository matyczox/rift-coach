//! Capture one frame of a window by exact title (default: the LoL game client) and save it
//! (plus the minimap crop when a calibration is given) as PNG. Sends no input, takes no focus.
//!
//! usage: capture_probe [title] [out.png] [x y w h winW winH]

use rc_vision::calibration::{Calibration, Rect};
use rc_vision::capture::{WindowCapture, GAME_WINDOW_TITLE};
use rc_vision::image::encode_png;
use std::time::{Duration, Instant};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let title = args.first().cloned().unwrap_or_else(|| GAME_WINDOW_TITLE.into());
    let out = args.get(1).cloned().unwrap_or_else(|| "capture.png".into());
    let t0 = Instant::now();
    let mut cap = WindowCapture::new(&title).unwrap_or_else(|e| panic!("{e}"));
    let f = cap.grab(Duration::from_secs(3)).unwrap_or_else(|e| panic!("{e}"));
    println!("first frame {}x{} after {:.0} ms", f.image.width, f.image.height, t0.elapsed().as_secs_f64() * 1e3);
    let n = 20;
    let t1 = Instant::now();
    for _ in 0..n {
        cap.grab(Duration::from_secs(1)).unwrap();
    }
    println!("grab(): {:.2} ms per call (incl. GPU->CPU copy + BGRA->RGBA when a new frame arrived)", t1.elapsed().as_secs_f64() * 1e3 / n as f64);
    std::fs::write(&out, encode_png(&f.image)).unwrap();
    println!("saved {out}");
    if args.len() >= 8 {
        let v: Vec<i32> = args[2..8].iter().map(|s| s.parse().unwrap()).collect();
        let cal = Calibration { rect: Rect { x: v[0], y: v[1], width: v[2], height: v[3] }, window_width: v[4], window_height: v[5] };
        if let Some(m) = f.minimap(&cal) {
            let p = out.replace(".png", "-minimap.png");
            std::fs::write(&p, encode_png(&m)).unwrap();
            println!("minimap {}x{} saved {p}", m.width, m.height);
        }
    }
}
