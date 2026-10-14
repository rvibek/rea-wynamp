# Wynamp — Roadmap & Release Plan

This document is the contract for how Wynamp ships: **one public GitHub Release per
phase**, each independently useful, all green on CI before tagging.

## Release strategy

- **Tag per phase**: every phase ends in a GitHub Release (`v0.1.0` … `v1.0.0`).
- **`main` is always releasable**: compiles, `cargo test`/`clippy`/`fmt` green on
  macOS, Windows, and Linux at every commit.
- **Each release ships**: *What's new*, install/build instructions, and
  *Known limitations* for each platform.
- **Pre-1.0 (v0.x)**: API and UI may change between minor versions; behavior
  listed as *Done* in an earlier phase stays working (phases are cumulative).
- **Scope control**: later phases may be trimmed or deferred — never earlier ones.
- **Repo launch**: public since inception at `github.com/rvibek/rea-wynamp`
  (CI green on all three OSes); the v0.1.0 release adds the first DMG asset.
- **Tagged releases are automated**: pushing a `v*` tag runs
  `.github/workflows/release.yml`, which builds the macOS DMG and attaches it
  to a GitHub Release with notes.

## Phase overview

| Phase | Version | Theme                                               | Status         |
| ----- | ------- | --------------------------------------------------- | -------------- |
| 0     | —       | Foundation: workspace, models, traits, CI, docs      | ✅ Done        |
| 1     | v0.1.0  | Playback engine + player UI + macOS DMG             | ✅ Done        |
| 1b    | v0.1.1  | Playlist editor, M3U/PLS, folder scan, SQLite library | ✅ Done        |
| 2     | v0.2.0  | Equalizer, visualizer, panels, themes                | ✅ Done        |
| 3     | v0.3.0  | Favorites                                            | ✅ Done        |
| 4     | v0.4.0  | Internet radio + Radio Browser                       | ✅ Done        |
| 5     | v0.5.0  | Navidrome streaming                                  | ✅ Done        |
| 6     | v1.0.0  | Media keys, tray, Win/Linux installers, launch       | ⬜ Not started |

---

## Phase 0 — Foundation ✅

_Workspace that everything else builds on; no user-facing features yet._

**Delivered**

- [x] Git repo, README, ROADMAP (this file), CONTRIBUTING, dual MIT/Apache-2.0 license
- [x] Cargo workspace: `core`, `platform`, `navidrome`, `radio`, `app` (+ `ui/` stub)
- [x] Data models: `PlayableItem`, `TrackMeta`, `Playlist`/`PlaylistEntry`,
      `PlayerStatus`, `PlaybackState`, `RepeatMode`
- [x] Engine protocol: `PlayerCommand` / `PlayerEvent` (channel types for Phase 1)
- [x] Platform traits: `Paths` (implemented), `Credentials` (stub → v0.5),
      `MediaControls` (stub → v1.0); `Platform::detect()` selection point
- [x] Typed errors (`thiserror`) in core and feature crates
- [x] CI: fmt + clippy + test on macOS / Windows / Linux (GitHub Actions)
- [x] Unit tests for models, paths, and crate basics

**Acceptance**: `cargo build` / `test` / `clippy -D warnings` / `fmt --check` green
locally and in CI on all three OSes.

---

## Phase 1 — v0.1.0 "First play" ✅

_The working app: a real Winamp-style player for local files, packaged as a DMG._

**Delivered**

- [x] Playback engine on its own thread (Symphonia decode → ring → cpal);
      commands/events over channels — **UI never blocks**
- [x] Gapless local playback, seek, pause/resume, volume, balance
- [x] Main player window: marquee title, elapsed/total time, bitrate/sample
      rate/codec, seek bar, transport, shuffle, repeat chips, queue list
- [x] Open-files dialog + drag & drop of audio files; lofty metadata display
- [x] Keyboard shortcuts: `Z X C V B`, Space, arrows (seek/volume)
- [x] Tauri 2 app + plain-TS frontend; retro dark theme (CSS variables)
- [x] Tests: engine internals + headless decode/gapless/seek suites (43 tests)
- [x] `examples/play.rs` verified on real speakers
- [x] CI green on macOS / Windows / Linux
- [x] Interactive verification pass
- [x] Screenshot in README
- [x] Version `0.1.0` + first **macOS `.app`/`.dmg`** (Tauri bundler, original
      icon) — release workflow at `.github/workflows/release.yml`
- [x] GitHub Release `v0.1.0` (DMG + auto source archives + notes)

**Definition of done**: open files → gapless playback → seek/volume/shortcuts
all work; a DMG installs and launches the app; CI green; build commands
documented; release published with DMG attached.

**Release**: tag `v0.1.0` with source (auto) + macOS DMG
(Windows/Linux installers arrive in v1.0.0).

**Known trade-ins**: no playlist editor/library yet (v0.1.1), no EQ, no
radio/Navidrome, no media keys, linear resampling when file rate ≠ device rate,
unsigned DMG (Gatekeeper right-click → Open).

---

## Phase 1b — v0.1.1 "Library" ✅

_The rest of the original Phase 1: file management and persistence._

**Delivered**

- [x] Playlist editor: multi-select (⌘-click) + delete, drag & drop reorder
      (`MoveEntry` keeps playback running), total duration summary
- [x] Save/load **M3U, M3U8, PLS** files (relative paths, both separators,
      `#EXTINF`, out-of-order PLS indices, portable exports)
- [x] Folder open: recursive background scan with progress events; audio-only
      extension filter; metadata via lofty (embedded album art deferred)
- [x] SQLite library persistence (rusqlite, WAL): tags, missing-file
      reconcile on launch, queue session restore (no autoplay)
- [x] Tests: M3U/PLS parsing, library store, scanner (54 core tests total)

**Definition of done**: open a folder → scan completes with progress → restart
the app with library intact → save and reload a playlist. ✅

---

## Phase 2 — v0.2.0 "Face the music" ✅

_The Winamp look and feel._

**Delivered**

- [x] 10-band graphic equalizer (RBJ peaking biquads, Q=1.0, ISO 31 Hz–16 kHz)
      + preamp ±12 dB, ON/OFF bypass, 8 presets
- [x] Spectrum analyzer + oscilloscope (rustfft), VIZ chip cycles off/scope/spec;
      tap is post-EQ so the EQ visibly shapes the display
- [x] Panel show/hide (EQ panel, visualizer canvas) + compact **mini mode**
      (window resizes, queue/EQ/file rows collapse)
- [x] Modern theme alongside retro default (same structure, CSS variables;
      SKIN chip cycles)
- [x] Tests: EQ filter math, spectrum peak placement, visualizer tap bounds

**Definition of done**: EQ audibly shapes output and toggles cleanly; visualizer
runs at smooth frame rate without stuttering audio; panels behave like classic
Winamp.

---

## Phase 3 — v0.3.0 "Heart collection" ✅

_Favorites, source-agnostic._

**Delivered**

- [x] Heart (♡/♥) on the current track; FAV panel with filters
      All / Local / Navidrome / Radio
- [x] Play from favorites (local paths, radio URLs, Navidrome refs)
- [x] Persist in `library.db`; pretty-JSON export/import (merge, no dupes)
- [x] Tests: favorites store + JSON roundtrip

---

## Phase 4 — v0.4.0 "On air" ✅

_Internet radio._

**Delivered**

- [x] Direct stream URLs (HTTP + ICY metadata stripping); PLAY URL for customs
- [x] Station browser via Radio Browser API (name/country search, codec,
      bitrate, clickcount ordering, UA-identified)
- [x] ICY `StreamTitle` → live "now playing" in the marquee
- [x] Buffering state + auto-reconnect (5 attempts, 2s delay)
- [x] Per-station favorite (FAV · RADIO filter)
- [x] Tests: ICY demux (boundaries, titles), Radio Browser parsing/URL
      encoding, headless engine test playing a WAV over local HTTP

---

## Phase 5 — v0.5.0 "Server time" ✅

_Navidrome (Subsonic/OpenSubsonic) streaming._

**Delivered**

- [x] Salted MD5 token auth (`u`, `t`, `s`); passwords in the OS keychain
      (Keychain / Credential Manager / Secret Service via `keyring`)
- [x] Multi-server settings (add/test/remove), Subsonic ping for "test"
- [x] Browse artists → albums → songs (cover art, disk-cached); search3
- [x] Stream with mp3 transcoding option; star = server star + local favorite
- [x] Scrobbles: now-playing on start, completed on track finish
- [x] Offline/unreachable → typed errors surfaced as toasts; app stays usable
- [x] Favorites resolve `server:song` refs to fresh stream URLs
- [x] Tests: token vector, URL building, response fixtures (6 client tests)

**Known limitations**: HLS unsupported (documented); linear resampler when
file rate ≠ device rate; album art extraction for local files deferred.

---

## Phase 6 — v1.0.0 "Ship it" ⬜

_Platform integration, packaging, public launch._

**Scope**

- Media controls & OS Now Playing via souvlaki (macOS media keys, Windows SMTC,
  Linux MPRIS) — `Platform::media_controls` implementation
- System tray / menu-bar mini controls
- Packaging parity: first-class installers for Windows (`.msi`) and Linux
  (`.AppImage`/`.deb`); macOS `.dmg` already ships since v0.1.0
- Full test pass, error-message polish, tracing/logging review
- README with screenshots, CONTRIBUTING finalized, per-platform **Known
  limitations** list, suggested next features
- **Public GitHub Release `v1.0.0`** with install artifacts

**Definition of done**: a new user on macOS downloads a DMG, installs, plays local
files, controls playback from media keys — no toolchain required.

---

## After v1.0 (candidates, unordered)

- Skin support (CSS-variable themes are the groundwork)
- Lyrics display
- Offline caching of Navidrome tracks
- Last.fm / ListenBrainz scrobbling
- Windows (`.msi`) and Linux (`.AppImage`/`.deb`) installers as first-class artifacts
- Cue-sheet / gapless album (.cue) support
- Customizable keyboard shortcuts

## Project-wide constraints & risk register

| Risk                                                | Mitigation                                                          |
| --------------------------------------------------- | ------------------------------------------------------------------- |
| Opus not supported by symphonia 0.6 as built        | Documented limitation; `symphonia-adapter-libopus` evaluated in v0.4 |
| HLS for radio                                       | Evaluate in Phase 4; ship without it if cost is high                |
| Gapless only meaningful for local files             | Stated per release; streams/radio are not gapless                   |
| souvlaki media-key quirks per OS                    | Isolated behind `platform` trait; Phase 6 can swap implementations  |
| Homebrew/package-manager Rust shadowing rustup      | Documented prerequisite: rustup owns the toolchain                  |
| Name discoverability (`REAmp` family is taken)      | Repo `rea-wynamp` verified unclaimed at plan time                   |
