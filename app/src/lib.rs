//! Hybrid Player Phase 5 application orchestration crate.
//!
//! Connects `hybrid-core`, `hybrid-media-services`, `hybrid-mpv-engine`,
//! `hybrid-win32-shell`, `hybrid-shaders`, and `hybrid-ui` into the desktop
//! runtime and `--headless-snapshot` CLI renderer.

pub mod cli;
pub mod input;
pub mod media_bridge;
pub mod renderer;
