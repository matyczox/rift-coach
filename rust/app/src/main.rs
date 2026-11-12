#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bridge;
mod game;
mod game_icons;
mod harvest;
mod hotkeys;
mod hud;
mod hud_state;
mod knowledge;
mod postgame;
mod recorder;
mod playback;
mod runes;
mod self_profile;
mod state;
mod vision;

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

/// Testing: treat an open replay like a live game (vision, tracker and HUD run on it).
pub fn allow_replay() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::args().any(|a| a == "--allow-replay"))
}

/// Testing: HUD shows the minimap outline and stays up even when the game is not the active window.
pub fn hud_test() -> bool {
    std::env::args().any(|a| a == "--hud-test")
}

/// Value of `--name=value`.
pub fn arg_value(name: &str) -> Option<String> {
    let p = format!("--{name}=");
    std::env::args().find_map(|a| a.strip_prefix(&p).map(String::from))
}

/// Off the main thread: some calls encode images or wait on locks.
#[tauri::command]
async fn rc_call(app: tauri::AppHandle, state: tauri::State<'_, Arc<AppState>>, method: String, args: Vec<Value>) -> Result<Value, String> {
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || bridge::call(&app, &st, &method, &args)).await.map_err(|e| e.to_string())?
}

/// External links (e.g. developer.riotgames.com) open in the default browser; https only.
#[tauri::command]
fn open_external(url: String) -> Result<(), String> {
    if !url.starts_with("https://") || url.chars().any(|c| c.is_whitespace() || c == '"') {
        return Err("Tylko adresy https://".into());
    }
    std::process::Command::new("rundll32.exe").args(["url.dll,FileProtocolHandler", &url]).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Global hotkeys: not in --smoke (it runs next to a normal instance) unless --smoke-hotkeys.
fn hotkeys_on() -> bool {
    !smoke() || std::env::args().any(|a| a == "--smoke-hotkeys")
}

pub fn on_settings_changed(app: &tauri::AppHandle, st: &Arc<AppState>) {
    // New / fixed Riot key: fetch the enemy profiles again (also mid-game).
    let key = st.settings.lock().unwrap().value().riot.api_key.clone();
    {
        let mut g = st.game.lock().unwrap();
        if g.profiles_riot_key != key {
            g.profiles_riot_key = key;
            g.profiles_key.clear();
        }
    }
    knowledge::apply_models(st);
    vision::sync_settings(st);
    if hotkeys_on() {
        hotkeys::register(app, st);
    }
    hotkeys::apply_login_item(st.settings.lock().unwrap().value().launch_at_login);
    hud::place(app, st);
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

/// `--smoke`: what the knowledge / post-game side has (counts and labels only; no Riot API
/// payloads, no player names).
fn smoke_knowledge(st: &AppState) -> Value {
    let learning = bridge::learning(st);
    let g = st.game.lock().unwrap();
    let hud: Value = serde_json::from_str(&st.hud.lock().unwrap().last_payload_text()).unwrap_or(Value::Null);
    serde_json::json!({
        "profiles": st.ui.lock().unwrap().profiles,
        "profilesLoaded": g.profiles.len(),
        "profilesWithGames": g.profiles.iter().filter(|p| p.games_analyzed > 0).count(),
        "patchBrief": g.patch_brief.as_ref().map(|b| &b.patch),
        "matchup": g.matchup.as_ref().map(|m| format!("{} vs {} ({}) curve {}", m.me, m.enemy, m.role, m.curve.len())),
        "matchupKey": g.matchup_key,
        "build": g.build.as_ref().map(|b| b.items.iter().map(|i| i.name.clone()).collect::<Vec<_>>()),
        "hudPower": hud.pointer("/state/power").map(|p| !p.is_null()),
        "hudDanger": hud.pointer("/state/danger/label"),
        "hudObjective": hud.pointer("/state/objective/verdict"),
        "hudLoadingCard": hud.pointer("/state/loadingCard").map(|p| !p.is_null()),
        "fed": hud.pointer("/state/fed"),
        "matchLog": { "key": !g.match_log.game_key.is_empty(), "moments": g.match_log.moments.len(), "goldCurve": g.match_log.gold_curve.len() },
        "obsFrames": g.obs.log.as_ref().map(|l| l.frames.len()),
        "recording": st.recorder.lock().unwrap().status,
        "harvest": harvest::status(st),
        "learning": learning,
        "playerProfile": self_profile::smoke_summary(st),
    })
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
    let mut builder = tauri::Builder::default();
    // --smoke runs next to a normal instance (and must never bring that one to the front).
    if !smoke() {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }));
    }
    builder
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .invoke_handler(tauri::generate_handler![rc_call, open_external])
        .setup(move |app| {
            let rt = tauri::async_runtime::block_on(async { tokio::runtime::Handle::current() });
            let (live_shared, mut live_rx) = live::spawn(&rt, live::POLL_MS);
            let (lcu_client, lcu_shared, mut lcu_rx) = lcu::spawn(&rt);
            let replay_api = replay::spawn_state_poller(&rt);
            let settings = SettingsStore::open(&data_dir);
            let vision_hub = vision::VisionHub::new(settings.value().minimap_fps.max(1) as f64, vision::calibration_from_settings(settings.value().calibration.as_ref()));
            let st = Arc::new(AppState {
                version: app.package_info().version.to_string(),
                settings: Mutex::new(settings),
                data_dir: data_dir.clone(),
                live: live_shared,
                lcu: lcu_shared,
                lcu_client,
                replay_api,
                ui: Mutex::new(UiState::default()),
                game: Mutex::new(game::Game::new(&data_dir)),
                vision: vision_hub,
                hud: Mutex::new(hud::HudCtl::default()),
                app: app.handle().clone(),
                rt: rt.clone(),
                recorder: Mutex::new(recorder::Recorder::new(&state::recordings_root(&data_dir))),
                // --harvest-root=<dir> (testing): harvest somewhere other than the real recordings.
                harvester: rc_core::harvest::Harvester::new(&arg_value("harvest-root").map(std::path::PathBuf::from).unwrap_or_else(|| state::recordings_root(&data_dir))),
                harvest: Mutex::new(harvest::HarvestCtl::default()),
                replay: Mutex::new(playback::ReplayCtl::default()),
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

            // In-game HUD (hidden until a panel has content) + capture -> vision -> tracker.
            hud::create(app.handle())?;
            on_settings_changed(app.handle(), &st);
            // Testing: keep the minimap outline on so the HUD window has something to show.
            if hud_test() {
                *st.vision.frame_until.lock().unwrap() = Some(std::time::Instant::now() + Duration::from_secs(3600));
            }
            vision::spawn_threads(app.handle(), &st);
            hud::spawn(app.handle(), &st);
            // Player profile for the coach (saved file at once; Riot API rebuild every 6 h).
            self_profile::spawn(&st);
            let st6 = st.clone();
            rt.spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    st6.game.lock().unwrap().field.save_learner();
                }
            });

            game::spawn_static_data(&rt, &st);
            game::spawn_backend_detection(&rt, &st);
            // Live API events -> intel -> coach.
            let (st3, rt3) = (st.clone(), rt.clone());
            rt.spawn(async move {
                while let Some(ev) = live_rx.recv().await {
                    match ev {
                        live::LiveEvent::State(s) if s != rc_core::types::SourceState::Connected => {
                            st3.ui.lock().unwrap().live_lost_at.get_or_insert_with(std::time::Instant::now);
                            game::on_live_lost(&st3);
                        }
                        live::LiveEvent::State(_) => {}
                        live::LiveEvent::Data(d, _) => {
                            st3.ui.lock().unwrap().live_lost_at = None;
                            let at = vision::now_ms();
                            st3.recorder.lock().unwrap().api_sample(&d, at);
                            // An open replay is not a live game (index.ts: replayApiState !== 'connected').
                            if !st3.replay_blocks() {
                                game::on_game_data(&rt3, &st3, &d);
                            }
                            let auto = st3.settings.lock().unwrap().value().auto_record;
                            if auto && !smoke() && !st3.replay_open() && !st3.recorder.lock().unwrap().active() && recorder::SUPPORTED_MODES.contains(&d.game_data.game_mode.as_str()) {
                                recorder::start_recording(&st3);
                            }
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
                        lcu::LcuEvent::Phase(p) => {
                            runes::on_phase(&rt4, &st4, &runes_state, &p);
                            st4.recorder.lock().unwrap().lcu_phase(&p);
                            if p == "PreEndOfGame" || p == "EndOfGame" {
                                let st = st4.clone();
                                rt4.spawn(async move {
                                    tokio::time::sleep(Duration::from_secs(3)).await;
                                    postgame::finish_game(st).await;
                                });
                            }
                        }
                        lcu::LcuEvent::ChampSelect(sess) => runes::on_champ_select(&rt4, &st4, &runes_state, &sess),
                    }
                }
            });
            // Status to the UI every 250 ms (only when it changed).
            let (app2, st2) = (app.handle().clone(), st.clone());
            let rt5 = rt.clone();
            rt.spawn(async move {
                let mut last = String::new();
                loop {
                    emit_status(&app2, &st2, &mut last);
                    let lost = st2.ui.lock().unwrap().live_lost_at.map(|t| t.elapsed());
                    // Stop an auto recording ~15 s after the game API disappears (game over).
                    let auto = st2.settings.lock().unwrap().value().auto_record;
                    if auto && lost.is_some_and(|l| l > Duration::from_secs(15)) && st2.recorder.lock().unwrap().active() {
                        recorder::stop_recording(&st2);
                    }
                    if lost.is_some_and(|l| l > Duration::from_secs(20)) {
                        let pending = {
                            let g = st2.game.lock().unwrap();
                            !g.match_log.game_key.is_empty() && g.match_log.game_key != g.review_done_for
                        };
                        if pending {
                            rt5.spawn(postgame::finish_game(st2.clone()));
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            });
            // Disk housekeeping at startup: newest 5 recordings, none older than 14 days.
            let st7 = st.clone();
            rt.spawn(async move {
                tokio::time::sleep(Duration::from_secs(30)).await;
                if !smoke() {
                    let _ = tokio::task::spawn_blocking(move || recorder::prune(&st7)).await;
                }
            });
            if smoke() {
                let (app3, st5) = (app.handle().clone(), st.clone());
                rt.spawn(async move {
                    let secs = arg_value("smoke-seconds").and_then(|s| s.parse().ok()).unwrap_or(12u64);
                    // --smoke-record: record for the whole smoke run (tests the recorder).
                    let record = std::env::args().any(|a| a == "--smoke-record");
                    if record {
                        tokio::time::sleep(Duration::from_secs(3)).await;
                        recorder::start_recording(&st5);
                    }
                    // --smoke-harvest: harvest the open replay (no fog-team waits) for the smoke run.
                    if std::env::args().any(|a| a == "--smoke-harvest") {
                        harvest::smoke_start(&st5);
                    }
                    tokio::time::sleep(Duration::from_secs(secs)).await;
                    if record {
                        recorder::stop_recording(&st5);
                    }
                    harvest::stop(&st5);
                    println!("SMOKE-KNOWLEDGE {}", smoke_knowledge(&st5));
                    // Minimap crop of the newest frame (like --smoke-frame in the Electron app).
                    if let Some(out) = arg_value("smoke-frame") {
                        if let (Some((_, f)), Some(c)) = (st5.vision.capture.latest(), st5.vision.calibration()) {
                            if let Some(crop) = rc_vision::capture::crop_rect(&f.image, c.rect_for(f.image.width as i32, f.image.height as i32)) {
                                let _ = std::fs::write(out, rc_vision::image::encode_png(&crop));
                            }
                        }
                    }
                    println!("SMOKE-VISION {}", vision::smoke_summary(&st5));
                    println!("SMOKE-HUD {}", hud::smoke_summary(&st5));
                    println!("SMOKE-HOTKEYS {}", hotkeys::smoke_summary(&app3, &st5));
                    let mut s = serde_json::to_value(st5.status()).unwrap_or_default();
                    // Never print the Riot API key.
                    if let Some(k) = s.pointer_mut("/settings/riot/apiKey") {
                        if k.as_str().is_some_and(|v| !v.is_empty()) {
                            *k = Value::String("<redacted>".into());
                        }
                    }
                    // Player profile: counts only (SMOKE-KNOWLEDGE), not champions / stats / lessons.
                    if let Some(p) = s.get_mut("playerProfile").filter(|p| !p.is_null()) {
                        *p = Value::String("<present>".into());
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
