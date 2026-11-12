//! Global hotkeys (settings.hotkeys) and starting with Windows (registerHotkeys /
//! applyLoginItem in index.ts).

use crate::state::AppState;
use rc_core::settings::Hotkeys;
use serde_json::json;
use std::sync::{Arc, Mutex};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

/// Hotkeys registered now (re-registered only when the settings change).
static CURRENT: Mutex<Option<Hotkeys>> = Mutex::new(None);

fn toggle_overlay(app: &tauri::AppHandle, st: &Arc<AppState>) {
    let next = !st.settings.lock().unwrap().value().overlay_visible;
    if let Err(e) = st.settings.lock().unwrap().update(&json!({ "overlayVisible": next })) {
        log::warn!("settings {e}");
    }
    crate::hud::place(app, st);
}

/// Mark Flash on my lane opponent (fallback: the first enemy) — markEnemyFlash.
fn enemy_flash(st: &Arc<AppState>) {
    let mut g = st.game.lock().unwrap();
    let Some(intel) = g.intel.as_ref() else { return };
    let Some(me) = intel.me.as_ref() else { return };
    let enemies: Vec<_> = intel.players.iter().filter(|p| !p.is_ally).collect();
    let target = enemies.iter().find(|p| p.role == me.role && p.role != "UNKNOWN").or(enemies.first()).map(|p| p.champion.clone());
    let t = intel.game_time;
    if let Some(c) = target {
        g.tracker.mark_flash(&c, t);
    }
}

pub fn register(app: &tauri::AppHandle, st: &Arc<AppState>) {
    let hk = st.settings.lock().unwrap().value().hotkeys.clone();
    let mut cur = CURRENT.lock().unwrap();
    if cur.as_ref() == Some(&hk) {
        return;
    }
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let bind = |acc: &str, f: Box<dyn Fn() + Send + Sync>| {
        if acc.is_empty() {
            return;
        }
        let Ok(sc) = acc.parse::<Shortcut>() else {
            eprintln!("invalid hotkey {acc}");
            return;
        };
        if let Err(e) = gs.on_shortcut(sc, move |_, _, ev| {
            if ev.state == ShortcutState::Pressed {
                f();
            }
        }) {
            eprintln!("hotkey {acc}: {e}");
        }
    };
    let (a, s) = (app.clone(), st.clone());
    bind(&hk.toggle_overlay, Box::new(move || toggle_overlay(&a, &s)));
    // Recording is not ported yet: the hotkey stays free until it is.
    let s = st.clone();
    bind(&hk.enemy_flash, Box::new(move || enemy_flash(&s)));
    *cur = Some(hk);
}

/// For `--smoke`: which configured hotkeys are registered with the system.
pub fn smoke_summary(app: &tauri::AppHandle, st: &AppState) -> serde_json::Value {
    let hk = st.settings.lock().unwrap().value().hotkeys.clone();
    let gs = app.global_shortcut();
    let reg = |acc: &str| acc.parse::<Shortcut>().map(|s| gs.is_registered(s)).unwrap_or(false);
    // A combination nobody uses, to tell "taken by another program" from "registration broken".
    let probe = "Ctrl+Alt+Shift+F11".parse::<Shortcut>().ok().map(|s| {
        let ok = gs.on_shortcut(s, |_, _, _| {}).is_ok() && gs.is_registered(s);
        let _ = gs.unregister(s);
        ok
    });
    json!({ "toggleOverlay": [hk.toggle_overlay, reg(&hk.toggle_overlay)], "toggleRecording": [hk.toggle_recording, reg(&hk.toggle_recording)], "enemyFlash": [hk.enemy_flash, reg(&hk.enemy_flash)], "probeCtrlAltShiftF11": probe })
}

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_VALUE: &str = "Rift Coach";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// HKCU Run entry `"<exe>" --background` when launchAtLogin is on, removed when off.
/// Debug builds and --smoke never touch the registry.
#[cfg(windows)]
pub fn apply_login_item(launch: bool) {
    use windows_sys::Win32::System::Registry::{RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ};
    if cfg!(debug_assertions) || crate::smoke() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let cmd = wide(&format!("\"{}\" --background", exe.display()));
    let (key, name) = (wide(RUN_KEY), wide(RUN_VALUE));
    // SAFETY: plain registry calls on a key we open and close here.
    unsafe {
        let mut h: HKEY = std::ptr::null_mut();
        if RegCreateKeyExW(HKEY_CURRENT_USER, key.as_ptr(), 0, std::ptr::null(), REG_OPTION_NON_VOLATILE, KEY_SET_VALUE, std::ptr::null(), &mut h, std::ptr::null_mut()) != 0 {
            return;
        }
        if launch {
            RegSetValueExW(h, name.as_ptr(), 0, REG_SZ, cmd.as_ptr() as *const u8, (cmd.len() * 2) as u32);
        } else {
            RegDeleteValueW(h, name.as_ptr());
        }
        RegCloseKey(h);
    }
}

#[cfg(not(windows))]
pub fn apply_login_item(_launch: bool) {}
