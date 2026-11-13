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
    /// Last HUD state written to the log file (logged again only when it changes).
    log_hud: String,
    log_field_at: Option<Instant>,
    /// Draggable panels' visible boxes (kind, [x, y, w, h] in CSS px of the HUD window), from the page.
    pub rects: Vec<(String, [f64; 4])>,
    hover_kind: Option<String>,
    hover_since: Option<Instant>,
    hover_sent: bool,
    /// Dragging: (panel, cursor x, cursor y at the press) in CSS px.
    drag: Option<(String, f64, f64)>,
    last_press: Option<(Instant, String)>,
    lbutton_prev: bool,
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
        "build": st["build"]["items"].as_array().is_some_and(|a| !a.is_empty()) || st["build"]["buyNow"].as_array().is_some_and(|a| !a.is_empty()),
        "notify": !arr("toasts").is_empty() || !arr("buffs").is_empty() || !arr("returns").is_empty(),
    })
}

/// "Kup teraz" for the build panel: only when I can shop (dead, in my base, or the first 90 s), from my
/// gold and the next planned item's recipe.
fn buy_now_json(g: &crate::game::Game) -> Option<Value> {
    let (intel, sd) = (g.intel.as_ref()?, g.static_data.as_ref()?);
    let me = intel.me.as_ref()?;
    let gold = intel.my_stats.as_ref()?.gold;
    let base = if me.team == "CHAOS" { (0.96, 0.04) } else { (0.04, 0.96) };
    let in_base = g.field.my_pos.is_some_and(|(x, y)| (x - base.0).hypot(y - base.1) < 0.12);
    if !(me.is_dead || in_base || intel.game_time < 90.0) || gold < 300.0 {
        return None;
    }
    let plan_first = g.build.as_ref().and_then(|b| b.items.first()).map(|i| i.name.to_lowercase());
    let target = plan_first.as_ref().and_then(|want| sd.items.values().find(|i| i.name.to_lowercase() == *want || i.name_en.as_ref().is_some_and(|n| n.to_lowercase() == *want)));
    // Game start, nothing bought yet: the starter (Doran's + potions / jungle pet / World Atlas), not a
    // full item ("pokazywało przedmiot, a nie pierścień z potkami").
    let bought_anything = me.items.iter().any(|i| sd.items.get(&i.id).is_some_and(|d| d.total >= 300.0));
    let ids = if !bought_anything && intel.game_time < 180.0 {
        let tag = |t: &str| target.is_some_and(|d| d.tags.iter().any(|x| x == t));
        let (start, pots): (i64, usize) = match me.role.as_str() {
            "UTILITY" => (3865, 2),
            "JUNGLE" => (if tag("SpellDamage") { 1102 } else if tag("Armor") || tag("Health") && !tag("Damage") { 1103 } else { 1101 }, 1),
            _ if tag("SpellDamage") => (1056, 2),
            "TOP" if (tag("Armor") || tag("Health")) && !tag("Damage") => (1054, 1),
            _ => (1055, 1),
        };
        std::iter::once(start).chain(std::iter::repeat_n(2003, pots)).filter(|id| sd.items.contains_key(id)).collect()
    } else {
        let Some(target) = target else { return None };
        let owned: Vec<i64> = me.items.iter().map(|i| i.id).collect();
        let has_boots = me.items.iter().any(|i| sd.items.get(&i.id).is_some_and(|d| d.tags.iter().any(|t| t == "Boots")));
        rc_core::build_advisor::buy_now(target, &sd.items, &owned, gold, !has_boots && intel.game_time > 4.0 * 60.0)
    };
    if ids.is_empty() {
        return None;
    }
    let version = &sd.version;
    Some(Value::Array(
        ids.iter()
            .filter_map(|id| sd.items.get(id))
            .map(|i| serde_json::json!({ "name": i.name, "iconUrl": format!("https://ddragon.leagueoflegends.com/cdn/{version}/img/item/{}.png", i.id), "cost": i.total }))
            .collect(),
    ))
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
    // Summoner's Rift only: no overlay at all in ARAM / Arena / other modes (no death card, no build).
    let rift = st.game.lock().unwrap().field.map11 || crate::hud_test();
    let visible = visible_setting && rift && !st.replay_blocks() && !crate::playback::is_open(st);
    let show_frame = st.vision.show_frame();
    let advice = st.ui.lock().unwrap().last_advice.clone();
    let mut field_line: Option<String> = None;
    let mut log_t = 0.0;
    let state = {
        let mut g = st.game.lock().unwrap();
        g.field.alerts.retain(|a| a.expires_at > now);
        g.field.toasts.retain(|t| t.expires_at > now);
        g.field.expire_if_stale(now);
        let g = &*g;
        let icon = |c: &str| champion_icon(g, c);
        let vis_fn = |x: f64, y: f64| g.field.visible_at(x, y).unwrap_or(0.0);
        let vision_at: Option<&dyn Fn(f64, f64) -> f64> = if g.field.mask.is_some() { Some(&vis_fn) } else { None };
        use crate::knowledge as k;
        // Fight here and now (who is near, HP, mana...). With the edge on our side a gank warning is noise:
        // the ganker is the one in trouble.
        // Only a real local fight counts (someone near); and a gank warning is muted only when allies
        // are actually with me and the fight there is clearly ours.
        let fight = g.intel.as_ref().and_then(|i| rc_core::intel::matchup::power_bar_near(i, g.matchup.as_ref(), &|c: &str| c.to_string(), &k::nearby(g))).filter(|p| p.local);
        let fight_p = fight.as_ref().map(|p| p.p_me);
        let mute_gank = fight.as_ref().is_some_and(|p| p.p_me >= 0.6 && !p.allies.is_empty());
        let alerts: Vec<crate::vision::Alert> = g.field.alerts.iter().filter(|a| !(a.kind == "gank" && mute_gank)).cloned().collect();
        let profiles: Vec<Value> = g.profiles.iter().filter_map(|p| serde_json::to_value(p).ok()).collect();
        let (power, danger, objective, fed) = match g.intel.as_ref() {
            Some(i) => {
                let fed = k::fed_names(i);
                let tracks = k::fog_tracks(g);
                let (allies, enemies) = (k::seen(&g.field.allies), k::seen(&g.field.visible_enemies));
                (
                    {
                        // Not while everyone is still walking to lane or I stand in my base: nothing to fight.
                        let base = if i.me.as_ref().is_some_and(|m| m.team == "CHAOS") { (0.96, 0.04) } else { (0.04, 0.96) };
                        let in_base = g.field.my_pos.is_some_and(|(x, y)| (x - base.0).hypot(y - base.1) < 0.15);
                        if i.game_time < 75.0 || in_base {
                            None
                        } else {
                            rc_core::intel::matchup::power_bar_near(i, g.matchup.as_ref(), &icon, &k::nearby(g)).and_then(|p| serde_json::to_value(p).ok())
                        }
                    },
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
            // No plan yet (game start): the panel still shows the starter to buy.
            build: match (g.build.as_ref().and_then(|b| serde_json::to_value(b).ok()), buy_now_json(g)) {
                (Some(mut v), buy) => {
                    if let (Some(list), Some(o)) = (buy, v.as_object_mut()) {
                        o.insert("buyNow".into(), list);
                    }
                    Some(v)
                }
                (None, Some(list)) => Some(json!({ "phase": "start", "updatedAt": now, "items": [], "buyNow": list })),
                (None, None) => None,
            },
            danger,
            objective,
            fed,
            vision_at,
            me: g.field.my_pos,
            now,
        });
        // Positions for the log file (tracker debugging): me, allies, seen enemies, fog guesses.
        field_line = g.intel.as_ref().filter(|_| g.field.map11).map(|i| {
            let r3 = |v: f64| (v * 1000.0).round() / 1000.0;
            let pos = |u: &crate::vision::SeenUnit| json!([u.champion, r3(u.x), r3(u.y)]);
            let n = k::nearby(g);
            let near = json!({ "allies": n.allies, "enemies": n.enemies, "underEnemyTower": n.me_under_enemy_tower, "underMyTower": n.me_under_my_tower });
            json!({
                "t": i.game_time.round(),
                "me": g.field.my_pos.map(|(x, y)| [r3(x), r3(y)]),
                "hp": i.my_stats.as_ref().filter(|s| s.max_hp > 0.0).map(|s| (s.hp / s.max_hp * 100.0).round()),
                "allies": g.field.allies.iter().map(pos).collect::<Vec<_>>(),
                "enemies": g.field.visible_enemies.iter().map(pos).collect::<Vec<_>>(),
                "ghosts": serde_json::to_value(&g.field.ghosts).map(|v| crate::applog::strip(&v)).unwrap_or(Value::Null),
                "near": near,
            })
            .to_string()
        });
        log_t = g.intel.as_ref().map(|i| i.game_time).unwrap_or(0.0);
        // Fight worth joining: arrow on the minimap from me to that ally (follows him while he is seen).
        if let (Some((ally, until)), Some((mx, my))) = (g.field.help.as_ref().filter(|h| h.1 > now), g.field.my_pos) {
            if let (Some(a), Some(hints)) = (g.field.allies.iter().find(|a| a.champion == *ally), s.get_mut("hints").and_then(Value::as_array_mut)) {
                hints.push(json!({ "kind": "move", "x": a.x, "y": a.y, "from": { "x": mx, "y": my }, "label": format!("Pomóż {ally}"), "expiresAt": until }));
            }
        }
        if let Some(o) = s.as_object_mut() {
            let gt = g.intel.as_ref().map(|i| i.game_time).unwrap_or(0.0);
            o.insert("toasts".into(), Value::Array(g.field.toasts.iter().map(|t| serde_json::json!({ "id": t.id, "kind": t.kind, "team": t.team, "text": t.text, "iconUrl": t.champion.as_deref().map(&icon), "expiresAt": t.expires_at })).collect()));
            o.insert("returns".into(), Value::Array(g.field.returns.iter().map(|(c, until)| serde_json::json!({ "champion": c, "iconUrl": icon(c), "secondsLeft": ((until - now) / 1000.0).max(0.0).round() })).collect()));
            o.insert("buffs".into(), Value::Array(g.intel.as_ref().map(|i| i.buffs.iter().map(|b| serde_json::json!({ "kind": b.kind, "team": b.team, "secondsLeft": (b.until - gt).max(0.0).round() })).collect()).unwrap_or_default()));
        }
        s
    };
    let mut h = st.hud.lock().unwrap();
    // Log file: what the overlay shows (on change) and where everyone is (every 2 s), in game only.
    if field_line.is_some() {
        let shown = crate::applog::strip(&state).to_string();
        if shown != h.log_hud {
            log::info!(target: "hud", "t={log_t:.0} {shown}");
            h.log_hud = shown;
        }
        if h.log_field_at.is_none_or(|t| t.elapsed() >= Duration::from_secs(2)) {
            h.log_field_at = Some(Instant::now());
            log::info!(target: "field", "{}", field_line.as_deref().unwrap_or(""));
        }
    }
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
    let layout = st.settings.lock().unwrap().json().get("hudLayout").cloned().unwrap_or(Value::Null);
    let payload = json!({ "state": state, "wanted": wanted, "minimap": minimap, "layout": layout });
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

/// Drag panels by just hovering. The HUD window stays click-through (an input-taking transparent
/// WebView2 window either passes every click on or stops drawing); instead the app watches the cursor and
/// the left button: rest on a panel for 0.3 s (the panel lights up), then press and drag. Double-click =
/// default place. The page reports panel boxes ('hud-rects') and gets 'hud-hover' / 'hud-drag' /
/// 'hud-drag-end' / 'hud-drag-reset'; it saves new positions with 'hud-layout' (settings.hudLayout,
/// fractions of the game client area).
pub fn spawn_drag(app: &tauri::AppHandle, st: &Arc<AppState>) {
    use tauri::Listener;
    {
        let st = st.clone();
        app.listen("hud-rects", move |e| {
            if let Ok(v) = serde_json::from_str::<Vec<(String, [f64; 4])>>(e.payload()) {
                st.hud.lock().unwrap().rects = v;
            }
        });
    }
    {
        let (st, app2) = (st.clone(), app.clone());
        app.listen("hud-layout", move |e| {
            let Ok(v) = serde_json::from_str::<Value>(e.payload()) else { return };
            let Some(kind) = v.get("kind").and_then(Value::as_str).filter(|k| PANELS.contains(k) && *k != "minimap") else { return };
            let mut s = st.settings.lock().unwrap();
            let mut layout = s.json().get("hudLayout").and_then(Value::as_object).cloned().unwrap_or_default();
            if v.get("reset").and_then(Value::as_bool) == Some(true) {
                layout.remove(kind);
            } else if let (Some(x), Some(y)) = (v.get("x").and_then(Value::as_f64), v.get("y").and_then(Value::as_f64)) {
                layout.insert(kind.to_string(), json!({ "x": x.clamp(0.0, 0.98), "y": y.clamp(0.0, 0.98) }));
            }
            if let Err(err) = s.update(&json!({ "hudLayout": layout })) {
                log::warn!("hud layout {err}");
            }
            drop(s);
            push(&app2, &st);
        });
    }
    let (app, st) = (app.clone(), st.clone());
    std::thread::Builder::new()
        .name("rc-hud-drag".into())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_millis(30));
            hover_tick(&app, &st);
        })
        .expect("spawn hud drag thread");
}

#[cfg(windows)]
fn hover_tick(app: &tauri::AppHandle, st: &Arc<AppState>) {
    use tauri::Emitter;
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let Some(win) = app.get_webview_window(LABEL) else { return };
    let scale = win.scale_factor().unwrap_or(1.0).max(0.1);
    let mut h = st.hud.lock().unwrap();
    // SAFETY: plain Win32 calls with valid out pointers.
    let lbutton = unsafe { GetAsyncKeyState(VK_LBUTTON as i32) } as u16 & 0x8000 != 0;
    let mut p = POINT { x: 0, y: 0 };
    let ok = unsafe { GetCursorPos(&mut p) } != 0;
    let (Some(g), true, true) = (h.placed, h.shown, ok) else {
        h.lbutton_prev = lbutton;
        return;
    };
    let (cx, cy) = ((p.x - g.x) as f64 / scale, (p.y - g.y) as f64 / scale);
    let mut emits: Vec<(&str, Value)> = Vec::new();
    if let Some((kind, sx, sy)) = h.drag.clone() {
        if lbutton {
            emits.push(("hud-drag", json!({ "kind": kind, "dx": cx - sx, "dy": cy - sy })));
        } else {
            emits.push(("hud-drag-end", json!({ "kind": kind })));
            h.drag = None;
        }
    } else {
        let over = h.rects.iter().find(|(_, r)| cx >= r[0] && cy >= r[1] && cx <= r[0] + r[2] && cy <= r[1] + r[3]).map(|(k, _)| k.clone());
        if over != h.hover_kind {
            h.hover_kind = over.clone();
            h.hover_since = over.as_ref().map(|_| Instant::now());
            if h.hover_sent {
                emits.push(("hud-hover", json!({ "kind": Value::Null })));
                h.hover_sent = false;
            }
        }
        let rested = h.hover_since.is_some_and(|t| t.elapsed() >= Duration::from_millis(300));
        if rested && !h.hover_sent {
            emits.push(("hud-hover", json!({ "kind": h.hover_kind })));
            h.hover_sent = true;
        }
        if lbutton && !h.lbutton_prev && rested {
            if let Some(kind) = h.hover_kind.clone() {
                let double = h.last_press.as_ref().is_some_and(|(t, k)| *k == kind && t.elapsed() < Duration::from_millis(400));
                if double {
                    emits.push(("hud-drag-reset", json!({ "kind": kind })));
                    h.last_press = None;
                } else {
                    h.drag = Some((kind.clone(), cx, cy));
                    h.last_press = Some((Instant::now(), kind));
                }
            }
        }
    }
    h.lbutton_prev = lbutton;
    drop(h);
    for (name, v) in emits {
        let _ = app.emit_to(LABEL, name, v);
    }
}

#[cfg(not(windows))]
fn hover_tick(_app: &tauri::AppHandle, _st: &Arc<AppState>) {}

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
