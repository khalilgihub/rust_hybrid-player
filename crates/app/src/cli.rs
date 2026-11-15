//! CLI argument parsing, headless snapshot options, userData directory resolution,
//! Win32 single-instance mutex + named-pipe forwarding, and startup shell bootstrap.

use std::path::{Path, PathBuf};

use hybrid_core::media_scanner::is_media_extension;
use hybrid_shaders::{ThemeMode, WelcomeBackgroundKind};
use hybrid_ui::state::ModalId;
use hybrid_win32_shell::{
    cleanup_temp_files, setup_windows_shell_integration, HighResTimerGuard, SingleInstanceOutcome,
    SingleInstanceServer,
};

pub const SINGLE_INSTANCE_MUTEX_NAME: &str = hybrid_win32_shell::DEFAULT_SINGLE_INSTANCE_MUTEX;
pub const SINGLE_INSTANCE_PIPE_NAME: &str = hybrid_win32_shell::DEFAULT_SINGLE_INSTANCE_PIPE;

/// Overlay state override for `--headless-snapshot` (`overlay-*` golden captures).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SnapshotOverlayKind {
    #[default]
    None,
    BurstPill,
    ClipBezel,
    Carousel,
}

/// Configuration for `--headless-snapshot` offscreen GPU rendering (`qa/` pixel-diff mode).
#[derive(Debug, Clone, PartialEq)]
pub struct HeadlessSnapshotOptions {
    pub output_path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub dpr: f32,
    pub theme: ThemeMode,
    pub welcome_bg: WelcomeBackgroundKind,
    pub modal: Option<ModalId>,
    pub playlist_open: bool,
    pub bg_accordion_open: bool,
    pub overlay: SnapshotOverlayKind,
    pub burst_count: usize,
    pub carousel_index: usize,
    pub clip_time_secs: f64,
    pub playback_sample: bool,
    pub snapshot_media: Option<PathBuf>,
    pub time_secs: f32,
    pub preview_time_secs: Option<f32>,
}

impl Default for HeadlessSnapshotOptions {
    fn default() -> Self {
        Self {
            output_path: PathBuf::from("snapshot.png"),
            width: 1280,
            height: 720,
            dpr: 1.0,
            theme: ThemeMode::Dark,
            welcome_bg: WelcomeBackgroundKind::Dither,
            modal: None,
            playlist_open: false,
            bg_accordion_open: false,
            overlay: SnapshotOverlayKind::None,
            burst_count: 12,
            carousel_index: 1,
            clip_time_secs: 4.2,
            playback_sample: false,
            snapshot_media: None,
            time_secs: 1.5,
            preview_time_secs: None,
        }
    }
}

/// Parsed command-line options for `hybrid-player.exe`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AppCliArgs {
    pub media_inputs: Vec<String>,
    pub app_data_dir: Option<PathBuf>,
    pub headless_snapshot: Option<HeadlessSnapshotOptions>,
    pub focus_only: bool,
}

impl AppCliArgs {
    /// Parses CLI arguments excluding `argv[0]`.
    pub fn parse_from_iter<I, S>(iter: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let raw: Vec<String> = iter.into_iter().map(Into::into).collect();
        let mut parsed = Self::default();
        let mut snapshot: Option<HeadlessSnapshotOptions> = None;

        let mut i = 0usize;
        while i < raw.len() {
            let arg = &raw[i];
            match arg.as_str() {
                "--headless-snapshot" => {
                    i += 1;
                    let out = raw
                        .get(i)
                        .ok_or_else(|| "--headless-snapshot requires an output path".to_string())?;
                    let mut opts = snapshot.take().unwrap_or_default();
                    opts.output_path = PathBuf::from(out);
                    snapshot = Some(opts);
                }
                "--width" => {
                    i += 1;
                    let v = raw
                        .get(i)
                        .ok_or_else(|| "--width requires a pixel value".to_string())?;
                    let width: u32 = v
                        .parse()
                        .map_err(|_| format!("invalid --width value: {v}"))?;
                    let mut opts = snapshot.take().unwrap_or_default();
                    opts.width = width.clamp(320, 7680);
                    snapshot = Some(opts);
                }
                "--height" => {
                    i += 1;
                    let v = raw
                        .get(i)
                        .ok_or_else(|| "--height requires a pixel value".to_string())?;
                    let height: u32 = v
                        .parse()
                        .map_err(|_| format!("invalid --height value: {v}"))?;
                    let mut opts = snapshot.take().unwrap_or_default();
                    opts.height = height.clamp(240, 4320);
                    snapshot = Some(opts);
                }
                "--dpr" => {
                    i += 1;
                    let v = raw
                        .get(i)
                        .ok_or_else(|| "--dpr requires a float scale factor".to_string())?;
                    let dpr: f32 = v.parse().map_err(|_| format!("invalid --dpr value: {v}"))?;
                    let mut opts = snapshot.take().unwrap_or_default();
                    opts.dpr = if dpr.is_finite() {
                        dpr.clamp(0.5, 4.0)
                    } else {
                        1.0
                    };
                    snapshot = Some(opts);
                }
                "--theme" => {
                    i += 1;
                    let v = raw
                        .get(i)
                        .ok_or_else(|| "--theme requires dark|oled|light".to_string())?;
                    let theme = parse_theme_arg(v)?;
                    let mut opts = snapshot.take().unwrap_or_default();
                    opts.theme = theme;
                    snapshot = Some(opts);
                }
                "--welcome-bg" => {
                    i += 1;
                    let v = raw
                        .get(i)
                        .ok_or_else(|| "--welcome-bg requires an effect name".to_string())?;
                    let bg = parse_welcome_bg_arg(v)?;
                    let mut opts = snapshot.take().unwrap_or_default();
                    opts.welcome_bg = bg;
                    snapshot = Some(opts);
                }
                "--modal" => {
                    i += 1;
                    let v = raw
                        .get(i)
                        .ok_or_else(|| "--modal requires a modal identifier".to_string())?;
                    let modal = parse_modal_arg(v)?;
                    let mut opts = snapshot.take().unwrap_or_default();
                    opts.modal = modal;
                    snapshot = Some(opts);
                }
                "--playlist-open" => {
                    let mut opts = snapshot.take().unwrap_or_default();
                    opts.playlist_open = true;
                    snapshot = Some(opts);
                }
                "--bg-accordion" => {
                    let mut opts = snapshot.take().unwrap_or_default();
                    opts.bg_accordion_open = true;
                    snapshot = Some(opts);
                }
                "--overlay" => {
                    i += 1;
                    let v = raw.get(i).ok_or_else(|| {
                        "--overlay requires none|burst-pill|clip-bezel|carousel".to_string()
                    })?;
                    let overlay = parse_overlay_arg(v)?;
                    let mut opts = snapshot.take().unwrap_or_default();
                    opts.overlay = overlay;
                    snapshot = Some(opts);
                }
                "--burst-count" => {
                    i += 1;
                    let v = raw
                        .get(i)
                        .ok_or_else(|| "--burst-count requires a positive integer".to_string())?;
                    let count: usize = v
                        .parse()
                        .map_err(|_| format!("invalid --burst-count value: {v}"))?;
                    let mut opts = snapshot.take().unwrap_or_default();
                    opts.burst_count = count;
                    snapshot = Some(opts);
                }
                "--carousel-index" => {
                    i += 1;
                    let v = raw
                        .get(i)
                        .ok_or_else(|| "--carousel-index requires an integer".to_string())?;
                    let idx: usize = v
                        .parse()
                        .map_err(|_| format!("invalid --carousel-index value: {v}"))?;
                    let mut opts = snapshot.take().unwrap_or_default();
                    opts.carousel_index = idx;
                    snapshot = Some(opts);
                }
                "--clip-time" => {
                    i += 1;
                    let v = raw
                        .get(i)
                        .ok_or_else(|| "--clip-time requires a float value".to_string())?;
                    let secs: f64 = v
                        .parse()
                        .map_err(|_| format!("invalid --clip-time value: {v}"))?;
                    let mut opts = snapshot.take().unwrap_or_default();
                    opts.clip_time_secs = if secs.is_finite() { secs.max(0.0) } else { 0.0 };
                    snapshot = Some(opts);
                }
                "--playback-sample" => {
                    let mut opts = snapshot.take().unwrap_or_default();
                    opts.playback_sample = true;
                    snapshot = Some(opts);
                }
                "--media" => {
                    i += 1;
                    let p = raw
                        .get(i)
                        .ok_or_else(|| "--media requires a file path".to_string())?;
                    let mut opts = snapshot.take().unwrap_or_default();
                    opts.playback_sample = true;
                    opts.snapshot_media = Some(PathBuf::from(p));
                    snapshot = Some(opts);
                }
                "--time-secs" => {
                    i += 1;
                    let v = raw
                        .get(i)
                        .ok_or_else(|| "--time-secs requires a float value".to_string())?;
                    let t: f32 = v
                        .parse()
                        .map_err(|_| format!("invalid --time-secs value: {v}"))?;
                    let mut opts = snapshot.take().unwrap_or_default();
                    opts.time_secs = if t.is_finite() { t.max(0.0) } else { 0.0 };
                    snapshot = Some(opts);
                }
                "--preview-time-secs" => {
                    i += 1;
                    let v = raw
                        .get(i)
                        .ok_or_else(|| "--preview-time-secs requires a float value".to_string())?;
                    let t: f32 = v
                        .parse()
                        .map_err(|_| format!("invalid --preview-time-secs value: {v}"))?;
                    let mut opts = snapshot.take().unwrap_or_default();
                    opts.preview_time_secs = Some(if t.is_finite() { t.max(0.0) } else { 0.0 });
                    snapshot = Some(opts);
                }
                "--app-data-dir" => {
                    i += 1;
                    let dir = raw
                        .get(i)
                        .ok_or_else(|| "--app-data-dir requires a directory path".to_string())?;
                    parsed.app_data_dir = Some(PathBuf::from(dir));
                }
                "--file" => {
                    i += 1;
                    let path = raw
                        .get(i)
                        .ok_or_else(|| "--file requires a path or URL".to_string())?;
                    if is_supported_cli_input(path) {
                        parsed.media_inputs.push(normalize_cli_input(path));
                    }
                }
                "--focus" | "--focus-only" => {
                    parsed.focus_only = true;
                }
                other if !other.starts_with('-') && is_supported_cli_input(other) => {
                    parsed.media_inputs.push(normalize_cli_input(other));
                }
                _ => {
                    // Ignore unrecognized OS/Chromium-style flags safely without crashing.
                }
            }
            i += 1;
        }

        parsed.headless_snapshot = snapshot;
        Ok(parsed)
    }

    pub fn parse_env() -> Result<Self, String> {
        Self::parse_from_iter(std::env::args().skip(1))
    }

    /// Arguments forwarded across the single-instance named pipe to an existing primary window.
    pub fn forward_payload(&self) -> Vec<String> {
        if self.media_inputs.is_empty() {
            vec!["--focus".to_string()]
        } else {
            self.media_inputs.clone()
        }
    }
}

pub fn parse_theme_arg(raw: &str) -> Result<ThemeMode, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "dark" => Ok(ThemeMode::Dark),
        "oled" => Ok(ThemeMode::Oled),
        "light" => Ok(ThemeMode::Light),
        other => Err(format!("unknown theme '{other}', expected dark|oled|light")),
    }
}

pub fn parse_welcome_bg_arg(raw: &str) -> Result<WelcomeBackgroundKind, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "dither" => Ok(WelcomeBackgroundKind::Dither),
        "particles" => Ok(WelcomeBackgroundKind::Particles),
        "faulty" | "terminal" => Ok(WelcomeBackgroundKind::Faulty),
        "gridmotion" | "grid-motion" | "grid" => Ok(WelcomeBackgroundKind::Gridmotion),
        "none" | "off" => Ok(WelcomeBackgroundKind::None),
        other => Err(format!(
            "unknown welcome background '{other}', expected dither|particles|faulty|gridmotion|none"
        )),
    }
}

pub fn parse_modal_arg(raw: &str) -> Result<Option<ModalId>, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "none" => Ok(None),
        "eq" | "equalizer" => Ok(Some(ModalId::Equalizer)),
        "speed" => Ok(Some(ModalId::Speed)),
        "sub" | "subs" | "subtitles" => Ok(Some(ModalId::Subtitles)),
        "audio" => Ok(Some(ModalId::Audio)),
        "settings" => Ok(Some(ModalId::Settings)),
        "network" | "stream" | "network-stream" => Ok(Some(ModalId::NetworkStream)),
        "bg" | "bg-settings" | "background" => Ok(Some(ModalId::BgSettings)),
        other => Err(format!("unknown modal '{other}'")),
    }
}

pub fn parse_overlay_arg(raw: &str) -> Result<SnapshotOverlayKind, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "none" => Ok(SnapshotOverlayKind::None),
        "burst-pill" | "burst" => Ok(SnapshotOverlayKind::BurstPill),
        "clip-bezel" | "clip" => Ok(SnapshotOverlayKind::ClipBezel),
        "carousel" | "screenshot-carousel" => Ok(SnapshotOverlayKind::Carousel),
        other => Err(format!(
            "unknown overlay '{other}', expected none|burst-pill|clip-bezel|carousel"
        )),
    }
}

/// Validates a positional CLI input (HTTP/HTTPS/ytdl URL, existing path, or media extension).
pub fn is_supported_cli_input(raw: &str) -> bool {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("ytdl://")
    {
        return true;
    }
    let p = Path::new(trimmed);
    p.exists() || is_media_extension(p)
}

fn normalize_cli_input(raw: &str) -> String {
    let trimmed = raw.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("ytdl://")
    {
        return trimmed.to_string();
    }
    let p = Path::new(trimmed);
    if let Ok(abs) = std::fs::canonicalize(p) {
        // Strip Windows verbatim `\\?\` prefix for cleaner display & libmpv interop.
        let s = abs.to_string_lossy();
        return s.strip_prefix(r"\\?\").unwrap_or(&s).to_string();
    }
    trimmed.to_string()
}

/// Resolves the Hybrid Player `userData` directory (`%APPDATA%\hybrid-player` on Windows).
pub fn resolve_user_data_dir(override_dir: Option<&Path>) -> PathBuf {
    if let Some(dir) = override_dir {
        return dir.to_path_buf();
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        return PathBuf::from(appdata).join("hybrid-player");
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local).join("hybrid-player");
    }
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("hybrid-player-data")
}

/// Result of attempting single-instance lock acquisition.
pub enum SingleInstanceBootstrap {
    Primary(SingleInstanceServer),
    SecondaryForwarded,
}

/// Acquires the single-instance mutex or forwards args to the running primary window.
pub fn acquire_single_instance(args: &AppCliArgs) -> Result<SingleInstanceBootstrap, String> {
    let payload = args.forward_payload();
    match SingleInstanceServer::acquire(
        SINGLE_INSTANCE_MUTEX_NAME,
        SINGLE_INSTANCE_PIPE_NAME,
        &payload,
    ) {
        Ok(SingleInstanceOutcome::Primary(server)) => Ok(SingleInstanceBootstrap::Primary(server)),
        Ok(SingleInstanceOutcome::SecondaryForwarded) => {
            Ok(SingleInstanceBootstrap::SecondaryForwarded)
        }
        Err(err) => Err(format!("single-instance initialization failed: {err}")),
    }
}

/// Holds OS startup guards released cleanly on application exit.
pub struct StartupRuntimeGuards {
    pub timer_guard: HighResTimerGuard,
}

/// Performs Win32 startup bootstrap: `timeBeginPeriod(1)`, AUMID + Start Menu shortcut,
/// and stale PID-scoped temp cleanup.
pub fn startup_shell_bootstrap(create_shortcut: bool) -> StartupRuntimeGuards {
    let timer_guard = HighResTimerGuard::acquire();
    let ico = locate_asset_icon("icon.ico");
    let png = locate_asset_icon("icon.png");
    let _ = setup_windows_shell_integration(ico.as_deref(), png.as_deref(), create_shortcut);
    cleanup_temp_files(&std::env::temp_dir(), std::process::id(), false);
    StartupRuntimeGuards { timer_guard }
}

/// Cleans up temp files created by this PID on graceful shutdown.
pub fn shutdown_temp_cleanup() {
    cleanup_temp_files(&std::env::temp_dir(), std::process::id(), true);
}

fn locate_asset_icon(file_name: &str) -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let candidate = parent.join("assets").join(file_name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    let workspace_candidate = PathBuf::from("hybrid-player/assets").join(file_name);
    if workspace_candidate.exists() {
        return Some(workspace_candidate);
    }
    None
}
