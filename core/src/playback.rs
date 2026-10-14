//! The command/event protocol between the UI and the playback engine.
//!
//! The engine owns a dedicated thread (Phase 1). The UI sends [`PlayerCommand`]s
//! over one channel and receives [`PlayerEvent`]s over another, so the window
//! event loop never blocks on audio work. See `docs/ARCHITECTURE.md`.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::models::{PlayableItem, PlayerStatus, PlaylistEntry, RepeatMode};

/// Requests sent to the playback engine thread.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PlayerCommand {
    /// Decode and play a single item, replacing the queue with just it.
    PlayItem(PlayableItem),

    /// Replace the whole queue; `start_at` is the index to begin with
    /// (defaults to the first entry when `None`).
    ReplaceQueue {
        entries: Vec<PlaylistEntry>,
        start_at: Option<usize>,
    },

    /// Append entries to the end of the queue.
    Enqueue(Vec<PlaylistEntry>),

    /// Remove the entry at the given index, if still present.
    RemoveAt(usize),

    /// Move an entry (reorder) without interrupting the current track.
    MoveEntry {
        from: usize,
        to: usize,
    },

    /// Stop and empty the queue.
    ClearQueue,

    Play,
    Pause,
    /// Toggle between paused and playing (Space bar).
    Toggle,
    Stop,
    Next,
    Previous,

    /// Seek to an absolute position in the current track.
    Seek(Duration),

    /// Relative seek in milliseconds; positive = forward (arrow keys).
    SeekRelative(i64),

    /// Absolute volume in `0.0..=1.0` (the engine clamps).
    SetVolume(f32),

    /// Relative volume change; the engine applies and clamps it.
    VolumeRelative(f32),

    /// Balance in `-1.0` (left) ..= `1.0` (right).
    SetBalance(f32),

    SetShuffle(bool),
    SetRepeat(RepeatMode),

    /// Ask the engine to emit a [`PlayerEvent::Status`] snapshot immediately.
    QueryStatus,

    /// Stop playback, flush the output device, and exit the engine thread.
    Shutdown,
}

/// Notifications sent from the playback engine thread to the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PlayerEvent {
    /// Snapshot of engine state; emitted on change and periodically (~4 Hz)
    /// while playing so clocks and seek bars stay live.
    Status(PlayerStatus),

    /// A new track started playing.
    TrackStarted(PlaylistEntry),

    /// The current track finished naturally (not via Stop/Next).
    TrackFinished(PlaylistEntry),

    /// The queue changed (enqueue/remove/clear/replace).
    QueueChanged(Vec<PlaylistEntry>),

    /// Non-fatal problem (e.g. one file in the queue failed to decode);
    /// playback continues.
    Warning { message: String },

    /// Fatal engine error; the engine has stopped.
    Fatal { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{PlayableItem, PlaylistEntry};

    #[test]
    fn commands_are_value_comparable() {
        // Enables assertion-based engine tests later (send vs. expected).
        let cmd = PlayerCommand::Seek(Duration::from_millis(1500));
        assert_eq!(cmd, PlayerCommand::Seek(Duration::from_millis(1500)));
        assert_ne!(cmd, PlayerCommand::Seek(Duration::from_millis(1501)));

        let entries = vec![PlaylistEntry::new(PlayableItem::local("/a.mp3"))];
        assert_eq!(
            PlayerCommand::ReplaceQueue {
                entries: entries.clone(),
                start_at: None
            },
            PlayerCommand::ReplaceQueue {
                entries,
                start_at: None
            }
        );
    }

    #[test]
    fn events_carry_status_snapshots() {
        let status = PlayerStatus::default();
        let event = PlayerEvent::Status(status.clone());
        assert_eq!(event, PlayerEvent::Status(status));
    }
}
