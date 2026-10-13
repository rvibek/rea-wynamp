# Wynamp (REA)

[![CI](https://github.com/rvibek/rea-wynamp/actions/workflows/ci.yml/badge.svg)](https://github.com/rvibek/rea-wynamp/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)

> A compact, retro, fast desktop audio player inspired by the classic Winamp 2.x experience — written in Rust.

## ⚠️ About this repository

**This repository exists to test [REA](https://github.com/morluto/rea)** (Reverse Engineer Anything) — it is a hands-on playground for driving an agent-assisted reverse-engineering/development workflow.

**Building this app is purely a learning exercise** — exploring Rust, Tauri, audio decoding, DSP, and cross-platform packaging by recreating the classic Winamp-style player experience. Expect rough edges; expect things to change.

Wynamp is free and open source (MIT OR Apache-2.0), uses original visual assets and its own name. It is not affiliated with, endorsed by, or derived from Winamp or its trademarks.

**Status: pre-alpha, in active development.** The playback engine (gapless, seek, volume/balance) and the player UI are working; packaging and the first tagged release come next. See **[ROADMAP.md](ROADMAP.md)** for the full release plan.

## Planned releases

| Release | Theme                                              |
| ------- | -------------------------------------------------- |
| v0.1.0  | Local playback player + DMG — first playable       |
| v0.1.1  | Playlist editor, M3U/PLS, folder scan, library     |
| v0.2.0  | 10-band equalizer, spectrum analyzer, themes        |
| v0.3.0  | Favorites across all sources                        |
| v0.4.0  | Internet radio + Radio Browser                      |
| v0.5.0  | Navidrome (Subsonic) streaming                      |
| v1.0.0  | Media keys, tray, installers, public launch         |

## Workspace

| Crate                  | Path          | Responsibility                                                     |
| ---------------------- | ------------- | ------------------------------------------------------------------ |
| `rea-wynamp-core`      | `core/`       | Models, playback protocol, errors, engine, DSP (no UI dependency)  |
| `rea-wynamp-platform`  | `platform/`   | OS traits: media controls, file paths, credentials                 |
| `rea-wynamp-navidrome` | `navidrome/`  | Subsonic/OpenSubsonic API client                                   |
| `rea-wynamp-radio`     | `radio/`      | Internet radio + Radio Browser API                                 |
| `rea-wynamp-app`       | `app/`        | Tauri desktop app bridging core to the frontend                    |
| (frontend)             | `ui/`         | HTML/CSS/TypeScript UI                                            |

Details: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Install (macOS)

Prebuilt universal DMG (Apple Silicon + Intel) ships on the
[**Releases**](https://github.com/rvibek/rea-wynamp/releases) page:

1. Download the `.dmg` from the latest release.
2. Open it and drag **Wynamp** to *Applications*.
3. First launch: right-click → **Open** — the build is **unsigned**, so
   Gatekeeper needs this one-time override (no notarization yet; that needs a
   paid Apple Developer account).

Windows (`.msi`) and Linux (`.AppImage`) installers are planned for v1.0.0 —
until then those platforms build from source (below). CI already compiles and
tests every commit on all three OSes.

## Building & running

### Prerequisites

- **Rust via [rustup](https://rustup.rs)** (required). A package-manager Rust (Homebrew, MSYS2, distro) can shadow rustup and break builds; `rustup default stable` should win in `PATH`.
- **Node.js 20+** (Tauri frontend tooling).
- Platform packages:
  - **macOS**: Xcode Command Line Tools — `xcode-select --install`
  - **Linux (Debian/Ubuntu)**:
    `sudo apt-get install -y libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libssl-dev libxdo-dev libasound2-dev build-essential curl wget file`
  - **Windows**: MSVC Build Tools + WebView2 runtime

### Run (development)

```sh
npm install --prefix ui     # once
cd ui && npm run tauri dev  # window opens; TS/CSS hot-reloads
```

### Test / lint (same gates as CI)

```sh
cargo test --workspace
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

### Play a file from the CLI (engine smoke test)

```sh
cargo run -p rea-wynamp-core --example play -- /path/to/song.mp3
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). The roadmap is the contract: pick items from the current phase.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
