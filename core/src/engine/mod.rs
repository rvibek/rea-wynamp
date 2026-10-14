//! The playback engine.
//!
//! One dedicated thread owns the whole pipeline:
//!
//! ```text
//! PlayerCommand ──► [engine thread] ──► PlayerEvent
//!                      │
//!        decode (Symphonia) → resample → PCM ring → cpal callback ─► speakers
//! ```
//!
//! The UI never blocks: commands and events travel over channels. Gapless
//! playback works because the next track is decoded into the ring *before*
//! the current one is audibly finished — the callback never sees a gap.
//!
//! When no audio device exists (CI, headless), the engine "consumes" the ring
//! itself at the output rate so tests can observe the full event stream.

pub mod decoder;
pub mod eq;
pub mod output;
pub mod queue;
pub mod resample;
pub mod ring;
pub mod visual;

use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::error::{Error, Result};
use crate::models::{normalize_volume, PlayableItem, PlaybackState, PlayerStatus, PlaylistEntry};
use crate::playback::{PlayerCommand, PlayerEvent};

use decoder::{DecodeOutcome, TrackDecoder};
use eq::EqChain;
use output::{open_output, OutputShared};
use queue::{remap_index_after_move, QueueState};
use resample::{to_stereo, LinearResampler};
use ring::PcmRing;
use visual::VisualTap;

/// Ring size basis: sized for a 96 kHz device so any real rate gets
/// ≥ `ring_seconds` of headroom (the guard that makes gapless switching safe).
const RING_BASIS_RATE: f32 = 96_000.0;

const STATUS_INTERVAL: Duration = Duration::from_millis(250);
const TICK: Duration = Duration::from_millis(10);
/// Don't decode when fewer than this many stereo samples of room remain.
const DECODE_HEADROOM: usize = 8_192;

/// Pending auto-reconnect of a dropped radio stream.
struct Reconnect {
    idx: usize,
    attempts: u32,
    at: Instant,
}

const MAX_RECONNECT_ATTEMPTS: u32 = 5;
const RECONNECT_DELAY: Duration = Duration::from_secs(2);

/// Engine configuration.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Attempt to open a real audio device. `false` = headless (tests/CI).
    pub enable_output: bool,
    /// Ring buffer size in seconds (at the 96 kHz basis rate).
    pub ring_seconds: f32,
    /// Auto-reconnect dropped internet-radio streams (up to 5 attempts).
    pub reconnect_streams: bool,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            enable_output: true,
            ring_seconds: 0.5,
            reconnect_streams: true,
        }
    }
}

/// UI-side handle: send commands, receive events.
pub struct EngineHandle {
    tx: Sender<PlayerCommand>,
    pub events: Receiver<PlayerEvent>,
    /// Live PCM tap for the visualizer (spectrum/scope).
    pub visual: Arc<VisualTap>,
}

impl EngineHandle {
    pub fn send(&self, cmd: PlayerCommand) -> Result<()> {
        self.tx
            .send(cmd)
            .map_err(|_| Error::Output("playback engine has stopped".into()))
    }

    /// Ask the engine to exit (idempotent).
    pub fn shutdown(&self) {
        let _ = self.tx.send(PlayerCommand::Shutdown);
    }

    /// Split into the command sender (UI → engine) and event receiver
    /// (engine → UI), e.g. to move the receiver into a forwarding thread.
    pub fn into_parts(self) -> (Sender<PlayerCommand>, Receiver<PlayerEvent>) {
        (self.tx, self.events)
    }
}

/// Spawn the engine thread and return its handle.
pub fn spawn(config: EngineConfig) -> Result<EngineHandle> {
    let (cmd_tx, cmd_rx) = channel::<PlayerCommand>();
    let (evt_tx, evt_rx) = channel::<PlayerEvent>();
    let visual = Arc::new(VisualTap::new());
    let visual_in_engine = Arc::clone(&visual);

    thread::Builder::new()
        .name("wynamp-engine".into())
        .spawn(move || {
            let mut engine = Engine::new(config, cmd_rx, evt_tx, visual_in_engine);
            engine.run();
        })
        .map_err(|e| Error::Output(format!("failed to spawn engine thread: {e}")))?;

    Ok(EngineHandle {
        tx: cmd_tx,
        events: evt_rx,
        visual,
    })
}

/// Audible timeline of one track (or one segment of it after a seek).
struct TrackMarker {
    queue_idx: usize,
    entry: PlaylistEntry,
    /// `consumed` value at which this track's first sample becomes audible.
    audible_start: u64,
    /// Position offset (output-rate frames) — non-zero after a mid-track seek.
    base_frames: u64,
    /// Output-rate frames pushed for this marker (final once sealed).
    total_frames: u64,
    /// True once the decoder moved past this track (total is final).
    sealed: bool,
    started: bool,
}

struct Engine {
    cmd_rx: Receiver<PlayerCommand>,
    event_tx: Sender<PlayerEvent>,
    running: bool,

    shared: Arc<OutputShared>,
    stream: Option<cpal::Stream>,
    out_rate: u32,

    queue: QueueState,
    eq: EqChain,
    visual: Arc<VisualTap>,
    decoder: Option<TrackDecoder>,
    decode_index: Option<usize>,
    resampler: Option<LinearResampler>,
    /// Decoded samples waiting for ring space (partial pushes never drop audio).
    pending: VecDeque<f32>,
    markers: VecDeque<TrackMarker>,
    /// Attributes for the next marker created (reset after use).
    next_marker_base: Duration,
    next_marker_started: bool,

    status: PlayerStatus,
    last_status_at: Instant,
    output_error_reported: bool,
    reconnect_streams: bool,
    reconnect: Option<Reconnect>,
    icy_generation: u64,
}

impl Engine {
    fn new(
        config: EngineConfig,
        cmd_rx: Receiver<PlayerCommand>,
        event_tx: Sender<PlayerEvent>,
        visual: Arc<VisualTap>,
    ) -> Self {
        let capacity = (RING_BASIS_RATE * config.ring_seconds) as usize * 2;
        let shared = Arc::new(OutputShared::new(Arc::new(PcmRing::new(capacity))));

        let mut out_rate = 48_000;
        let mut stream = None;
        if config.enable_output {
            match open_output(&shared) {
                Ok((s, info)) => {
                    stream = Some(s);
                    out_rate = info.sample_rate;
                }
                Err(e) => {
                    let _ = event_tx.send(PlayerEvent::Warning {
                        message: format!("Audio output unavailable ({e}). Running silently."),
                    });
                }
            }
        }

        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() ^ (u64::from(d.subsec_nanos()) << 20))
            .unwrap_or(0x2545_F491_4F6C_DD1D);

        let eq = EqChain::new(out_rate);

        Self {
            cmd_rx,
            event_tx,
            running: true,
            shared,
            stream,
            out_rate,
            queue: QueueState::new(seed),
            eq,
            visual,
            decoder: None,
            decode_index: None,
            resampler: None,
            pending: VecDeque::new(),
            markers: VecDeque::new(),
            next_marker_base: Duration::ZERO,
            next_marker_started: false,
            status: PlayerStatus::default(),
            last_status_at: Instant::now(),
            output_error_reported: false,
            reconnect_streams: config.reconnect_streams,
            reconnect: None,
            icy_generation: 0,
        }
    }

    fn run(&mut self) {
        'outer: loop {
            self.pump_decode();
            self.pump_virtual_consumption();
            if !self.tick() {
                break 'outer;
            }
            self.pump_reconnects();

            match self.cmd_rx.recv_timeout(TICK) {
                Ok(cmd) => {
                    if !self.handle_command(cmd) {
                        break 'outer;
                    }
                    while let Ok(cmd) = self.cmd_rx.try_recv() {
                        if !self.handle_command(cmd) {
                            break 'outer;
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break 'outer,
            }
        }

        self.shared.playing.store(false, Ordering::Release);
        self.stream.take();
    }

    /// Fire due radio reconnects.
    fn pump_reconnects(&mut self) {
        if let Some(r) = &self.reconnect {
            if Instant::now() >= r.at {
                let (idx, attempts) = (r.idx, r.attempts);
                self.reconnect = None;
                self.fire_reconnect(idx, attempts);
            }
        }
    }

    // ── Commands ────────────────────────────────────────────────────────────

    /// Returns `false` when the engine should shut down.
    fn handle_command(&mut self, cmd: PlayerCommand) -> bool {
        match cmd {
            PlayerCommand::Shutdown => {
                self.stop_playback();
                return false;
            }

            PlayerCommand::PlayItem(item) => {
                let entry = PlaylistEntry::new(item);
                self.queue.replace(vec![entry], Some(0));
                self.start_track(0, Duration::ZERO);
                self.emit_queue_changed();
                self.emit_status();
            }

            PlayerCommand::ReplaceQueue { entries, start_at } => {
                let start = self.queue.replace(entries, start_at);
                match start {
                    Some(idx) => self.start_track(idx, Duration::ZERO),
                    None => self.stop_playback(),
                }
                self.emit_queue_changed();
                self.emit_status();
            }

            PlayerCommand::Enqueue(items) => {
                self.queue.enqueue(items);
                self.emit_queue_changed();
            }

            PlayerCommand::RemoveAt(index) => {
                let audible = self.markers.front().map(|m| m.queue_idx) == Some(index)
                    || self.decode_index == Some(index);
                if audible {
                    self.stop_playback();
                }
                if self.queue.remove(index).is_some() {
                    if !audible {
                        for m in self.markers.iter_mut() {
                            if m.queue_idx > index {
                                m.queue_idx -= 1;
                            }
                        }
                        self.decode_index =
                            self.decode_index.map(|d| if d > index { d - 1 } else { d });
                    }
                    self.emit_queue_changed();
                    self.emit_status();
                }
            }

            PlayerCommand::MoveEntry { from, to } => {
                if self.queue.move_entry(from, to) {
                    for m in self.markers.iter_mut() {
                        m.queue_idx = remap_index_after_move(m.queue_idx, from, to);
                    }
                    self.decode_index = self
                        .decode_index
                        .map(|d| remap_index_after_move(d, from, to));
                    self.emit_queue_changed();
                }
            }

            PlayerCommand::ClearQueue => {
                self.queue.clear();
                self.stop_playback();
                self.emit_queue_changed();
            }

            PlayerCommand::Play => match self.status.state {
                PlaybackState::Paused => self.resume(),
                PlaybackState::Stopped => {
                    let idx = self
                        .queue
                        .current()
                        .or_else(|| (!self.queue.entries().is_empty()).then_some(0));
                    if let Some(idx) = idx {
                        self.start_track(idx, Duration::ZERO);
                        self.emit_status();
                    }
                }
                PlaybackState::Playing | PlaybackState::Buffering => {}
            },

            PlayerCommand::Pause => self.pause(),
            PlayerCommand::Toggle => match self.status.state {
                PlaybackState::Playing | PlaybackState::Buffering => self.pause(),
                PlaybackState::Paused => self.resume(),
                PlaybackState::Stopped => {
                    if let Some(idx) = self
                        .queue
                        .current()
                        .or_else(|| (!self.queue.entries().is_empty()).then_some(0))
                    {
                        self.start_track(idx, Duration::ZERO);
                        self.emit_status();
                    }
                }
            },

            PlayerCommand::Stop => {
                self.stop_playback();
            }

            PlayerCommand::Next => match self.queue.next_manual() {
                Some(idx) => {
                    self.start_track(idx, Duration::ZERO);
                    self.emit_status();
                }
                None => self.stop_playback(),
            },

            PlayerCommand::Previous => {
                // Classic behavior: restart the track if we're past 3 seconds.
                if self.status.position > Duration::from_secs(3) {
                    self.seek_to(Duration::ZERO);
                } else {
                    match self.queue.prev_manual() {
                        Some(idx) => {
                            self.start_track(idx, Duration::ZERO);
                        }
                        None => {
                            if let Some(idx) = self.markers.front().map(|m| m.queue_idx) {
                                self.start_track(idx, Duration::ZERO);
                            } else {
                                self.stop_playback();
                            }
                        }
                    }
                    self.emit_status();
                }
            }

            PlayerCommand::Seek(pos) => self.seek_to(pos),

            PlayerCommand::SeekRelative(delta_ms) => {
                let base = self.status.position;
                let target = if delta_ms >= 0 {
                    base.saturating_add(Duration::from_millis(delta_ms as u64))
                } else {
                    base.saturating_sub(Duration::from_millis(delta_ms.unsigned_abs()))
                };
                let target = match self.status.duration {
                    Some(d) if target > d => d,
                    _ => target,
                };
                self.seek_to(target);
            }

            PlayerCommand::SetVolume(v) => {
                let v = normalize_volume(v);
                self.status.volume = v;
                self.shared.volume.store(v.to_bits(), Ordering::Release);
                self.emit_status();
            }

            PlayerCommand::VolumeRelative(delta) => {
                let v = normalize_volume(self.status.volume + delta);
                self.status.volume = v;
                self.shared.volume.store(v.to_bits(), Ordering::Release);
                self.emit_status();
            }

            PlayerCommand::SetBalance(b) => {
                self.status.set_balance(b);
                self.shared
                    .balance
                    .store(self.status.balance.to_bits(), Ordering::Release);
                self.emit_status();
            }

            PlayerCommand::SetShuffle(s) => {
                self.queue.set_shuffle(s);
                self.status.shuffle = s;
                self.emit_status();
            }

            PlayerCommand::SetRepeat(r) => {
                self.queue.set_repeat(r);
                self.status.repeat = r;
                self.emit_status();
            }

            PlayerCommand::SetEqEnabled(on) => {
                self.eq.set_enabled(on);
                self.sync_eq_status();
                self.emit_status();
            }

            PlayerCommand::SetEqGains(gains) => {
                if !self.eq.set_gains_db(&gains) {
                    self.warn("Equalizer needs exactly 10 band gains".into());
                }
                self.sync_eq_status();
                self.emit_status();
            }

            PlayerCommand::SetPreamp(db) => {
                self.eq.set_preamp_db(db);
                self.sync_eq_status();
                self.emit_status();
            }

            PlayerCommand::QueryStatus => self.emit_status(),
        }
        true
    }

    // ── Playback control ────────────────────────────────────────────────────

    /// Hard-cut to `idx` at `pos` (flushes the audible state first).
    /// Unopenable files are skipped (bounded), matching Winamp behavior.
    fn start_track(&mut self, idx: usize, pos: Duration) {
        self.reconnect = None;
        self.decoder = None;
        self.resampler = None;
        self.pending.clear();
        self.markers.clear();
        let _ = self.shared.ring.clear();
        self.next_marker_base = Duration::ZERO;
        self.next_marker_started = false;

        let len = self.queue.entries().len();
        let mut attempt = Some((idx, pos));
        let mut tries = 0usize;

        while let Some((i, at)) = attempt {
            tries += 1;
            match self.open_decoder_at(i, at) {
                Ok(()) => {
                    self.queue.set_current(Some(i));
                    self.shared.playing.store(true, Ordering::Release);
                    self.status.state = PlaybackState::Playing;
                    self.status.position = at;
                    self.status.current = self.queue.entry(i).cloned();
                    return;
                }
                Err(e) => {
                    self.warn(format!(
                        "Cannot play {}: {e}",
                        self.queue
                            .entry(i)
                            .map(|e| e.display_title())
                            .unwrap_or_default()
                    ));
                    attempt = if tries > len {
                        None
                    } else {
                        self.queue
                            .advance_from(i)
                            .map(|next| (next, Duration::ZERO))
                    };
                }
            }
        }
        self.stop_playback();
    }

    fn pause(&mut self) {
        if self.status.state == PlaybackState::Playing
            || self.status.state == PlaybackState::Buffering
        {
            self.shared.playing.store(false, Ordering::Release);
            self.status.state = PlaybackState::Paused;
            self.emit_status();
        }
    }

    fn resume(&mut self) {
        if self.status.state == PlaybackState::Paused {
            self.shared.playing.store(true, Ordering::Release);
            self.status.state = PlaybackState::Playing;
            self.emit_status();
        }
    }

    fn stop_playback(&mut self) {
        self.reconnect = None;
        self.decoder = None;
        self.decode_index = None;
        self.resampler = None;
        self.pending.clear();
        self.markers.clear();
        let _ = self.shared.ring.clear();
        self.shared.playing.store(false, Ordering::Release);

        self.status.state = PlaybackState::Stopped;
        self.status.current = None;
        self.status.position = Duration::ZERO;
        self.status.duration = None;
        self.status.sample_rate_hz = None;
        self.status.bitrate_kbps = None;
        self.emit_status();
    }

    fn seek_to(&mut self, pos: Duration) {
        let Some(idx) = self
            .markers
            .front()
            .map(|m| m.queue_idx)
            .or(self.decode_index)
        else {
            return;
        };

        // Get the decoder onto the target track and position BEFORE touching
        // audible state, so a failed seek leaves everything intact.
        if self.decode_index != Some(idx) {
            self.decoder = None;
            self.resampler = None;
            self.pending.clear();
            if let Err(e) = self.open_decoder_at(idx, pos) {
                self.warn(format!("Seek failed: {e}"));
                self.stop_playback();
                return;
            }
            self.queue.set_current(Some(idx));
        } else if let Some(dec) = self.decoder.as_mut() {
            if let Err(e) = dec.seek(pos) {
                self.warn(format!("Seek failed: {e}"));
                return;
            }
            self.resampler = if dec.sample_rate != self.out_rate {
                Some(LinearResampler::new(dec.sample_rate, self.out_rate))
            } else {
                None
            };
        }

        let _ = self.shared.ring.clear();
        self.pending.clear();
        self.markers.clear();
        self.next_marker_base = pos;
        self.next_marker_started = true;

        self.status.position = pos;
        if let Some(entry) = self.queue.entry(idx) {
            self.status.current = Some(entry.clone());
        }
        self.emit_status();
    }

    /// Open the file (or URL) at `queue[i]` and stage it as the active decoder.
    fn open_decoder_at(&mut self, idx: usize, pos: Duration) -> Result<()> {
        let entry = self
            .queue
            .entry(idx)
            .ok_or_else(|| Error::NotFound("queue entry".into()))?;
        let mut decoder = match &entry.item {
            PlayableItem::LocalFile { path } => TrackDecoder::open(path)?,
            PlayableItem::RadioStream { url, .. } => {
                self.status.state = PlaybackState::Buffering;
                TrackDecoder::open_http(
                    url,
                    std::sync::Arc::new(crate::http_stream::IcyState::new()),
                )?
            }
            _ => {
                return Err(Error::Unsupported(
                    "this source is not supported yet".into(),
                ))
            }
        };

        if pos > Duration::ZERO {
            decoder.seek(pos)?;
        }

        self.resampler = if decoder.sample_rate != self.out_rate {
            Some(LinearResampler::new(decoder.sample_rate, self.out_rate))
        } else {
            None
        };
        self.status.sample_rate_hz = Some(decoder.sample_rate);
        self.status.bitrate_kbps = None;
        self.status.duration = None;
        self.icy_generation = 0;

        self.decode_index = Some(idx);
        self.decoder = Some(decoder);
        Ok(())
    }

    /// The current decode session ended (EOF or fatal decode error): seal its
    /// marker and either reconnect (radio) or open the next track *without*
    /// flushing the ring — that is what makes local transitions gapless.
    fn end_current_track(&mut self) {
        let ended_idx = self.decode_index;
        let ended_is_radio = self
            .markers
            .back()
            .is_some_and(|m| matches!(m.entry.item, PlayableItem::RadioStream { .. }));

        if let Some(m) = self.markers.back_mut() {
            if !m.sealed {
                m.sealed = true;
            }
        }
        self.decoder = None;
        self.resampler = None;
        self.pending.clear();
        self.decode_index = None;

        // Internet radio "ends" only when the connection drops — reconnect.
        if self.reconnect_streams && ended_is_radio {
            if let Some(idx) = ended_idx {
                let attempts = match &self.reconnect {
                    Some(r) if r.idx == idx => r.attempts + 1,
                    _ => 1,
                };
                self.schedule_reconnect(idx, attempts);
                return;
            }
        }

        let Some(next) = self.queue.natural_end() else {
            return; // End of queue: ring drains, tick() stops playback.
        };

        let len = self.queue.entries().len();
        let mut attempt = Some(next);
        let mut tries = 0usize;
        while let Some(i) = attempt {
            tries += 1;
            match self.open_decoder_at(i, Duration::ZERO) {
                Ok(()) => {
                    self.queue.set_current(Some(i));
                    return;
                }
                Err(e) => {
                    self.warn(format!(
                        "Skipping {}: {e}",
                        self.queue
                            .entry(i)
                            .map(|e| e.display_title())
                            .unwrap_or_default()
                    ));
                    attempt = if tries > len {
                        None
                    } else {
                        self.queue.advance_from(i)
                    };
                }
            }
        }
        // Nothing playable left; drain and stop.
    }

    /// Enter the buffering state and arm a reconnect attempt.
    fn schedule_reconnect(&mut self, idx: usize, attempts: u32) {
        if attempts > MAX_RECONNECT_ATTEMPTS {
            self.warn(format!(
                "Stream unreachable after {MAX_RECONNECT_ATTEMPTS} attempts"
            ));
            self.reconnect = None;
            self.stop_playback();
            return;
        }
        self.reconnect = Some(Reconnect {
            idx,
            attempts,
            at: Instant::now() + RECONNECT_DELAY,
        });
        self.shared.playing.store(false, Ordering::Release);
        self.status.state = PlaybackState::Buffering;
        self.status.position = Duration::ZERO;
        self.emit_status();
    }

    /// Try to reopen a radio stream; failures re-arm the reconnect.
    fn fire_reconnect(&mut self, idx: usize, attempts: u32) {
        self.decoder = None;
        self.resampler = None;
        self.pending.clear();
        self.markers.clear();
        let _ = self.shared.ring.clear();

        match self.open_decoder_at(idx, Duration::ZERO) {
            Ok(()) => {
                self.queue.set_current(Some(idx));
                self.shared.playing.store(true, Ordering::Release);
                self.status.state = PlaybackState::Playing;
                self.status.current = self.queue.entry(idx).cloned();
                self.emit_status();
            }
            Err(e) => {
                self.warn(format!("Reconnect failed: {e}"));
                self.schedule_reconnect(idx, attempts + 1);
            }
        }
    }

    // ── Decode pump ─────────────────────────────────────────────────────────

    fn pump_decode(&mut self) {
        loop {
            // 1. Move pending samples into the ring first — never drop audio.
            if !self.pending.is_empty() {
                self.shared.ring.push_from_front(&mut self.pending);
                if !self.pending.is_empty() {
                    return; // ring full — try again next tick
                }
                continue;
            }

            // 2. Need a decoder and enough room for at least a packet.
            if self.decoder.is_none() {
                return;
            }
            if self
                .shared
                .ring
                .capacity()
                .saturating_sub(self.shared.ring.len())
                < DECODE_HEADROOM
            {
                return;
            }

            // 3. Decode one packet.
            let outcome = {
                let dec = self.decoder.as_mut().expect("checked above");
                dec.decode_packet()
            };

            match outcome {
                Ok(DecodeOutcome::Frame {
                    samples,
                    rate: _,
                    channels,
                }) => {
                    let stereo = match to_stereo(&samples, channels) {
                        Ok(s) => s,
                        Err(e) => {
                            self.warn(format!("Decode error: {e}"));
                            self.end_current_track();
                            continue;
                        }
                    };
                    let converted = match self.resampler.as_mut() {
                        Some(r) => r.process(&stereo),
                        None => stereo,
                    };
                    if converted.is_empty() {
                        continue;
                    }
                    let mut converted = converted;
                    // EQ at the output rate, then tap the visualizer.
                    self.eq.process(&mut converted);
                    self.visual.push(&converted);
                    self.ensure_marker();
                    if let Some(m) = self.markers.back_mut() {
                        m.total_frames += (converted.len() / 2) as u64;
                    }
                    self.pending.extend(converted);
                }

                Ok(DecodeOutcome::EndOfTrack) => {
                    self.end_current_track();
                    if self.decoder.is_none() {
                        return;
                    }
                }

                Err(e) => {
                    self.warn(format!("Decode error: {e}"));
                    self.end_current_track();
                    if self.decoder.is_none() {
                        return;
                    }
                }
            }
        }
    }

    /// Create the audible marker for the active decode session if it doesn't
    /// exist yet (first push after a switch/seek).
    fn ensure_marker(&mut self) {
        let Some(idx) = self.decode_index else {
            return;
        };
        let fresh_needed = match self.markers.back() {
            Some(m) => m.queue_idx != idx || m.sealed,
            None => true,
        };
        if !fresh_needed {
            return;
        }
        let Some(entry) = self.queue.entry(idx).cloned() else {
            return;
        };

        let audible_start =
            self.shared.consumed.load(Ordering::Acquire) + self.shared.ring.len() as u64;
        let base_frames = (self.next_marker_base.as_secs_f64() * f64::from(self.out_rate)) as u64;
        let started = self.next_marker_started;
        self.next_marker_base = Duration::ZERO;
        self.next_marker_started = false;

        self.markers.push_back(TrackMarker {
            queue_idx: idx,
            entry,
            audible_start,
            base_frames,
            total_frames: 0,
            sealed: false,
            started,
        });
    }

    /// Headless mode: advance `consumed` in real time so markers, events,
    /// and position math behave exactly as with a real device.
    fn pump_virtual_consumption(&mut self) {
        if self.stream.is_some() || !self.shared.playing.load(Ordering::Acquire) {
            return;
        }
        let samples_per_tick = (self.out_rate as usize / 100).max(1) * 2;
        let discarded = self.shared.ring.pop_discard(samples_per_tick);
        if discarded > 0 {
            self.shared
                .consumed
                .fetch_add(discarded as u64, Ordering::Release);
        }
    }

    // ── Events / status ─────────────────────────────────────────────────────

    /// Fire start/finish events, update position, detect natural stop.
    /// Returns `false` when the event channel is gone (UI dropped).
    fn tick(&mut self) -> bool {
        let c = self.shared.consumed.load(Ordering::Acquire);

        // Tracks that became audible now.
        let mut started: Option<PlaylistEntry> = None;
        for m in self.markers.iter_mut() {
            if !m.started && c >= m.audible_start {
                m.started = true;
                started = Some(m.entry.clone());
            }
        }
        if let Some(entry) = started {
            self.status.current = Some(entry.clone());
            if !self.emit(PlayerEvent::TrackStarted(entry)) {
                return false;
            }
        }

        // Tracks whose audible playback finished.
        while let Some(front) = self.markers.front() {
            if front.sealed && front.started && c >= front.audible_start + front.total_frames {
                let done = self.markers.pop_front().expect("front exists");
                if !self.emit(PlayerEvent::TrackFinished(done.entry)) {
                    return false;
                }
            } else {
                break;
            }
        }

        // Everything played out → stop (queue retained).
        if self.status.state == PlaybackState::Playing
            && self.decoder.is_none()
            && self.markers.is_empty()
        {
            self.stop_playback();
            return self.running;
        }

        // Position from the front (audible) marker.
        if let Some(front) = self.markers.front() {
            if front.started {
                let played = c.saturating_sub(front.audible_start);
                let played = if front.sealed {
                    played.min(front.total_frames)
                } else {
                    played
                };
                let frames = front.base_frames + played;
                self.status.position =
                    Duration::from_secs_f64(frames as f64 / f64::from(self.out_rate));
                if front.sealed {
                    let total = front.base_frames + front.total_frames;
                    self.status.duration = Some(Duration::from_secs_f64(
                        total as f64 / f64::from(self.out_rate),
                    ));
                }
            }
        }

        // Periodic live status while playing.
        if self.status.state == PlaybackState::Playing
            && self.last_status_at.elapsed() >= STATUS_INTERVAL
        {
            self.last_status_at = Instant::now();
            if !self.emit(PlayerEvent::Status(self.status.clone())) {
                return false;
            }
        }

        // Surface a device error at most once.
        if self.shared.output_error.swap(false, Ordering::AcqRel) && !self.output_error_reported {
            self.output_error_reported = true;
            if !self.emit(PlayerEvent::Warning {
                message: "The audio device reported an error; playback may have stopped.".into(),
            }) {
                return false;
            }
        }

        // ICY "now playing" title updates from the active radio stream.
        if let Some(dec) = &self.decoder {
            if let Some(icy) = &dec.icy {
                let gen = icy.generation();
                if gen > self.icy_generation {
                    self.icy_generation = gen;
                    let title = icy.title();
                    if !title.is_empty() && !self.emit(PlayerEvent::StreamTitle { title }) {
                        return false;
                    }
                }
            }
        }

        true
    }

    fn emit(&mut self, event: PlayerEvent) -> bool {
        if self.event_tx.send(event).is_ok() {
            true
        } else {
            self.running = false;
            false
        }
    }

    fn emit_status(&mut self) {
        self.last_status_at = Instant::now();
        let _ = self.emit(PlayerEvent::Status(self.status.clone()));
    }

    fn sync_eq_status(&mut self) {
        self.status.eq_enabled = self.eq.enabled();
        self.status.eq_gains_db = self.eq.gains_db();
        self.status.preamp_db = self.eq.preamp_db();
    }

    fn emit_queue_changed(&mut self) {
        let entries = self.queue.entries().to_vec();
        let _ = self.emit(PlayerEvent::QueueChanged(entries));
    }

    fn warn(&mut self, message: String) {
        let _ = self.emit(PlayerEvent::Warning { message });
    }
}
