//! Champion health bars drawn above champions' heads (port of `src/main/vision/healthbars.ts`).
//!
//! Geometry (measured on 2560x1440, scaled by frame height):
//!   [lvl box 26px] gap [frame | HP fill area 126px x 12px | frame]
//! Only what the player can see: off-screen / fogged champions are simply absent.

use crate::image::{js_round, Rgba};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BarColor {
    Red,
    Blue,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HealthBar {
    /// Left edge of the HP fill area (screen px).
    pub x: i32,
    /// Top row of the HP fill (screen px).
    pub y: i32,
    /// Width of the full HP fill area (screen px).
    pub width: i32,
    /// HP fraction 0..1 of the full bar.
    pub pct: f64,
    /// Shield fraction 0..1.
    pub shield_pct: f64,
    /// HP + shield fill the whole bar (pct is then a lower bound).
    pub shield_compressed: bool,
    pub level: Option<u32>,
    pub confidence: f64,
    /// Spell resource fraction from the thin strip right under the HP fill: blue = mana, yellow =
    /// energy (None = no such strip: fury / no resource, or not readable).
    #[serde(default)]
    pub mana: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReadOptions {
    pub enemy_color: BarColor,
    /// Minimum confidence to report (default 0.5).
    pub min_confidence: f64,
    /// Height of the full game window the frame comes from (None = frame height).
    pub screen_height: Option<usize>,
}

impl ReadOptions {
    pub fn new(enemy_color: BarColor) -> Self {
        ReadOptions { enemy_color, min_confidence: 0.5, screen_height: None }
    }
}

const REF_H: f64 = 1440.0;
const INNER_W: f64 = 125.0;
const FILL_H: f64 = 12.0;
const RIGHT_FRAME: f64 = 128.0;
const LEFT_FRAME: f64 = 3.0;
const BORDER_TOP: f64 = 4.0;
const BOX_L: f64 = 30.0;
const BOX_R: f64 = 7.0;
const BOX_T: f64 = 5.0;
const BOX_B: f64 = 21.0;

#[inline]
fn is_red(r: u8, g: u8, b: u8) -> bool {
    let (r, g, b) = (r as i32, g as i32, b as i32);
    r >= 110 && r - g >= 60 && r - b >= 60
}
#[inline]
fn is_blue(r: u8, g: u8, b: u8) -> bool {
    let (r, g, b) = (r as i32, g as i32, b as i32);
    b >= 150 && b - r >= 90 && g >= 90 && g < b
}
#[inline]
fn is_shield(r: u8, g: u8, b: u8) -> bool {
    let mx = r.max(g).max(b) as i32;
    let mn = r.min(g).min(b) as i32;
    mn >= 130 && mx - mn <= 45
}
#[inline]
fn is_frame(r: u8, g: u8, b: u8) -> bool {
    let mx = r.max(g).max(b) as i32;
    let mn = r.min(g).min(b) as i32;
    mn >= 45 && mx <= 140 && mx - mn <= 14
}

#[inline]
fn rnd(v: f64) -> i32 {
    js_round(v) as i32
}

struct Ctx<'a> {
    w: i32,
    h: i32,
    data: &'a [u8],
    fill: fn(u8, u8, u8) -> bool,
}

impl Ctx<'_> {
    #[inline]
    fn px(&self, x: i32, y: i32) -> usize {
        ((y * self.w + x) * 4) as usize
    }
    #[inline]
    fn rgb(&self, x: i32, y: i32) -> (u8, u8, u8) {
        let o = self.px(x, y);
        (self.data[o], self.data[o + 1], self.data[o + 2])
    }
    #[inline]
    fn is_fill_at(&self, x: i32, y: i32) -> bool {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return false;
        }
        let (r, g, b) = self.rgb(x, y);
        (self.fill)(r, g, b)
    }
    fn frame_near(&self, x: i32, y: i32) -> bool {
        for d in -1..=1 {
            let xx = x + d;
            if xx < 0 || xx >= self.w {
                continue;
            }
            let (r, g, b) = self.rgb(xx, y);
            if is_frame(r, g, b) {
                return true;
            }
        }
        false
    }
    fn frame_near_v(&self, x: i32, y: i32) -> bool {
        for d in -1..=1 {
            let yy = y + d;
            if yy < 0 || yy >= self.h {
                continue;
            }
            let (r, g, b) = self.rgb(x, yy);
            if is_frame(r, g, b) {
                return true;
            }
        }
        false
    }
}

/// Find all champion health bars of the given colour (live with relative colours: enemies red).
pub fn read_health_bars(frame: &Rgba, opts: &ReadOptions) -> Vec<HealthBar> {
    let (w, h) = (frame.width as i32, frame.height as i32);
    let s = opts.screen_height.unwrap_or(frame.height) as f64 / REF_H;
    let ctx = Ctx { w, h, data: &frame.data, fill: if opts.enemy_color == BarColor::Red { is_red } else { is_blue } };
    let fill_h = FILL_H * s;
    let step = ((fill_h / 2.0).floor() as i32).max(2);
    let inner_w = rnd(INNER_W * s);
    let box_l = rnd(BOX_L * s);
    let mut out: Vec<HealthBar> = Vec::new();
    let mut seen: HashSet<i64> = HashSet::new();

    let x_min = box_l + 2;
    let x_max = w - rnd(RIGHT_FRAME * s) - 2;
    let mut y = 2;
    while y < h - 2 {
        let mut prev_fill = false;
        let mut x = x_min;
        while x < x_max {
            let (r, g, b) = ctx.rgb(x, y);
            let f = (ctx.fill)(r, g, b);
            if f && !prev_fill {
                let x0 = if ctx.is_fill_at(x - 1, y) { x - 1 } else { x };
                if let Some(bar) = verify(&ctx, frame, x0, y, s, fill_h, inner_w, box_l, opts.min_confidence, &mut seen) {
                    x = bar.x + inner_w;
                    out.push(bar);
                    prev_fill = false;
                    x += 2;
                    continue;
                }
            }
            prev_fill = f;
            x += 2;
        }
        y += step;
    }
    for b in &mut out {
        b.mana = mana_below(frame, b, fill_h, s);
    }
    out.sort_by(|a, b| a.y.cmp(&b.y).then(a.x.cmp(&b.x)));
    out
}

/// The resource strip sits a few px under the HP fill, same left edge and width. Take the row with
/// the most mana-blue pixels in that band; no blue at all -> None (energy / no resource / unreadable;
/// an empty mana strip looks like the dark frame, so 0 mana is not claimed from darkness).
fn mana_below(frame: &Rgba, b: &HealthBar, fill_h: f64, s: f64) -> Option<f64> {
    let (w, h) = (frame.width as i32, frame.height as i32);
    let y0 = b.y + fill_h.round() as i32;
    let y1 = y0 + (8.0 * s).ceil() as i32;
    let mut best = 0;
    for y in y0.max(0)..y1.min(h) {
        let mut n = 0;
        for x in b.x.max(0)..(b.x + b.width).min(w) {
            let i = (y as usize * frame.width + x as usize) * 4;
            let (r, g, bl) = (frame.data[i] as f64, frame.data[i + 1] as f64, frame.data[i + 2] as f64);
            let mana = bl > 110.0 && bl > r * 1.5 && bl > g * 1.1;
            // Energy (Akali, Lee Sin, Zed...): yellow.
            let energy = r > 170.0 && g > 130.0 && bl < 90.0 && r > bl * 2.2;
            if mana || energy {
                n += 1;
            }
        }
        best = best.max(n);
    }
    (best >= 2).then(|| (best as f64 / b.width.max(1) as f64).min(1.0))
}

#[allow(clippy::too_many_arguments)]
fn verify(ctx: &Ctx, frame: &Rgba, x0: i32, y0: i32, s: f64, fill_h: f64, inner_w: i32, box_l: i32, min_conf: f64, seen: &mut HashSet<i64>) -> Option<HealthBar> {
    let (w, h, data) = (ctx.w, ctx.h, ctx.data);
    if ctx.is_fill_at(x0 - 1, y0) {
        return None;
    }
    if !ctx.frame_near(x0 - rnd(LEFT_FRAME * s), y0) {
        return None;
    }
    let cx = x0 + 1;
    let (mut top, mut bot) = (y0, y0);
    let lim = (fill_h * 1.6).ceil() as i32;
    while top > 0 && y0 - top < lim && (ctx.is_fill_at(cx, top - 1) || ctx.is_fill_at(x0, top - 1)) {
        top -= 1;
    }
    while bot < h - 1 && bot - y0 < lim && (ctx.is_fill_at(cx, bot + 1) || ctx.is_fill_at(x0, bot + 1)) {
        bot += 1;
    }
    let bh = bot - top + 1;
    if (bh as f64) < fill_h * 0.7 || (bh as f64) > fill_h * 1.4 {
        return None;
    }
    let key = rnd(x0 as f64 / 3.0) as i64 * 100000 + rnd(top as f64 / 3.0) as i64;
    if seen.contains(&key) {
        return None;
    }

    let (mut score, mut total, mut worst) = (0f64, 0f64, 1f64);
    // (1) left edge consistency
    {
        let (mut ok, mut n) = (0f64, 0f64);
        for y in top..=bot {
            n += 1.0;
            if (ctx.is_fill_at(x0, y) || ctx.is_fill_at(x0 + 1, y)) && !ctx.is_fill_at(x0 - 2, y) {
                ok += 1.0;
            }
        }
        score += ok / n;
        total += 1.0;
        worst = worst.min(ok / n);
    }
    // (2) grey frame left of the fill and on the right end
    let lf = x0 - rnd(LEFT_FRAME * s);
    let rf = x0 + rnd(RIGHT_FRAME * s);
    {
        let (mut ok, mut n) = (0f64, 0f64);
        for y in top..=bot {
            n += 2.0;
            if ctx.frame_near(lf, y) {
                ok += 1.0;
            }
            if ctx.frame_near(rf, y) {
                ok += 1.0;
            }
        }
        score += ok / n;
        total += 1.0;
        worst = worst.min(ok / n);
    }
    // (3) top grey border row spanning the bar
    {
        let ty = top - rnd(BORDER_TOP * s);
        let (mut ok, mut n) = (0f64, 0f64);
        let mut x = x0;
        while x < x0 + inner_w {
            n += 1.0;
            if ctx.frame_near_v(x, ty) {
                ok += 1.0;
            }
            x += 3;
        }
        score += ok / n;
        total += 1.0;
        worst = worst.min(ok / n);
    }
    // (4) dark level box left of the bar
    let bx0 = x0 - box_l;
    let bx1 = x0 - rnd(BOX_R * s);
    let by0 = top - rnd(BOX_T * s);
    let by1 = top + rnd(BOX_B * s);
    {
        let (mut dark, mut n) = (0f64, 0f64);
        let mut y = by0;
        while y <= by1 {
            if y >= 0 && y < h {
                let mut x = bx0 + 2;
                while x <= bx1 - 2 {
                    let o = ctx.px(x, y);
                    n += 1.0;
                    let (r, g, b) = (data[o] as i32, data[o + 1] as i32, data[o + 2] as i32);
                    if r + g + b < 140 || r.min(g).min(b) > 150 {
                        dark += 1.0;
                    }
                    x += 2;
                }
            }
            y += 2;
        }
        let v = if n > 0.0 { dark / n } else { 0.0 };
        score += v;
        total += 1.0;
        worst = worst.min(v);
    }

    // Measurement: rows in the lower part of the fill (below the segment ticks).
    let x_end = (w - 1).min(x0 + inner_w - 1);
    let mut last = x0;
    let mut shield_end = -1;
    let rows = [top + rnd(bh as f64 * 0.7), top + rnd(bh as f64 * 0.8)];
    let gap_max = (3.0 * s).max(2.0);
    for &ry in &rows {
        let (mut l, mut gap) = (x0, 0f64);
        for x in x0..=x_end {
            if ctx.is_fill_at(x, ry) {
                l = x;
                gap = 0.0;
            } else {
                gap += 1.0;
                if gap > gap_max {
                    break;
                }
            }
        }
        if l > last {
            last = l;
        }
    }
    for &ry in &rows {
        let (mut e, mut gap) = (-1, 0f64);
        for x in last + 1..=x_end {
            let (r, g, b) = ctx.rgb(x, ry);
            if is_shield(r, g, b) {
                e = x;
                gap = 0.0;
            } else {
                gap += 1.0;
                if gap > gap_max {
                    break;
                }
            }
        }
        if e > shield_end {
            shield_end = e;
        }
    }
    // (5) fill solidity, (6) the empty remainder is near-black
    {
        let (mut ok, mut n) = (0f64, 0f64);
        for x in x0..=last {
            n += 1.0;
            let mut c = 0;
            for y in top..=bot {
                if ctx.is_fill_at(x, y) {
                    c += 1;
                }
            }
            if c as f64 >= bh as f64 * 0.25 {
                ok += 1.0;
            }
        }
        let solid = if n > 0.0 { ok / n } else { 0.0 };
        score += solid;
        total += 1.0;
        worst = worst.min(solid);
        let e0 = last.max(shield_end) + 2;
        let e1 = x_end - 1;
        if e1 - e0 >= 3 {
            let (mut dk, mut m) = (0f64, 0f64);
            for &ry in &rows {
                for x in e0..=e1 {
                    let (r, g, b) = ctx.rgb(x, ry);
                    m += 1.0;
                    if r.max(g).max(b) <= 50 {
                        dk += 1.0;
                    }
                }
            }
            score += dk / m;
            total += 1.0;
            worst = worst.min(dk / m);
        }
    }
    let confidence = score / total;
    if confidence < min_conf || worst < 0.6 {
        return None;
    }
    seen.insert(key);
    let pct = ((last - x0 + 1) as f64 / inner_w as f64).min(1.0);
    let shield_pct = if shield_end > last { (1.0 - pct).min((shield_end - last) as f64 / inner_w as f64) } else { 0.0 };
    let shield_compressed = shield_pct >= 0.02 && pct + shield_pct >= 0.97;
    let level = read_level(frame, x0, top, Some(s));
    Some(HealthBar { x: x0, y: top, width: inner_w, pct, shield_pct, shield_compressed, level, confidence, mana: None })
}

// ---- level digits ----------------------------------------------------------------------

pub const GLYPH_W: usize = 6;
pub const GLYPH_H: usize = 9;

/// Extract digit glyphs from the level box as GLYPH_W x GLYPH_H grids of 0..1 brightness.
pub fn level_glyphs(frame: &Rgba, bar_x: i32, bar_y: i32, scale: Option<f64>) -> Vec<Vec<f64>> {
    let (w, h) = (frame.width as i32, frame.height as i32);
    let data = &frame.data;
    let s = scale.unwrap_or(h as f64 / REF_H);
    let x0 = bar_x - rnd(BOX_L * s);
    let x1 = bar_x - rnd(BOX_R * s);
    let y0 = bar_y - rnd(BOX_T * s);
    let y1 = bar_y + rnd(BOX_B * s);
    if x0 < 0 || y0 < 0 || x1 >= w || y1 >= h {
        return Vec::new();
    }
    let bw = (x1 - x0 + 1) as usize;
    let bh = (y1 - y0 + 1) as usize;
    let mut v = vec![0f32; bw * bh];
    for y in 0..bh {
        for x in 0..bw {
            let o = (((y0 as usize + y) * w as usize) + x0 as usize + x) * 4;
            let mn = data[o].min(data[o + 1]).min(data[o + 2]) as f64;
            v[y * bw + x] = ((mn - 70.0) / 130.0).clamp(0.0, 1.0) as f32;
        }
    }
    let on = |x: usize, y: usize| v[y * bw + x] > 0.45;
    let col_on: Vec<bool> = (0..bw).map(|x| (0..bh).any(|y| on(x, y))).collect();
    let (mut r_top, mut r_bot) = (-1i32, -1i32);
    for y in 0..bh {
        if (0..bw).any(|x| on(x, y)) {
            if r_top < 0 {
                r_top = y as i32;
            }
            r_bot = y as i32;
        }
    }
    if r_top < 0 {
        return Vec::new();
    }
    let gh = (r_bot - r_top + 1) as f64;
    if gh < 6.0 * s || gh > 14.0 * s {
        return Vec::new();
    }
    let mut groups: Vec<(usize, usize)> = Vec::new();
    let mut x = 0;
    while x < bw {
        if !col_on[x] {
            x += 1;
            continue;
        }
        let mut e = x;
        while e + 1 < bw && col_on[e + 1] {
            e += 1;
        }
        groups.push((x, e));
        x = e + 1;
    }
    let mut glyphs = Vec::new();
    for &(a, b) in &groups {
        let gw = (b - a + 1) as f64;
        if gw > gh {
            return Vec::new();
        }
        let cell_w = gh * (GLYPH_W as f64 / GLYPH_H as f64);
        let cx = (a + b + 1) as f64 / 2.0;
        let mut g = Vec::with_capacity(GLYPH_W * GLYPH_H);
        for gy in 0..GLYPH_H {
            for gx in 0..GLYPH_W {
                let sx0 = cx - cell_w / 2.0 + (gx as f64 * cell_w) / GLYPH_W as f64;
                let sx1 = sx0 + cell_w / GLYPH_W as f64;
                let sy0 = r_top as f64 + (gy as f64 * gh) / GLYPH_H as f64;
                let sy1 = sy0 + gh / GLYPH_H as f64;
                let (mut acc, mut n) = (0f64, 0f64);
                let mut yy = sy0.floor() as i64;
                while (yy as f64) < sy1.ceil() {
                    let wy = ((yy + 1) as f64).min(sy1) - (yy as f64).max(sy0);
                    let mut xx = sx0.floor() as i64;
                    while (xx as f64) < sx1.ceil() {
                        let wgt = wy * (((xx + 1) as f64).min(sx1) - (xx as f64).max(sx0));
                        n += wgt;
                        if xx >= 0 && (xx as usize) < bw && yy >= 0 && (yy as usize) < bh {
                            acc += wgt * v[yy as usize * bw + xx as usize] as f64;
                        }
                        xx += 1;
                    }
                    yy += 1;
                }
                g.push(if n != 0.0 { acc / n } else { 0.0 });
            }
        }
        glyphs.push(g);
    }
    if (1..=2).contains(&glyphs.len()) {
        glyphs
    } else {
        Vec::new()
    }
}

/// Digit templates (GLYPH_W x GLYPH_H, row-major, intensity 0..9).
const DIGIT_TEMPLATES: [(u32, &str); 10] = [
    (0, "048830484683861177840068840068850068861067584473058830"),
    (1, "003840038950046850012850002850002850002850002850005960"),
    (2, "168951784585850088750186010581002830017610179322699998"),
    (3, "158861004486001176006672118882002277000068883377278862"),
    (4, "002992016992027492165292371293673495577897011393000393"),
    (5, "289984384331471000586510357882001387210178763485389841"),
    (6, "015850157300384210787773872277860058760058584385158841"),
    (7, "888898432486000482000661003830005710028500049300169300"),
    (8, "158861574486771176386672278882662277860068883377278862"),
    (9, "268861662386840068840068773388267796001582014840168400"),
];

fn normalise(g: &[f64]) -> Vec<f64> {
    let m = g.iter().sum::<f64>() / g.len() as f64;
    let c: Vec<f64> = g.iter().map(|v| v - m).collect();
    let mut n = c.iter().map(|v| v * v).sum::<f64>().sqrt();
    if n == 0.0 {
        n = 1.0;
    }
    c.iter().map(|v| v / n).collect()
}

fn digit_templates() -> &'static Vec<(u32, Vec<f64>)> {
    static T: std::sync::OnceLock<Vec<(u32, Vec<f64>)>> = std::sync::OnceLock::new();
    T.get_or_init(|| DIGIT_TEMPLATES.iter().map(|(d, s)| (*d, normalise(&s.bytes().map(|c| (c - b'0') as f64 / 9.0).collect::<Vec<_>>()))).collect())
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphClass {
    pub d: u32,
    pub corr: f64,
    pub margin: f64,
}

/// Nearest digit by normalised cross-correlation.
pub fn classify_glyph(glyph: &[f64]) -> Option<GlyphClass> {
    let g = normalise(glyph);
    let (mut best, mut bc, mut second) = (None, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for (d, t) in digit_templates() {
        let c: f64 = g.iter().zip(t).map(|(a, b)| a * b).sum();
        if c > bc {
            second = bc;
            bc = c;
            best = Some(*d);
        } else if c > second {
            second = c;
        }
    }
    best.map(|d| GlyphClass { d, corr: bc, margin: bc - second })
}

const MIN_CORR: f64 = 0.6;
const MIN_MARGIN: f64 = 0.08;

/// Read the champion level (1..20) from the level box, None when unsure.
pub fn read_level(frame: &Rgba, bar_x: i32, bar_y: i32, scale: Option<f64>) -> Option<u32> {
    let glyphs = level_glyphs(frame, bar_x, bar_y, scale);
    if glyphs.is_empty() {
        return None;
    }
    let mut n = 0;
    for g in &glyphs {
        let c = classify_glyph(g)?;
        if c.corr < MIN_CORR || c.margin < MIN_MARGIN {
            return None;
        }
        n = n * 10 + c.d;
    }
    if (1..=20).contains(&n) {
        Some(n)
    } else {
        None
    }
}

// ---- assignment ------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct ChampionCandidate {
    pub champion: String,
    pub level: u32,
    /// Champion's projected ground position on screen, if known.
    pub screen: Option<(f64, f64)>,
}

/// Minimal bar shape needed for assignment.
pub trait BarLike {
    fn bx(&self) -> f64;
    fn by(&self) -> f64;
    fn bwidth(&self) -> f64;
    fn blevel(&self) -> Option<u32>;
}
impl BarLike for HealthBar {
    fn bx(&self) -> f64 {
        self.x as f64
    }
    fn by(&self) -> f64 {
        self.y as f64
    }
    fn bwidth(&self) -> f64 {
        self.width as f64
    }
    fn blevel(&self) -> Option<u32> {
        self.level
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MatchOptions {
    pub max_dist: Option<f64>,
    pub ambiguity_ratio: f64,
    pub bar_above: f64,
}
impl Default for MatchOptions {
    fn default() -> Self {
        MatchOptions { max_dist: None, ambiguity_ratio: 0.6, bar_above: 1.6 }
    }
}

/// Assign bars (all of one team colour) to that team's champions: level first, then screen
/// proximity among equal-level candidates; ambiguous bars stay unassigned (None).
/// Returns one entry per bar (same order).
pub fn match_bars_to_champions<B: BarLike>(bars: &[B], candidates: &[ChampionCandidate], opts: &MatchOptions) -> Vec<Option<String>> {
    let max_d = |b: &B| opts.max_dist.unwrap_or(3.0 * b.bwidth());
    let dist = |b: &B, c: &ChampionCandidate| match c.screen {
        None => f64::INFINITY,
        Some((sx, sy)) => (b.bx() + b.bwidth() / 2.0 - sx).hypot(b.by() - (sy - opts.bar_above * b.bwidth())),
    };
    let mut result: Vec<Option<String>> = vec![None; bars.len()];
    let mut taken: HashSet<String> = HashSet::new();
    let mut pending: Vec<usize> = Vec::new();
    // Map preserving insertion order of levels.
    let mut by_level: Vec<(u32, Vec<usize>)> = Vec::new();
    for (i, b) in bars.iter().enumerate() {
        match b.blevel() {
            None => pending.push(i),
            Some(l) => match by_level.iter_mut().find(|(lv, _)| *lv == l) {
                Some((_, v)) => v.push(i),
                None => by_level.push((l, vec![i])),
            },
        }
    }
    let assign = |idxs: &[usize], cands: &[&ChampionCandidate], result: &mut Vec<Option<String>>, taken: &mut HashSet<String>| {
        struct P<'c> {
            i: usize,
            c: &'c ChampionCandidate,
            d: f64,
        }
        let mut pairs: Vec<P> = Vec::new();
        for &i in idxs {
            for &c in cands {
                if taken.contains(&c.champion) {
                    continue;
                }
                let d = dist(&bars[i], c);
                if d <= max_d(&bars[i]) {
                    pairs.push(P { i, c, d });
                }
            }
        }
        pairs.sort_by(|a, b| a.d.partial_cmp(&b.d).unwrap_or(std::cmp::Ordering::Equal));
        let mut done: HashSet<usize> = HashSet::new();
        for p in &pairs {
            if done.contains(&p.i) || taken.contains(&p.c.champion) {
                continue;
            }
            let rival_c = pairs.iter().find(|q| q.i == p.i && !std::ptr::eq(q.c, p.c) && !taken.contains(&q.c.champion));
            let rival_b = pairs.iter().find(|q| std::ptr::eq(q.c, p.c) && q.i != p.i && !done.contains(&q.i));
            if rival_c.is_some_and(|r| p.d > r.d * opts.ambiguity_ratio) || rival_b.is_some_and(|r| p.d > r.d * opts.ambiguity_ratio) {
                continue;
            }
            result[p.i] = Some(p.c.champion.clone());
            taken.insert(p.c.champion.clone());
            done.insert(p.i);
        }
    };
    for (lvl, idxs) in &by_level {
        let cands: Vec<&ChampionCandidate> = candidates.iter().filter(|c| c.level == *lvl).collect();
        if cands.len() == 1 && idxs.len() == 1 {
            let b = &bars[idxs[0]];
            let c = cands[0];
            if c.screen.is_some() && dist(b, c) > 2.0 * max_d(b) {
                pending.extend(idxs);
                continue;
            }
            result[idxs[0]] = Some(c.champion.clone());
            taken.insert(c.champion.clone());
        } else if !cands.is_empty() {
            assign(idxs, &cands, &mut result, &mut taken);
        } else {
            pending.extend(idxs);
        }
    }
    if !pending.is_empty() {
        let rest: Vec<&ChampionCandidate> = candidates.iter().filter(|c| !taken.contains(&c.champion)).collect();
        assign(&pending, &rest, &mut result, &mut taken);
    }
    result
}

#[cfg(test)]
mod mana_tests {
    use super::*;

    /// Geometry measured on a real 1440p frame (enemy Lissandra, ~10% HP): HP fill top y=461,
    /// 12 px tall, mana strip 4 px tall at y=477..480, same left edge, blue to ~91% of the width.
    #[test]
    fn mana_strip_under_hp_bar() {
        let (w, h) = (300usize, 200usize);
        let mut f = Rgba { width: w, height: h, data: vec![30; w * h * 4] };
        let bar = HealthBar { x: 100, y: 61, width: 125, pct: 0.1, shield_pct: 0.0, shield_compressed: false, level: None, confidence: 1.0, mana: None };
        for y in 77..81 {
            for x in 100..214 {
                let i = (y * w + x) * 4;
                f.data[i] = 50;
                f.data[i + 1] = 110;
                f.data[i + 2] = 210;
            }
        }
        let m = mana_below(&f, &bar, 12.0, 1.0).unwrap();
        assert!((m - 0.91).abs() < 0.03, "{m}");
        // Energy strip (yellow, Akali / Lee Sin): read the same way.
        let mut en = Rgba { width: w, height: h, data: vec![30; w * h * 4] };
        for y in 77..81 {
            for x in 100..163 {
                let i = (y * w + x) * 4;
                en.data[i] = 225;
                en.data[i + 1] = 190;
                en.data[i + 2] = 40;
            }
        }
        assert!((mana_below(&en, &bar, 12.0, 1.0).unwrap() - 0.5).abs() < 0.03);
        // No strip (fury / no resource): None, not 0.
        let empty = Rgba { width: w, height: h, data: vec![30; w * h * 4] };
        assert_eq!(mana_below(&empty, &bar, 12.0, 1.0), None);
    }
}
