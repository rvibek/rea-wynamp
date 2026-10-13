//! Wynamp desktop application (Tauri 2).
//!
//! Wires the playback engine to the frontend:
//! - `player_command` forwards any [`PlayerCommand`] to the engine thread
//! - engine [`PlayerEvent`]s are re-emitted as the `player:event` Tauri event
//! - `read_track_meta` extracts tags/properties via lofty for the display

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::mpsc::Sender;
use std::thread;

use rea_wynamp_core::engine::{self, EngineConfig};
use rea_wynamp_core::models::TrackMeta;
use rea_wynamp_core::playback::PlayerCommand;
use serde::Serialize;
use tauri::Emitter;

/// Tracks metadata for one file, as read from its tags.
#[derive(Debug, Clone, Serialize)]
struct TrackInfo {
    path: String,
    /// Container/codec name, e.g. "Mp3", "Flac", "Aiff".
    codec: String,
    meta: TrackMeta,
}

/// The engine's command channel, managed as Tauri state.
struct EngineState(Sender<PlayerCommand>);

/// Forward a player command to the engine thread.
#[tauri::command]
fn player_command(
    state: tauri::State<'_, EngineState>,
    command: PlayerCommand,
) -> Result<(), String> {
    state
        .0
        .send(command)
        .map_err(|_| "playback engine has stopped".to_string())
}

/// Read tags and properties for a set of files (best effort: files that
/// cannot be read are simply absent from the result).
#[tauri::command]
fn read_track_meta(paths: Vec<String>) -> Vec<TrackInfo> {
    paths.iter().filter_map(|p| read_track_info(p)).collect()
}

fn read_track_info(path: &str) -> Option<TrackInfo> {
    use lofty::prelude::*;
    use lofty::probe::Probe;

    let tagged = Probe::open(path).ok()?.read().ok()?;
    let props = tagged.properties();

    let mut meta = TrackMeta {
        duration: (props.duration() > std::time::Duration::ZERO).then_some(props.duration()),
        bitrate_kbps: props.audio_bitrate().or(props.overall_bitrate()),
        sample_rate_hz: props.sample_rate(),
        channels: props.channels().map(u16::from),
        codec: Some(format!("{:?}", tagged.file_type())),
        ..TrackMeta::default()
    };

    if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
        meta.title = tag.title().map(|s| s.to_string());
        meta.artist = tag.artist().map(|s| s.to_string());
        meta.album = tag.album().map(|s| s.to_string());
        meta.album_artist = tag.get_string(ItemKey::AlbumArtist).map(str::to_owned);
        meta.genre = tag.genre().map(|s| s.to_string());
        meta.track_no = tag.track();
        meta.disc_no = tag.disk();
        meta.year = tag.date().map(|ts| u32::from(ts.year));
    }

    Some(TrackInfo {
        path: path.to_string(),
        codec: format!("{:?}", tagged.file_type()),
        meta,
    })
}

/// Application version shown in the UI footer/About.
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
    let engine = engine::spawn(EngineConfig::default()).expect("failed to start playback engine");
    let (cmd_tx, evt_rx) = engine.into_parts();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(EngineState(cmd_tx))
        .setup(move |app| {
            // Bridge engine events to the frontend as `player:event`.
            let app_handle = app.handle().clone();
            thread::Builder::new()
                .name("wynamp-events".into())
                .spawn(move || {
                    while let Ok(event) = evt_rx.recv() {
                        if app_handle.emit("player:event", event).is_err() {
                            break; // window/app gone
                        }
                    }
                })
                .expect("spawn event forwarder");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            player_command,
            read_track_meta,
            app_version,
            app_paths
        ])
        .run(tauri::generate_context!())
        .expect("error while running Wynamp");
}
