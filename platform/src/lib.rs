//! OS abstraction layer.
//!
//! Everything that differs between macOS, Windows, and Linux lives behind the
//! traits in this crate. Feature crates and the Tauri app depend only on the
//! traits; concrete implementations are chosen in [`Platform::detect`] with
//! `cfg(target_os = ...)` so porting stays a local change.
//!
//! Phase map:
//! - [`Paths`] — implemented for all platforms from day one ([`SystemPaths`])
//! - [`Credentials`] — stub now; keyring-backed implementation arrives with
//!   Navidrome (v0.5.0)
//! - [`MediaControls`] — stub now; souvlaki-backed implementation arrives in
//!   v1.0.0 (media keys / SMTC / MPRIS)
//!
//! See `docs/ARCHITECTURE.md`.

mod credentials;
mod media;
mod paths;

pub use credentials::{Credentials, StubCredentials};
pub use media::{MediaControls, NowPlaying, StubMediaControls};
pub use paths::{Paths, SystemPaths};

/// The set of platform services available on this OS.
///
/// Created once at startup and passed (or re-discovered) wherever OS services
/// are needed. Feature code must use the boxed traits, never concrete types,
/// so tests can substitute stubs.
pub struct Platform {
    pub paths: Box<dyn Paths>,
    pub credentials: Box<dyn Credentials>,
    pub media_controls: Box<dyn MediaControls>,
}

impl Platform {
    /// Detect the host OS and build its concrete service implementations.
    ///
    /// This is the single place where per-OS selection happens; as stubs are
    /// replaced (keyring, souvlaki) their `cfg(target_os = ...)` branches go
    /// here.
    pub fn detect() -> Self {
        Self {
            paths: Box::new(SystemPaths::detect()),
            // v0.5.0: swap for the keyring-backed implementation.
            credentials: Box::new(StubCredentials),
            // v1.0.0: swap for souvlaki-backed per-OS media controls.
            media_controls: Box::new(StubMediaControls),
        }
    }
}

impl Default for Platform {
    fn default() -> Self {
        Self::detect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rea_wynamp_core::PlaybackState;

    #[test]
    fn detect_provides_working_services() {
        let platform = Platform::detect();

        // Paths are absolute; cache is always separate (config may share the
        // data dir on macOS/Windows — see paths module docs).
        let data = platform.paths.data_dir();
        let config = platform.paths.config_dir();
        let cache = platform.paths.cache_dir();
        assert!(data.is_absolute());
        assert!(config.is_absolute());
        assert!(cache.is_absolute());
        assert_ne!(data, cache);
        assert_ne!(config, cache);

        // Stub credentials: reads succeed (nothing stored), writes are explicit.
        assert_eq!(
            platform.credentials.get("navidrome", "srv-1").unwrap(),
            None
        );
        assert!(platform
            .credentials
            .set("navidrome", "srv-1", "secret")
            .is_err());

        // Stub media controls are safe no-ops.
        let mut media = platform.media_controls;
        assert!(media.set_now_playing(&NowPlaying::default()).is_ok());
        assert!(media.set_playback_state(PlaybackState::Playing).is_ok());
        assert!(media.set_position(std::time::Duration::ZERO).is_ok());
        assert!(media.clear().is_ok());
    }
}
