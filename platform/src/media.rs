//! OS "Now Playing" and media-key integration.
//!
//! macOS: Now Playing widget / media keys · Windows: SMTC · Linux: MPRIS.
//! Concrete implementations arrive in v1.0.0 via the `souvlaki` crate,
//! selected in [`crate::Platform::detect`].

use std::path::PathBuf;
use std::time::Duration;

use rea_wynamp_core::{PlaybackState, Result};

/// What the OS media overlay / media keys should display.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: Option<Duration>,
    /// Local path to cover art, when available.
    pub artwork_path: Option<PathBuf>,
}

/// Media controls and OS now-playing integration.
pub trait MediaControls: Send {
    /// Publish/replace the current track's metadata.
    fn set_now_playing(&mut self, info: &NowPlaying) -> Result<()>;

    /// Reflect play/pause/stop in the OS.
    fn set_playback_state(&mut self, state: PlaybackState) -> Result<()>;

    /// Keep the OS scrubber in sync with the engine position.
    fn set_position(&mut self, position: Duration) -> Result<()>;

    /// Remove the entry from the OS (on Stop).
    fn clear(&mut self) -> Result<()>;
}

/// No-op implementation until souvlaki lands in v1.0.0.
#[derive(Debug, Default, Clone, Copy)]
pub struct StubMediaControls;

impl MediaControls for StubMediaControls {
    fn set_now_playing(&mut self, _info: &NowPlaying) -> Result<()> {
        Ok(())
    }

    fn set_playback_state(&mut self, _state: PlaybackState) -> Result<()> {
        Ok(())
    }

    fn set_position(&mut self, _position: Duration) -> Result<()> {
        Ok(())
    }

    fn clear(&mut self) -> Result<()> {
        Ok(())
    }
}
