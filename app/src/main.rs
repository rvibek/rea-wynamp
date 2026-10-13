//! Wynamp desktop application (Tauri 2).
//!
//! Phase 1a: window + frontend bridge. The playback engine wiring, library,
//! and playlist commands land in the following Phase 1 steps (see ROADMAP.md).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::Serialize;

/// Application version, shown in the UI footer/About.
#[tauri::command]
fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Serialization-friendly snapshot of the platform directories.
#[derive(Debug, Serialize)]
struct PathsSnapshot {
    data_dir: String,
    config_dir: String,
    cache_dir: String,
}

/// Resolved OS directories (settings/debug view).
#[tauri::command]
fn app_paths() -> PathsSnapshot {
    let platform = rea_wynamp_platform::Platform::detect();
    PathsSnapshot {
        data_dir: platform.paths.data_dir().display().to_string(),
        config_dir: platform.paths.config_dir().display().to_string(),
        cache_dir: platform.paths.cache_dir().display().to_string(),
    }
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![app_version, app_paths])
        .run(tauri::generate_context!())
        .expect("error while running Wynamp");
}
