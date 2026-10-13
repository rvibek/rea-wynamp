//! Headless engine integration test: real decode → ring → event stream.
//!
//! Runs with `enable_output: false` so it works on CI machines without an
//! audio device. The engine consumes the ring itself at the output rate,
//! producing the same event sequence a real device would.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rea_wynamp_core::engine::{spawn, EngineConfig};
use rea_wynamp_core::{PlayableItem, PlayerCommand, PlayerEvent, PlaylistEntry};

/// Write a mono 16-bit PCM WAV with a 440 Hz sine.
fn write_wav(path: &Path, seconds: f32, rate: u32) -> std::io::Result<()> {
    let frames = (seconds * rate as f32) as u32;
    let data_bytes = frames * 2;
    let mut buf = Vec::with_capacity(44 + data_bytes as usize);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
    buf.extend_from_slice(&1u16.to_le_bytes()); // mono
    buf.extend_from_slice(&rate.to_le_bytes());
    buf.extend_from_slice(&(rate * 2).to_le_bytes()); // byte rate
    buf.extend_from_slice(&2u16.to_le_bytes()); // block align
    buf.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_bytes.to_le_bytes());
    for i in 0..frames {
        let phase = i as f32 / rate as f32 * 440.0 * 2.0 * std::f32::consts::PI;
        let sample = (phase.sin() * 12_000.0) as i16;
        buf.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(path, buf)
}

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "wynamp-engine-test-{}-{}",
        label,
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn entry(path: &Path) -> PlaylistEntry {
    PlaylistEntry::new(PlayableItem::local(path.to_path_buf()))
}

/// Collect events until `done` returns true, the deadline passes, or the
/// engine disconnects.
fn collect_until(
    engine: &rea_wynamp_core::engine::EngineHandle,
    deadline: Duration,
    mut done: impl FnMut(&[PlayerEvent]) -> bool,
) -> Vec<PlayerEvent> {
    let end = Instant::now() + deadline;
    let mut events = Vec::new();
    while Instant::now() < end {
        match engine.events.recv_timeout(Duration::from_millis(200)) {
            Ok(event) => {
                events.push(event);
                if done(&events) {
                    break;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    events
}

fn started_titles(events: &[PlayerEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            PlayerEvent::TrackStarted(entry) => Some(entry.display_title()),
            _ => None,
        })
        .collect()
}

#[test]
fn plays_two_tracks_gapless_and_reports_events() {
    let dir = temp_dir("gapless");
    let a = dir.join("01-first.wav");
    let b = dir.join("02-second.wav");
    write_wav(&a, 0.3, 8_000).unwrap();
    write_wav(&b, 0.3, 8_000).unwrap();

    let engine = spawn(EngineConfig {
        enable_output: false,
        ..EngineConfig::default()
    })
    .expect("spawn engine");

    engine
        .send(PlayerCommand::ReplaceQueue {
            entries: vec![entry(&a), entry(&b)],
            start_at: Some(0),
        })
        .unwrap();

    let events = collect_until(&engine, Duration::from_secs(10), |evs| {
        evs.iter()
            .filter(|e| matches!(e, PlayerEvent::TrackFinished(_)))
            .count()
            == 2
    });

    // Expected sequence: started a, started b, finished a, finished b.
    // (b's TrackStarted coincides with a's audible end; the started loop runs
    // before the finished loop in the same tick.)
    let started = started_titles(&events);
    assert_eq!(started, ["01-first", "02-second"], "events: {events:?}");

    let finished: Vec<String> = events
        .iter()
        .filter_map(|e| match e {
            PlayerEvent::TrackFinished(entry) => Some(entry.display_title()),
            _ => None,
        })
        .collect();
    assert_eq!(finished, ["01-first", "02-second"], "events: {events:?}");

    // QueueChanged was emitted for the replace.
    assert!(
        events
            .iter()
            .any(|e| matches!(e, PlayerEvent::QueueChanged(q) if q.len() == 2)),
        "events: {events:?}"
    );

    // After the last track, the engine stops itself.
    let stop_events = collect_until(&engine, Duration::from_secs(5), |evs| {
        evs.iter().any(|e| {
            matches!(e, PlayerEvent::Status(s)
                if s.state == rea_wynamp_core::PlaybackState::Stopped)
        })
    });
    assert!(
        stop_events
            .iter()
            .any(|e| matches!(e, PlayerEvent::Status(_))),
        "engine should emit a stopped status; got {stop_events:?}"
    );

    engine.shutdown();
}

#[test]
fn pause_and_seek_commands_work_headless() {
    let dir = temp_dir("seek");
    let file = dir.join("long.wav");
    write_wav(&file, 2.0, 44_100).unwrap();

    let engine = spawn(EngineConfig {
        enable_output: false,
        ..EngineConfig::default()
    })
    .unwrap();

    engine
        .send(PlayerCommand::PlayItem(PlayableItem::local(file)))
        .unwrap();

    // Wait for playback to start.
    let start = collect_until(&engine, Duration::from_secs(5), |evs| {
        evs.iter()
            .any(|e| matches!(e, PlayerEvent::TrackStarted(_)))
    });
    assert!(!started_titles(&start).is_empty(), "should start playing");

    engine.send(PlayerCommand::Pause).unwrap();
    engine.send(PlayerCommand::QueryStatus).unwrap();
    let paused = collect_until(&engine, Duration::from_secs(2), |evs| {
        evs.iter()
            .any(|e| matches!(e, PlayerEvent::Status(s) if s.state == rea_wynamp_core::PlaybackState::Paused))
    });
    assert!(
        paused.iter().any(|e| matches!(e, PlayerEvent::Status(s)
            if s.state == rea_wynamp_core::PlaybackState::Paused)),
        "should report paused"
    );

    // Seek while paused: status should report the target position.
    engine
        .send(PlayerCommand::Seek(Duration::from_millis(1200)))
        .unwrap();
    let sought = collect_until(&engine, Duration::from_secs(2), |evs| {
        evs.iter().any(|e| {
            matches!(e, PlayerEvent::Status(s)
                if s.position >= Duration::from_millis(1150)
                    && s.position <= Duration::from_millis(1400))
        })
    });
    assert!(
        sought.iter().any(|e| matches!(e, PlayerEvent::Status(s)
            if s.position >= Duration::from_millis(1150))),
        "seek should move the position to ~1.2s; got {sought:?}"
    );

    // Resume and stop.
    engine.send(PlayerCommand::Toggle).unwrap();
    engine.send(PlayerCommand::Stop).unwrap();
    let stopped = collect_until(&engine, Duration::from_secs(2), |evs| {
        evs.iter()
            .any(|e| matches!(e, PlayerEvent::Status(s) if s.state == rea_wynamp_core::PlaybackState::Stopped))
    });
    assert!(!stopped.is_empty(), "stop should emit status");

    engine.shutdown();
}

#[test]
fn missing_file_emits_warning_and_stops_cleanly() {
    let engine = spawn(EngineConfig {
        enable_output: false,
        ..EngineConfig::default()
    })
    .unwrap();

    engine
        .send(PlayerCommand::PlayItem(PlayableItem::local(
            "/definitely/not/a/real/file.wav",
        )))
        .unwrap();

    let events = collect_until(&engine, Duration::from_secs(5), |evs| {
        evs.iter().any(|e| {
            matches!(e, PlayerEvent::Status(s)
            if s.state == rea_wynamp_core::PlaybackState::Stopped)
        })
    });

    assert!(
        events
            .iter()
            .any(|e| matches!(e, PlayerEvent::Warning { .. })),
        "missing file should warn; got {events:?}"
    );
    assert!(
        events.iter().any(|e| matches!(e, PlayerEvent::Status(s)
            if s.state == rea_wynamp_core::PlaybackState::Stopped)),
        "should end stopped"
    );

    engine.shutdown();
}
