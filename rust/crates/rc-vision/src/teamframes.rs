//! Ally portraits above the minimap ("team frames"): 4 slots, each with a green HP strip and
//! under it a resource strip (blue = mana, grey = no mana / other resource). They show every
//! ally's HP all the time, unlike health bars over champions, which only exist on screen.
//!
//! Layout measured on a 2560x1440 game (minimap 341 px): HP strip ~10 px tall, 60 px wide,
//! slots every 86 px, the last slot ends at the minimap's right edge, the resource strip ~1.5
//! strip heights below the HP strip top. Sizes are taken from the HP strip height, so another
//! HUD scale works too.

use crate::image::Rgba;

#[derive(Clone, Debug, PartialEq)]
pub struct AllyFrame {
    /// 0 = leftmost slot (allies in team order, me excluded).
    pub slot: usize,
    /// HP fraction 0..1 (0 = dead or empty).
    pub hp: f64,
    /// Mana fraction 0..1; None = not a mana champion (grey / other resource bar).
    pub mana: Option<f64>,
}

fn px(f: &Rgba, x: i32, y: i32) -> (u8, u8, u8) {
    let i = (y as usize * f.width + x as usize) * 4;
    (f.data[i], f.data[i + 1], f.data[i + 2])
}

fn is_green(r: u8, g: u8, b: u8) -> bool {
    g > 140 && (g as f64) > r as f64 * 1.5 && (g as f64) > b as f64 * 1.4
}

fn is_mana(r: u8, g: u8, b: u8) -> bool {
    b > 120 && (b as f64) > r as f64 * 1.6 && (b as f64) > g as f64 * 1.15
}

fn is_dark(r: u8, g: u8, b: u8) -> bool {
    (r as u32 + g as u32 + b as u32) < 90
}

/// `mm` = minimap rect (x, y, width, height) in frame pixels.
pub fn read_team_frames(f: &Rgba, mm: (i32, i32, i32, i32)) -> Vec<AllyFrame> {
    let (mx, my, mw, mh) = mm;
    let (w, h) = (f.width as i32, f.height as i32);
    let (x0, x1) = ((mx - mw / 10).max(0), (mx + mw).min(w));
    let (y0, y1) = ((my - mh * 3 / 10).max(0), my.min(h));
    if x1 - x0 < 20 || y1 - y0 < 10 {
        return Vec::new();
    }
    // HP band: the longest run of rows with plenty of green.
    let counts: Vec<i32> = (y0..y1).map(|y| (x0..x1).filter(|&x| { let (r, g, b) = px(f, x, y); is_green(r, g, b) }).count() as i32).collect();
    let max = *counts.iter().max().unwrap_or(&0);
    if max < 6 {
        return Vec::new();
    }
    let (mut best, mut run_start) = ((0usize, 0usize), None::<usize>);
    for (i, &c) in counts.iter().chain(std::iter::once(&0)).enumerate() {
        if c * 2 >= max {
            run_start.get_or_insert(i);
        } else if let Some(s) = run_start.take() {
            if i - s > best.1 - best.0 {
                best = (s, i);
            }
        }
    }
    let band_h = (best.1 - best.0) as f64;
    if !(5.0..=30.0).contains(&band_h) {
        return Vec::new();
    }
    let top = y0 + best.0 as i32;
    let (width, spacing) = (band_h * 6.0, band_h * 8.6);
    let hp_row = top + (band_h / 2.0) as i32;
    let res_row = top + (band_h * 1.5).round() as i32;
    let right = (mx + mw) as f64;
    let mut out = Vec::new();
    for k in 0..4 {
        let sx1 = right - k as f64 * spacing;
        let sx0 = sx1 - width;
        if sx0 < 0.0 || sx1 as i32 > w || res_row >= h {
            continue;
        }
        let xs = sx0.round() as i32..sx1.round() as i32;
        let n = xs.len().max(1) as f64;
        let green = xs.clone().filter(|&x| { let (r, g, b) = px(f, x, hp_row); is_green(r, g, b) }).count() as f64;
        let (mut blue, mut dark) = (0.0, 0.0);
        for x in xs {
            let (r, g, b) = px(f, x, res_row);
            if is_mana(r, g, b) {
                blue += 1.0;
            } else if is_dark(r, g, b) {
                dark += 1.0;
            }
        }
        // Mana bar = blue fill + dark empty part; grey (no mana) has neither.
        let mana = if blue > 0.0 { Some((blue / n).min(1.0)) } else if dark >= 0.9 * n && green > 0.0 { Some(0.0) } else { None };
        out.push(AllyFrame { slot: 3 - k, hp: (green / n).min(1.0), mana });
    }
    out.sort_by_key(|a| a.slot);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic HUD with the measured layout: minimap at (220, 108) 34x34 scaled x10 -> keep
    /// real sizes: HP strip 10 px, slots of 60 px every 86 px, right-aligned to the minimap edge.
    fn hud(hps: [f64; 4], mana: [Option<f64>; 4]) -> (Rgba, (i32, i32, i32, i32)) {
        let (w, h) = (700usize, 500usize);
        let mut f = Rgba { width: w, height: h, data: vec![40; w * h * 4] };
        let mm = (340, 150, 341, 341);
        let set = |f: &mut Rgba, x: i32, y: i32, c: (u8, u8, u8)| {
            let i = (y as usize * w + x as usize) * 4;
            f.data[i] = c.0;
            f.data[i + 1] = c.1;
            f.data[i + 2] = c.2;
        };
        let right = mm.0 + mm.2;
        for s in 0..4 {
            let x1 = right - (3 - s) as i32 * 86;
            let x0 = x1 - 60;
            for y in 104..114 {
                for x in x0..x1 {
                    let fill = (x - x0) as f64 / 60.0 < hps[s];
                    set(&mut f, x, y, if fill { (15, 204, 74) } else { (10, 10, 10) });
                }
            }
            for y in 117..123 {
                for x in x0..x1 {
                    let c = match mana[s] {
                        Some(m) => if ((x - x0) as f64 / 60.0) < m { (60, 150, 225) } else { (10, 12, 20) },
                        None => (62, 61, 62),
                    };
                    set(&mut f, x, y, c);
                }
            }
        }
        (f, mm)
    }

    #[test]
    fn reads_hp_and_mana_per_slot() {
        let (f, mm) = hud([0.53, 1.0, 1.0, 0.25], [Some(0.66), None, Some(1.0), Some(0.0)]);
        let got = read_team_frames(&f, mm);
        assert_eq!(got.len(), 4);
        assert!((got[0].hp - 0.53).abs() < 0.05, "{:?}", got[0]);
        assert!((got[3].hp - 0.25).abs() < 0.05);
        assert!((got[0].mana.unwrap() - 0.66).abs() < 0.05);
        assert_eq!(got[1].mana, None);
        assert!(got[2].mana.unwrap() > 0.95);
        assert_eq!(got[3].mana, Some(0.0));
    }

    #[test]
    fn nothing_when_no_frames() {
        let f = Rgba { width: 700, height: 500, data: vec![40; 700 * 500 * 4] };
        assert!(read_team_frames(&f, (340, 150, 341, 341)).is_empty());
    }
}
