//! Log file for debugging games afterwards: %APPDATA%\Rift Coach\logs\rift-coach-YYYY-MM-DD.log,
//! one file per day, kept 14 days. Everything the app logs (log::info! and up from our code, warnings
//! from libraries) plus panics. Targets used for game debugging: "hud" (what the overlay shows, on
//! every change), "field" (positions on the minimap, every 2 s), "coach" (what the model got and
//! answered), "build".
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

struct FileLog {
    dir: PathBuf,
    file: Mutex<Option<(String, std::fs::File)>>,
}

/// (YYYY-MM-DD, HH:MM:SS.mmm) local time.
#[cfg(windows)]
fn local_now() -> (String, String) {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    // SAFETY: plain Win32 call with a valid out pointer.
    let mut t: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut t) };
    (format!("{:04}-{:02}-{:02}", t.wYear, t.wMonth, t.wDay), format!("{:02}:{:02}:{:02}.{:03}", t.wHour, t.wMinute, t.wSecond, t.wMilliseconds))
}

#[cfg(not(windows))]
fn local_now() -> (String, String) {
    let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    ((s / 86_400_000).to_string(), format!("{}", s % 86_400_000))
}

fn ours(target: &str) -> bool {
    target.starts_with("rift_coach") || target.starts_with("rc_") || matches!(target, "hud" | "field" | "coach" | "build" | "game")
}

impl log::Log for FileLog {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Warn || (m.level() <= log::Level::Info && ours(m.target()))
    }

    fn log(&self, r: &log::Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let (date, time) = local_now();
        let mut f = self.file.lock().unwrap_or_else(|e| e.into_inner());
        if f.as_ref().is_none_or(|(d, _)| *d != date) {
            let path = self.dir.join(format!("rift-coach-{date}.log"));
            *f = std::fs::OpenOptions::new().create(true).append(true).open(path).ok().map(|file| (date, file));
        }
        if let Some((_, file)) = f.as_mut() {
            let _ = writeln!(file, "{time} {:<5} [{}] {}", r.level(), r.target(), r.args());
        }
    }

    fn flush(&self) {
        if let Some((_, file)) = self.file.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            let _ = file.flush();
        }
    }
}

/// Logs folder (created).
pub fn dir() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("Rift Coach").join("logs")
}

pub fn init() {
    let dir = dir();
    let _ = std::fs::create_dir_all(&dir);
    // Keep 14 days.
    if let Ok(rd) = std::fs::read_dir(&dir) {
        let mut logs: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("rift-coach-") && n.ends_with(".log"))).collect();
        logs.sort();
        let n = logs.len();
        for p in logs.into_iter().take(n.saturating_sub(14)) {
            let _ = std::fs::remove_file(p);
        }
    }
    if log::set_logger(Box::leak(Box::new(FileLog { dir, file: Mutex::new(None) }))).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("PANIC {info}");
        log::logger().flush();
        default(info);
    }));
    log::info!("start {}", env!("CARGO_PKG_VERSION"));
}

/// JSON without what only the page needs (icons) or what changes every second anyway (clocks), so
/// "changed" means something on screen changed.
pub fn strip(v: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match v {
        Value::Object(o) => Value::Object(
            o.iter()
                .filter(|(k, _)| !matches!(k.as_str(), "iconUrl" | "icon" | "imageUrl" | "expiresAt" | "secondsLeft" | "updatedAt" | "gameTime" | "profiles" | "ghosts" | "at" | "id"))
                .map(|(k, v)| (k.clone(), strip(v)))
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(strip).collect()),
        Value::String(s) if s.starts_with("data:") || s.starts_with("http") => Value::String("…".into()),
        x => x.clone(),
    }
}
