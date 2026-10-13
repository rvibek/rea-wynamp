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
- **Repo launch**: the repository goes public when v0.1.0 content works
  (end of Phase 1), so the first thing the world sees is a player that plays.

## Phase overview

| Phase | Version  | Theme                                          | Status         |
| ----- | -------- | ---------------------------------------------- | -------------- |
| 0     | —        | Foundation: workspace, models, traits, CI, docs | ✅ Done        |
| 1     | v0.1.0   | Local playback, playlists, library             | ⬜ Not started |
| 2     | v0.2.0   | Equalizer, visualizer, panels, themes          | ⬜ Not started |
| 3     | v0.3.0   | Favorites                                       | ⬜ Not started |
| 4     | v0.4.0   | Internet radio + Radio Browser                 | ⬜ Not started |
| 5     | v0.5.0   | Navidrome streaming                            | ⬜ Not started |
| 6     | v1.0.0   | Media keys, tray, packaging, public launch     | ⬜ Not started |

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

## Phase 1 — v0.1.0 "First play" ⬜

_The working app: a real Winamp-style player for local files._

**Scope**

- Playback engine on its own thread (Symphonia decode → cpal output), commands and
  events over channels; **UI never blocks**
- Gapless local playback; seek, volume, balance
- Main player window: scrolling marquee, elapsed/remaining time, bitrate/sample
  rate, seek bar, transport, shuffle, repeat, open file
- Playlist editor: add/remove/reorder (drag & drop), multi-select, total duration,
  save/load **M3U, M3U8, PLS** (relative paths + both separators)
- Folder open: recursive background scan with progress; metadata + embedded album
  art (lofty)
- SQLite library persistence (rusqlite) with moved/missing file handling
- Drag & drop files/folders onto the window
- Keyboard shortcuts: `Z X C V B`, Space, arrows (seek/volume)
- Tauri 2 app + plain-TS frontend; retro dark theme (CSS variables)
- Tests: M3U/PLS parsing, library store

**Definition of done**: install nothing but prerequisites → open a folder → gapless
playback → edit and save a playlist → restart the app with library intact; CI green
on all three OSes; build commands documented.

**Release**: GitHub repo goes **public**; tag `v0.1.0` (source release; installers
arrive in v1.0.0).

**Known trade-ins**: no EQ/visualizer yet, no radio/Navidrome, no media keys.

---

## Phase 2 — v0.2.0 "Face the music" ⬜

_The Winamp look and feel._

**Scope**

- 10-band graphic equalizer (biquad filters) + preamp, presets, on/off
- Spectrum analyzer + oscilloscope (rustfft), toggle
- Panel show/hide & stacking, compact "mini mode"
- Modern theme option alongside retro default (both via CSS variables)
- Tests: EQ filter math

**Definition of done**: EQ audibly shapes output and toggles cleanly; visualizer
runs at smooth frame rate without stuttering audio; panels behave like classic
Winamp.

---

## Phase 3 — v0.3.0 "Heart collection" ⬜

_Favorites, source-agnostic._

**Scope**

- One favorite (heart/star) action on tracks; favorites view with filters
  (All, Local — Navidrome/Radio filters activate in later phases)
- Play directly from favorites, add to playlist
- Persist across launches; export/import favorites as JSON
- Tests: favorites store

**Definition of done**: favorite a track → restart → still there → play it; JSON
export re-imports losslessly.

---

## Phase 4 — v0.4.0 "On air" ⬜

_Internet radio._

**Scope**

- Direct stream URLs (MP3/AAC), PLS/M3U stream links
- Station browser via Radio Browser API: search by name/country/genre/language,
  logo, codec, bitrate
- ICY metadata → "now playing" in the marquee
- Buffering indicator; auto-reconnect when a stream drops
- Custom stations (manual add); favorites integration
- Tests: PLS/M3U stream list parsing, Radio Browser response parsing

**Definition of done**: browse → play a station → see now-playing → kill the network
→ reconnect indicator appears → network returns → playback resumes.

**Documented limitations**: HLS evaluated here — if not feasible, listed as known
limitation rather than blocking the release.

---

## Phase 5 — v0.5.0 "Server time" ⬜

_Navidrome (Subsonic/OpenSubsonic) streaming._

**Scope**

- Salted token auth (`u`, `t`, `s`); secrets in OS credential vault (keyring via
  `Platform::credentials`); multi-server settings screen; "test connection"
- Browse artists, albums, genres, playlists; search; cover art with caching
- Stream with optional max-bitrate/transcoding; add streams to the playlist editor
- Playlist sync (read; write if API allows); scrobble plays; stars ↔ local favorites
- Graceful behavior when server is offline/unreachable
- Favorites filter for Navidrome activates
- Tests: auth token generation, API response parsing (mocked)

**Definition of done**: add server → test connection → browse and play → star syncs
to favorites → disconnect network → friendly error, app stays usable.

---

## Phase 6 — v1.0.0 "Ship it" ⬜

_Platform integration, packaging, public launch._

**Scope**

- Media controls & OS Now Playing via souvlaki (macOS media keys, Windows SMTC,
  Linux MPRIS) — `Platform::media_controls` implementation
- System tray / menu-bar mini controls
- Packaging: `.app`/`.dmg` (macOS first), documented path to `.msi` (Windows) and
  `.AppImage`/`.deb` (Linux)
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
| Opus decoding gaps in Symphonia                     | Evaluate in Phase 1; fallback crate or documented limitation        |
| HLS for radio                                       | Evaluate in Phase 4; ship without it if cost is high                |
| Gapless only meaningful for local files             | Stated per release; streams/radio are not gapless                   |
| souvlaki media-key quirks per OS                    | Isolated behind `platform` trait; Phase 6 can swap implementations  |
| Homebrew/package-manager Rust shadowing rustup      | Documented prerequisite: rustup owns the toolchain                  |
| Name discoverability (`REAmp` family is taken)      | Repo `rea-wynamp` verified unclaimed at plan time                   |
