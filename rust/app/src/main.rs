#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::{WebviewUrl, WebviewWindowBuilder};

#[tauri::command]
fn rc_call(method: String, args: Vec<serde_json::Value>) -> Result<serde_json::Value, String> {
    let _ = args;
    Err(format!("{method}: jeszcze niedostępne w wersji Rust."))
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![rc_call])
        .setup(|app| {
            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("control.html".into()))
                .title("Rift Coach")
                .inner_size(1180.0, 780.0)
                .min_inner_size(900.0, 600.0)
                .decorations(false)
                .focused(false)
                .initialization_script(include_str!("../init.js"))
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("tauri run");
}
