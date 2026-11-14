//! The white camera rectangle on the minimap. With a locked camera its centre is my champion, so it
//! stands in for my own icon when that is hidden under others in a fight. The game gives no position
//! through its API and reading its memory is anti-cheat territory — the screen is the honest source.
use crate::image::Rgba;

fn white(img: &Rgba, x: usize, y: usize) -> bool {
    let o = (y * img.width + x) * 4;
    let d = &img.data;
    d[o] > 205 && d[o + 1] > 205 && d[o + 2] > 205
}

/// Longest run of white pixels in row `y`: (start, length).
fn row_run(img: &Rgba, y: usize) -> (usize, usize) {
    let (mut best, mut cur_start, mut cur) = ((0, 0), 0, 0);
    for x in 0..img.width {
        if white(img, x, y) {
            if cur == 0 {
                cur_start = x;
            }
            cur += 1;
            if cur > best.1 {
                best = (cur_start, cur);
            }
        } else {
            cur = 0;
        }
    }
    best
}

/// Longest run of white pixels in column `x`: (start, length).
fn col_run(img: &Rgba, x: usize) -> (usize, usize) {
    let (mut best, mut cur_start, mut cur) = ((0, 0), 0, 0);
    for y in 0..img.height {
        if white(img, x, y) {
            if cur == 0 {
                cur_start = y;
            }
            cur += 1;
            if cur > best.1 {
                best = (cur_start, cur);
            }
        } else {
            cur = 0;
        }
    }
    best
}

/// Centre of the camera rectangle as minimap fractions, or None when no clear box is seen.
/// Uses the two horizontal edges (long white runs in two rows with the same x span); when the box
/// is cut by the map edge, the two vertical edges instead.
pub fn camera_center(img: &Rgba) -> Option<(f64, f64)> {
    camera_rect(img).map(|(x0, y0, x1, y1)| ((x0 + x1) / 2.0, (y0 + y1) / 2.0))
}

/// The camera rectangle (left, top, right, bottom) as minimap fractions = what the screen shows.
pub fn camera_rect(img: &Rgba) -> Option<(f64, f64, f64, f64)> {
    let (w, h) = (img.width, img.height);
    if w < 64 || h < 64 || img.data.len() < w * h * 4 {
        return None;
    }
    let min_len = (w as f64 * 0.10) as usize;
    let rows: Vec<(usize, usize, usize)> = (0..h).filter_map(|y| { let (s, l) = row_run(img, y); (l >= min_len).then_some((y, s, l)) }).collect();
    // Pair a top and a bottom edge: same span (±4 px), 6–40 % of the height apart.
    let mut best: Option<(f64, f64, f64, f64, usize)> = None;
    for (i, &(y1, s1, l1)) in rows.iter().enumerate() {
        for &(y2, s2, l2) in rows.iter().skip(i + 1) {
            let gap = y2 - y1;
            if (gap as f64) < h as f64 * 0.06 || (gap as f64) > h as f64 * 0.4 {
                continue;
            }
            if s1.abs_diff(s2) > 4 || l1.abs_diff(l2) > 6 {
                continue;
            }
            let score = l1.min(l2);
            if best.is_none_or(|b| score > b.4) {
                best = Some((s1 as f64 / w as f64, y1 as f64 / h as f64, (s1 + l1) as f64 / w as f64, y2 as f64 / h as f64, score));
            }
        }
    }
    if let Some((x0, y0, x1, y1, _)) = best {
        return Some((x0, y0, x1, y1));
    }
    let min_len_v = (h as f64 * 0.06) as usize;
    let cols: Vec<(usize, usize, usize)> = (0..w).filter_map(|x| { let (s, l) = col_run(img, x); (l >= min_len_v).then_some((x, s, l)) }).collect();
    let mut bestv: Option<(f64, f64, f64, f64, usize)> = None;
    for (i, &(x1, s1, l1)) in cols.iter().enumerate() {
        for &(x2, s2, l2) in cols.iter().skip(i + 1) {
            let gap = x2 - x1;
            if (gap as f64) < w as f64 * 0.10 || (gap as f64) > w as f64 * 0.5 {
                continue;
            }
            if s1.abs_diff(s2) > 4 || l1.abs_diff(l2) > 6 {
                continue;
            }
            let score = l1.min(l2);
            if bestv.is_none_or(|b| score > b.4) {
                bestv = Some((x1 as f64 / w as f64, s1 as f64 / h as f64, x2 as f64 / w as f64, (s1 + l1) as f64 / h as f64, score));
            }
        }
    }
    bestv.map(|(x0, y0, x1, y1, _)| (x0, y0, x1, y1))
}

/// Trinket slot (Stealth Ward) in the bottom HUD: Some(true) = has a charge (white count digit in the
/// slot's bottom-right corner), Some(false) = recharging (big white countdown in the middle), None =
/// no trinket / not readable. Measured on 2560x1440, default HUD scale: slot top-left at
/// (0.6145 W, 0.916 H), side 0.0236 H. (Checked on recorded frames: ready corner 0.21-0.23 white,
/// recharging centre 0.17-0.22 white, empty slot dark.)
pub fn trinket_ready(frame: &Rgba) -> Option<bool> {
    let (w, h) = (frame.width as f64, frame.height as f64);
    if w < 640.0 || h < 360.0 || frame.data.len() < frame.width * frame.height * 4 {
        return None;
    }
    let side = (0.0236 * h).round().max(10.0) as usize;
    let (x0, y0) = ((0.6145 * w) as usize, (0.916 * h) as usize);
    if x0 + side >= frame.width || y0 + side >= frame.height {
        return None;
    }
    let (mut cw, mut cn, mut kw, mut kn, mut lum) = (0usize, 0usize, 0usize, 0usize, 0f64);
    for dy in 0..side {
        for dx in 0..side {
            let o = ((y0 + dy) * frame.width + x0 + dx) * 4;
            let (r, g, b) = (frame.data[o], frame.data[o + 1], frame.data[o + 2]);
            let wh = r > 190 && g > 190 && b > 190;
            lum += (r as f64 + g as f64 + b as f64) / 3.0;
            let (fx, fy) = (dx as f64 / side as f64, dy as f64 / side as f64);
            if fx > 0.2 && fx < 0.8 && fy > 0.25 && fy < 0.7 {
                cn += 1;
                cw += wh as usize;
            }
            if fx > 0.7 && fy > 0.65 {
                kn += 1;
                kw += wh as usize;
            }
        }
    }
    let (center, corner) = (cw as f64 / cn.max(1) as f64, kw as f64 / kn.max(1) as f64);
    let lum = lum / (side * side) as f64;
    if corner >= 0.12 {
        Some(true)
    } else if center >= 0.12 {
        Some(false)
    } else {
        let _ = lum;
        None
    }
}

/// My own champion's health bar (green) on screen -> where I stand, screen fractions (feet ~0.135 H
/// under the bar's top). Simple and own: several rows of green HP fill starting at the same x in the
/// middle of the screen (the generic bar reader expects the enemy / ally frame and missed it). The HP
/// fill can be short (low HP); its LEFT edge is what we trust, the bar is ~125 px wide at 1440p.
pub fn my_bar(frame: &Rgba) -> Option<(f64, f64)> {
    let (w, h) = (frame.width, frame.height);
    if w < 640 || h < 360 || frame.data.len() < w * h * 4 {
        return None;
    }
    let s = h as f64 / 1440.0;
    let min_run = (14.0 * s).round().max(6.0) as usize;
    let green = |x: usize, y: usize| {
        let o = (y * w + x) * 4;
        let (r, g, b) = (frame.data[o] as i32, frame.data[o + 1] as i32, frame.data[o + 2] as i32);
        g >= 140 && g - r >= 50 && g - b >= 60
    };
    // (row, run start) of every long green run.
    let mut runs: Vec<(usize, usize)> = Vec::new();
    // Middle of the screen only: the side panels (ally portraits have green HP bars too) and the
    // bottom HUD are left out.
    let (xa, xb) = (w * 18 / 100, w * 80 / 100);
    for y in (h * 8 / 100..h * 78 / 100).step_by(2) {
        let mut x = xa;
        while x < xb {
            if green(x, y) {
                // The fill is cut by thin dark tick marks every ~100 HP: allow gaps of up to 3 px.
                let start = x;
                let mut end = x;
                while x < xb && (green(x, y) || (x + 3 < xb && (1..=3).any(|k| green(x + k, y)))) {
                    if green(x, y) {
                        end = x;
                    }
                    x += 1;
                }
                let len = end + 1 - start;
                if len >= min_run && len <= (140.0 * s) as usize {
                    runs.push((y, start));
                }
                continue;
            }
            x += 1;
        }
    }
    // Clusters: rows within ~14 px with the same start (±3 px); the best has the most rows.
    let mut best: Option<(usize, usize, usize)> = None; // (top row, start, rows)
    for &(y, x0) in &runs {
        let rows = runs.iter().filter(|&&(y2, x2)| y2 >= y && y2 <= y + (14.0 * s) as usize && x2.abs_diff(x0) <= 3).count();
        if rows >= 3 && best.is_none_or(|b| rows > b.2) {
            best = Some((y, x0, rows));
        }
    }
    best.map(|(y, x0, _)| ((x0 as f64 + 62.0 * s) / w as f64, (y as f64 + 0.135 * h as f64) / h as f64))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_with_box(w: usize, x0: usize, y0: usize, bw: usize, bh: usize) -> Rgba {
        let mut data = vec![0u8; w * w * 4];
        for p in data.chunks_mut(4) {
            p.copy_from_slice(&[40, 90, 60, 255]);
        }
        let mut put = |x: usize, y: usize| {
            if x < w && y < w {
                let o = (y * w + x) * 4;
                data[o..o + 4].copy_from_slice(&[240, 240, 240, 255]);
            }
        };
        for x in x0..x0 + bw {
            put(x, y0);
            put(x, y0 + bh);
        }
        for y in y0..=y0 + bh {
            put(x0, y);
            put(x0 + bw - 1, y);
        }
        Rgba { width: w, height: w, data }
    }

    #[test]
    fn finds_box_centre() {
        let img = frame_with_box(341, 100, 150, 80, 45);
        let (x, y) = camera_center(&img).unwrap();
        assert!((x - 140.0 / 341.0).abs() < 0.01, "{x}");
        assert!((y - 172.5 / 341.0).abs() < 0.01, "{y}");
    }

    #[test]
    fn nothing_on_plain_map() {
        assert!(camera_center(&Rgba { width: 341, height: 341, data: vec![50; 341 * 341 * 4] }).is_none());
    }
}
/// A fresh player position cannot follow a camera pan across the map.
pub fn plausible_player_step(previous: Option<(f64, f64)>, next: (f64, f64)) -> bool {
    previous.is_none_or(|p| (p.0-next.0).hypot(p.1-next.1) <= 0.3)
}

#[cfg(test)]
mod player_position_tests {
    use super::*;
    #[test]
    fn camera_pan_from_previous_game_is_not_player_motion() {
        assert!(!plausible_player_step(Some((0.874,0.650)),(0.264,0.273)));
        assert!(plausible_player_step(Some((0.874,0.650)),(0.816,0.677)));
    }
}
