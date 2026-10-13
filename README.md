# Wynamp (REA)

> A compact, retro, fast desktop audio player inspired by the classic Winamp 2.x experience — written in Rust.

**Status: pre-alpha.** Phase 0 (workspace foundation) is in place; the first playable release is **v0.1.0**. See **[ROADMAP.md](ROADMAP.md)** for the full release plan.

Wynamp is free and open source (MIT OR Apache-2.0), uses original visual assets and its own name. It is not affiliated with, endorsed by, or derived from Winamp or its trademarks.

## Planned releases

| Release | Theme                                              |
| ------- | -------------------------------------------------- |
| v0.1.0  | Local playback, playlists, library — first playable |
| v0.2.0  | 10-band equalizer, spectrum analyzer, themes        |
| v0.3.0  | Favorites across all sources                        |
| v0.4.0  | Internet radio + Radio Browser                      |
| v0.5.0  | Navidrome (Subsonic) streaming                      |
| v1.0.0  | Media keys, tray, packaging, public release         |

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

## Building

### Prerequisites

- **Rust via [rustup](https://rustup.rs)** (required). A package-manager Rust (Homebrew, MSYS2, distro) can shadow rustup and break builds; `rustup default stable` should win in `PATH`.
- **Phase 1+**: Node.js 20+ (Tauri frontend tooling).
- Platform packages (Phase 1+, when Tauri and cpal land):
  - **macOS**: Xcode Command Line Tools — `xcode-select --install`
  - **Linux (Debian/Ubuntu)**:
    `sudo apt-get install -y libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libssl-dev libxdo-dev libasound2-dev build-essential curl wget file`
  - **Windows**: MSVC Build Tools + WebView2 runtime

### Build, test, run (Phase 0)

```sh
cargo run -p rea-wynamp-app      # prints foundation info
cargo test --workspace
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). The roadmap is the contract: pick items from the current phase.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
