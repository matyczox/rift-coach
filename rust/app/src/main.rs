#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bridge;
mod game;
mod runes;
mod state;

use rc_core::settings::{user_data_dir, SettingsStore};
use rc_core::sources::{lcu, live, replay};
use serde_json::Value;
use state::{AppState, UiState};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

/// Started by Windows at login: stay in the background until the user opens the window.
/// `--smoke`: hidden window, coach off, print the status after 12 s and quit (like the Electron app).
pub fn smoke() -> bool {
    std::env::args().any(|a| a == "--smoke")
}

fn background() -> bool {
    std::env::args().any(|a| a == "--background")
}

#[tauri::command]
fn rc_call(app: tauri::AppHandle, state: tauri::State<'_, Arc<AppState>>, method: String, args: Vec<Value>) -> Result<Value, String> {
    bridge::call(&app, state.inner(), &method, &args)
}

/// External links (e.g. developer.riotgames.com) open in the default browser; https only.
#[tauri::command]
fn open_external(url: String) -> Result<(), String> {
    if !url.starts_with("https://") || url.chars().any(|c| c.is_whitespace() || c == '"') {
        return Err("Tylko adresy https://".into());
    }
    std::process::Command::new("rundll32.exe").args(["url.dll,FileProtocolHandler", &url]).spawn().map(|_| ()).map_err(|e| e.to_string())
}

pub fn on_settings_changed(_app: &tauri::AppHandle, _st: &Arc<AppState>) {
    // Hotkeys, login item, overlay placement and capture config follow in later steps.
}

/// Show the window without activating it (Tauri's `show()` activates it).
fn show_inactive(w: &tauri::WebviewWindow) {
    #[cfg(windows)]
    if let Ok(h) = w.hwnd() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_SHOWNOACTIVATE};
        // SAFETY: valid top-level window handle owned by this process.
        unsafe { ShowWindow(h.0 as _, SW_SHOWNOACTIVATE) };
        return;
    }
    let _ = w.show();
}

fn emit_status(app: &tauri::AppHandle, st: &Arc<AppState>, last: &mut String) {
    let s = st.status();
    let Ok(text) = serde_json::to_string(&s) else { return };
    if text != *last {
        *last = text;
        let _ = app.emit("status", &s);
    }
}

fn main() {
    let data_dir = user_data_dir();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .invoke_handler(tauri::generate_handler![rc_call, open_external])
        .setup(move |app| {
            let rt = tauri::async_runtime::block_on(async { tokio::runtime::Handle::current() });
            let (live_shared, mut live_rx) = live::spawn(&rt, live::POLL_MS);
            let (lcu_client, lcu_shared, mut lcu_rx) = lcu::spawn(&rt);
            let replay_api = replay::spawn_state_poller(&rt);
            let st = Arc::new(AppState {
                version: app.package_info().version.to_string(),
                settings: Mutex::new(SettingsStore::open(&data_dir)),
                data_dir: data_dir.clone(),
                live: live_shared,
                lcu: lcu_shared,
                lcu_client,
                replay_api,
                ui: Mutex::new(UiState::default()),
                game: Mutex::new(game::Game::default()),
            });
            app.manage(st.clone());

            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("control.html".into()))
                .title("Rift Coach")
                .inner_size(1180.0, 780.0)
                .min_inner_size(900.0, 600.0)
                .decorations(false)
                .background_color(tauri::window::Color(0x0c, 0x0f, 0x14, 0xff))
                .focused(false)
                .visible(false)
                .initialization_script(include_str!("../init.js"))
                .build()?;
            // Do not steal focus from the game when the app starts mid-match (Electron: showInactive).
            if !background() && !smoke() {
                if let Some(w) = app.get_webview_window("main") {
                    show_inactive(&w);
                }
            }

            game::spawn_static_data(&rt, &st);
            game::spawn_backend_detection(&rt, &st);
            // Live API events -> intel -> coach.
            let (st3, rt3) = (st.clone(), rt.clone());
            rt.spawn(async move {
                while let Some(ev) = live_rx.recv().await {
                    match ev {
                        live::LiveEvent::State(s) if s != rc_core::types::SourceState::Connected => game::on_live_lost(&st3),
                        live::LiveEvent::State(_) => {}
                        live::LiveEvent::Data(d, _) => {
                            // Enemy profiles need a Riot API key (Riot API client not ported yet).
                            if st3.settings.lock().unwrap().value().riot.api_key.is_empty() {
                                st3.ui.lock().unwrap().profiles = rc_core::types::ProfilesStatus { state: "no-key".into(), count: 0, error: None };
                            }
                            game::on_game_data(&rt3, &st3, &d);
                        }
                    }
                }
            });
            let (st4, rt4) = (st.clone(), rt.clone());
            let runes_state = Arc::new(Mutex::new(runes::RunesState::default()));
            rt.spawn(async move {
                while let Some(ev) = lcu_rx.recv().await {
                    match ev {
                        lcu::LcuEvent::State(_) => {}
                        lcu::LcuEvent::Phase(p) => runes::on_phase(&runes_state, &p),
                        lcu::LcuEvent::ChampSelect(sess) => runes::on_champ_select(&rt4, &st4, &runes_state, &sess),
                    }
                }
            });
            // Status to the UI every 250 ms (only when it changed).
            let (app2, st2) = (app.handle().clone(), st.clone());
            rt.spawn(async move {
                let mut last = String::new();
                loop {
                    emit_status(&app2, &st2, &mut last);
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            });
            if smoke() {
                let (app3, st5) = (app.handle().clone(), st.clone());
                rt.spawn(async move {
                    tokio::time::sleep(Duration::from_secs(12)).await;
                    let mut s = serde_json::to_value(st5.status()).unwrap_or_default();
                    // Never print the Riot API key.
                    if let Some(k) = s.pointer_mut("/settings/riot/apiKey") {
                        if k.as_str().is_some_and(|v| !v.is_empty()) {
                            *k = Value::String("<redacted>".into());
                        }
                    }
                    let intel = st5.game.lock().unwrap().intel.as_ref().map(|i| serde_json::json!({ "me": i.me.as_ref().map(|m| &m.champion), "gameTime": i.game_time, "goldDiff": i.gold_diff, "objectives": i.objectives.len(), "recent": i.recent.first().map(|r| &r.text) }));
                    println!("SMOKE {s}");
                    println!("SMOKE-INTEL {}", intel.unwrap_or(Value::Null));
                    app3.exit(0);
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing keeps the coach running in the background when it starts with Windows.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let st = window.state::<Arc<AppState>>();
                let keep = st.settings.lock().unwrap().value().launch_at_login && !st.ui.lock().unwrap().quitting;
                if keep {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("tauri run");
}
