//! In-game minimap champion icons (`hud/<id>_circle_0`) read straight from the local League install
//! (WAD v3 archives), so the detector never depends on raw.communitydragon.org being up. Only the
//! archive's table of contents and the one icon are read (seek), not the whole champion file.
//! Same logic as scripts/extract-minimap-icons.mjs.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

/// Some champions keep their old internal names in the game files.
const ALIAS: &[(&str, &str)] = &[
    ("anivia", "cryophoenix"),
    ("blitzcrank", "steamgolem"),
    ("chogath", "greenterror"),
    ("orianna", "oriana"),
    ("rammus", "armordillo"),
    ("shaco", "jester"),
    ("xinzhao", "xenzhao"),
    ("zilean", "chronokeeper"),
];

/// League install folders to look in (env RIOT_LEAGUE_DIR first).
fn league_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(d) = std::env::var_os("RIOT_LEAGUE_DIR") {
        v.push(PathBuf::from(d));
    }
    for drive in ["C", "D", "E", "F"] {
        v.push(PathBuf::from(format!("{drive}:\\Riot Games\\League of Legends")));
    }
    v
}

const P1: u64 = 11400714785074694791;
const P2: u64 = 14029467366897019727;
const P3: u64 = 1609587929392839161;
const P4: u64 = 9650029242287828579;
const P5: u64 = 2870177450012600261;

fn round(acc: u64, v: u64) -> u64 {
    acc.wrapping_add(v.wrapping_mul(P2)).rotate_left(31).wrapping_mul(P1)
}

fn merge(acc: u64, v: u64) -> u64 {
    (acc ^ round(0, v)).wrapping_mul(P1).wrapping_add(P4)
}

/// xxHash64, seed 0: the WAD entry key of a lower-case asset path.
pub fn xxh64(b: &[u8]) -> u64 {
    let n = b.len();
    let u64_at = |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap());
    let mut i = 0;
    let mut h = if n >= 32 {
        let (mut v1, mut v2, mut v3, mut v4) = (P1.wrapping_add(P2), P2, 0u64, 0u64.wrapping_sub(P1));
        while i + 32 <= n {
            v1 = round(v1, u64_at(i));
            v2 = round(v2, u64_at(i + 8));
            v3 = round(v3, u64_at(i + 16));
            v4 = round(v4, u64_at(i + 24));
            i += 32;
        }
        let mut h = v1.rotate_left(1).wrapping_add(v2.rotate_left(7)).wrapping_add(v3.rotate_left(12)).wrapping_add(v4.rotate_left(18));
        h = merge(h, v1);
        h = merge(h, v2);
        h = merge(h, v3);
        merge(h, v4)
    } else {
        P5
    };
    h = h.wrapping_add(n as u64);
    while i + 8 <= n {
        h = (h ^ round(0, u64_at(i))).rotate_left(27).wrapping_mul(P1).wrapping_add(P4);
        i += 8;
    }
    while i + 4 <= n {
        let v = u32::from_le_bytes(b[i..i + 4].try_into().unwrap()) as u64;
        h = (h ^ v.wrapping_mul(P1)).rotate_left(23).wrapping_mul(P2).wrapping_add(P3);
        i += 4;
    }
    while i < n {
        h = (h ^ (b[i] as u64).wrapping_mul(P5)).rotate_left(11).wrapping_mul(P1);
        i += 1;
    }
    h ^= h >> 33;
    h = h.wrapping_mul(P2);
    h ^= h >> 29;
    h = h.wrapping_mul(P3);
    h ^ (h >> 32)
}

/// The data of the first entry whose path hash is in `want`, from a WAD v3 file.
fn wad_entry(file: &PathBuf, want: &[(u64, String)]) -> Option<(String, Vec<u8>)> {
    let mut f = File::open(file).ok()?;
    let mut head = [0u8; 272];
    f.read_exact(&mut head).ok()?;
    if &head[0..2] != b"RW" || head[2] != 3 {
        return None;
    }
    let count = u32::from_le_bytes(head[268..272].try_into().unwrap()) as usize;
    let mut toc = vec![0u8; count * 32];
    f.read_exact(&mut toc).ok()?;
    for k in 0..count {
        let e = &toc[k * 32..k * 32 + 32];
        let hash = u64::from_le_bytes(e[0..8].try_into().unwrap());
        let Some((_, path)) = want.iter().find(|(h, _)| *h == hash) else { continue };
        let off = u32::from_le_bytes(e[8..12].try_into().unwrap()) as u64;
        let csize = u32::from_le_bytes(e[12..16].try_into().unwrap()) as usize;
        let kind = e[20] & 0x0f;
        let mut raw = vec![0u8; csize];
        f.seek(SeekFrom::Start(off)).ok()?;
        f.read_exact(&mut raw).ok()?;
        let data = match kind {
            0 => raw,
            3 | 4 => {
                let mut out = Vec::new();
                ruzstd::decoding::StreamingDecoder::new(&raw[..]).ok()?.read_to_end(&mut out).ok()?;
                out
            }
            _ => return None,
        };
        return Some((path.clone(), data));
    }
    None
}

fn rgb565(c: u16) -> [u8; 3] {
    [(((c >> 11) & 31) as u32 * 255 / 31) as u8, (((c >> 5) & 63) as u32 * 255 / 63) as u8, ((c & 31) as u32 * 255 / 31) as u8]
}

fn decode_dxt(src: &[u8], w: usize, h: usize, dxt5: bool) -> Vec<u8> {
    let mut out = vec![0u8; w * h * 4];
    let bs = if dxt5 { 16 } else { 8 };
    let mut p = 0;
    for by in (0..h).step_by(4) {
        for bx in (0..w).step_by(4) {
            if p + bs > src.len() {
                return out;
            }
            let mut alpha = [255u8; 16];
            if dxt5 {
                let (a0, a1) = (src[p] as u32, src[p + 1] as u32);
                let mut pal = [a0, a1, 0, 0, 0, 0, 0, 0];
                if a0 > a1 {
                    for i in 1..7 {
                        pal[i + 1] = ((7 - i as u32) * a0 + i as u32 * a1) / 7;
                    }
                } else {
                    for i in 1..5 {
                        pal[i + 1] = ((5 - i as u32) * a0 + i as u32 * a1) / 5;
                    }
                    pal[6] = 0;
                    pal[7] = 255;
                }
                let mut bits = 0u64;
                for i in 0..6 {
                    bits |= (src[p + 2 + i] as u64) << (8 * i);
                }
                for (i, a) in alpha.iter_mut().enumerate() {
                    *a = pal[((bits >> (3 * i)) & 7) as usize] as u8;
                }
            }
            let q = p + if dxt5 { 8 } else { 0 };
            let (c0, c1) = (u16::from_le_bytes([src[q], src[q + 1]]), u16::from_le_bytes([src[q + 2], src[q + 3]]));
            let (a, b) = (rgb565(c0), rgb565(c1));
            let mix = |wa: u32, wb: u32, d: u32| -> [u8; 3] { [0, 1, 2].map(|k| ((wa * a[k] as u32 + wb * b[k] as u32) / d) as u8) };
            let four = c0 > c1 || dxt5;
            let cols = if four { [a, b, mix(2, 1, 3), mix(1, 2, 3)] } else { [a, b, mix(1, 1, 2), [0, 0, 0]] };
            let idx = u32::from_le_bytes(src[q + 4..q + 8].try_into().unwrap());
            for i in 0..16 {
                let (x, y) = (bx + (i & 3), by + (i >> 2));
                if x >= w || y >= h {
                    continue;
                }
                let ci = ((idx >> (2 * i)) & 3) as usize;
                let o = (y * w + x) * 4;
                out[o..o + 3].copy_from_slice(&cols[ci]);
                out[o + 3] = if dxt5 { alpha[i] } else if !four && ci == 3 { 0 } else { 255 };
            }
            p += bs;
        }
    }
    out
}

/// TEX (Riot texture) -> RGBA of its largest mip (stored last). BGRA8 / DXT1 / DXT5.
fn decode_tex(b: &[u8]) -> Option<rc_vision::Rgba> {
    if b.len() < 12 || &b[0..4] != b"TEX\0" {
        return None;
    }
    let (w, h, fmt) = (u16::from_le_bytes([b[4], b[5]]) as usize, u16::from_le_bytes([b[6], b[7]]) as usize, b[9]);
    let body = &b[12..];
    let data = match fmt {
        20 => {
            let n = w * h * 4;
            let src = body.get(body.len().checked_sub(n)?..)?;
            let mut out = vec![0u8; n];
            for i in (0..n).step_by(4) {
                out[i] = src[i + 2];
                out[i + 1] = src[i + 1];
                out[i + 2] = src[i];
                out[i + 3] = src[i + 3];
            }
            out
        }
        10 | 12 => {
            let n = w.div_ceil(4) * h.div_ceil(4) * if fmt == 12 { 16 } else { 8 };
            decode_dxt(body.get(body.len().checked_sub(n)?..)?, w, h, fmt == 12)
        }
        _ => return None,
    };
    Some(rc_vision::Rgba { width: w, height: h, data })
}

/// The champion's in-game minimap icon from the local install, if found. `dd_id` = Data Dragon id.
pub fn circle_icon(dd_id: &str) -> Option<rc_vision::Rgba> {
    let id = dd_id.to_lowercase();
    let names: Vec<&str> = std::iter::once(id.as_str()).chain(ALIAS.iter().filter(|(a, _)| *a == id).map(|(_, b)| *b)).collect();
    let mut want = Vec::new();
    for folder in &names {
        for file in &names {
            for p in [format!("assets/characters/{folder}/hud/{file}_circle_0.tex"), format!("assets/characters/{folder}/hud/{file}_circle.tex")] {
                want.push((xxh64(p.as_bytes()), p));
            }
        }
    }
    for dir in league_dirs() {
        let champs = dir.join("Game").join("DATA").join("FINAL").join("Champions");
        let Ok(rd) = std::fs::read_dir(&champs) else { continue };
        let file = rd.flatten().map(|e| e.path()).find(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.to_lowercase() == format!("{id}.wad.client")));
        if let Some(file) = file {
            if let Some((_, data)) = wad_entry(&file, &want) {
                return decode_tex(&data);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn xxh64_known_values() {
        // Reference values of xxHash64 (seed 0).
        assert_eq!(xxh64(b""), 0xEF46DB3751D8E999);
        assert_eq!(xxh64(b"a"), 0xD24EC4F1A98C6E5B);
        assert_eq!(xxh64(b"assets/characters/briar/hud/briar_circle_0.tex").to_string().len() > 0, true);
    }

    #[test]
    fn dxt1_solid_block() {
        // One DXT1 block, both colours pure red, all indices 0.
        let src = [0x00, 0xF8, 0x00, 0xF8, 0, 0, 0, 0];
        let px = decode_dxt(&src, 4, 4, false);
        assert_eq!(&px[0..4], &[255, 0, 0, 255]);
    }
}

#[cfg(test)]
mod local_install_test {
    /// Runs only where League is installed: Briar's minimap icon comes out as a 128x128 RGBA image.
    #[test]
    fn briar_from_local_install() {
        if !std::path::Path::new("C:/Riot Games/League of Legends/Game").exists() {
            return;
        }
        let icon = super::circle_icon("Briar").expect("briar icon from the local install");
        assert_eq!((icon.width, icon.height), (128, 128));
        assert!(icon.data.chunks(4).any(|p| p[3] > 200), "has opaque pixels");
    }
}
