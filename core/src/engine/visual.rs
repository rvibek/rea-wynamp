//! Visualizer: a live PCM tap + spectrum computation (rustfft).
//!
//! The decode thread pushes post-EQ samples into a shared ring; the app
//! thread snapshots it ~30×/s for the scope canvas and computes log-spaced
//! spectrum bars for the analyzer canvas.

use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

/// Stereo samples retained (2048 frames at 2 ch).
const TAP_CAPACITY: usize = 8_192;
/// FFT size used for spectrum snapshots.
const FFT_SIZE: usize = 1_024;

/// Ring of recent interleaved-stereo samples shared with the UI side.
pub struct VisualTap {
    buf: Mutex<VecDeque<f32>>,
}

impl Default for VisualTap {
    fn default() -> Self {
        Self::new()
    }
}

impl VisualTap {
    pub fn new() -> Self {
        Self {
            buf: Mutex::new(VecDeque::with_capacity(TAP_CAPACITY)),
        }
    }

    /// Append interleaved stereo samples, dropping the oldest overflow.
    pub fn push(&self, samples: &[f32]) {
        let mut q = self.lock();
        q.extend(samples);
        let overflow = q.len().saturating_sub(TAP_CAPACITY);
        if overflow > 0 {
            q.drain(..overflow);
        }
    }

    /// Last `frames` frames as a mono mixdown (L+R averaged).
    pub fn snapshot_mono(&self, frames: usize) -> Vec<f32> {
        let q = self.lock();
        let want = frames * 2;
        let start = q.len().saturating_sub(want);
        let recent: Vec<f32> = q.range(start..).copied().collect();
        recent
            .as_chunks::<2>()
            .0
            .iter()
            .map(|f| (f[0] + f[1]) * 0.5)
            .collect()
    }

    fn lock(&self) -> MutexGuard<'_, VecDeque<f32>> {
        self.buf.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn cached_fft(len: usize) -> &'static dyn Fft<f32> {
    // The engine/UI use one size (FFT_SIZE); cache it process-wide.
    static FFT: OnceLock<std::sync::Arc<dyn Fft<f32>>> = OnceLock::new();
    let _ = len; // sized for future flexibility
    FFT.get_or_init(|| {
        let mut planner = FftPlanner::new();
        planner.plan_fft_forward(FFT_SIZE)
    })
    .as_ref()
}

/// Log-spaced spectrum bars, `0.0..=1.0`, from a mono window.
///
/// Hann window, dB mapping of −70..0 dB relative to the frame max, then
/// mean magnitude per log-frequency band (40 Hz – 16 kHz).
pub fn spectrum(mono: &[f32], bands: usize) -> Vec<f32> {
    if bands == 0 {
        return Vec::new();
    }
    if mono.iter().all(|&s| s == 0.0) {
        return vec![0.0; bands];
    }

    // Window + zero-pad/truncate to FFT_SIZE.
    let n = mono.len().min(FFT_SIZE);
    let mut buf: Vec<Complex<f32>> = Vec::with_capacity(FFT_SIZE);
    for (i, &s) in mono.iter().take(FFT_SIZE).enumerate() {
        let window = 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / n as f32).cos();
        buf.push(Complex::new(s * window, 0.0));
    }
    buf.resize(FFT_SIZE, Complex::new(0.0, 0.0));
    cached_fft(FFT_SIZE).process(&mut buf);

    let half = FFT_SIZE / 2;
    // Magnitudes for bins 1..half (skip DC).
    let mags: Vec<f32> = buf[1..=half].iter().map(|c| c.norm()).collect();

    // Log-spaced band edges over 40 Hz..16 kHz (relative to sample positions).
    // We don't know the sample rate here; map bands over bin indices using a
    // nominal 48 kHz rate (visual layout, not measurement-grade).
    let nyquist_bins = half as f32;
    let min_bin = (40.0 / 24_000.0 * nyquist_bins).max(1.0);
    let max_bin = (16_000.0 / 24_000.0 * nyquist_bins).min(nyquist_bins);

    let mut out = Vec::with_capacity(bands);
    let mut max_db = f32::NEG_INFINITY;
    let mut raw = Vec::with_capacity(bands);
    for band in 0..bands {
        let lo = min_bin * (max_bin / min_bin).powf(band as f32 / bands as f32);
        let hi = min_bin * (max_bin / min_bin).powf((band + 1) as f32 / bands as f32);
        let start_f = (lo - 1.0).round().clamp(0.0, nyquist_bins - 1.0);
        let end_f = (hi - 1.0).round().clamp(start_f + 1.0, nyquist_bins);
        let start = start_f as usize;
        let end = end_f as usize;
        let mean = mags[start..end].iter().sum::<f32>() / (end - start) as f32;
        let db = 20.0 * (mean + 1e-9).log10();
        max_db = max_db.max(db);
        raw.push(db);
    }
    // Map −70..(max) dB → 0..1 so the display is always dynamic.
    let ceiling = max_db.max(-10.0);
    for db in raw {
        let v = ((db - (ceiling - 60.0)) / 60.0).clamp(0.0, 1.0);
        out.push(v);
    }
    out
}

/// Evenly resample a mono buffer down to `points` for the oscilloscope.
pub fn downsample(mono: &[f32], points: usize) -> Vec<f32> {
    if points == 0 || mono.is_empty() {
        return Vec::new();
    }
    if mono.len() <= points {
        return mono.to_vec();
    }
    let step = mono.len() as f32 / points as f32;
    (0..points)
        .map(|i| {
            let start = (i as f32 * step) as usize;
            let end = (((i + 1) as f32) * step) as usize;
            let end = end.max(start + 1).min(mono.len());
            let slice = &mono[start..end];
            slice.iter().sum::<f32>() / slice.len() as f32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tap_keeps_only_the_newest_samples() {
        let tap = VisualTap::new();
        tap.push(&[1.0, 1.0]); // 1 frame
        let snap = tap.snapshot_mono(1);
        assert_eq!(snap, vec![1.0]);

        // Overflow the capacity.
        let big = vec![0.5f32; TAP_CAPACITY + 100];
        tap.push(&big);
        let snap = tap.snapshot_mono(10);
        assert_eq!(snap.len(), 10);
        assert!(snap.iter().all(|&s| s == 0.5));
    }

    #[test]
    fn silence_yields_silent_spectrum() {
        assert_eq!(spectrum(&[0.0; 1_024], 16), vec![0.0; 16]);
    }

    #[test]
    fn sine_peaks_near_its_band() {
        // 1 kHz sine at nominal 48 kHz.
        let mono: Vec<f32> = (0..1_024)
            .map(|i| (2.0 * std::f32::consts::PI * 1_000.0 * i as f32 / 48_000.0).sin())
            .collect();
        let bars = spectrum(&mono, 16);
        assert_eq!(bars.len(), 16);
        let peak = bars
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;
        // Bands are log-spaced 40..16k over 16 slots; 1 kHz sits around band 8-10.
        assert!(
            (6..=11).contains(&peak),
            "1 kHz should peak around bands 6–11, got {peak} ({bars:?})"
        );
    }

    #[test]
    fn downsample_picks_representative_points() {
        let mono: Vec<f32> = (0..100).map(|i| i as f32).collect();
        let out = downsample(&mono, 10);
        assert_eq!(out.len(), 10);
        assert!(out[0] < out[9]);
        // Short input passes through untouched.
        assert_eq!(downsample(&[1.0, 2.0], 10), vec![1.0, 2.0]);
        assert!(downsample(&[], 5).is_empty());
    }
}
