# Wynamp Architecture

## Workspace layout

```
rea-wynamp/
├── core/        rea-wynamp-core      — models, playback protocol, errors, engine, DSP
├── platform/    rea-wynamp-platform  — OS traits: paths, credentials, media controls
├── navidrome/   rea-wynamp-navidrome — Subsonic/OpenSubsonic API client
├── radio/       rea-wynamp-radio     — internet radio + Radio Browser API
├── app/         rea-wynamp-app       — Tauri 2 app (commands/events bridge)
├── ui/          (frontend)           — HTML/CSS/TypeScript, built into the Tauri app
└── docs/                             — architecture & roadmap docs
```

### Crate responsibilities & dependencies

| Crate                 | Depends on                  | Must NOT depend on             |
| --------------------- | --------------------------- | ------------------------------ |
| `rea-wynamp-core`     | (external crates only)      | Tauri, any UI, any OS service  |
| `rea-wynamp-platform` | `core`                      | Tauri, feature crates          |
| `rea-wynamp-navidrome`| `core`                      | `platform`, Tauri, `radio`     |
| `rea-wynamp-radio`    | `core`                      | `platform`, Tauri, `navidrome` |
| `rea-wynamp-app`      | all of the above            | — (composition root)           |

Dependency direction is strictly downward: `app → {navidrome, radio, platform} → core`.
`core` never imports sibling crates. This keeps the engine and parsers unit-testable
headlessly.

## Key data models (core)

- **`PlayableItem`** — one enum covering all sources:
  `LocalFile { path }`, `NavidromeTrack { server_id, track_id }`,
  `RadioStream { url, name }`. The engine and playlist editor speak only this type.
- **`TrackMeta`** — title/artist/album/duration/bitrate/sample-rate/codec… (lofty
  fills it for local files; API responses fill it for network items).
- **`Playlist` / `PlaylistEntry`** — ordered queue items with metadata;
  entry display titles degrade gracefully (title → file stem → id/URL).
- **`PlayerStatus`** — snapshot of engine state (state, current track, position,
  duration, volume 0..=1, balance -1..=1, shuffle, repeat, bitrate, sample rate).
- **`PlaybackState`** — `Stopped | Playing | Paused | Buffering`;
  **`RepeatMode`** — `Off | All | One`.

## Engine protocol (Phase 1+)

The playback engine owns a dedicated thread. UI (Tauri commands) never blocks:

```
┌──────────────┐  PlayerCommand   ┌─────────────────────────────────────┐
│  UI (JS/TS)  │ ───────────────► │ Engine thread                       │
│  via Tauri   │                  │  decode (Symphonia)                 │
│  commands    │ ◄─────────────── │  → EQ (Phase 2) → cpal output       │
└──────────────┘  PlayerEvent     │  ring buffer, gapless transitions   │
                                  └─────────────────────────────────────┘
```

- **`PlayerCommand`**: play/pause/toggle/stop, next/prev, seek(+relative),
  volume(+relative), balance, shuffle, repeat, queue ops (replace/enqueue/remove/
  clear), `QueryStatus`, `Shutdown`.
- **`PlayerEvent`**: `Status` snapshots, `TrackStarted`/`TrackFinished`,
  `QueueChanged`, `Warning` (non-fatal, playback continues), `Fatal`.
- Network work (Navidrome, Radio Browser, scans) runs on tokio/background
  threads and feeds the engine via the same command channel.

## Platform isolation

All OS variance lives in `rea-wynamp-platform`; selection happens in
`Platform::detect()` using `cfg(target_os = ...)`:

| Trait             | Phase 0 state        | Lands in                                             |
| ----------------- | -------------------- | ---------------------------------------------------- |
| `Paths`           | ✅ implemented (`directories`) | —                                        |
| `Credentials`     | stub (returns error) | v0.5 — keyring (Keychain / Credential Manager / Secret Service) |
| `MediaControls`   | stub (no-op)         | v1.0 — souvlaki (media keys / SMTC / MPRIS)          |

Rule: feature code and the frontend depend on **traits only**. Porting Wynamp to a
new OS means adding implementations here plus build config in `app/`.

## Storage

Directories come from `Platform::paths` (never hardcoded):

| Purpose            | Location (via `directories::ProjectDirs`, application name `Wynamp`)                     |
| ------------------ | ---------------------------------------------------------------------------------------- |
| Library/favorites DB, logs | data dir (`library.db`, …)                                                      |
| Settings, saved playlists   | config dir (shares the data dir on macOS/Windows)                             |
| Cover-art & station logos   | cache dir (always separate, regenerable)                                      |

SQLite is used from Phase 1 (rusqlite, WAL mode); schema migrations live next to
the store code in `core`.

## Conventions

- **Errors**: `thiserror` enums per crate (`core::Error` is the workspace-wide
  type); the app maps them to short, friendly UI messages. Never `unwrap()` on
  user input or I/O.
- **Logging**: `tracing` with spans (`engine`, `scan`, `net`) from Phase 1;
  logs go to the data dir.
- **Naming**: crates `rea-wynamp-*`, binary `wynamp`, display name **Wynamp**,
  repo `rea-wynamp`.
- **Tests**: pure logic (parsers, DSP math, stores, auth) is unit-tested inside
  `core`/`navidrome`/`radio`; each phase adds its tests before release.
- **No placeholders**: files land complete; unfinished behavior is a typed
  `Error::Unsupported` with a version reference, not a `todo!()`.
