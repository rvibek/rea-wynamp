//! Queue state machine: which track plays next under shuffle/repeat rules.
//!
//! Pure logic with no I/O — fully unit tested. The engine thread owns one
//! instance and mutates it in response to `PlayerCommand`s.

use crate::models::{PlaylistEntry, RepeatMode};

pub struct QueueState {
    entries: Vec<PlaylistEntry>,
    /// Index of the track the decoder is on (or heading to).
    current: Option<usize>,
    shuffle: bool,
    repeat: RepeatMode,
    rng: u64,
}

impl QueueState {
    pub fn new(seed: u64) -> Self {
        Self {
            entries: Vec::new(),
            current: None,
            shuffle: false,
            repeat: RepeatMode::Off,
            // xorshift must not start at zero.
            rng: if seed == 0 {
                0x2545_F491_4F6C_DD1D
            } else {
                seed
            },
        }
    }

    pub fn entries(&self) -> &[PlaylistEntry] {
        &self.entries
    }

    pub fn current(&self) -> Option<usize> {
        self.current
    }

    pub fn entry(&self, index: usize) -> Option<&PlaylistEntry> {
        self.entries.get(index)
    }

    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    pub fn repeat(&self) -> RepeatMode {
        self.repeat
    }

    pub fn set_shuffle(&mut self, shuffle: bool) {
        self.shuffle = shuffle;
    }

    pub fn set_repeat(&mut self, repeat: RepeatMode) {
        self.repeat = repeat;
    }

    pub fn set_current(&mut self, index: Option<usize>) {
        self.current = index.filter(|&i| i < self.entries.len());
    }

    /// Replace the whole queue and return the index to start at
    /// (`None` when the new queue is empty).
    pub fn replace(
        &mut self,
        entries: Vec<PlaylistEntry>,
        start_at: Option<usize>,
    ) -> Option<usize> {
        self.entries = entries;
        self.current = if self.entries.is_empty() {
            None
        } else {
            Some(start_at.unwrap_or(0).min(self.entries.len() - 1))
        };
        self.current
    }

    /// Append entries; playback state is untouched (the engine decides).
    pub fn enqueue(&mut self, items: Vec<PlaylistEntry>) {
        self.entries.extend(items);
    }

    /// Remove an entry. If it was the current track, `current` becomes `None`
    /// (the engine stops playback); indices after it shift down.
    pub fn remove(&mut self, index: usize) -> Option<PlaylistEntry> {
        if index >= self.entries.len() {
            return None;
        }
        let removed = self.entries.remove(index);
        self.current = match self.current {
            Some(c) if c == index => None,
            Some(c) if c > index => Some(c - 1),
            other => other,
        };
        Some(removed)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.current = None;
    }

    /// Where to go when the current track ends naturally (not a manual skip).
    pub fn natural_end(&mut self) -> Option<usize> {
        let cur = self.current?;
        match self.repeat {
            RepeatMode::One => Some(cur),
            _ => self.advance_forward(cur),
        }
    }

    /// Manual "next": repeat-one does not hold playback in place.
    pub fn next_manual(&mut self) -> Option<usize> {
        let cur = self.current?;
        self.advance_forward(cur)
    }

    /// Manual "previous".
    pub fn prev_manual(&mut self) -> Option<usize> {
        let cur = self.current?;
        let len = self.entries.len();
        if self.shuffle && len > 1 {
            return Some(self.random_other(cur));
        }
        if cur > 0 {
            Some(cur - 1)
        } else if self.repeat == RepeatMode::All && len > 0 {
            Some(len - 1)
        } else {
            None
        }
    }

    /// Advance from `idx` by the shuffle/repeat rules (also used to skip
    /// unopenable files). Does not mutate `current`.
    pub fn advance_from(&mut self, idx: usize) -> Option<usize> {
        self.advance_forward(idx)
    }

    fn advance_forward(&mut self, cur: usize) -> Option<usize> {
        let len = self.entries.len();
        if len == 0 {
            return None;
        }
        if self.shuffle && len > 1 {
            return Some(self.random_other(cur));
        }
        if cur + 1 < len {
            Some(cur + 1)
        } else if self.repeat == RepeatMode::All {
            Some(0)
        } else {
            None
        }
    }

    fn random_other(&mut self, cur: usize) -> usize {
        let len = self.entries.len();
        debug_assert!(len > 1);
        loop {
            let pick = (self.next_rand() % len as u64) as usize;
            if pick != cur {
                return pick;
            }
        }
    }

    /// xorshift64* — deterministic under a seed, no external RNG dependency.
    fn next_rand(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{PlayableItem, PlaylistEntry};

    fn entries(n: usize) -> Vec<PlaylistEntry> {
        (0..n)
            .map(|i| PlaylistEntry::new(PlayableItem::local(format!("/t/{i}.mp3"))))
            .collect()
    }

    #[test]
    fn replace_clamps_start_index_and_handles_empty() {
        let mut q = QueueState::new(1);
        assert_eq!(q.replace(entries(3), Some(99)), Some(2));
        assert_eq!(q.replace(entries(3), Some(1)), Some(1));
        assert_eq!(q.replace(entries(3), None), Some(0));
        assert_eq!(q.replace(Vec::new(), Some(0)), None);
        assert_eq!(q.current(), None);
    }

    #[test]
    fn remove_shifts_or_clears_current() {
        let mut q = QueueState::new(1);
        q.replace(entries(3), Some(1));

        // Removing before current shifts it down.
        let removed = q.remove(0);
        assert!(removed.is_some());
        assert_eq!(q.current(), Some(0));
        assert_eq!(q.entries().len(), 2);

        // Removing current clears it (engine stops playback).
        assert!(q.remove(0).is_some());
        assert_eq!(q.current(), None);

        // Out of range is a no-op.
        assert!(q.remove(10).is_none());
    }

    #[test]
    fn natural_end_respects_repeat_modes() {
        let mut q = QueueState::new(7);
        q.replace(entries(3), Some(0));

        // Off: walks to the end, then stops. (natural_end does not mutate
        // current — the engine sets it when the next track opens.)
        assert_eq!(q.natural_end(), Some(1));
        q.set_current(Some(1));
        assert_eq!(q.natural_end(), Some(2));
        q.set_current(Some(2));
        assert_eq!(q.natural_end(), None);

        // All: wraps.
        q.set_repeat(RepeatMode::All);
        q.set_current(Some(2));
        assert_eq!(q.natural_end(), Some(0));

        // One: stays on the same track.
        q.set_repeat(RepeatMode::One);
        q.set_current(Some(1));
        assert_eq!(q.natural_end(), Some(1));
    }

    #[test]
    fn manual_next_ignores_repeat_one() {
        let mut q = QueueState::new(7);
        q.replace(entries(3), Some(1));
        q.set_repeat(RepeatMode::One);
        assert_eq!(q.next_manual(), Some(2));
        q.set_current(Some(2));
        assert_eq!(q.next_manual(), None); // One does not wrap on manual next
    }

    #[test]
    fn single_track_queue_rules() {
        let mut q = QueueState::new(3);
        q.replace(entries(1), Some(0));
        assert_eq!(q.natural_end(), None); // Off: done
        q.set_repeat(RepeatMode::All);
        assert_eq!(q.natural_end(), Some(0));
        q.set_shuffle(true);
        assert_eq!(q.natural_end(), Some(0)); // shuffle can't leave a 1-track queue
        q.set_repeat(RepeatMode::Off);
        assert_eq!(q.next_manual(), None);
    }

    #[test]
    fn shuffle_never_picks_current_track() {
        let mut q = QueueState::new(42);
        q.replace(entries(5), Some(2));
        q.set_shuffle(true);
        for _ in 0..50 {
            let cur = q.current().expect("has current");
            let next = q.next_manual().expect("shuffle with len>1 always advances");
            assert_ne!(next, cur, "shuffle must leave the current track");
            assert!(next < 5);
            q.set_current(Some(next));
        }
        // Previous also avoids the current track under shuffle.
        let cur = q.current().expect("has current");
        let prev = q.prev_manual().expect("shuffle prev");
        assert_ne!(prev, cur);
    }

    #[test]
    fn prev_wraps_only_under_repeat_all() {
        let mut q = QueueState::new(1);
        q.replace(entries(3), Some(0));
        assert_eq!(q.prev_manual(), None);
        q.set_repeat(RepeatMode::All);
        assert_eq!(q.prev_manual(), Some(2));
    }

    #[test]
    fn enqueue_keeps_current() {
        let mut q = QueueState::new(1);
        q.replace(entries(2), Some(1));
        q.enqueue(entries(2));
        assert_eq!(q.current(), Some(1));
        assert_eq!(q.entries().len(), 4);
    }
}
