//! Symphonia-backed decoding of one local file into stereo f32 frames.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::Time;
use symphonia::default::{get_codecs, get_probe};

use crate::error::{Error, Result};
use crate::http_stream::{HttpStream, IcyState};

/// One decoded chunk or the end of the track.
pub enum DecodeOutcome {
    /// Interleaved samples at the file's native rate/channels.
    Frame {
        samples: Vec<f32>,
        rate: u32,
        channels: u16,
    },
    EndOfTrack,
}

/// A single open file or network stream: demuxer + decoder + track identity.
pub struct TrackDecoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    /// Native sample rate of the media.
    pub sample_rate: u32,
    /// Native channel count of the media.
    pub channels: u16,
    /// Present for ICY network streams; engine watches it for title updates.
    pub icy: Option<Arc<IcyState>>,
}

impl TrackDecoder {
    pub fn open(path: &Path) -> Result<Self> {
        let file = std::fs::File::open(path)?;
        let mut me =
            Self::open_source(Box::new(file), path_hint(path), path.display().to_string())?;
        me.icy = None;
        Ok(me)
    }

    /// Open a network stream (internet radio). `state` is shared so the
    /// engine can observe live ICY title updates.
    pub fn open_http(url: &str, state: Arc<IcyState>) -> Result<Self> {
        let source = HttpStream::open_with_state(url, Arc::clone(&state))?;
        let mut me = Self::open_source(Box::new(source), http_hint(url), url.to_string())?;
        me.icy = Some(state);
        Ok(me)
    }

    fn open_source(source: Box<dyn MediaSource>, hint: Hint, label: String) -> Result<Self> {
        let mss = MediaSourceStream::new(source, Default::default());

        let format = get_probe()
            .probe(
                &hint,
                mss,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(|e| Error::Decode(format!("{label}: {e}")))?;

        let track = format
            .first_track_known_codec(TrackType::Audio)
            .ok_or_else(|| Error::Decode(format!("{label}: no audio track")))?;

        let track_id = track.id;
        let params = track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .cloned()
            .ok_or_else(|| Error::Decode(format!("{label}: missing audio parameters")))?;

        let decoder = get_codecs()
            .make_audio_decoder(&params, &AudioDecoderOptions::default())
            .map_err(|e| Error::Decode(format!("{label}: {e}")))?;

        let sample_rate = params.sample_rate.unwrap_or(44_100);
        let channels = params.channels.map(|c| c.count() as u16).unwrap_or(2);

        Ok(Self {
            format,
            decoder,
            track_id,
            sample_rate,
            channels,
            icy: None,
        })
    }

    /// Decode the next packet. Corrupt packets are skipped; end of media
    /// yields [`DecodeOutcome::EndOfTrack`].
    pub fn decode_packet(&mut self) -> Result<DecodeOutcome> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(Some(packet)) => packet,
                // Clean end of media.
                Ok(None) => return Ok(DecodeOutcome::EndOfTrack),
                Err(SymError::ResetRequired) => {
                    self.recreate_decoder()?;
                    continue;
                }
                Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(DecodeOutcome::EndOfTrack);
                }
                Err(e) => return Err(Error::Decode(e.to_string())),
            };

            if packet.track_id != self.track_id {
                continue;
            }

            match self.decoder.decode(&packet) {
                Ok(buf) => {
                    let rate = buf.spec().rate();
                    let channels = buf.spec().channels().count() as u16;
                    let frames = buf.frames();
                    let mut samples = Vec::with_capacity(frames * channels as usize);
                    buf.copy_to_vec_interleaved(&mut samples);
                    if samples.is_empty() {
                        continue;
                    }
                    return Ok(DecodeOutcome::Frame {
                        samples,
                        rate,
                        channels,
                    });
                }
                // Per symphonia docs: packet is undecodeable, skip it.
                Err(SymError::DecodeError(_)) | Err(SymError::IoError(_)) => continue,
                Err(SymError::ResetRequired) => {
                    self.recreate_decoder()?;
                    continue;
                }
                Err(e) => return Err(Error::Decode(e.to_string())),
            }
        }
    }

    /// Seek to an absolute position. The decoder is reset afterwards as
    /// required by symphonia ("after a seek, decoders must be reset").
    pub fn seek(&mut self, pos: Duration) -> Result<()> {
        let time = Time::try_from_secs_f64(pos.as_secs_f64())
            .ok_or_else(|| Error::Decode("invalid seek position".into()))?;
        self.format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: Some(self.track_id),
                },
            )
            .map_err(|e| Error::Decode(format!("seek failed: {e}")))?;
        self.decoder.reset();
        Ok(())
    }

    /// Re-examine the track list and build a fresh decoder (symphonia
    /// signals `ResetRequired` when stream parameters change).
    fn recreate_decoder(&mut self) -> Result<()> {
        let params = {
            let track = self
                .format
                .tracks()
                .iter()
                .find(|t| t.id == self.track_id)
                .ok_or_else(|| Error::NotFound("audio track".into()))?;
            track
                .codec_params
                .as_ref()
                .and_then(|p| p.audio())
                .cloned()
                .ok_or_else(|| Error::Decode("audio parameters unavailable".into()))?
        };
        self.decoder = get_codecs()
            .make_audio_decoder(&params, &AudioDecoderOptions::default())
            .map_err(|e| Error::Decode(e.to_string()))?;
        if let Some(rate) = params.sample_rate {
            self.sample_rate = rate;
        }
        if let Some(channels) = params.channels {
            self.channels = channels.count() as u16;
        }
        Ok(())
    }
}

fn path_hint(path: &Path) -> Hint {
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    hint
}

fn http_hint(url_str: &str) -> Hint {
    let mut hint = Hint::new();
    // Path extension of the URL, e.g. ".mp3" for /stream.mp3 (query stripped).
    let path = url_str.split(['?', '#']).next().unwrap_or(url_str);
    if let Some(ext) = path
        .rsplit('/')
        .next()
        .and_then(|file| file.rsplit_once('.'))
        .map(|(_, e)| e)
        .filter(|e| !e.is_empty() && e.len() <= 8)
    {
        hint.with_extension(ext);
    }
    hint
}
