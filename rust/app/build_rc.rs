//! Minimal Windows resource compiler used ONLY when no `windres` is installed
//! (GNU target without MSYS2/MinGW). tauri-build always compiles a `.rc` file
//! through `embed-resource`, which runs `windres --input X --output Y
//! --output-format=coff`. The build script copies itself as `windres.exe` and,
//! when called under that name, handles the subset of RC that tauri-winres
//! writes: `<id> ICON "file.ico"` and the inline application manifest
//! (`<id> 24 { "..." }` or `<id> 24 "file"`). VERSIONINFO is skipped
//! (cosmetic: only the Explorer "Details" tab).
//!
//! Output: an x86-64 COFF object with one `.rsrc` section (same thing windres
//! produces), linked into the exe by the GNU linker.

use std::fs;
use std::path::Path;

const RT_ICON: u32 = 3;
const RT_GROUP_ICON: u32 = 14;
const RT_MANIFEST: u32 = 24;
const LANG_EN_US: u32 = 0x0409;

struct Res {
    ty: u32,
    id: u32,
    data: Vec<u8>,
}

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\\' => match it.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some(o) => out.push(o),
                None => {}
            },
            '"' if it.peek() == Some(&'"') => {
                it.next();
                out.push('"');
            }
            _ => out.push(c),
        }
    }
    out
}

/// Text between the first and last double quote of a line.
fn quoted(line: &str) -> Option<String> {
    let a = line.find('"')?;
    let b = line.rfind('"')?;
    (b > a).then(|| unescape(&line[a + 1..b]))
}

fn rd16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn rd32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn icon_resources(group_id: u32, ico: &[u8], next_icon_id: &mut u32, out: &mut Vec<Res>) -> Result<(), String> {
    if ico.len() < 6 || rd16(ico, 2) != 1 {
        return Err("not an .ico file".into());
    }
    let count = rd16(ico, 4) as usize;
    let mut group = Vec::new();
    group.extend_from_slice(&0u16.to_le_bytes());
    group.extend_from_slice(&1u16.to_le_bytes());
    group.extend_from_slice(&(count as u16).to_le_bytes());
    for i in 0..count {
        let e = 6 + i * 16;
        if e + 16 > ico.len() {
            return Err("truncated .ico".into());
        }
        let size = rd32(ico, e + 8) as usize;
        let off = rd32(ico, e + 12) as usize;
        let data = ico.get(off..off + size).ok_or("truncated .ico image")?.to_vec();
        let id = *next_icon_id;
        *next_icon_id += 1;
        group.extend_from_slice(&ico[e..e + 12]); // w, h, colors, reserved, planes, bitcount, bytes
        group.extend_from_slice(&(id as u16).to_le_bytes());
        out.push(Res { ty: RT_ICON, id, data });
    }
    out.push(Res { ty: RT_GROUP_ICON, id: group_id, data: group });
    Ok(())
}

/// Parse the tauri-winres subset of an .rc file.
fn parse_rc(src: &str, base: &Path) -> Result<Vec<Res>, String> {
    let lines: Vec<&str> = src.lines().map(str::trim).collect();
    let mut out = Vec::new();
    let mut next_icon_id = 1u32;
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i];
        let mut words = l.split_whitespace();
        let (a, b) = (words.next().unwrap_or(""), words.next().unwrap_or(""));
        let id = a.parse::<u32>().ok();
        match (id, b) {
            (Some(id), "ICON") => {
                let p = quoted(l).ok_or_else(|| format!("bad ICON line: {l}"))?;
                let path = base.join(&p);
                let ico = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                icon_resources(id, &ico, &mut next_icon_id, &mut out)?;
            }
            (Some(id), "24") => {
                if let Some(p) = quoted(l) {
                    let path = base.join(&p);
                    let data = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                    out.push(Res { ty: RT_MANIFEST, id, data });
                } else {
                    // Inline block: { "line" ... }
                    let mut text = String::new();
                    i += 1;
                    while i < lines.len() && lines[i] != "}" {
                        if let Some(q) = quoted(lines[i]) {
                            text.push_str(&q);
                        }
                        i += 1;
                    }
                    out.push(Res { ty: RT_MANIFEST, id, data: text.into_bytes() });
                }
            }
            (Some(_), "VERSIONINFO") => {
                // Skip the whole block (balanced braces).
                let mut depth = 0i32;
                let mut opened = false;
                i += 1;
                while i < lines.len() {
                    depth += lines[i].matches('{').count() as i32 - lines[i].matches('}').count() as i32;
                    opened |= lines[i].contains('{');
                    if opened && depth <= 0 {
                        break;
                    }
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    Ok(out)
}

fn align8(v: &mut Vec<u8>) {
    while v.len() % 8 != 0 {
        v.push(0);
    }
}

/// Build the `.rsrc` section: Type -> Name -> Language directory tree + data.
/// Returns (section bytes, offsets of the RVA fields needing relocations).
fn build_rsrc(mut res: Vec<Res>) -> (Vec<u8>, Vec<u32>) {
    res.sort_by_key(|r| (r.ty, r.id));
    let mut types: Vec<u32> = res.iter().map(|r| r.ty).collect();
    types.dedup();
    let dir = |n: usize| 16 + 8 * n;
    // Layout: root dir, then per type a name dir, then per resource a lang dir, then data entries, then data.
    let mut size = dir(types.len());
    let mut name_dir_off = Vec::new();
    for t in &types {
        name_dir_off.push(size);
        size += dir(res.iter().filter(|r| r.ty == *t).count());
    }
    let mut lang_dir_off = Vec::new();
    for _ in &res {
        lang_dir_off.push(size);
        size += dir(1);
    }
    let mut entry_off = Vec::new();
    for _ in &res {
        entry_off.push(size);
        size += 16;
    }
    let mut data_off = Vec::new();
    for r in &res {
        size = (size + 7) & !7;
        data_off.push(size);
        size += r.data.len();
    }

    let mut s = Vec::with_capacity(size);
    let put_dir = |s: &mut Vec<u8>, n: usize| {
        s.extend_from_slice(&[0u8; 12]);
        s.extend_from_slice(&0u16.to_le_bytes());
        s.extend_from_slice(&(n as u16).to_le_bytes());
    };
    let put_entry = |s: &mut Vec<u8>, id: u32, off: usize, subdir: bool| {
        s.extend_from_slice(&id.to_le_bytes());
        s.extend_from_slice(&((off as u32) | if subdir { 0x8000_0000 } else { 0 }).to_le_bytes());
    };
    put_dir(&mut s, types.len());
    for (k, t) in types.iter().enumerate() {
        put_entry(&mut s, *t, name_dir_off[k], true);
    }
    for t in &types {
        let idx: Vec<usize> = (0..res.len()).filter(|&j| res[j].ty == *t).collect();
        put_dir(&mut s, idx.len());
        for j in idx {
            put_entry(&mut s, res[j].id, lang_dir_off[j], true);
        }
    }
    for j in 0..res.len() {
        put_dir(&mut s, 1);
        put_entry(&mut s, LANG_EN_US, entry_off[j], false);
    }
    let mut relocs = Vec::new();
    for (j, r) in res.iter().enumerate() {
        relocs.push(s.len() as u32);
        s.extend_from_slice(&(data_off[j] as u32).to_le_bytes()); // RVA (relocated)
        s.extend_from_slice(&(r.data.len() as u32).to_le_bytes());
        s.extend_from_slice(&0u32.to_le_bytes()); // code page
        s.extend_from_slice(&0u32.to_le_bytes());
    }
    for r in &res {
        align8(&mut s);
        s.extend_from_slice(&r.data);
    }
    align8(&mut s);
    (s, relocs)
}

/// x86-64 COFF object with a single `.rsrc` section.
fn coff_object(rsrc: &[u8], relocs: &[u32]) -> Vec<u8> {
    const IMAGE_REL_AMD64_ADDR32NB: u16 = 3;
    let raw_ptr = 20 + 40;
    let reloc_ptr = raw_ptr + rsrc.len();
    let sym_ptr = reloc_ptr + relocs.len() * 10;
    let mut o = Vec::new();
    // File header
    o.extend_from_slice(&0x8664u16.to_le_bytes());
    o.extend_from_slice(&1u16.to_le_bytes());
    o.extend_from_slice(&0u32.to_le_bytes());
    o.extend_from_slice(&(sym_ptr as u32).to_le_bytes());
    o.extend_from_slice(&1u32.to_le_bytes()); // one symbol
    o.extend_from_slice(&0u16.to_le_bytes());
    o.extend_from_slice(&0u16.to_le_bytes());
    // Section header
    o.extend_from_slice(b".rsrc\0\0\0");
    o.extend_from_slice(&0u32.to_le_bytes());
    o.extend_from_slice(&0u32.to_le_bytes());
    o.extend_from_slice(&(rsrc.len() as u32).to_le_bytes());
    o.extend_from_slice(&(raw_ptr as u32).to_le_bytes());
    o.extend_from_slice(&(reloc_ptr as u32).to_le_bytes());
    o.extend_from_slice(&0u32.to_le_bytes());
    o.extend_from_slice(&(relocs.len() as u16).to_le_bytes());
    o.extend_from_slice(&0u16.to_le_bytes());
    // INITIALIZED_DATA | ALIGN_4BYTES | MEM_READ
    o.extend_from_slice(&0x4030_0040u32.to_le_bytes());
    o.extend_from_slice(rsrc);
    for r in relocs {
        o.extend_from_slice(&r.to_le_bytes());
        o.extend_from_slice(&0u32.to_le_bytes()); // symbol 0 = section symbol
        o.extend_from_slice(&IMAGE_REL_AMD64_ADDR32NB.to_le_bytes());
    }
    // Symbol table: the section symbol.
    o.extend_from_slice(b".rsrc\0\0\0");
    o.extend_from_slice(&0u32.to_le_bytes());
    o.extend_from_slice(&1i16.to_le_bytes());
    o.extend_from_slice(&0u16.to_le_bytes());
    o.push(3); // IMAGE_SYM_CLASS_STATIC
    o.push(0);
    // Empty string table.
    o.extend_from_slice(&4u32.to_le_bytes());
    o
}

/// Entry point when the build script runs as `windres.exe`.
pub fn windres_main() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let get = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    let (Some(input), Some(output)) = (get("--input"), get("--output")) else {
        return 0; // probe run ("is it runnable?")
    };
    let run = || -> Result<(), String> {
        let src = fs::read_to_string(&input).map_err(|e| format!("{input}: {e}"))?;
        let base = Path::new(&input).parent().unwrap_or(Path::new("."));
        let res = parse_rc(&src, base)?;
        let (rsrc, relocs) = build_rsrc(res);
        fs::write(&output, coff_object(&rsrc, &relocs)).map_err(|e| format!("{output}: {e}"))
    };
    match run() {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("rc-windres: {e}");
            1
        }
    }
}
