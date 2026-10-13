//! Wynamp desktop application entry point.
//!
//! Phase 0: workspace foundation only — this binary proves the workspace
//! builds end-to-end and shows where the OS services point. The Tauri window,
//! playback engine wiring, and the HTML/CSS/TypeScript frontend arrive in
//! Phase 1 (v0.1.0). See ROADMAP.md.

fn main() {
    let platform = rea_wynamp_platform::Platform::detect();

    println!(
        "Wynamp {} — pre-alpha foundation (Phase 0)",
        env!("CARGO_PKG_VERSION")
    );
    println!("  data dir:   {}", platform.paths.data_dir().display());
    println!("  config dir: {}", platform.paths.config_dir().display());
    println!("  cache dir:  {}", platform.paths.cache_dir().display());
    println!();
    println!("Nothing to play yet — see ROADMAP.md for the release plan.");
}
