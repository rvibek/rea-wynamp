//! Wynamp desktop application (Tauri 2).
//!
//! Wires the playback engine, the SQLite library, and the playlist file
//! formats to the frontend:
//! - `player_command` — any [`PlayerCommand`] to the engine thread
//! - engine events re-emitted as the `player:event` Tauri event
//! - `library_*` — folder scan (background + progress events), session
//!   persist/restore, missing-file reconcile
//! - `playlist_import` / `playlist_export` — M3U/M3U8/PLS files
//! - `read_track_meta` — lofty tags/properties for the display

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;

use rea_wynamp_core::engine::{self, EngineConfig};
use rea_wynamp_core::library::{Library, TrackRecord};
use rea_wynamp_core::models::{Playlist, PlaylistEntry, TrackMeta};
use rea_wynamp_core::playback::PlayerCommand;
use rea_wynamp_core::playlist_io::{self, PlaylistFormat};
use rea_wynamp_core::scan;
use serde::Serialize;
use tauri::{Emitter, Manager};

// ── Shared state ────────────────────────────────────────────────────────────

/// The engine's command channel + visualizer tap, managed as Tauri state.
struct EngineState {
    tx: Sender<PlayerCommand>,
    visual: Arc<rea_wynamp_core::engine::visual::VisualTap>,
}

/// The SQLite library (Mutex: rusqlite Connection is single-threaded).
struct LibraryState(Mutex<Library>);

fn library_guard<'a>(
    state: &'a tauri::State<'_, LibraryState>,
) -> Result<std::sync::MutexGuard<'a, Library>, String> {
    state
        .0
        .lock()
        .map_err(|_| "library database is unavailable".to_string())
}

// ── Player ──────────────────────────────────────────────────────────────────

/// Forward a player command to the engine thread.
#[tauri::command]
fn player_command(
    state: tauri::State<'_, EngineState>,
    command: PlayerCommand,
) -> Result<(), String> {
    state
        .tx
        .send(command)
        .map_err(|_| "playback engine has stopped".to_string())
}

#[derive(Debug, Serialize)]
struct VisualizerSnapshot {
    /// Mono waveform points for the oscilloscope.
    wave: Vec<f32>,
    /// Normalized spectrum bars (0..=1).
    spectrum: Vec<f32>,
}

/// Latest visualizer data: mono wave slice + spectrum bars.
#[tauri::command]
fn visualizer_snapshot(
    state: tauri::State<'_, EngineState>,
    bands: usize,
    wave_points: usize,
) -> Result<VisualizerSnapshot, String> {
    let raw = state.visual.snapshot_mono(2048);
    let spectrum = rea_wynamp_core::engine::visual::spectrum(&raw, bands.clamp(8, 64));
    let wave = rea_wynamp_core::engine::visual::downsample(&raw, wave_points.clamp(16, 512));
    Ok(VisualizerSnapshot { wave, spectrum })
}

/// Tracks metadata for one file, as read from its tags.
#[derive(Debug, Clone, Serialize)]
struct TrackInfo {
    path: String,
    codec: String,
    meta: TrackMeta,
}

/// Read tags and properties for a set of files (best effort).
#[tauri::command]
fn read_track_meta(paths: Vec<String>) -> Vec<TrackInfo> {
    paths.iter().filter_map(|p| read_track_info(p)).collect()
}

fn read_track_info(path: &str) -> Option<TrackInfo> {
    use lofty::prelude::*;
    use lofty::probe::Probe;

    let tagged = Probe::open(path).ok()?.read().ok()?;
    let props = tagged.properties();

    let meta = TrackMeta {
        duration: (props.duration() > std::time::Duration::ZERO).then_some(props.duration()),
        bitrate_kbps: props.audio_bitrate().or(props.overall_bitrate()),
        sample_rate_hz: props.sample_rate(),
        channels: props.channels().map(u16::from),
        codec: Some(format!("{:?}", tagged.file_type())),
        title: None,
        artist: None,
        album: None,
        album_artist: None,
        genre: None,
        track_no: None,
        disc_no: None,
        year: None,
    };
    let mut meta = meta;

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

// ── Library ─────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct LibraryStatus {
    track_count: i64,
    /// Paths that disappeared since they were last seen.
    missing: Vec<String>,
}

/// Reconcile the library against the filesystem and report status.
#[tauri::command]
fn library_status(state: tauri::State<'_, LibraryState>) -> Result<LibraryStatus, String> {
    let lib = library_guard(&state)?;
    let missing = lib.reconcile_missing().map_err(|e| e.to_string())?;
    let track_count = lib.track_count().map_err(|e| e.to_string())?;
    Ok(LibraryStatus {
        track_count,
        missing,
    })
}

#[derive(Debug, Clone, Serialize)]
struct ScanProgress {
    found: usize,
    path: String,
}

#[derive(Debug, Clone, Serialize)]
struct ScanDone {
    total: usize,
    paths: Vec<String>,
}

/// Start a background scan of `path`. Progress arrives as `scan:event`,
/// completion as `scan:done`. Returns immediately.
#[tauri::command]
fn library_scan_start(app: tauri::AppHandle, path: String) -> Result<(), String> {
    if !Path::new(&path).is_dir() {
        return Err(format!("not a folder: {path}"));
    }

    thread::Builder::new()
        .name("wynamp-scan".into())
        .spawn(move || {
            let found = scan::scan_folder(Path::new(&path), |count, p| {
                if count == 1 || count % 10 == 0 {
                    let _ = app.emit(
                        "scan:event",
                        ScanProgress {
                            found: count,
                            path: p.display().to_string(),
                        },
                    );
                }
            });

            match found {
                Ok(paths) => {
                    // Upsert in small batches so the UI thread stays responsive.
                    for chunk in paths.chunks(50) {
                        let infos: Vec<TrackInfo> = chunk
                            .iter()
                            .filter_map(|p| read_track_info(&p.display().to_string()))
                            .collect();
                        let state = app.state::<LibraryState>();
                        let Ok(lib) = state.0.lock() else {
                            let _ = app.emit(
                                "scan:error",
                                "library database unavailable during scan".to_string(),
                            );
                            return;
                        };
                        for info in &infos {
                            let record = track_record_from_info(info);
                            if let Err(e) = lib.upsert_track(&record) {
                                let _ = app.emit("scan:error", e.to_string());
                                return;
                            }
                        }
                    }

                    let total = paths.len();
                    let path_strings: Vec<String> =
                        paths.iter().map(|p| p.display().to_string()).collect();
                    let _ = app.emit(
                        "scan:done",
                        ScanDone {
                            total,
                            paths: path_strings,
                        },
                    );
                }
                Err(e) => {
                    let _ = app.emit("scan:error", e.to_string());
                }
            }
        })
        .map_err(|e| format!("failed to start scan: {e}"))?;
    Ok(())
}

fn track_record_from_info(info: &TrackInfo) -> TrackRecord {
    TrackRecord {
        path: info.path.clone(),
        title: info.meta.title.clone(),
        artist: info.meta.artist.clone(),
        album: info.meta.album.clone(),
        duration_ms: info.meta.duration.map(|d| d.as_millis() as i64),
        bitrate_kbps: info.meta.bitrate_kbps.map(i64::from),
        sample_rate_hz: info.meta.sample_rate_hz.map(i64::from),
        codec: Some(info.codec.clone()),
        mtime: None,
        size_bytes: None,
        missing: false,
    }
}

/// Persist the current queue for restore on next launch.
#[tauri::command]
fn library_save_session(
    state: tauri::State<'_, LibraryState>,
    paths: Vec<String>,
) -> Result<(), String> {
    let lib = library_guard(&state)?;
    lib.save_session(&paths).map_err(|e| e.to_string())
}

/// Paths of the queue as it was when the app last quit.
#[tauri::command]
fn library_session_load(state: tauri::State<'_, LibraryState>) -> Result<Vec<String>, String> {
    let lib = library_guard(&state)?;
    lib.load_session().map_err(|e| e.to_string())
}

// ── Playlist files ──────────────────────────────────────────────────────────

/// Write the given entries to a playlist file (`.m3u`, `.m3u8`, `.pls`).
#[tauri::command]
fn playlist_export(dest_path: String, entries: Vec<PlaylistEntry>) -> Result<(), String> {
    let dest = PathBuf::from(&dest_path);
    let format = PlaylistFormat::from_path(&dest)
        .ok_or_else(|| "unsupported playlist extension (use .m3u, .m3u8, or .pls)".to_string())?;
    let name = dest
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "playlist".to_string());
    let playlist = Playlist {
        id: None,
        name,
        entries,
    };
    let base = dest.parent().unwrap_or(Path::new("."));
    let text = playlist_io::export(&playlist, format, base);
    std::fs::write(&dest, text).map_err(|e| format!("cannot write {}: {e}", dest.display()))
}

#[derive(Debug, Serialize)]
struct ImportedPlaylist {
    name: String,
    entries: Vec<PlaylistEntry>,
}

/// Parse a playlist file (`.m3u`, `.m3u8`, `.pls`) into entries.
#[tauri::command]
fn playlist_import(src_path: String) -> Result<ImportedPlaylist, String> {
    let src = PathBuf::from(&src_path);
    let format = PlaylistFormat::from_path(&src)
        .ok_or_else(|| "unsupported playlist extension (use .m3u, .m3u8, or .pls)".to_string())?;
    let bytes = std::fs::read(&src).map_err(|e| format!("cannot read {}: {e}", src.display()))?;
    // .m3u is often Latin-1 in the wild; lossy UTF-8 keeps every path usable.
    let text = String::from_utf8_lossy(&bytes);
    let base = src.parent().unwrap_or(Path::new("."));
    let name = src
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "playlist".to_string());
    let playlist = playlist_io::parse(&text, format, name, base).map_err(|e| e.to_string())?;
    Ok(ImportedPlaylist {
        name: playlist.name,
        entries: playlist.entries,
    })
}

// ── Misc ────────────────────────────────────────────────────────────────────

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
    let visual = Arc::clone(&engine.visual);
    let (cmd_tx, evt_rx) = engine.into_parts();

    let data_dir = rea_wynamp_platform::Platform::detect().paths.data_dir();
    let library = Library::open(&data_dir.join("library.db")).expect("failed to open library.db");

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(EngineState { tx: cmd_tx, visual })
        .manage(LibraryState(Mutex::new(library)))
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
            visualizer_snapshot,
            read_track_meta,
            library_status,
            library_scan_start,
            library_save_session,
            library_session_load,
            playlist_export,
            playlist_import,
            app_version,
            app_paths
        ])
        .run(tauri::generate_context!())
        .expect("error while running Wynamp");
}
