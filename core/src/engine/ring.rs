//! The PCM ring buffer between the decode loop and the audio callback.
//!
//! Interleaved stereo `f32` samples. The decode thread pushes, the cpal
//! callback pops; a `Mutex<VecDeque>` is plenty at these rates (the lock is
//! held for microseconds) and recovers from poisoned locks so one panicking
//! side can never wedge playback.

use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard, PoisonError};

pub struct PcmRing {
    inner: Mutex<VecDeque<f32>>,
    capacity: usize,
}

impl PcmRing {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(VecDeque::with_capacity(capacity.max(1))),
            capacity: capacity.max(1),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// Push as many samples as fit; returns how many were accepted.
    pub fn push(&self, samples: &[f32]) -> usize {
        let mut q = self.lock();
        let room = self.capacity.saturating_sub(q.len());
        let n = room.min(samples.len());
        q.extend(&samples[..n]);
        n
    }

    /// Pop samples from the front of `pending` into the ring while room
    /// remains; returns how many were moved. `pending` keeps its remainder.
    pub fn push_from_front(&self, pending: &mut VecDeque<f32>) -> usize {
        let mut q = self.lock();
        let room = self.capacity.saturating_sub(q.len());
        let n = room.min(pending.len());
        for _ in 0..n {
            match pending.pop_front() {
                Some(s) => q.push_back(s),
                None => break,
            }
        }
        n
    }

    /// Fill `out` from the ring, zeroing any shortfall. Returns the number of
    /// samples that actually came from the ring (i.e. that count as consumed).
    pub fn pop_into(&self, out: &mut [f32]) -> usize {
        let mut q = self.lock();
        let mut i = 0;
        while i < out.len() {
            match q.pop_front() {
                Some(s) => {
                    out[i] = s;
                    i += 1;
                }
                None => break,
            }
        }
        for slot in out[i..].iter_mut() {
            *slot = 0.0;
        }
        i
    }

    /// Discard up to `max_samples` from the front (headless "consumption").
    /// Returns how many were discarded.
    pub fn pop_discard(&self, max_samples: usize) -> usize {
        let mut q = self.lock();
        let n = max_samples.min(q.len());
        for _ in 0..n {
            q.pop_front();
        }
        n
    }

    /// Drop everything; returns the number of samples discarded.
    pub fn clear(&self) -> usize {
        let mut q = self.lock();
        let n = q.len();
        q.clear();
        n
    }

    fn lock(&self) -> MutexGuard<'_, VecDeque<f32>> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_respects_capacity_and_reports_accepted() {
        let ring = PcmRing::new(8);
        assert_eq!(ring.push(&[1.0, 2.0, 3.0]), 3);
        assert_eq!(ring.push(&[4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0]), 5);
        assert_eq!(ring.len(), 8);
        // Full: nothing accepted.
        assert_eq!(ring.push(&[11.0]), 0);
    }

    #[test]
    fn pop_into_fills_shortfall_with_silence() {
        let ring = PcmRing::new(16);
        ring.push(&[1.0, 2.0, 3.0]);
        let mut out = [9.0; 5];
        let popped = ring.pop_into(&mut out);
        assert_eq!(popped, 3);
        assert_eq!(out, [1.0, 2.0, 3.0, 0.0, 0.0]);
        assert!(ring.is_empty());

        // Empty ring → all silence, nothing consumed.
        let mut out2 = [7.0; 2];
        assert_eq!(ring.pop_into(&mut out2), 0);
        assert_eq!(out2, [0.0, 0.0]);
    }

    #[test]
    fn push_from_front_moves_partials_and_keeps_remainder() {
        let ring = PcmRing::new(4);
        ring.push(&[1.0, 2.0]); // 2 slots used, 2 free
        let mut pending: VecDeque<f32> = [10.0, 20.0, 30.0].into();
        let moved = ring.push_from_front(&mut pending);
        assert_eq!(moved, 2);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending.front(), Some(&30.0));
        assert_eq!(ring.len(), 4);

        // Full: nothing moves.
        assert_eq!(ring.push_from_front(&mut pending), 0);
    }

    #[test]
    fn clear_reports_discarded_count() {
        let ring = PcmRing::new(8);
        ring.push(&[1.0, 2.0, 3.0]);
        assert_eq!(ring.clear(), 3);
        assert!(ring.is_empty());
        assert_eq!(ring.clear(), 0);
    }

    #[test]
    fn pop_discard_caps_at_content() {
        let ring = PcmRing::new(8);
        ring.push(&[1.0, 2.0]);
        assert_eq!(ring.pop_discard(10), 2);
        assert_eq!(ring.pop_discard(10), 0);
    }
}
