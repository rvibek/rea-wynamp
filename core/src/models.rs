//! Domain models shared by the engine, the UI, and the feature crates.

use std::path::PathBuf;
use std::time::Duration;

/// A single playable thing, whatever its origin.
///
/// The engine treats all variants uniformly: it resolves them to a stream of
/// decoded PCM samples. `LocalFile` is handled from v0.1.0; the network
/// variants arrive with their phases (v0.4.0 / v0.5.0).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PlayableItem {
    /// A file on the local filesystem.
    LocalFile { path: PathBuf },
    /// A track on a Navidrome (Subsonic) server.
    NavidromeTrack {
        /// Stable id of the server as configured in settings.
        server_id: String,
        /// The Subsonic `id` of the track.
        track_id: String,
    },
    /// An internet radio stream.
    RadioStream {
        /// Direct stream URL (or a PLS/M3U playlist URL).
        url: String,
        /// Station name, when known.
        name: Option<String>,
    },
}

impl PlayableItem {
    /// Convenience constructor for local files.
    pub fn local(path: impl Into<PathBuf>) -> Self {
        PlayableItem::LocalFile { path: path.into() }
    }
}

/// Descriptive metadata for a track; empty fields are simply unknown.
///
/// For local files this is filled from tags via `lofty`; for network sources
/// it comes from API responses or stream headers.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrackMeta {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub track_no: Option<u32>,
    pub disc_no: Option<u32>,
    pub year: Option<u32>,
    pub duration: Option<Duration>,
    pub bitrate_kbps: Option<u32>,
    pub sample_rate_hz: Option<u32>,
    pub channels: Option<u16>,
    /// Codec/format name, e.g. "MP3", "FLAC".
    pub codec: Option<String>,
}

impl TrackMeta {
    /// True when no field carries information.
    pub fn is_empty(&self) -> bool {
        *self == TrackMeta::default()
    }
}

/// One entry in a playlist or the play queue.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaylistEntry {
    pub item: PlayableItem,
    pub meta: TrackMeta,
}

impl PlaylistEntry {
    pub fn new(item: PlayableItem) -> Self {
        Self {
            item,
            meta: TrackMeta::default(),
        }
    }

    pub fn with_meta(item: PlayableItem, meta: TrackMeta) -> Self {
        Self { item, meta }
    }

    /// Best-effort label for UI surfaces (marquee, playlist rows).
    ///
    /// Falls back from tag title → file stem → station name/URL → track id.
    pub fn display_title(&self) -> String {
        if let Some(title) = self
            .meta
            .title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            return title.to_string();
        }
        match &self.item {
            PlayableItem::LocalFile { path } => path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Unknown".to_string()),
            PlayableItem::NavidromeTrack { track_id, .. } => track_id.clone(),
            PlayableItem::RadioStream { url, name } => name
                .as_deref()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| url.clone()),
        }
    }

    /// Known duration, if any.
    pub fn duration(&self) -> Option<Duration> {
        self.meta.duration
    }
}

/// An ordered, named list of tracks (playlists and the play queue).
#[derive(Debug, Clone, PartialEq)]
pub struct Playlist {
    /// Database row id; `None` until persisted.
    pub id: Option<i64>,
    pub name: String,
    pub entries: Vec<PlaylistEntry>,
}

impl Playlist {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: None,
            name: name.into(),
            entries: Vec::new(),
        }
    }

    /// Sum of all known entry durations; unknown durations count as zero.
    pub fn total_duration(&self) -> Duration {
        self.entries.iter().filter_map(|e| e.duration()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn push(&mut self, entry: PlaylistEntry) {
        self.entries.push(entry);
    }

    /// Remove and return the entry at `index`, if present.
    pub fn remove(&mut self, index: usize) -> Option<PlaylistEntry> {
        (index < self.entries.len()).then(|| self.entries.remove(index))
    }
}

/// Engine lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlaybackState {
    #[default]
    Stopped,
    Playing,
    Paused,
    /// Waiting on a network/buffer or the output device.
    Buffering,
}

/// What happens when the current track ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RepeatMode {
    #[default]
    Off,
    /// Loop the whole queue.
    All,
    /// Loop the current track.
    One,
}

/// Snapshot of everything the UI needs to render the player.
///
/// The engine emits these as [`crate::PlayerEvent::Status`] (a few times per
/// second while playing); the UI treats them as the single source of truth.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerStatus {
    pub state: PlaybackState,
    pub current: Option<PlaylistEntry>,
    pub position: Duration,
    /// Total duration of the current track, when known (streams may not know).
    pub duration: Option<Duration>,
    /// Output level, `0.0` (silent) ..= `1.0` (full).
    pub volume: f32,
    /// `-1.0` (hard left) ..= `1.0` (hard right), `0.0` centered.
    pub balance: f32,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub bitrate_kbps: Option<u32>,
    pub sample_rate_hz: Option<u32>,
}

impl Default for PlayerStatus {
    fn default() -> Self {
        Self {
            state: PlaybackState::Stopped,
            current: None,
            position: Duration::ZERO,
            duration: None,
            volume: 1.0,
            balance: 0.0,
            shuffle: false,
            repeat: RepeatMode::Off,
            bitrate_kbps: None,
            sample_rate_hz: None,
        }
    }
}

impl PlayerStatus {
    /// Set the volume, clamped into `0.0..=1.0` (NaN becomes silence).
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = normalize_volume(volume);
    }

    /// Set the balance, clamped into `-1.0..=1.0` (NaN becomes center).
    pub fn set_balance(&mut self, balance: f32) {
        self.balance = if balance.is_nan() {
            0.0
        } else {
            balance.clamp(-1.0, 1.0)
        };
    }

    pub fn is_playing(&self) -> bool {
        self.state == PlaybackState::Playing
    }

    /// Seek-bar position in `0.0..=1.0`; `0.0` when duration is unknown.
    pub fn progress_fraction(&self) -> f32 {
        let Some(duration) = self.duration else {
            return 0.0;
        };
        if duration.is_zero() {
            return 0.0;
        }
        let fraction = self.position.as_secs_f32() / duration.as_secs_f32();
        if fraction.is_nan() {
            0.0
        } else {
            fraction.clamp(0.0, 1.0)
        }
    }
}

/// Clamp a raw volume value into `0.0..=1.0`; NaN is treated as silence.
pub fn normalize_volume(volume: f32) -> f32 {
    if volume.is_nan() {
        0.0
    } else {
        volume.clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn volume_and_balance_are_clamped() {
        let mut status = PlayerStatus::default();
        assert_eq!(status.volume, 1.0);

        status.set_volume(1.7);
        assert_eq!(status.volume, 1.0);
        status.set_volume(-0.4);
        assert_eq!(status.volume, 0.0);
        status.set_volume(f32::NAN);
        assert_eq!(status.volume, 0.0);
        status.set_volume(0.25);
        assert_eq!(status.volume, 0.25);

        status.set_balance(2.0);
        assert_eq!(status.balance, 1.0);
        status.set_balance(-3.5);
        assert_eq!(status.balance, -1.0);
        status.set_balance(f32::NAN);
        assert_eq!(status.balance, 0.0);
        status.set_balance(-0.5);
        assert_eq!(status.balance, -0.5);
    }

    #[test]
    fn default_status_is_stopped_and_full_volume() {
        let status = PlayerStatus::default();
        assert_eq!(status.state, PlaybackState::Stopped);
        assert_eq!(status.volume, 1.0);
        assert_eq!(status.balance, 0.0);
        assert!(!status.is_playing());
        assert_eq!(status.progress_fraction(), 0.0);
    }

    #[test]
    fn progress_fraction_is_clamped_and_handles_unknown_duration() {
        let mut status = PlayerStatus::default();

        // Unknown duration → 0.
        assert_eq!(status.progress_fraction(), 0.0);

        status.duration = Some(Duration::from_secs(100));
        status.position = Duration::from_secs(50);
        assert!((status.progress_fraction() - 0.5).abs() < 1e-6);

        status.position = Duration::from_secs(500);
        assert_eq!(status.progress_fraction(), 1.0);

        // Zero duration → 0 (no division panic).
        status.duration = Some(Duration::ZERO);
        status.position = Duration::ZERO;
        assert_eq!(status.progress_fraction(), 0.0);
    }

    #[test]
    fn playlist_total_duration_sums_known_entries_only() {
        let mut playlist = Playlist::new("mix");

        let mut with_duration = PlaylistEntry::new(PlayableItem::local("/a.mp3"));
        with_duration.meta.duration = Some(Duration::from_secs(90));
        playlist.push(with_duration);

        let unknown = PlaylistEntry::new(PlayableItem::local("/b.mp3"));
        playlist.push(unknown);

        let mut live = PlaylistEntry::new(PlayableItem::RadioStream {
            url: "http://example.com/stream".into(),
            name: Some("Cool FM".into()),
        });
        live.meta.duration = Some(Duration::from_secs(30));
        playlist.push(live);

        assert_eq!(playlist.len(), 3);
        assert_eq!(playlist.total_duration(), Duration::from_secs(120));

        let removed = playlist.remove(1);
        assert!(removed.is_some());
        assert_eq!(playlist.len(), 2);
        assert_eq!(playlist.total_duration(), Duration::from_secs(120));
        assert!(playlist.remove(99).is_none());
    }

    #[test]
    fn display_title_falls_back_gracefully() {
        // Tagged title wins.
        let mut entry = PlaylistEntry::new(PlayableItem::local("/music/song.mp3"));
        entry.meta.title = Some("  ".into()); // blank → treated as missing
        assert_eq!(entry.display_title(), "song");

        entry.meta.title = Some("Real Title".into());
        assert_eq!(entry.display_title(), "Real Title");

        // Untagged local file → file stem.
        let entry = PlaylistEntry::new(PlayableItem::local("/music/01 - Intro.flac"));
        assert_eq!(entry.display_title(), "01 - Intro");

        // Radio with name → name; without → URL.
        let entry = PlaylistEntry::new(PlayableItem::RadioStream {
            url: "http://example.com/stream.mp3".into(),
            name: Some("Cool FM".into()),
        });
        assert_eq!(entry.display_title(), "Cool FM");

        let entry = PlaylistEntry::new(PlayableItem::RadioStream {
            url: "http://example.com/stream.mp3".into(),
            name: None,
        });
        assert_eq!(entry.display_title(), "http://example.com/stream.mp3");

        // Navidrome → track id.
        let entry = PlaylistEntry::new(PlayableItem::NavidromeTrack {
            server_id: "home".into(),
            track_id: "1042".into(),
        });
        assert_eq!(entry.display_title(), "1042");
    }

    #[test]
    fn track_meta_is_empty_reported_correctly() {
        assert!(TrackMeta::default().is_empty());
        let meta = TrackMeta {
            artist: Some("Someone".into()),
            ..TrackMeta::default()
        };
        assert!(!meta.is_empty());
    }

    #[test]
    fn playable_item_local_constructor() {
        let item = PlayableItem::local("/tmp/x.mp3");
        assert_eq!(
            item,
            PlayableItem::LocalFile {
                path: PathBuf::from("/tmp/x.mp3")
            }
        );
        // Hashable (usable as map keys).
        use std::collections::HashSet;
        let mut seen = HashSet::new();
        assert!(seen.insert(item.clone()));
        assert!(!seen.insert(PlayableItem::local(Path::new("/tmp/x.mp3"))));
    }
}
