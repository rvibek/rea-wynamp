# Wynamp frontend (`ui/`)

The Tauri 2 frontend: plain TypeScript + HTML/CSS (Vite dev server in Phase 1).
No heavy UI framework — the retro Winamp-style UI is hand-rolled CSS driven by
CSS variables (so themes/skins can be layered on later).

- **Phase 1**: main player window, playlist editor, retro dark theme
- **Phase 2**: equalizer window, spectrum analyzer, panel stacking, mini mode
- **Later**: favorites view, radio browser, Navidrome browser, settings

Commands/events flow through the Tauri bridge in `app/`; the frontend never
talks to audio directly. See `docs/ARCHITECTURE.md`.
