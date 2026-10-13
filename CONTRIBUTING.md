# Contributing to Wynamp

Thanks for wanting to help. This project is a phased build — the roadmap
([ROADMAP.md](ROADMAP.md)) is the contract for what ships when.

## Ground rules

1. **Work from the current phase.** Pick items from the phase marked *In
   progress*; don't start next-phase features ahead of schedule.
2. **`main` stays releasable.** Every commit must pass:
   ```sh
   cargo fmt --all --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   ```
   CI runs the same on macOS, Windows, and Linux.
3. **Complete files only.** No `...` placeholders or stubbed-out bodies in
   merged code.
4. **No Winamp assets or trademarks.** Original names, icons, and skin artwork
   only.
5. **Platform code stays isolated.** Anything OS-specific belongs behind the
   traits in `rea-wynamp-platform`, selected with `cfg(target_os = ...)`.
6. **The UI never blocks.** Audio work happens on the engine thread; UI talks
   to it via channels (`PlayerCommand` / `PlayerEvent`).

## Development setup

1. Install Rust via [rustup](https://rustup.rs) (`rustup default stable`).
2. Clone and run:
   ```sh
   cargo test --workspace
   cargo run -p rea-wynamp-app
   ```
3. Phase 1+ additionally needs Node.js 20+ and the platform packages listed in
   [README.md](README.md#prerequisites).

## Pull requests

- One phase item or fix per PR when possible.
- Rebase on `main`; keep commits scoped and messages imperative
  ("Add M3U parser", not "added parser").
- Tests required for logic changes (parsers, DSP math, stores).
- Update ROADMAP/ARCHITECTURE docs when behavior or structure changes.

## Where things live

- Architecture, traits, threading: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- Original spec: [plan.md](plan.md)
