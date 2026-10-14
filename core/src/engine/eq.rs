//! 10-band graphic equalizer (RBJ peaking biquads) + preamp.
//!
//! Interleaved stereo, in-place, applied at the *output* sample rate after
//! resampling — biquad coefficients are sample-rate dependent, so the chain
//! is rebuilt whenever the rate or gains change. Zeroed gains are an exact
//! identity, so "flat" is bit-transparent.

use std::f32::consts::PI;

/// ISO center frequencies for a classic graphic EQ (Hz).
pub const BAND_FREQUENCIES: [f32; 10] = [
    31.25, 62.5, 125.0, 250.0, 500.0, 1_000.0, 2_000.0, 4_000.0, 8_000.0, 16_000.0,
];

/// Per-band gain clamp (dB). Graphic EQs rarely exceed ±14.
pub const GAIN_MIN_DB: f32 = -14.0;
pub const GAIN_MAX_DB: f32 = 14.0;
/// Preamp clamp (dB).
pub const PREAMP_MIN_DB: f32 = -12.0;
pub const PREAMP_MAX_DB: f32 = 12.0;

/// Filter bandwidth. Q=1.0 gives broad, musical overlap between octave bands.
const BAND_Q: f32 = 1.0;

pub fn clamp_gain_db(db: f32) -> f32 {
    if db.is_nan() {
        0.0
    } else {
        db.clamp(GAIN_MIN_DB, GAIN_MAX_DB)
    }
}

pub fn clamp_preamp_db(db: f32) -> f32 {
    if db.is_nan() {
        0.0
    } else {
        db.clamp(PREAMP_MIN_DB, PREAMP_MAX_DB)
    }
}

fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// One peaking filter's normalized coefficients.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Coeffs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Coeffs {
    /// RBJ cookbook peaking-EQ section.
    fn peaking(freq: f32, gain_db: f32, q: f32, rate: f32) -> Self {
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * freq / rate;
        let (sin_w0, cos_w0) = w0.sin_cos();
        let alpha = sin_w0 / (2.0 * q);

        let b0 = 1.0 + alpha * a;
        let b1 = -2.0 * cos_w0;
        let b2 = 1.0 - alpha * a;
        let a0 = 1.0 + alpha / a;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha / a;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }
}

/// Transposed direct-form II state for one channel of one band.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct State {
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl State {
    fn process(&mut self, c: &Coeffs, x: f32) -> f32 {
        let y = c.b0 * x + c.b1 * self.x1 + c.b2 * self.x2 - c.a1 * self.y1 - c.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

struct Band {
    gain_db: f32,
    coeffs: Coeffs,
    /// Per-channel state: [left, right].
    state: [State; 2],
}

/// The full EQ chain: preamp + 10 stereo peaking filters.
pub struct EqChain {
    enabled: bool,
    preamp_db: f32,
    gains_db: [f32; 10],
    rate: u32,
    bands: Vec<Band>,
}

impl EqChain {
    /// Build a flat, enabled chain for the given output rate.
    pub fn new(rate: u32) -> Self {
        let mut chain = Self {
            enabled: true,
            preamp_db: 0.0,
            gains_db: [0.0; 10],
            rate: rate.max(1),
            bands: Vec::new(),
        };
        chain.rebuild();
        chain
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn preamp_db(&self) -> f32 {
        self.preamp_db
    }

    pub fn gains_db(&self) -> [f32; 10] {
        self.gains_db
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        if enabled && !self.enabled {
            // Fresh states on re-enable — avoid a click from stale history.
            self.reset_states();
        }
        self.enabled = enabled;
    }

    pub fn set_preamp_db(&mut self, db: f32) {
        self.preamp_db = clamp_preamp_db(db);
    }

    /// Set all band gains at once (must be exactly 10 values; others ignored).
    pub fn set_gains_db(&mut self, gains: &[f32]) -> bool {
        if gains.len() != 10 {
            return false;
        }
        for (slot, &g) in self.gains_db.iter_mut().zip(gains) {
            *slot = clamp_gain_db(g);
        }
        self.rebuild();
        true
    }

    pub fn set_gain_db(&mut self, band: usize, db: f32) -> bool {
        if band >= 10 {
            return false;
        }
        self.gains_db[band] = clamp_gain_db(db);
        self.rebuild();
        true
    }

    /// Apply EQ in place to interleaved stereo samples at `self.rate`.
    pub fn process(&mut self, samples: &mut [f32]) {
        if !self.enabled {
            return;
        }
        let preamp = db_to_linear(self.preamp_db);
        // Flat chain with unity preamp is an exact identity — skip the math.
        if preamp == 1.0 && self.gains_db.iter().all(|&g| g == 0.0) {
            return;
        }
        for frame in samples.as_chunks_mut::<2>().0 {
            let mut l = frame[0] * preamp;
            let mut r = frame[1] * preamp;
            for band in &mut self.bands {
                l = band.state[0].process(&band.coeffs, l);
                r = band.state[1].process(&band.coeffs, r);
            }
            frame[0] = l;
            frame[1] = r;
        }
    }

    fn rebuild(&mut self) {
        let rate = self.rate as f32;
        if self.bands.is_empty() {
            self.bands = BAND_FREQUENCIES
                .iter()
                .map(|&freq| Band {
                    gain_db: 0.0,
                    coeffs: Coeffs::peaking(freq, 0.0, BAND_Q, rate),
                    state: [State::default(); 2],
                })
                .collect();
        }
        for (band, (&freq, &gain)) in self
            .bands
            .iter_mut()
            .zip(BAND_FREQUENCIES.iter().zip(&self.gains_db))
        {
            band.gain_db = gain;
            band.coeffs = Coeffs::peaking(freq, gain, BAND_Q, rate);
        }
    }

    fn reset_states(&mut self) {
        for band in &mut self.bands {
            band.state = [State::default(); 2];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, rate: f32, frames: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                let s = (2.0 * PI * freq * i as f32 / rate).sin() * 0.5;
                [s, s]
            })
            .collect()
    }

    fn rms(samples: &[f32]) -> f32 {
        let sum: f32 = samples.iter().map(|s| s * s).sum();
        (sum / samples.len() as f32).sqrt()
    }

    #[test]
    fn zero_gains_are_an_exact_identity() {
        let mut eq = EqChain::new(48_000);
        let mut buf = sine(1_000.0, 48_000.0, 4_096);
        let original = buf.clone();
        eq.process(&mut buf);
        assert_eq!(buf, original, "flat EQ must not touch samples");
    }

    #[test]
    fn disabled_chain_is_bypassed() {
        let mut eq = EqChain::new(48_000);
        assert!(eq.set_gains_db(&[12.0; 10]));
        eq.set_enabled(false);
        let mut buf = sine(62.5, 48_000.0, 2_048);
        let original = buf.clone();
        eq.process(&mut buf);
        assert_eq!(buf, original);
    }

    #[test]
    fn boost_raises_rms_at_band_center() {
        let mut eq = EqChain::new(48_000);
        assert!(eq.set_gain_db(5, 12.0)); // 1 kHz band +12 dB
        let mut buf = sine(1_000.0, 48_000.0, 8_192);
        let before = rms(&buf);
        // Skip the filter warm-up, measure the tail.
        eq.process(&mut buf);
        let after = rms(&buf[4_096..]);
        assert!(
            after > before * 2.5,
            "+12 dB at center should more than double RMS: {before} -> {after}"
        );
    }

    #[test]
    fn cut_lowers_rms_at_band_center() {
        let mut eq = EqChain::new(48_000);
        assert!(eq.set_gain_db(5, -24.0));
        let mut buf = sine(1_000.0, 48_000.0, 8_192);
        let before = rms(&buf);
        eq.process(&mut buf);
        let after = rms(&buf[4_096..]);
        assert!(
            after < before * 0.25,
            "-24 dB at center should quarter RMS: {before} -> {after}"
        );
    }

    #[test]
    fn preamp_scales_everything() {
        let mut eq = EqChain::new(48_000);
        eq.set_preamp_db(6.0);
        let mut buf = sine(500.0, 48_000.0, 4_096);
        let before = rms(&buf);
        eq.process(&mut buf);
        let after = rms(&buf);
        // +6 dB ≈ ×2.0
        assert!(
            (after / before - 2.0).abs() < 0.05,
            "preamp +6 dB should double RMS: {before} -> {after}"
        );
    }

    #[test]
    fn impulse_response_stays_finite() {
        let mut eq = EqChain::new(44_100);
        assert!(eq.set_gains_db(&[14.0, -14.0, 10.0, -10.0, 6.0, -6.0, 3.0, -3.0, 12.0, -12.0]));
        let mut buf = vec![0.0; 2];
        buf[0] = 1.0;
        buf[1] = 1.0;
        buf.extend(std::iter::repeat_n(0.0, 8_000));
        eq.process(&mut buf);
        assert!(buf.iter().all(|s| s.is_finite()), "filter must stay stable");
    }

    #[test]
    fn clamps_and_validation() {
        assert_eq!(clamp_gain_db(99.0), GAIN_MAX_DB);
        assert_eq!(clamp_gain_db(-99.0), GAIN_MIN_DB);
        assert_eq!(clamp_gain_db(f32::NAN), 0.0);
        assert_eq!(clamp_preamp_db(99.0), PREAMP_MAX_DB);

        let mut eq = EqChain::new(48_000);
        assert!(!eq.set_gains_db(&[0.0; 9]));
        assert!(!eq.set_gain_db(10, 3.0));
        assert!(eq.set_gains_db(&[1.0; 10]));
        assert_eq!(eq.gains_db(), [1.0; 10]);
        eq.set_preamp_db(f32::NAN);
        assert_eq!(eq.preamp_db(), 0.0);
    }

    #[test]
    fn re_enabling_resets_states() {
        let mut eq = EqChain::new(48_000);
        eq.set_gain_db(0, 12.0);
        eq.set_enabled(false);
        let mut buf = sine(31.25, 48_000.0, 512);
        eq.process(&mut buf); // bypassed
        eq.set_enabled(true);
        // Must not blow up after re-enable.
        eq.process(&mut buf);
        assert!(buf.iter().all(|s| s.is_finite()));
    }
}
