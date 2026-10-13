//! Sample-format plumbing: channel mixing and sample-rate conversion.
//!
//! The device runs at its own sample rate; decoded tracks may differ. When
//! rates match the engine bypasses the resampler entirely (the common case
//! for 48 kHz devices + 48 kHz files). Otherwise a streaming linear
//! resampler converts — good enough for v0.1.0 (documented limitation;
//! a proper resampler such as `rubato` is a later upgrade).

use crate::error::{Error, Result};

/// Downmix/mix any channel layout to interleaved stereo.
pub fn to_stereo(samples: &[f32], channels: u16) -> Result<Vec<f32>> {
    match channels {
        0 => Err(Error::Decode("file reports zero channels".into())),
        1 => Ok(samples.iter().flat_map(|&s| [s, s]).collect()),
        2 => Ok(samples.to_vec()),
        n => {
            let n = n as usize;
            if !samples.len().is_multiple_of(n) {
                return Err(Error::Decode(
                    "partial audio frame in decoded packet".into(),
                ));
            }
            // Layouts beyond stereo (surround etc.): take the first two
            // channels, which are L/R in all standard layouts.
            Ok(samples
                .chunks_exact(n)
                .flat_map(|frame| [frame[0], frame[1]])
                .collect())
        }
    }
}

/// Streaming linear resampler for interleaved stereo frames.
///
/// Keeps one anchor frame between calls so output is continuous across
/// packet boundaries. Inherent pipeline delay is a single input frame.
pub struct LinearResampler {
    /// Input frames consumed per output frame.
    ratio: f64,
    /// Read position (in input frames) into the current virtual buffer.
    pos: f64,
    /// Last frame of the previous call — becomes frame 0 of the next call.
    last: Option<(f32, f32)>,
}

impl LinearResampler {
    pub fn new(in_rate: u32, out_rate: u32) -> Self {
        let in_rate = in_rate.max(1) as f64;
        let out_rate = out_rate.max(1) as f64;
        Self {
            ratio: in_rate / out_rate,
            pos: 0.0,
            last: None,
        }
    }

    /// Convert one chunk of interleaved stereo samples; returns interleaved
    /// stereo at the output rate (possibly empty while it accumulates input).
    pub fn process(&mut self, chunk: &[f32]) -> Vec<f32> {
        if chunk.len() < 2 {
            return Vec::new();
        }

        // Virtual buffer: carried anchor frame + this chunk's frames.
        let mut frames: Vec<(f32, f32)> = Vec::with_capacity(chunk.len() / 2 + 1);
        if let Some(last) = self.last {
            frames.push(last);
        }
        frames.extend(chunk.as_chunks::<2>().0.iter().map(|f| (f[0], f[1])));

        let n = frames.len();
        if n < 2 {
            // Only the anchor (or nothing) — nothing to interpolate yet.
            return Vec::new();
        }

        let mut out = Vec::with_capacity((n as f64 / self.ratio) as usize * 2 + 4);
        while (self.pos + 1.0) < n as f64 {
            let i = self.pos.floor() as usize;
            let frac = (self.pos - i as f64) as f32;
            let a = frames[i];
            let b = frames[i + 1];
            out.push(a.0 + (b.0 - a.0) * frac);
            out.push(a.1 + (b.1 - a.1) * frac);
            self.pos += self.ratio;
        }

        // Rebase: keep the final frame as the next call's anchor.
        self.pos -= (n - 1) as f64;
        self.last = Some(frames[n - 1]);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_is_duplicated_to_both_channels() {
        let out = to_stereo(&[0.5, -0.5], 1).unwrap();
        assert_eq!(out, [0.5, 0.5, -0.5, -0.5]);
    }

    #[test]
    fn stereo_passes_through() {
        let out = to_stereo(&[1.0, 2.0, 3.0, 4.0], 2).unwrap();
        assert_eq!(out, [1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn surround_takes_front_left_right() {
        // Two 4-channel frames: L R Ls Rs → keep L R.
        let out = to_stereo(&[0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8], 4).unwrap();
        assert_eq!(out, [0.1, 0.2, 0.5, 0.6]);
    }

    #[test]
    fn zero_channels_is_an_error() {
        assert!(to_stereo(&[1.0], 0).is_err());
        // Whole frames of 4 channels that don't divide evenly → error.
        assert!(to_stereo(&[1.0, 2.0, 3.0], 4).is_err());
        assert!(to_stereo(&[1.0, 2.0, 3.0, 4.0], 4).is_ok());
    }

    #[test]
    fn resampler_downsamples_by_exact_ratio() {
        let mut r = LinearResampler::new(2, 1); // 2:1
                                                // Frames 0..7 with distinguishable L/R values.
        let input: Vec<f32> = (0..8).flat_map(|i| [i as f32, 100.0 + i as f32]).collect();
        let out = r.process(&input);
        // Output frames at input positions 0,2,4,6 (7th would be pos 8 → stop).
        let expected: Vec<f32> = [0, 2, 4, 6]
            .iter()
            .flat_map(|&i| [i as f32, 100.0 + i as f32])
            .collect();
        assert_eq!(out, expected);
    }

    #[test]
    fn resampler_upsamples_with_interpolation() {
        let mut r = LinearResampler::new(1, 2); // 1:2
                                                // Two frames: 0.0 and 1.0 on both channels.
        let out = r.process(&[0.0, 0.0, 1.0, 1.0]);
        // Positions 0.0, 0.5 → (0,0) and midpoint (0.5,0.5); pos 1.0 exits
        // because it needs frames[1]+frames[2].
        assert_eq!(out.len(), 4);
        assert_eq!(out[0], 0.0);
        assert!((out[2] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn resampler_rate_one_preserves_frame_count_across_chunks() {
        let mut r = LinearResampler::new(1, 1);
        // 16 stereo frames split across two chunks: count outputs ≈ inputs
        // (single-frame pipeline delay).
        let chunk: Vec<f32> = (0..16).flat_map(|i| [i as f32, 0.0]).collect();
        let first = r.process(&chunk[..16]); // 8 frames
        let second = r.process(&chunk[16..]); // 8 frames
        let total_frames = (first.len() + second.len()) / 2;
        // First call: 8 frames → 7 outputs (one carried). Second call:
        // 1 anchor + 8 = 9 frames → 8 outputs. Total 15 of 16 (delay = 1).
        assert_eq!(total_frames, 15);
        // Values arrive in order; one input frame is inherent pipeline delay
        // (frame 15 is the carried anchor and is emitted with the next call).
        assert_eq!(first[0], 0.0);
        assert_eq!(second[second.len() - 2], 14.0);
    }

    #[test]
    fn resampler_carries_anchor_across_calls() {
        let mut r = LinearResampler::new(2, 1);
        let a: Vec<f32> = (0..4).flat_map(|i| [i as f32, 0.0]).collect();
        let out1 = r.process(&a);
        let b: Vec<f32> = (4..8).flat_map(|i| [i as f32, 0.0]).collect();
        let out2 = r.process(&b);
        // Whole-input reference for the same data:
        let mut reference = LinearResampler::new(2, 1);
        let whole: Vec<f32> = (0..8).flat_map(|i| [i as f32, 0.0]).collect();
        let out_all = reference.process(&whole);
        // Split processing produces the same frames as one shot.
        let mut combined = out1;
        combined.extend(out2);
        assert_eq!(combined, out_all);
    }
}
