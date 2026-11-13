//! Where am I on a recorded full-screen frame? My green health bar + camera rectangle -> minimap spot.
//! `cargo run --release -p rc-vision --example check_me -- frame.png [x y w h]` (minimap rect in px)
use rc_vision::camera::{camera_rect, my_bar, trinket_ready};
use rc_vision::healthbars::{read_health_bars, BarColor, ReadOptions};
use rc_vision::image::decode_png;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let frame = decode_png(&std::fs::read(&args[1]).expect("read")).expect("png");
    let r: Vec<usize> = if args.len() >= 6 { args[2..6].iter().map(|s| s.parse().unwrap()).collect() } else { vec![2205, 1083, 341, 341] };
    let mm = frame.crop(r[0], r[1], r[2], r[3]);
    let rect = camera_rect(&mm);
    let (fw, fh) = (frame.width as f64, frame.height as f64);
    let bars = read_health_bars(&frame, &ReadOptions::new(BarColor::Green));
    let _ = (fw, fh);
    let me = my_bar(&frame);
    println!("camera rect {rect:?}");
    println!("green bars {:?}", bars.iter().map(|b| (b.x, b.y, b.width, (b.confidence * 100.0).round())).collect::<Vec<_>>());
    println!("me on screen {me:?}");
    if let (Some((sx, sy)), Some((x0, y0, x1, y1))) = (me, rect) {
        println!("me on minimap ({:.3}, {:.3})", x0 + sx * (x1 - x0), y0 + sy * (y1 - y0));
    }
    println!("trinket ready {:?}", trinket_ready(&frame));
}
