//! In-game HUD: ONE transparent, click-through, always-on-top, never-focusable window laid
//! exactly over the game's client area (TS used one BrowserWindow per panel, see hud.ts).
//! The page (`hud-all.html`) lays the panels out itself; Rust decides which panels have
//! content (`wants`) and hides the whole window when none does.
//!
//! The window never takes focus and never receives input (WS_EX_NOACTIVATE |
//! WS_EX_TRANSPARENT | WS_EX_LAYERED | WS_EX_TOOLWINDOW, shown with SW_SHOWNOACTIVATE).
//! Nothing is ever sent to the game window: we only read its position.

use crate::hud_state::{compose_hud, HudInputs};
use crate::state::AppState;
use crate::vision::{champion_icon, now_ms};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub const LABEL: &str = "hud";
const PANELS: [&str; 7] = ["minimap", "topbar", "death", "loading", "power", "build", "notify"];
/// Keep the window mapped this long after its content goes away (renderer fades out).
const HIDE_DELAY: Duration = Duration::from_millis(180);

/// Game client area in physical screen pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub foreground: bool,
}

#[derive(Default)]
pub struct HudCtl {
    pub game: Option<ClientRect>,
    shown: bool,
    placed: Option<ClientRect>,
    hide_at: Option<Instant>,
    last_payload: String,
    last_emit: Option<Instant>,
}

pub type Hud = Mutex<HudCtl>;

impl HudCtl {
    /// Last payload pushed to the HUD page (JSON text).
    pub fn last_payload_text(&self) -> String {
        self.last_payload.clone()
    }
}

#[cfg(windows)]
fn game_client_rect() -> Option<ClientRect> {
    use windows_sys::Win32::Foundation::{POINT, RECT};
    use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, GetClientRect, GetForegroundWindow, IsIconic};
    let title: Vec<u16> = rc_vision::capture::GAME_WINDOW_TITLE.encode_utf16().chain(Some(0)).collect();
    // SAFETY: read-only Win32 queries on a window handle; no messages are sent to the game.
    unsafe {
        let h = FindWindowW(std::ptr::null(), title.as_ptr());
        if h.is_null() || IsIconic(h) != 0 {
            return None;
        }
        let mut r: RECT = std::mem::zeroed();
        let mut p = POINT { x: 0, y: 0 };
        if GetClientRect(h, &mut r) == 0 || ClientToScreen(h, &mut p) == 0 {
            return None;
        }
        let rect = ClientRect { x: p.x, y: p.y, width: r.right - r.left, height: r.bottom - r.top, foreground: GetForegroundWindow() == h };
        (rect.width >= 100).then_some(rect)
    }
}

#[cfg(not(windows))]
fn game_client_rect() -> Option<ClientRect> {
    None
}

/// Create the HUD window (hidden). Its styles make it inert for mouse and keyboard.
pub fn create(app: &tauri::AppHandle) -> tauri::Result<()> {
    let w = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("hud-all.html".into()))
        .title("Rift Coach HUD")
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .focusable(false)
        .visible(false)
        .inner_size(100.0, 100.0)
        .position(-32000.0, -32000.0)
        .background_color(tauri::window::Color(0, 0, 0, 0))
        .build()?;
    w.set_ignore_cursor_events(true)?;
    #[cfg(windows)]
    if let Ok(h) = w.hwnd() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_APPWINDOW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT};
        // SAFETY: our own top-level window.
        unsafe {
            let h = h.0 as _;
            let ex = GetWindowLongPtrW(h, GWL_EXSTYLE) as u32;
            SetWindowLongPtrW(h, GWL_EXSTYLE, ((ex & !WS_EX_APPWINDOW) | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST) as isize);
        }
    }
    Ok(())
}

/// Show/move/hide on the main thread, never activating the window.
fn apply(app: &tauri::AppHandle, show: Option<ClientRect>) {
    // --smoke never shows a window (it may run next to a live game) unless --smoke-show-hud.
    let show = show.filter(|_| !crate::smoke() || std::env::args().any(|a| a == "--smoke-show-hud"));
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(w) = app2.get_webview_window(LABEL) else { return };
        #[cfg(windows)]
        if let Ok(h) = w.hwnd() {
            use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowPos, ShowWindow, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SW_HIDE, SW_SHOWNOACTIVATE};
            let h = h.0 as _;
            // SAFETY: our own top-level window; SWP_NOACTIVATE / SW_SHOWNOACTIVATE never take focus.
            unsafe {
                match show {
                    Some(r) => {
                        SetWindowPos(h, HWND_TOPMOST, r.x, r.y, r.width, r.height, SWP_NOACTIVATE | SWP_NOOWNERZORDER);
                        ShowWindow(h, SW_SHOWNOACTIVATE);
                    }
                    None => {
                        ShowWindow(h, SW_HIDE);
                    }
                }
            }
            return;
        }
        let _ = w.hide();
    });
}

fn is_live(expires_at: Option<f64>, game_time: f64, now: f64) -> bool {
    match expires_at {
        Some(e) if e > 1e11 => e > now,
        Some(e) => e > game_time,
        None => false,
    }
}

/// Which panels have something to show (`wants` in hud.ts).
fn wants(st: &Value, has_minimap: bool, now: f64) -> Value {
    if !st["visible"].as_bool().unwrap_or(false) {
        return json!({});
    }
    let gt = st["gameTime"].as_f64().unwrap_or(0.0);
    let exp = |v: &Value| v.get("expiresAt").and_then(Value::as_f64);
    let urgent = |o: &Value| o.is_object() && o.get("inSec").and_then(Value::as_f64).is_none_or(|s| s <= 30.0);
    let arr = |k: &str| st[k].as_array().cloned().unwrap_or_default();
    let minimap = has_minimap
        && (!arr("ghosts").is_empty()
            || st["showFrame"].as_bool().unwrap_or(false)
            || st["recording"].as_bool().unwrap_or(false)
            || st["objective"].is_object()
            || arr("hints").iter().any(|h| is_live(exp(h), gt, now))
            || arr("timers").iter().any(|t| t["secondsLeft"].as_f64().is_some_and(|s| s > 0.0 && s <= 60.0)));
    let high_danger = matches!(st["danger"]["label"].as_str(), Some("niebezpiecznie" | "uciekaj")) && !st["deathCard"].is_object();
    let topbar = (st["topBar"].is_object() && is_live(exp(&st["topBar"]), gt, now)) || (st["alert"].is_object() && is_live(exp(&st["alert"]), gt, now)) || urgent(&st["objective"]) || high_danger;
    json!({
        "minimap": minimap,
        "topbar": topbar,
        "death": st["deathCard"].is_object(),
        "loading": st["loadingCard"].is_object(),
        "power": (st["power"].is_object() || st["danger"].is_object() || st["alert"].is_object() || st["objective"].is_object()) && !st["deathCard"].is_object(),
        "build": st["build"]["items"].as_array().is_some_and(|a| !a.is_empty()),
        "notify": !arr("toasts").is_empty() || !arr("buffs").is_empty() || !arr("returns").is_empty(),
    })
}

/// Game window moved / calibration changed: re-place and re-push (placeOverlay).
pub fn place(app: &tauri::AppHandle, st: &Arc<AppState>) {
    push(app, st);
}

/// Compose and push the HUD state; show/hide/move the window (pushOverlay + Hud.update).
pub fn push(app: &tauri::AppHandle, st: &Arc<AppState>) {
    let now = now_ms();
    let visible_setting = st.settings.lock().unwrap().value().overlay_visible;
    let recording = st.recorder.lock().unwrap().active();
    let visible = visible_setting && !st.replay_blocks() && !crate::playback::is_open(st);
    let show_frame = st.vision.show_frame();
    let advice = st.ui.lock().unwrap().last_advice.clone();
    let state = {
        let mut g = st.game.lock().unwrap();
        g.field.alerts.retain(|a| a.expires_at > now);
        g.field.toasts.retain(|t| t.expires_at > now);
        let g = &*g;
        let icon = |c: &str| champion_icon(g, c);
        let vis_fn = |x: f64, y: f64| g.field.visible_at(x, y).unwrap_or(0.0);
        let vision_at: Option<&dyn Fn(f64, f64) -> f64> = if g.field.mask.is_some() { Some(&vis_fn) } else { None };
        use crate::knowledge as k;
        // Fight here and now (who is near, HP, mana...). With the edge on our side a gank warning is noise:
        // the ganker is the one in trouble.
        let fight_p = g.intel.as_ref().and_then(|i| rc_core::intel::matchup::power_bar_near(i, g.matchup.as_ref(), &|c: &str| c.to_string(), &k::nearby(g))).map(|p| p.p_me);
        let alerts: Vec<crate::vision::Alert> = g.field.alerts.iter().filter(|a| !(a.kind == "gank" && fight_p.is_some_and(|p| p >= 0.6))).cloned().collect();
        let profiles: Vec<Value> = g.profiles.iter().filter_map(|p| serde_json::to_value(p).ok()).collect();
        let (power, danger, objective, fed) = match g.intel.as_ref() {
            Some(i) => {
                let fed = k::fed_names(i);
                let tracks = k::fog_tracks(g);
                let (allies, enemies) = (k::seen(&g.field.allies), k::seen(&g.field.visible_enemies));
                (
                    rc_core::intel::matchup::power_bar_near(i, g.matchup.as_ref(), &icon, &k::nearby(g)).and_then(|p| serde_json::to_value(p).ok()),
                    rc_core::intel::situational::danger_at(i, k::pos(g.field.my_pos), &tracks, &enemies, fight_p.or_else(|| k::current_p_me(g)), &fed).and_then(|d| serde_json::to_value(d).ok()),
                    rc_core::intel::situational::objective_call(i, &tracks, &allies, &enemies).and_then(|o| serde_json::to_value(o).ok()),
                    fed,
                )
            }
            None => (None, None, None, Vec::new()),
        };
        let mut s = compose_hud(&HudInputs {
            visible,
            intel: g.intel.as_ref(),
            advice: advice.as_ref(),
            alerts: &alerts,
            ghosts: &g.field.ghosts,
            profiles: &profiles,
            champion_icon: &icon,
            show_frame,
            recording,
            power,
            build: g.build.as_ref().and_then(|b| serde_json::to_value(b).ok()),
            danger,
            objective,
            fed,
            vision_at,
            me: g.field.my_pos,
            now,
        });
        if let Some(o) = s.as_object_mut() {
            let gt = g.intel.as_ref().map(|i| i.game_time).unwrap_or(0.0);
            o.insert("toasts".into(), Value::Array(g.field.toasts.iter().map(|t| serde_json::json!({ "id": t.id, "kind": t.kind, "team": t.team, "text": t.text, "iconUrl": t.champion.as_deref().map(&icon), "expiresAt": t.expires_at })).collect()));
            o.insert("returns".into(), Value::Array(g.field.returns.iter().map(|(c, until)| serde_json::json!({ "champion": c, "iconUrl": icon(c), "secondsLeft": ((until - now) / 1000.0).max(0.0).round() })).collect()));
            o.insert("buffs".into(), Value::Array(g.intel.as_ref().map(|i| i.buffs.iter().map(|b| serde_json::json!({ "kind": b.kind, "team": b.team, "secondsLeft": (b.until - gt).max(0.0).round() })).collect()).unwrap_or_default()));
        }
        s
    };
    let mut h = st.hud.lock().unwrap();
    let game = h.game;
    // Minimap square as fractions of the game client area (the page works in its own CSS px).
    let minimap = match (game, st.vision.calibration()) {
        (Some(g), Some(c)) => {
            let r = c.rect_for(g.width, g.height);
            Some(json!({ "x": r.x as f64 / g.width as f64, "y": r.y as f64 / g.height as f64, "width": r.width as f64 / g.width as f64, "height": r.height as f64 / g.height as f64 }))
        }
        _ => None,
    };
    let wanted = if game.is_some() { wants(&state, minimap.is_some(), now) } else { json!({}) };
    let any = PANELS.iter().any(|k| wanted[*k].as_bool().unwrap_or(false));
    let payload = json!({ "state": state, "wanted": wanted, "minimap": minimap });
    let text = payload.to_string();
    // Changed, or a 1 s refresh (the page may have loaded after the last change).
    if text != h.last_payload || h.last_emit.is_none_or(|t| t.elapsed() >= Duration::from_secs(1)) {
        h.last_payload = text;
        h.last_emit = Some(Instant::now());
        let _ = app.emit_to(LABEL, "hud", &payload);
    }
    match (any, game) {
        (true, Some(g)) => {
            h.hide_at = None;
            if !h.shown || h.placed != Some(g) {
                h.shown = true;
                h.placed = Some(g);
                apply(app, Some(g));
            }
        }
        _ if h.shown => {
            let at = *h.hide_at.get_or_insert_with(|| Instant::now() + if game.is_some() { HIDE_DELAY } else { Duration::ZERO });
            if Instant::now() >= at {
                h.shown = false;
                h.placed = None;
                h.hide_at = None;
                apply(app, None);
            }
        }
        _ => {}
    }
}

/// Window tracker (1 s, read-only Win32 like winrect.ts) + HUD pushes every 100 ms.
pub fn spawn(app: &tauri::AppHandle, st: &Arc<AppState>) {
    let (app, st) = (app.clone(), st.clone());
    std::thread::Builder::new()
        .name("rc-hud".into())
        .spawn(move || {
            let mut last_rect = Instant::now() - Duration::from_secs(5);
            loop {
                if last_rect.elapsed() >= Duration::from_millis(500) {
                    last_rect = Instant::now();
                    // Only over the game while it is the active window: never float over the
                    // browser / desktop after alt-tab (and never hide the taskbar there).
                    let r = game_client_rect().filter(|r| r.foreground || crate::hud_test());
                    st.hud.lock().unwrap().game = r;
                }
                push(&app, &st);
                std::thread::sleep(Duration::from_millis(100));
            }
        })
        .expect("spawn hud thread");
}

/// One-line state for `--smoke`.
pub fn smoke_summary(st: &AppState) -> Value {
    let h = st.hud.lock().unwrap();
    let payload: Value = serde_json::from_str(&h.last_payload).unwrap_or(Value::Null);
    json!({
        "game": h.game.map(|g| json!([g.x, g.y, g.width, g.height, g.foreground])),
        "shown": h.shown,
        "wanted": payload.get("wanted"),
        "minimap": payload.get("minimap"),
        "visible": payload.pointer("/state/visible"),
        "ghosts": payload.pointer("/state/ghosts").and_then(Value::as_array).map(|a| a.len()),
        "hints": payload.pointer("/state/hints"),
        "alert": payload.pointer("/state/alert"),
        "timers": payload.pointer("/state/timers"),
    })
}
