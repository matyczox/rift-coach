//! dlltool-lite: the part of GNU `dlltool` that rustc needs on the
//! `x86_64-pc-windows-gnu` target without MinGW installed.
//!
//! rustc builds import libraries for `#[link(kind = "raw-dylib")]` (used by
//! `windows-link`, i.e. every recent `windows` / `windows-sys` crate) by calling
//!   dlltool -d X.def -D name.dll -l out.lib -m i386:x86-64 -f --64
//!           --no-leading-underscore --temp-prefix name.dll
//! The real dlltool needs the GNU assembler (`as`), which the Rust GNU
//! toolchain doesn't ship. This writes the same archive layout directly
//! (head / one stub per symbol / tail objects, member names `<prefix>h.o`,
//! `<prefix>sNNNNN.o`, `<prefix>t.o`), matching what binutils produces, so
//! GNU ld sorts and merges the `.idata$N` sections exactly as usual.
//!
//! Build: `rustc -O tools/dlltool.rs -o <dir>/dlltool.exe` and put <dir> first
//! in PATH (see `cargo-gnu.ps1`). Standalone, std only.

use std::fs;
use std::process::exit;

const REL_ADDR32NB: u16 = 3;
const REL_REL32: u16 = 4;
const CH_TEXT: u32 = 0x6030_0020; // code | align 4 | exec | read
const CH_IDATA: u32 = 0xc030_0040; // data | align 4 | read | write
const CH_IDATA_STUB: u32 = 0xc030_0000;
const CH_IDATA6: u32 = 0xc020_0000; // align 2

struct Export {
    name: String,
    ordinal: Option<u16>,
    noname: bool,
    data: bool,
}

#[derive(Default)]
struct Obj {
    sections: Vec<Sec>,
    syms: Vec<Sym>,
}
struct Sec {
    name: &'static str,
    data: Vec<u8>,
    relocs: Vec<(u32, usize, u16)>, // (offset, symbol index, type)
    ch: u32,
}
struct Sym {
    name: String,
    value: u32,
    section: i16, // 1-based, 0 = undefined
    class: u8,    // 2 = external, 3 = static
}

impl Obj {
    /// Add a section and its section symbol; returns the symbol index.
    fn section(&mut self, name: &'static str, data: Vec<u8>, ch: u32) -> usize {
        self.sections.push(Sec { name, data, relocs: Vec::new(), ch });
        let n = self.sections.len() as i16;
        self.sym(name, 0, n, 3)
    }
    fn sym(&mut self, name: &str, value: u32, section: i16, class: u8) -> usize {
        self.syms.push(Sym { name: name.into(), value, section, class });
        self.syms.len() - 1
    }
    fn reloc(&mut self, sec: usize, offset: u32, sym: usize, ty: u16) {
        self.sections[sec].relocs.push((offset, sym, ty));
    }

    fn bytes(&self) -> Vec<u8> {
        let nsec = self.sections.len();
        let mut data_ptr = 20 + 40 * nsec;
        let mut layout = Vec::new();
        for s in &self.sections {
            let raw = if s.data.is_empty() { 0 } else { data_ptr };
            data_ptr += s.data.len();
            let rel = if s.relocs.is_empty() { 0 } else { data_ptr };
            data_ptr += s.relocs.len() * 10;
            layout.push((raw, rel));
        }
        let sym_ptr = data_ptr;
        let mut strtab: Vec<u8> = Vec::new();
        let mut o = Vec::new();
        o.extend_from_slice(&0x8664u16.to_le_bytes());
        o.extend_from_slice(&(nsec as u16).to_le_bytes());
        o.extend_from_slice(&0u32.to_le_bytes());
        o.extend_from_slice(&(sym_ptr as u32).to_le_bytes());
        o.extend_from_slice(&(self.syms.len() as u32).to_le_bytes());
        o.extend_from_slice(&0u16.to_le_bytes());
        o.extend_from_slice(&0u16.to_le_bytes());
        for (s, (raw, rel)) in self.sections.iter().zip(&layout) {
            let mut name = [0u8; 8];
            name[..s.name.len()].copy_from_slice(s.name.as_bytes());
            o.extend_from_slice(&name);
            o.extend_from_slice(&0u32.to_le_bytes());
            o.extend_from_slice(&0u32.to_le_bytes());
            o.extend_from_slice(&(s.data.len() as u32).to_le_bytes());
            o.extend_from_slice(&(*raw as u32).to_le_bytes());
            o.extend_from_slice(&(*rel as u32).to_le_bytes());
            o.extend_from_slice(&0u32.to_le_bytes());
            o.extend_from_slice(&(s.relocs.len() as u16).to_le_bytes());
            o.extend_from_slice(&0u16.to_le_bytes());
            o.extend_from_slice(&s.ch.to_le_bytes());
        }
        for s in &self.sections {
            o.extend_from_slice(&s.data);
            for (off, sym, ty) in &s.relocs {
                o.extend_from_slice(&off.to_le_bytes());
                o.extend_from_slice(&(*sym as u32).to_le_bytes());
                o.extend_from_slice(&ty.to_le_bytes());
            }
        }
        for s in &self.syms {
            if s.name.len() <= 8 {
                let mut name = [0u8; 8];
                name[..s.name.len()].copy_from_slice(s.name.as_bytes());
                o.extend_from_slice(&name);
            } else {
                o.extend_from_slice(&0u32.to_le_bytes());
                o.extend_from_slice(&((strtab.len() + 4) as u32).to_le_bytes());
                strtab.extend_from_slice(s.name.as_bytes());
                strtab.push(0);
            }
            o.extend_from_slice(&s.value.to_le_bytes());
            o.extend_from_slice(&s.section.to_le_bytes());
            o.extend_from_slice(&0u16.to_le_bytes());
            o.push(s.class);
            o.push(0);
        }
        o.extend_from_slice(&((strtab.len() + 4) as u32).to_le_bytes());
        o.extend_from_slice(&strtab);
        o
    }

    fn globals(&self) -> Vec<String> {
        self.syms.iter().filter(|s| s.class == 2 && s.section != 0).map(|s| s.name.clone()).collect()
    }
}

fn parse_def(text: &str) -> Vec<Export> {
    let mut out = Vec::new();
    let mut in_exports = false;
    for line in text.lines() {
        let line = line.split(';').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let up = line.to_ascii_uppercase();
        if up.starts_with("LIBRARY") || up.starts_with("NAME ") {
            continue;
        }
        if up.starts_with("EXPORTS") {
            in_exports = true;
            let rest = line[7..].trim();
            if rest.is_empty() {
                continue;
            }
        }
        if !in_exports {
            continue;
        }
        let line = if up.starts_with("EXPORTS") { line[7..].trim() } else { line };
        let mut toks = line.split_whitespace();
        let Some(first) = toks.next() else { continue };
        // "name=internal" / "name==importname": the exported (link) name is the left side.
        let name = first.split('=').next().unwrap_or(first).trim_matches('"').to_string();
        let mut e = Export { name, ordinal: None, noname: false, data: false };
        let rest: Vec<&str> = toks.collect();
        let mut i = 0;
        while i < rest.len() {
            let t = rest[i];
            if let Some(n) = t.strip_prefix('@') {
                if n.is_empty() {
                    i += 1;
                    e.ordinal = rest.get(i).and_then(|x| x.parse().ok());
                } else {
                    e.ordinal = n.parse().ok();
                }
            } else {
                match t.to_ascii_uppercase().as_str() {
                    "NONAME" => e.noname = true,
                    "DATA" => e.data = true,
                    _ => {}
                }
            }
            i += 1;
        }
        out.push(e);
    }
    out
}

fn label(dll: &str) -> String {
    dll.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
}

fn head(dll_lab: &str) -> Obj {
    let mut o = Obj::default();
    let s2 = o.section(".idata$2", vec![0; 20], CH_IDATA);
    let s5 = o.section(".idata$5", Vec::new(), CH_IDATA);
    let s4 = o.section(".idata$4", Vec::new(), CH_IDATA);
    o.sym(&format!("_head_{dll_lab}"), 0, 1, 2);
    let iname = o.sym(&format!("{dll_lab}_iname"), 0, 0, 2);
    o.reloc(0, 0, s4, REL_ADDR32NB); // OriginalFirstThunk
    o.reloc(0, 12, iname, REL_ADDR32NB); // Name
    o.reloc(0, 16, s5, REL_ADDR32NB); // FirstThunk
    let _ = s2;
    o
}

fn tail(dll: &str, dll_lab: &str) -> Obj {
    let mut o = Obj::default();
    o.section(".idata$4", vec![0; 8], CH_IDATA);
    o.section(".idata$5", vec![0; 8], CH_IDATA);
    let mut name = dll.as_bytes().to_vec();
    name.push(0);
    while name.len() % 4 != 0 {
        name.push(0);
    }
    o.section(".idata$7", name, CH_IDATA);
    o.sym(&format!("{dll_lab}_iname"), 0, 3, 2);
    o
}

fn stub(e: &Export, dll_lab: &str) -> Obj {
    let mut o = Obj::default();
    let by_ordinal = e.noname && e.ordinal.is_some();
    let thunk = if by_ordinal { (0x8000_0000_0000_0000u64 | e.ordinal.unwrap() as u64).to_le_bytes().to_vec() } else { vec![0; 8] };
    let text = (!e.data).then(|| o.section(".text", vec![0xff, 0x25, 0, 0, 0, 0, 0x90, 0x90], CH_TEXT));
    let s7 = o.sections.len();
    o.section(".idata$7", vec![0; 4], CH_IDATA_STUB);
    let s5 = o.sections.len();
    o.section(".idata$5", thunk.clone(), CH_IDATA_STUB);
    let s4 = o.sections.len();
    o.section(".idata$4", thunk, CH_IDATA_STUB);
    let hint_name = if by_ordinal {
        None
    } else {
        let mut v = e.ordinal.unwrap_or(0).to_le_bytes().to_vec();
        v.extend_from_slice(e.name.as_bytes());
        v.push(0);
        if v.len() % 2 != 0 {
            v.push(0);
        }
        let idx = o.sections.len();
        Some((idx, o.section(".idata$6", v, CH_IDATA6)))
    };
    if let Some(t) = text {
        let _ = t;
        o.sym(&e.name, 0, 1, 2);
    }
    let imp = o.sym(&format!("__imp_{}", e.name), 0, (s5 + 1) as i16, 2);
    let head = o.sym(&format!("_head_{dll_lab}"), 0, 0, 2);
    if text.is_some() {
        o.reloc(0, 2, imp, REL_REL32);
    }
    o.reloc(s7, 0, head, REL_ADDR32NB);
    if let Some((_, hn_sym)) = hint_name {
        o.reloc(s5, 0, hn_sym, REL_ADDR32NB);
        o.reloc(s4, 0, hn_sym, REL_ADDR32NB);
    }
    o
}

fn ar_header(name: &str, size: usize) -> Vec<u8> {
    let mut h = format!("{:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n", name, 0, 0, 0, 644, size).into_bytes();
    h.truncate(60);
    h
}

fn archive(members: Vec<(String, Obj)>) -> Vec<u8> {
    // Long names table.
    let mut longnames = Vec::new();
    let mut names = Vec::new();
    for (n, _) in &members {
        let full = format!("{n}/");
        if full.len() <= 16 {
            names.push(full);
        } else {
            names.push(format!("/{}", longnames.len()));
            longnames.extend_from_slice(full.as_bytes());
            longnames.push(b'\n');
        }
    }
    let bodies: Vec<(Vec<u8>, Vec<String>)> = members.iter().map(|(_, o)| (o.bytes(), o.globals())).collect();
    let nsyms: usize = bodies.iter().map(|b| b.1.len()).sum();
    let symnames_len: usize = bodies.iter().flat_map(|b| b.1.iter()).map(|s| s.len() + 1).sum();
    let symtab_size = 4 + 4 * nsyms + symnames_len;
    let pad = |n: usize| n + (n & 1);
    let mut offset = 8 + 60 + pad(symtab_size);
    if !longnames.is_empty() {
        offset += 60 + pad(longnames.len());
    }
    let mut offsets = Vec::new();
    let mut member_off = Vec::new();
    for (body, syms) in &bodies {
        member_off.push(offset);
        for _ in syms {
            offsets.push(offset as u32);
        }
        offset += 60 + pad(body.len());
    }
    let mut out = b"!<arch>\n".to_vec();
    out.extend(ar_header("/", symtab_size));
    out.extend_from_slice(&(nsyms as u32).to_be_bytes());
    for off in &offsets {
        out.extend_from_slice(&off.to_be_bytes());
    }
    for (_, syms) in &bodies {
        for s in syms {
            out.extend_from_slice(s.as_bytes());
            out.push(0);
        }
    }
    if symtab_size & 1 == 1 {
        out.push(b'\n');
    }
    if !longnames.is_empty() {
        out.extend(ar_header("//", longnames.len()));
        out.extend_from_slice(&longnames);
        if longnames.len() & 1 == 1 {
            out.push(b'\n');
        }
    }
    for ((body, _), name) in bodies.iter().zip(&names) {
        out.extend(ar_header(name, body.len()));
        out.extend_from_slice(body);
        if body.len() & 1 == 1 {
            out.push(b'\n');
        }
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut def = None;
    let mut dll = None;
    let mut out = None;
    let mut prefix = None;
    let mut machine = String::from("i386:x86-64");
    let mut i = 0;
    while i < args.len() {
        let next = || args.get(i + 1).cloned();
        match args[i].as_str() {
            "-d" | "--input-def" => { def = next(); i += 1; }
            "-D" | "--dllname" => { dll = next(); i += 1; }
            "-l" | "--output-lib" => { out = next(); i += 1; }
            "--temp-prefix" => { prefix = next(); i += 1; }
            "-m" | "--machine" => { machine = next().unwrap_or_default(); i += 1; }
            "--version" | "-V" => { println!("dlltool-lite 0.1 (Rift Coach)"); return; }
            _ => {}
        }
        i += 1;
    }
    if machine != "i386:x86-64" {
        eprintln!("dlltool-lite: only i386:x86-64 is supported (got {machine})");
        exit(1);
    }
    let (Some(def), Some(out)) = (def, out) else {
        eprintln!("dlltool-lite: usage: -d X.def -D name.dll -l out.lib [--temp-prefix P]");
        exit(1);
    };
    let text = match fs::read_to_string(&def) {
        Ok(t) => t,
        Err(e) => { eprintln!("dlltool-lite: {def}: {e}"); exit(1); }
    };
    let dll = dll.unwrap_or_else(|| {
        text.lines().find_map(|l| l.trim().strip_prefix("LIBRARY").map(|x| x.trim().trim_matches('"').to_string())).unwrap_or_else(|| "unknown.dll".into())
    });
    let prefix = prefix.unwrap_or_else(|| label(&dll));
    // Labels from the OUTPUT path, like binutils: every archive gets its own head/tail
    // (import descriptor). Labels from the dll name would merge several archives of the
    // same dll into one descriptor, and ld (which orders .idata$N by archive, then member)
    // would leave the second archive's thunks after the first one's terminator.
    let lab = label(&out);
    let exports = parse_def(&text);
    let mut members = vec![(format!("{prefix}h.o"), head(&lab))];
    for (k, e) in exports.iter().enumerate() {
        members.push((format!("{prefix}s{:05}.o", k + 1), stub(e, &lab)));
    }
    members.push((format!("{prefix}t.o"), tail(&dll, &lab)));
    if let Err(e) = fs::write(&out, archive(members)) {
        eprintln!("dlltool-lite: {out}: {e}");
        exit(1);
    }
}
