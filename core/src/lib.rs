//! Wynamp core — UI-independent models, playback protocol, and shared errors.
//!
//! This crate must never depend on Tauri or any UI framework. The playback
//! engine, playlist logic, library scanning, and DSP live here so they can be
//! developed and tested headlessly. See `docs/ARCHITECTURE.md`.

pub mod engine;
pub mod error;
pub mod library;
pub mod models;
pub mod playback;
pub mod playlist_io;
pub mod scan;

pub use error::{Error, Result};
pub use models::{
    normalize_volume, PlayableItem, PlaybackState, PlayerStatus, Playlist, PlaylistEntry,
    RepeatMode, TrackMeta,
};
pub use playback::{PlayerCommand, PlayerEvent};
