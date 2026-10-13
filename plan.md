You are a senior Rust engineer. Build a cross-platform desktop audio player inspired by the classic Winamp 2.x experience: compact, retro, fast, keyboard-friendly. Use original visual assets and an original app name (do not copy Winamp logos, skins, or trademarks).

The first release targets macOS, but the codebase must be designed from day one to build and run on Windows and Linux with minimal changes. Keep all platform-specific code isolated behind traits/modules.

## Tech stack
- Rust (stable, 2021 edition), Cargo workspace
- UI: Tauri 2 (Rust backend + HTML/CSS/TypeScript frontend, no heavy framework required; Svelte or plain TS is fine) to recreate the retro Winamp-style UI
- Audio decoding: Symphonia (MP3, AAC/M4A, ALAC, FLAC, WAV, AIFF, Ogg Vorbis); Opus via a suitable crate if feasible (explain limitations)
- Audio output: cpal, with our own playback engine (ring buffer, decoding thread, gapless transitions)
- DSP: 10-band equalizer (biquad filters) + preamp; FFT with rustfft for the spectrum visualizer
- Networking: reqwest + tokio (async)
- Storage: SQLite via rusqlite or sqlx for library, playlists, favorites, settings
- Credentials: keyring crate (macOS Keychain, Windows Credential Manager, Linux Secret Service)
- Media controls: souvlaki crate (macOS Now Playing/media keys, Windows SMTC, Linux MPRIS)
- Packaging: Tauri bundler (.app/.dmg for macOS; later .msi/.exe for Windows and .AppImage/.deb for Linux)

## Project structure
Use a Cargo workspace that keeps the core independent of the UI:
- `core/` – pure Rust library: models, playback engine, DSP, playlist parsing, favorites, library scanning (no Tauri dependency)
- `navidrome/` – Subsonic/OpenSubsonic API client
- `radio/` – internet radio + Radio Browser API client
- `platform/` – trait-based abstraction for media controls, file paths, credentials, with per-OS implementations
- `app/` – Tauri app: commands/events bridging core to the frontend
- `ui/` – frontend
Use OS-appropriate data/config directories (the `directories` crate). Never hardcode paths or separators.

## Core UI (Winamp-style)
- Main player window: scrolling track title marquee, elapsed/remaining time, bitrate/sample rate, seek bar, volume, balance, play/pause/stop/prev/next, shuffle, repeat, open file
- Playlist editor: add/remove/reorder (drag & drop), multi-select, search/filter, total duration, save/load playlists
- 10-band graphic equalizer with presets, preamp, on/off
- Spectrum analyzer / oscilloscope visualizer
- Panels can be shown, hidden, and stacked like classic Winamp; compact "mini mode"
- Retro dark theme by default, modern theme option; theme defined via CSS variables so skins can be added later
- Keyboard shortcuts (Z X C V B for prev/play/pause/stop/next, arrows for seek/volume, Space for play/pause)
- Media keys and OS "Now Playing" integration on all platforms via the platform layer
- Drag and drop files/folders onto the window; system tray / menu bar mini controls

## Feature 1: Local audio files
- Open files and folders (recursive scan, runs in background with progress)
- Read metadata and embedded album art (lofty crate)
- Import/export M3U, M3U8, and PLS playlists (handle relative paths and both path separators)
- Gapless playback
- Persist library and handle moved/missing files gracefully

## Feature 2: Navidrome server streaming
- Implement the Subsonic/OpenSubsonic API that Navidrome supports
- Settings screen to add multiple servers (URL, username, password); use salted token auth (t + s params); store secrets via keyring; "test connection" button
- Browse artists, albums, genres, playlists; search; cover art with caching
- Stream tracks with optional max-bitrate/transcoding setting; add them to the playlist editor
- Sync Navidrome playlists (read; write if possible)
- Scrobble plays and sync stars with local favorites
- Handle offline/unreachable servers gracefully

## Feature 3: Internet radio and online music
- Play direct stream URLs (MP3/AAC, HLS if feasible) and PLS/M3U stream links
- Built-in station browser using the Radio Browser API (radio-browser.info): search by name, country, genre, language; show logo, codec, bitrate
- Parse ICY metadata and show "now playing" in the marquee
- Buffering indicator and auto-reconnect when a stream drops
- Allow adding custom stations manually

## Feature 4: Favorites
- One favorite (heart/star) action for local tracks, Navidrome tracks, and radio stations
- Favorites view with filters: All, Local, Navidrome, Radio
- Persist across launches; play directly or add to playlists
- Export/import favorites as JSON

## Architecture and quality
- A unified `PlayableItem` enum covering LocalFile, NavidromeTrack, and RadioStream
- Playback engine runs on its own thread and communicates via channels; UI never blocks
- Typed errors (thiserror) with user-friendly messages in the UI; logging via tracing
- Unit tests for: Navidrome auth/parsing, M3U/PLS parsing, EQ filter math, favorites store
- Set up GitHub Actions CI that builds and tests on macOS, Windows, and Linux from the start, even though macOS is the first release target

## How to work
1. First, present the workspace structure, key traits, and data models, and briefly confirm the plan.
2. Build incrementally in this order, ensuring the app compiles and runs after each step:
   a. Workspace setup + playback engine + local file playback + playlist
   b. Equalizer + visualizer
   c. Favorites
   d. Internet radio + Radio Browser
   e. Navidrome integration
   f. Media controls, tray, polish, tests, CI
3. Provide complete files (no "..." placeholders) and exact commands to install prerequisites and build/run on macOS (and note what Windows/Linux need, e.g. WebView2, WebKitGTK, ALSA/PulseAudio dev packages).
4. At the end, list known limitations per platform and suggested next features (e.g. skin support, lyrics, offline caching of Navidrome tracks).
