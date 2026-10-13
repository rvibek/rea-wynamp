//! Play an audio file through the real output device.
//!
//! ```sh
//! cargo run -p rea-wynamp-core --example play -- /path/to/song.mp3
//! ```
//!
//! Prints engine events until the track finishes (Ctrl+C to quit early).

use std::time::Duration;

use rea_wynamp_core::engine::{spawn, EngineConfig};
use rea_wynamp_core::{PlayableItem, PlayerCommand, PlayerEvent};

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: play <audio-file>");
        std::process::exit(2);
    };

    let engine = spawn(EngineConfig::default()).expect("spawn engine");
    engine
        .send(PlayerCommand::PlayItem(PlayableItem::local(path)))
        .expect("send play");

    println!("playing… (Ctrl+C to stop)");
    loop {
        match engine.events.recv_timeout(Duration::from_secs(1)) {
            Ok(PlayerEvent::Status(s)) => {
                println!(
                    "  [status] {:?} pos={:.1}s vol={:.2} track={:?}",
                    s.state,
                    s.position.as_secs_f32(),
                    s.volume,
                    s.current.as_ref().map(|c| c.display_title()),
                );
                if s.state == rea_wynamp_core::PlaybackState::Stopped && s.current.is_none() {
                    println!("done.");
                    break;
                }
            }
            Ok(PlayerEvent::TrackStarted(entry)) => {
                println!("  ▶ started: {}", entry.display_title())
            }
            Ok(PlayerEvent::TrackFinished(entry)) => {
                println!("  ■ finished: {}", entry.display_title())
            }
            Ok(PlayerEvent::Warning { message }) => eprintln!("  ! {message}"),
            Ok(PlayerEvent::Fatal { message }) => {
                eprintln!("  ✗ fatal: {message}");
                break;
            }
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    engine.shutdown();
}
