#[path = "build_rc.rs"]
mod build_rc;

use std::path::PathBuf;
use std::process::Command;

fn main() {
    // Second life of this binary: acting as `windres.exe` for embed-resource (see build_rc.rs).
    let exe = std::env::current_exe().unwrap_or_default();
    if exe.file_stem().and_then(|s| s.to_str()) == Some("windres") {
        std::process::exit(build_rc::windres_main());
    }

    let gnu = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu")
        && std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let has_windres = Command::new("windres").arg("--version").output().is_ok();
    if gnu && !has_windres {
        let dir = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("rc-windres");
        std::fs::create_dir_all(&dir).expect("create rc-windres dir");
        std::fs::copy(&exe, dir.join("windres.exe")).expect("copy build script as windres.exe");
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut paths = vec![dir];
        paths.extend(std::env::split_paths(&path));
        std::env::set_var("PATH", std::env::join_paths(paths).unwrap());
    }
    println!("cargo:rerun-if-changed=build_rc.rs");
    println!("cargo:rerun-if-changed=init.js");
    tauri_build::build();
}
