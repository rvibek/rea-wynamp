//! cpal output stream and the state shared with the audio callback.
//!
//! The callback only touches atomics + the ring buffer: no locks beyond the
//! ring's short mutex, no allocations (scratch buffer is reused), so audio
//! stays glitch-free while the engine thread does the heavy lifting.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

use super::ring::PcmRing;
use crate::error::{Error, Result};

/// State shared between the engine thread and the cpal callback.
pub struct OutputShared {
    pub ring: Arc<PcmRing>,
    /// Total samples the callback has pulled from the ring (stereo samples).
    pub consumed: Arc<AtomicU64>,
    /// Master volume as f32 bits, `0.0..=1.0`.
    pub volume: Arc<AtomicU32>,
    /// Balance as f32 bits, `-1.0..=1.0`.
    pub balance: Arc<AtomicU32>,
    /// Playback gate: callback outputs silence while `false` (pause/stop).
    pub playing: Arc<AtomicBool>,
    /// Set by the stream error callback; the engine reports it once.
    pub output_error: Arc<AtomicBool>,
}

impl OutputShared {
    pub fn new(ring: Arc<PcmRing>) -> Self {
        Self {
            ring,
            consumed: Arc::new(AtomicU64::new(0)),
            volume: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            balance: Arc::new(AtomicU32::new(0.0f32.to_bits())),
            playing: Arc::new(AtomicBool::new(false)),
            output_error: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// Device parameters the engine needs for resampling and position math.
#[derive(Debug, Clone, Copy)]
pub struct OutputInfo {
    pub sample_rate: u32,
    pub channels: u16,
}

/// Open the default output device and start it. On any failure the engine
/// falls back to headless mode (decode + virtual consumption, with a warning).
pub fn open_output(shared: &Arc<OutputShared>) -> Result<(cpal::Stream, OutputInfo)> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| Error::Output("no audio output device available".into()))?;

    let supported = device
        .default_output_config()
        .map_err(|e| Error::Output(e.to_string()))?;
    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();

    if config.channels != 2 {
        return Err(Error::Output(format!(
            "audio device is not stereo ({} channels)",
            config.channels
        )));
    }

    let info = OutputInfo {
        sample_rate: config.sample_rate,
        channels: config.channels,
    };

    let err_shared = Arc::clone(shared);
    let error_callback = move |err: cpal::Error| {
        eprintln!("[wynamp] audio stream error: {err}");
        err_shared.output_error.store(true, Ordering::Release);
    };

    let stream = match sample_format {
        SampleFormat::F32 => {
            build_stream::<f32>(&device, config, Arc::clone(shared), error_callback)?
        }
        SampleFormat::I16 => {
            build_stream::<i16>(&device, config, Arc::clone(shared), error_callback)?
        }
        SampleFormat::U16 => {
            build_stream::<u16>(&device, config, Arc::clone(shared), error_callback)?
        }
        other => {
            return Err(Error::Output(format!(
                "unsupported device sample format: {other:?}"
            )))
        }
    };

    stream.play().map_err(|e| Error::Output(e.to_string()))?;
    Ok((stream, info))
}

fn build_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    shared: Arc<OutputShared>,
    error_callback: impl FnMut(cpal::Error) + Send + 'static,
) -> Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let mut scratch: Vec<f32> = Vec::new();
    let cb = Arc::clone(&shared);

    let data_callback = move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
        if !cb.playing.load(Ordering::Relaxed) {
            for sample in data.iter_mut() {
                *sample = T::from_sample_(0.0);
            }
            return;
        }

        let volume = f32::from_bits(cb.volume.load(Ordering::Relaxed));
        let balance = f32::from_bits(cb.balance.load(Ordering::Relaxed));
        // Constant-power-ish pan law: full one side fades the other out.
        let gain_l = volume * (1.0 - balance.max(0.0));
        let gain_r = volume * (1.0 + balance.min(0.0));

        scratch.clear();
        scratch.resize(data.len(), 0.0);
        let popped = cb.ring.pop_into(&mut scratch);
        if popped > 0 {
            cb.consumed.fetch_add(popped as u64, Ordering::Release);
        }

        for (i, sample) in scratch.iter().enumerate() {
            let gain = if i % 2 == 0 { gain_l } else { gain_r };
            data[i] = T::from_sample_(sample * gain);
        }
    };

    device
        .build_output_stream(config, data_callback, error_callback, None)
        .map_err(|e| Error::Output(e.to_string()))
}
