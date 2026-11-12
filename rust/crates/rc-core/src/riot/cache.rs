//! JSON-file cache (`src/main/riot/cache.ts`): `<dir>/<namespace>/<key>.json`
//! containing `{ savedAt, data }` — same files as the Electron app, so an
//! existing `riot-cache` folder is reused. TTL is decided by the reader
//! (immutable data such as matches/timelines is read with `ttl = None`).
//! A small in-memory layer avoids re-reading big timeline files. All I/O
//! errors are swallowed: the cache is an optimisation, never a failure source.

use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

const MEM_MAX: usize = 400;

/// Epoch milliseconds source (injectable for tests).
pub type NowFn = Arc<dyn Fn() -> f64 + Send + Sync>;

pub fn epoch_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0)
}

#[derive(Default)]
struct Mem {
    map: HashMap<String, (f64, Value)>,
    order: VecDeque<String>,
}

pub struct DiskCache {
    dir: Option<PathBuf>,
    now: NowFn,
    mem: Mutex<Mem>,
}

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

impl DiskCache {
    pub fn new(dir: Option<PathBuf>, now: NowFn) -> DiskCache {
        DiskCache { dir, now, mem: Mutex::new(Mem::default()) }
    }

    /// `None` on miss or expiry. `ttl_ms = None` never expires.
    pub fn get(&self, ns: &str, key: &str, ttl_ms: Option<f64>) -> Option<Value> {
        let id = format!("{ns}/{key}");
        let mut e = self.mem.lock().unwrap().map.get(&id).cloned();
        if e.is_none() {
            if let Some(f) = self.file(ns, key) {
                let parsed = std::fs::read_to_string(&f).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok());
                if let Some(Value::Object(mut o)) = parsed {
                    if let Some(saved) = o.get("savedAt").and_then(Value::as_f64) {
                        let data = o.remove("data").unwrap_or(Value::Null);
                        self.remember(&id, saved, data.clone());
                        e = Some((saved, data));
                    }
                }
            }
        }
        let (saved, data) = e?;
        if let Some(ttl) = ttl_ms {
            if (self.now)() - saved > ttl {
                return None;
            }
        }
        Some(data)
    }

    pub fn set(&self, ns: &str, key: &str, data: &Value) {
        let saved = (self.now)();
        self.remember(&format!("{ns}/{key}"), saved, data.clone());
        let Some(f) = self.file(ns, key) else { return };
        let _ = (|| -> std::io::Result<()> {
            std::fs::create_dir_all(f.parent().unwrap_or(Path::new(".")))?;
            let tmp = PathBuf::from(format!("{}.{}.{}.tmp", f.display(), std::process::id(), TMP_SEQ.fetch_add(1, Ordering::Relaxed)));
            std::fs::write(&tmp, serde_json::to_string(&json!({ "savedAt": num(saved), "data": data })).unwrap_or_default())?;
            std::fs::rename(&tmp, &f).inspect_err(|_| {
                let _ = std::fs::remove_file(&tmp);
            })
        })();
    }

    fn remember(&self, id: &str, saved: f64, data: Value) {
        let mut m = self.mem.lock().unwrap();
        if m.map.remove(id).is_some() {
            m.order.retain(|k| k != id);
        }
        m.map.insert(id.to_string(), (saved, data));
        m.order.push_back(id.to_string());
        if m.map.len() > MEM_MAX {
            if let Some(old) = m.order.pop_front() {
                m.map.remove(&old);
            }
        }
    }

    fn file(&self, ns: &str, key: &str) -> Option<PathBuf> {
        let dir = self.dir.as_ref()?;
        let ns: String = ns.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
        Some(dir.join(ns).join(format!("{}.json", safe_name(key))))
    }
}

/// Whole numbers as JSON integers (like JS `JSON.stringify(Date.now())`).
pub fn num(v: f64) -> Value {
    if v.fract() == 0.0 && v.abs() < 9.0e15 {
        json!(v as i64)
    } else {
        json!(v)
    }
}

/// File-system safe, collision-free name. A hash suffix is added whenever the
/// string was altered or contains lowercase letters (puuids are case-sensitive,
/// Windows file names are not).
pub fn safe_name(s: &str) -> String {
    // JS `.slice(0, 80)` counts UTF-16 units; every non-ASCII char became '_' (1 unit) except
    // astral chars (2 units, 1 '_' each per surrogate) — replicate by mapping per UTF-16 unit.
    let units: Vec<u16> = s.encode_utf16().collect();
    let clean: String = units
        .iter()
        .take(80)
        .map(|&u| {
            let c = char::from_u32(u as u32).unwrap_or('_');
            if c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if clean == s && !s.is_empty() && !s.starts_with('.') && !s.chars().any(|c| c.is_ascii_lowercase()) {
        return s.to_string();
    }
    let h = sha1_hex(s.as_bytes());
    format!("{clean}-{}", &h[..12])
}

/// SHA-1 (only for cache file names, matching Node's `createHash('sha1')`).
pub fn sha1_hex(data: &[u8]) -> String {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_vectors() {
        assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(sha1_hex(&[b'a'; 1000]), "291e9a6c66994949b57ba5e650361e98fc36b1ba");
    }

    #[test]
    fn safe_names() {
        assert_eq!(safe_name("EUN1_42"), "EUN1_42");
        let p = safe_name("abc?queue=420");
        assert!(p.starts_with("abc_queue_420-") && p.len() == "abc_queue_420-".len() + 12, "{p}");
        assert_ne!(safe_name("ABC_x"), safe_name("abc_x"));
        assert!(safe_name(".hidden").starts_with(".hidden-"));
    }

    #[test]
    fn ttl_disk_and_memory() {
        let d = tempfile::tempdir().unwrap();
        let t = Arc::new(Mutex::new(1000.0));
        let tt = t.clone();
        let now: NowFn = Arc::new(move || *tt.lock().unwrap());
        let c = DiskCache::new(Some(d.path().to_path_buf()), now.clone());
        assert!(c.get("match", "EUN1_1", None).is_none());
        c.set("match", "EUN1_1", &json!({ "a": 1 }));
        assert_eq!(c.get("match", "EUN1_1", Some(10.0)).unwrap()["a"], 1);
        *t.lock().unwrap() = 1011.0;
        assert!(c.get("match", "EUN1_1", Some(10.0)).is_none());
        assert!(c.get("match", "EUN1_1", None).is_some());
        // A fresh instance reads the file (same format as the TS cache).
        let text = std::fs::read_to_string(d.path().join("match").join("EUN1_1.json")).unwrap();
        assert_eq!(text, "{\"savedAt\":1000,\"data\":{\"a\":1}}");
        let c2 = DiskCache::new(Some(d.path().to_path_buf()), now);
        assert_eq!(c2.get("match", "EUN1_1", None).unwrap()["a"], 1);
    }
}
