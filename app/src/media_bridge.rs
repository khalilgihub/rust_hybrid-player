//! Application state coordinator bridging `hybrid-core` (DB, playlist, equalizer,
//! tracks, folder scanner, binary resolver), `hybrid-media-services` (FFmpeg clip,
//! yt-dlp probe/update, Option A YouTube auth), `hybrid-mpv-engine` (`MpvClient`,
//! `SeekThumbnailer`), `hybrid-win32-shell` (SMTC, Recycle Bin, Companion Auth window),
//! and `hybrid-ui` (`UiState`).

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hybrid_core::binary_resolver::{
    resolve_ffmpeg_binary, resolve_lib_mpv_with_options, resolve_mpv_binary, resolve_yt_dlp_binary,
    BinarySearchPaths, ResolveOptions,
};
use hybrid_core::db::{
    load_database, now_millis, save_database_atomic, Database, PlaylistItemEntry,
};
use hybrid_core::equalizer::{EqualizerPreset, EqualizerState};
use hybrid_core::media_scanner::{is_unc_or_network_path, scan_folder_for_media};
use hybrid_core::playlist::{
    NavigationAction, PlaylistItem, PlaylistState, RepeatMode as CoreRepeatMode,
};
use hybrid_core::shortcuts::{format_combo_compact, ShortcutManager};
use hybrid_core::tracks::{
    clamp_audio_sync_ms, format_language_name, next_audio_track, next_subtitle_track,
    select_subs_dubs_mode, subtitle_sec_to_ms, MediaTrack, SubsDubsMode, SubtitleStyle,
    DEFAULT_SUBTITLE_FONT_FAMILY, SUBTITLE_SYNC_MAX_SEC, SUBTITLE_SYNC_MIN_SEC,
};
use hybrid_media_services::ffmpeg_clip::{clip_media_segment, ClipRequest, ClipResult};
use hybrid_media_services::youtube_auth::{
    find_installed_chromium_browser, get_cookie_file_path, get_cookie_file_session_state,
    has_youtube_cookie_lines, import_cookies_from_browser, run_option_a_sign_in,
    sign_out_youtube_in_dir, BrowserImportResult, CompanionAuthController, CompanionAuthState,
};
use hybrid_media_services::ytdlp::{
    extract_youtube_id, extract_youtube_playlist_id, get_youtube_playlist, get_youtube_video_info,
    YoutubeInfoCache, YoutubeVideoInfo,
};
use hybrid_media_services::ytdlp_updater::{
    check_for_ytdlp_update, download_ytdlp_update, get_ytdlp_version, ReleaseCheckCache,
    UpdateCheckResult,
};
use hybrid_mpv_engine::{
    extract_video_dimensions, MpvApi, MpvClient, MpvEvent, MpvStartupConfig, ScreenshotFormat,
    ScreenshotMode, SeekThumbnailer,
};
use hybrid_shaders::{
    parse_hex_rgb, DitherWavesConfig, FaultyTerminalConfig, GridAspectRatio, GridMotionConfig,
    ParticlesConfig, QualityTier, ThemeMode, WelcomeBackgroundKind,
};
use hybrid_ui::motion::MotionProfile;
use hybrid_ui::primitives::HitTarget;
use hybrid_ui::state::{
    AbLoopState, BgControlInputType, CarouselMode, ColorPickerTarget, DropdownKind, ModalId,
    RecordingIndicatorMode, RepeatMode as UiRepeatMode, ToastItem, ToastKind, TrackOption, UiState,
    BG_CONTROLS_CATALOG, SPEED_PRESETS,
};
use hybrid_win32_shell::{
    trash_file, CompanionAuthAction, CompanionAuthHandle, CompanionAuthOptions, SmtcCommand,
    SmtcController, SmtcMetadata, SmtcPlaybackStatus, SmtcTimeline,
};
use serde_json::{json, Value};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

const YOUTUBE_CACHE_TTL: Duration = Duration::from_secs(30 * 60);
const RELEASE_CACHE_TTL: Duration = Duration::from_secs(15 * 60);
const MAX_SESSION_SCREENSHOTS: usize = 100;

/// Staged deleted screenshot record for `Ctrl+Z` / carousel undo recovery.
#[derive(Debug, Clone)]
pub struct DeletedScreenshotRecord {
    pub original_path: PathBuf,
    pub staged_path: PathBuf,
    pub file_name: String,
}

/// Active session screenshot entry shown in the preview card & carousel.
#[derive(Debug, Clone)]
pub struct SessionScreenshot {
    pub file_path: PathBuf,
    pub file_name: String,
    pub timestamp_ms: u64,
}

/// Asynchronous task completions routed back into the UI/event loop thread.
#[derive(Debug)]
pub enum BridgeAsyncMsg {
    SingleScreenshotCaptured(Result<PathBuf, String>),
    BurstFrameCaptured {
        session_id: String,
        result: Result<PathBuf, String>,
    },
    ClipFinished(Result<ClipResult, String>),
    YoutubeInfoReady {
        video_id: String,
        result: Result<Box<YoutubeVideoInfo>, String>,
    },
    YoutubePlaylistReady {
        entries: Vec<String>,
    },
    YtdlpUpdateChecked(Result<UpdateCheckResult, String>),
    YtdlpBackgroundUpdateChecked(UpdateCheckResult),
    YtdlpUpdateInstalled(Result<String, String>),
    YtdlpVersionResolved(Option<String>),
    YoutubeAuthStatus {
        text: String,
        finishing: bool,
    },
    YoutubeAuthFinished(Result<bool, String>),
    YoutubeBrowserImportFinished(BrowserImportResult),
    ThumbnailReady {
        key: String,
        path: PathBuf,
    },
    SeekThumbnailDecoded {
        cache_key: String,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
    SeekThumbnailFailed {
        cache_key: String,
    },
    ScreenshotImageDecoded {
        key: String,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
    ScreenshotImageFailed {
        key: String,
    },
    Toast {
        kind: ToastKind,
        message: String,
    },
}

type WakeCallback = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone)]
struct WakeAsyncSender {
    tx: mpsc::UnboundedSender<BridgeAsyncMsg>,
    wake: Arc<std::sync::Mutex<Option<WakeCallback>>>,
}

impl WakeAsyncSender {
    fn send(&self, msg: BridgeAsyncMsg) -> Result<(), mpsc::error::SendError<BridgeAsyncMsg>> {
        let res = self.tx.send(msg);
        if res.is_ok() {
            if let Ok(guard) = self.wake.lock() {
                if let Some(cb) = guard.as_ref() {
                    cb();
                }
            }
        }
        res
    }
}

/// Active text input focused by pointer click, keyboard Tab, or AccessKit `Focus` action.
pub use hybrid_ui::state::FocusedTextField;

/// Main-thread coordinator synchronizing `Database`, `MpvClient`, `UiState`,
/// media services, SMTC, and native Win32 companion auth.
pub struct MediaBridge {
    pub user_data_dir: PathBuf,
    pub db_path: PathBuf,
    pub db: Database,
    pub ui: UiState,
    pub playlist: PlaylistState,
    pub equalizer: EqualizerState,
    pub subtitle_style: SubtitleStyle,

    pub rt: Arc<Runtime>,
    pub mpv_api: Option<Arc<MpvApi>>,
    pub mpv: Option<Arc<MpvClient>>,
    pub mpv_rx: Option<tokio::sync::broadcast::Receiver<MpvEvent>>,
    pub thumbnailer: Option<Arc<SeekThumbnailer>>,

    pub smtc: Option<SmtcController>,
    pub smtc_rx: Option<std::sync::mpsc::Receiver<SmtcCommand>>,

    async_tx: WakeAsyncSender,
    async_rx: mpsc::UnboundedReceiver<BridgeAsyncMsg>,

    youtube_cache: Arc<YoutubeInfoCache>,
    release_cache: Arc<ReleaseCheckCache>,

    // Active playback metadata & track lists
    pub current_file: Option<String>,
    pub current_title: Option<String>,
    pub current_uploader: Option<String>,
    pub is_youtube: bool,
    pub youtube_heights: Vec<u32>,
    pub current_youtube_quality: String,
    pub audio_tracks: Vec<MediaTrack>,
    pub subtitle_tracks: Vec<MediaTrack>,
    pub current_aid: Option<i64>,
    pub current_sid: Option<i64>,
    pub video_dimensions: Option<(u32, u32)>,
    pub video_fps: Option<f64>,
    pub playback_session_id: u64,
    pub pending_resume_seek: Option<(u64, String, f64)>,
    pub last_skip_at: Option<Instant>,
    pub eq_debounce_deadline: Option<Instant>,
    pub sub_visible: bool,

    // Screenshot / burst / clip state
    pub single_screenshot_stack: Vec<PathBuf>,
    pub deleted_screenshot_stack: Vec<DeletedScreenshotRecord>,
    pub session_screenshots: Vec<SessionScreenshot>,
    pub burst_session_id: Option<String>,
    pub burst_seq: u32,
    pub burst_in_flight: bool,
    pub clip_start_secs: Option<f64>,
    pub clip_started_at: Option<Instant>,
    pub active_clip_file: Option<String>,

    // Companion auth & window focus/exit flags
    pub owner_hwnd: Option<isize>,
    pub yt_auto_retry_used: bool,
    pub yt_retry_deadline: Option<(Instant, String)>,
    pub last_failed_load_url: Option<String>,
    auth_controller: Option<Arc<CompanionAuthController>>,
    auth_window: Option<CompanionAuthHandle>,
    next_toast_id: u64,
    last_resume_save: Instant,
    pub focused_input: Option<FocusedTextField>,
    pub is_dragging_progress: bool,
    pub quit_requested: bool,

    // Decoded thumbnail & screenshot RGBA caches + GridMotion custom pictures
    decoded_seek_thumbs: HashMap<String, (u32, u32, Vec<u8>)>,
    seek_thumb_order: VecDeque<String>,
    pending_seek_thumbs: HashSet<String>,
    decoded_screenshots: HashMap<String, (u32, u32, Vec<u8>)>,
    screenshot_order: VecDeque<String>,
    pending_screenshots: HashSet<String>,
    pub gridmotion_custom_pictures: Vec<String>,
    pub gridmotion_pictures_gen: u64,
    pub gridmotion_zoom_save_deadline: Option<Instant>,
}

impl MediaBridge {
    /// Initializes persistence (`hybrid-player-db.json`), populates `UiState`, and prepares async channels.
    pub fn new(user_data_dir: PathBuf, rt: Arc<Runtime>) -> Self {
        let _ = std::fs::create_dir_all(&user_data_dir);
        let db_path = user_data_dir.join("hybrid-player-db.json");
        let loaded = load_database(&db_path);
        let db = loaded.db;

        let (raw_tx, async_rx) = mpsc::unbounded_channel();
        let async_tx = WakeAsyncSender {
            tx: raw_tx,
            wake: Arc::new(std::sync::Mutex::new(None)),
        };
        let mut bridge = Self {
            user_data_dir,
            db_path,
            db,
            ui: UiState::default(),
            playlist: PlaylistState::new(),
            equalizer: EqualizerState::default(),
            subtitle_style: SubtitleStyle::default(),
            rt,
            mpv_api: None,
            mpv: None,
            mpv_rx: None,
            thumbnailer: None,
            smtc: None,
            smtc_rx: None,
            async_tx,
            async_rx,
            youtube_cache: Arc::new(YoutubeInfoCache::new(YOUTUBE_CACHE_TTL)),
            release_cache: Arc::new(ReleaseCheckCache::new(RELEASE_CACHE_TTL)),
            current_file: None,
            current_title: None,
            current_uploader: None,
            is_youtube: false,
            youtube_heights: Vec::new(),
            current_youtube_quality: "auto".to_string(),
            audio_tracks: Vec::new(),
            subtitle_tracks: Vec::new(),
            current_aid: None,
            current_sid: None,
            video_dimensions: None,
            video_fps: None,
            playback_session_id: 0,
            pending_resume_seek: None,
            last_skip_at: None,
            eq_debounce_deadline: None,
            sub_visible: true,
            single_screenshot_stack: Vec::new(),
            deleted_screenshot_stack: Vec::new(),
            session_screenshots: Vec::new(),
            burst_session_id: None,
            burst_seq: 0,
            burst_in_flight: false,
            clip_start_secs: None,
            clip_started_at: None,
            active_clip_file: None,
            owner_hwnd: None,
            yt_auto_retry_used: false,
            yt_retry_deadline: None,
            last_failed_load_url: None,
            auth_controller: None,
            auth_window: None,
            next_toast_id: 1,
            last_resume_save: Instant::now(),
            focused_input: None,
            is_dragging_progress: false,
            quit_requested: false,
            decoded_seek_thumbs: HashMap::new(),
            seek_thumb_order: VecDeque::new(),
            pending_seek_thumbs: HashSet::new(),
            decoded_screenshots: HashMap::new(),
            screenshot_order: VecDeque::new(),
            pending_screenshots: HashSet::new(),
            gridmotion_custom_pictures: Vec::new(),
            gridmotion_pictures_gen: 0,
            gridmotion_zoom_save_deadline: None,
        };

        bridge.ui.default_screenshot_dir = bridge
            .default_screenshot_dir()
            .to_string_lossy()
            .into_owned();
        bridge.sync_preferences_to_ui();
        bridge.refresh_youtube_cookie_state();
        if let Some(ytdlp_bin) = bridge.resolve_ytdlp_bin() {
            let tx = bridge.async_tx.clone();
            let ver_bin = ytdlp_bin.clone();
            bridge.rt.spawn(async move {
                let ver = get_ytdlp_version(&ver_bin).await;
                let _ = tx.send(BridgeAsyncMsg::YtdlpVersionResolved(ver));
            });

            // Match Electron `startYtDlpBackgroundCheck(win, { delayMs = 6000 })` (`main.js:2503`, `ytdlp-updater.js:343`)
            let bg_tx = bridge.async_tx.clone();
            let bg_cache = Arc::clone(&bridge.release_cache);
            let bg_bin = Some(ytdlp_bin);
            bridge.rt.spawn(async move {
                tokio::time::sleep(Duration::from_secs(6)).await;
                let res = check_for_ytdlp_update(bg_bin.as_deref(), Some(&bg_cache), false).await;
                let _ = bg_tx.send(BridgeAsyncMsg::YtdlpBackgroundUpdateChecked(res));
            });
        }
        if let Some(diag) = loaded.diagnostic {
            if let Some(detail) = diag.detail {
                eprintln!("[hybrid-db] {}: {detail}", diag.code);
            }
        }
        bridge
    }

    pub fn set_wake_callback(&mut self, wake: WakeCallback) {
        if let Ok(mut guard) = self.async_tx.wake.lock() {
            *guard = Some(wake);
        }
    }

    fn resolve_ytdlp_bin(&self) -> Option<PathBuf> {
        let search = BinarySearchPaths::from_env();
        resolve_yt_dlp_binary(
            Some(&self.user_data_dir),
            search.resources_dir.as_deref(),
            search.exec_path.as_deref(),
            search.app_dir.as_deref(),
            ResolveOptions::for_runtime(),
        )
    }

    fn resolve_ffmpeg_bin(&self) -> Option<PathBuf> {
        let search = BinarySearchPaths::from_env();
        resolve_ffmpeg_binary(
            search.resources_dir.as_deref(),
            search.exec_path.as_deref(),
            search.app_dir.as_deref(),
            ResolveOptions::for_runtime(),
        )
    }

    fn build_mpv_startup_config(&self, libmpv_path: Option<PathBuf>) -> MpvStartupConfig {
        let ytdl_path = self.resolve_ytdlp_bin();
        let cookies_path = get_cookie_file_path(&self.user_data_dir);
        let cookies_opt = cookies_path.exists().then_some(cookies_path);

        let screenshot_dir = if !self.db.preferences.screenshot_dir.trim().is_empty() {
            PathBuf::from(&self.db.preferences.screenshot_dir)
        } else {
            self.default_screenshot_dir()
        };

        let fmt = ScreenshotFormat::from_str_lossy(&self.db.preferences.screenshot_format)
            .unwrap_or(ScreenshotFormat::Jpg);

        MpvStartupConfig {
            libmpv_path,
            ytdl_path,
            cookies_path: cookies_opt,
            enable_ytdl_raw_options: true,
            screenshot_dir,
            screenshot_format: fmt,
            auto_organize_screenshots: self.db.preferences.auto_organize_screenshots,
            ..MpvStartupConfig::default()
        }
    }

    /// Initializes live `MpvClient`, `SeekThumbnailer`, and observed property tracking when `libmpv-2.dll` is present,
    /// falling back to external `mpv.exe --wid=<hwnd>` IPC subprocess when `libmpv` is unavailable or fails.
    pub fn init_mpv_engine(&mut self) -> Result<(), String> {
        self.init_mpv_engine_with_wid(self.owner_hwnd)
    }

    pub fn init_mpv_engine_with_wid(&mut self, wid_hwnd: Option<isize>) -> Result<(), String> {
        let search = BinarySearchPaths::from_env();
        let opts = ResolveOptions::for_runtime();
        let libmpv_attempt: Result<(Arc<MpvApi>, Arc<MpvClient>), String> = (|| {
            let libmpv_path = resolve_lib_mpv_with_options(
                search.resources_dir.as_deref(),
                search.exec_path.as_deref(),
                search.app_dir.as_deref(),
                opts,
            )
            .ok_or_else(|| "libmpv-2.dll not found in bundled or app paths".to_string())?;
            let cfg = self.build_mpv_startup_config(Some(libmpv_path.clone()));
            let api = MpvApi::load(&libmpv_path).map_err(|e| format!("MpvApi::load: {e}"))?;
            let client = MpvClient::new(Arc::clone(&api), &cfg)
                .map_err(|e| format!("MpvClient::new: {e}"))?;
            Ok((api, client))
        })();

        match libmpv_attempt {
            Ok((api, client)) => {
                let rx = client.subscribe();
                let thumb = SeekThumbnailer::new(Arc::clone(&api)).ok().map(Arc::new);
                self.mpv_api = Some(api);
                self.mpv = Some(Arc::clone(&client));
                self.mpv_rx = Some(rx);
                self.thumbnailer = thumb;
                self.apply_initial_mpv_preferences(&client);
                Ok(())
            }
            Err(lib_err) => {
                if let Some(hwnd) = wid_hwnd {
                    if self.fallback_to_mpv_subprocess(hwnd).is_ok() {
                        return Ok(());
                    }
                }
                Err(lib_err)
            }
        }
    }

    /// Falls back to external `mpv.exe --wid=<hwnd>` IPC subprocess (`mpv-process.js:414-510`).
    pub fn fallback_to_mpv_subprocess(&mut self, wid_hwnd: isize) -> Result<(), String> {
        let search = BinarySearchPaths::from_env();
        let mpv_exe = resolve_mpv_binary(
            search.resources_dir.as_deref(),
            search.exec_path.as_deref(),
            search.app_dir.as_deref(),
            ResolveOptions::for_runtime(),
        )
        .ok_or_else(|| "mpv.exe not found for --wid fallback".to_string())?;

        if let Some(old) = self.mpv.take() {
            old.shutdown();
        }
        self.mpv_api = None;
        self.thumbnailer = None;

        let cfg = self.build_mpv_startup_config(None);
        let client = MpvClient::new_subprocess(&mpv_exe, Some(wid_hwnd), &cfg)
            .map_err(|e| format!("mpv.exe --wid fallback failed: {e}"))?;
        let rx = client.subscribe();
        self.mpv = Some(Arc::clone(&client));
        self.mpv_rx = Some(rx);
        self.apply_initial_mpv_preferences(&client);
        Ok(())
    }

    pub fn set_owner_hwnd(&mut self, hwnd: isize) {
        self.owner_hwnd = if hwnd != 0 { Some(hwnd) } else { None };
    }

    /// Attaches Windows SMTC (`SystemMediaTransportControls`) to the primary `HWND`.
    pub fn attach_smtc(&mut self, hwnd: windows::Win32::Foundation::HWND) {
        self.set_owner_hwnd(hwnd.0 as isize);
        if let Ok((ctrl, rx)) = SmtcController::attach(hwnd) {
            self.smtc = Some(ctrl);
            self.smtc_rx = Some(rx);
        }
    }

    pub fn default_screenshot_dir(&self) -> PathBuf {
        self.user_data_dir.join("screenshots")
    }

    /// Applies persisted `Database::preferences` into `UiState`, `EqualizerState`, and `SubtitleStyle`.
    pub fn sync_preferences_to_ui(&mut self) {
        let prefs = self.db.preferences.clone();
        self.ui.theme = match prefs.theme.as_str() {
            "oled" => ThemeMode::Oled,
            "light" => ThemeMode::Light,
            _ => ThemeMode::Dark,
        };
        self.ui.accent_hex = prefs.accent_color.clone();
        self.ui.font_config.brand_font_enabled = prefs.brand_font_enabled;
        self.ui.motion_profile = match prefs.motion_profile.as_str() {
            "reduced" => MotionProfile::Reduced,
            "showcase" => MotionProfile::Showcase,
            _ => MotionProfile::Balanced,
        };
        self.ui.welcome_background = match prefs.welcome_background.as_str() {
            "particles" => WelcomeBackgroundKind::Particles,
            "faulty" => WelcomeBackgroundKind::Faulty,
            "gridmotion" => WelcomeBackgroundKind::Gridmotion,
            "none" => WelcomeBackgroundKind::None,
            _ => WelcomeBackgroundKind::Dither,
        };
        self.ui.welcome_quality = match prefs.welcome_quality.as_str() {
            "low" => QualityTier::Low,
            "high" => QualityTier::High,
            "custom" => QualityTier::Custom,
            _ => QualityTier::Medium,
        };

        self.apply_saved_bg_configs_to_ui();

        let preset = EqualizerPreset::from_str_name(&prefs.equalizer_preset)
            .unwrap_or(EqualizerPreset::Flat);
        let bands = EqualizerState::normalize_bands(&prefs.equalizer_bands);
        self.equalizer = EqualizerState { preset, bands };
        self.ui.equalizer = self.equalizer.clone();

        self.ui.auto_resume = prefs.auto_resume;
        self.ui.double_click_fullscreen = prefs.double_click_fullscreen;
        self.ui.seek_step_secs = prefs.seek_step.clamp(1, 60);
        self.ui.volume_boost_enabled = prefs.volume_boost;
        let max_vol = if prefs.volume_boost { 200.0 } else { 100.0 };
        self.ui.volume_pct = (prefs.volume * 100.0).clamp(0.0, max_vol).round() as u32;
        self.ui.show_unlock_button = prefs.show_unlock_button;
        self.ui.screenshot_dir = prefs.screenshot_dir.clone();
        self.ui.screenshot_format = prefs.screenshot_format.clone();
        self.ui.auto_organize_screenshots = prefs.auto_organize_screenshots;
        self.ui.screenshot_mouse_button = prefs.screenshot_mouse_button;
        self.ui.custom_shortcuts = prefs.custom_shortcuts.clone();

        self.subtitle_style = SubtitleStyle {
            font_size: prefs.sub_font_size.round().clamp(12.0, 96.0) as u16,
            font_family: DEFAULT_SUBTITLE_FONT_FAMILY.to_string(),
            font_color: prefs.sub_font_color.clone(),
            bg_color: prefs.sub_bg_color.clone(),
            bg_opacity: prefs.sub_bg_opacity,
        }
        .sanitized();
        self.ui.subtitle_font_size = self.subtitle_style.font_size as u32;
        self.ui.subtitle_bg_opacity_pct = (self.subtitle_style.bg_opacity * 100.0).round() as u32;
        self.ui.subtitle_font_color = self.subtitle_style.font_color.clone();
        self.ui.subtitle_bg_color = self.subtitle_style.bg_color.clone();

        self.ui.recent_files = self.db.history.iter().take(5).cloned().collect();
    }

    fn apply_saved_bg_configs_to_ui(&mut self) {
        let q = self.ui.welcome_quality;
        self.ui.dither_cfg = DitherWavesConfig::for_quality(q);
        self.ui.particles_cfg = ParticlesConfig::for_quality(q);
        self.ui.faulty_cfg = FaultyTerminalConfig::for_quality(q);
        self.ui.gridmotion_cfg = GridMotionConfig::for_quality(q);
        self.ui.gridmotion_cfg.brand_font_enabled = self.ui.font_config.brand_font_enabled;

        // Always load customPictures for GridMotion (settings.js:2692 preserves pictures across presets).
        if let Some(raw) = &self.db.preferences.bg_opts_gridmotion {
            if let Ok(v) = serde_json::from_str::<Value>(raw) {
                if let Some(arr) = v.get("customPictures").and_then(|x| x.as_array()) {
                    let pics: Vec<String> = arr
                        .iter()
                        .filter_map(|item| item.as_str().map(str::to_string))
                        .filter(|s| !s.trim().is_empty())
                        .collect();
                    if pics != self.gridmotion_custom_pictures {
                        self.gridmotion_custom_pictures = pics.clone();
                        self.ui.gridmotion_custom_pictures = pics;
                        self.gridmotion_pictures_gen = self.gridmotion_pictures_gen.wrapping_add(1);
                    }
                }
            }
        }

        if q != QualityTier::Custom {
            return;
        }

        if let Some(raw) = &self.db.preferences.bg_opts_dither {
            if let Ok(v) = serde_json::from_str::<Value>(raw) {
                if let Some(s) = v.get("speed").and_then(|x| x.as_f64()) {
                    self.ui.dither_cfg.wave_speed = s as f32;
                }
                if let Some(s) = v.get("frequency").and_then(|x| x.as_f64()) {
                    self.ui.dither_cfg.wave_frequency = s as f32;
                }
                if let Some(s) = v.get("amplitude").and_then(|x| x.as_f64()) {
                    self.ui.dither_cfg.wave_amplitude = s as f32;
                }
                if let Some(rgb) = v
                    .get("color")
                    .and_then(|x| x.as_str())
                    .and_then(parse_hex_rgb)
                {
                    self.ui.dither_cfg.wave_color = rgb;
                }
                if let Some(rgb) = v
                    .get("bgColor")
                    .and_then(|x| x.as_str())
                    .and_then(parse_hex_rgb)
                {
                    self.ui.dither_cfg.bg_color = rgb;
                }
                if let Some(p) = v.get("pixelSize").and_then(|x| x.as_f64()) {
                    self.ui.dither_cfg.pixel_size = p as f32;
                }
                if let Some(c) = v.get("colorNum").and_then(|x| x.as_f64()) {
                    self.ui.dither_cfg.color_num = c as f32;
                }
            }
        }
        if let Some(raw) = &self.db.preferences.bg_opts_particles {
            if let Ok(v) = serde_json::from_str::<Value>(raw) {
                if let Some(c) = v.get("count").and_then(|x| x.as_u64()) {
                    self.ui.particles_cfg.particle_count = c as u32;
                }
                if let Some(s) = v.get("speed").and_then(|x| x.as_f64()) {
                    self.ui.particles_cfg.speed = s as f32;
                }
                if let Some(s) = v.get("spread").and_then(|x| x.as_f64()) {
                    self.ui.particles_cfg.particle_spread = s as f32;
                }
                if let Some(rgb) = v
                    .get("color")
                    .and_then(|x| x.as_str())
                    .and_then(parse_hex_rgb)
                {
                    self.ui.particles_cfg.particle_colors = vec![rgb];
                }
                if let Some(s) = v.get("size").and_then(|x| x.as_f64()) {
                    self.ui.particles_cfg.particle_base_size = s as f32;
                }
                if let Some(a) = v.get("alpha").and_then(|x| x.as_bool()) {
                    self.ui.particles_cfg.alpha_particles = a;
                }
            }
        }
        if let Some(raw) = &self.db.preferences.bg_opts_faulty {
            if let Ok(v) = serde_json::from_str::<Value>(raw) {
                if let Some(s) = v.get("scale").and_then(|x| x.as_f64()) {
                    self.ui.faulty_cfg.scale = s as f32;
                }
                if let Some(g) = v.get("glitch").and_then(|x| x.as_f64()) {
                    self.ui.faulty_cfg.glitch_amount = g as f32;
                }
                if let Some(s) = v.get("scanlines").and_then(|x| x.as_f64()) {
                    self.ui.faulty_cfg.scanline_intensity = s as f32;
                }
                if let Some(f) = v.get("flicker").and_then(|x| x.as_f64()) {
                    self.ui.faulty_cfg.flicker_amount = f as f32;
                }
                if let Some(a) = v.get("aberration").and_then(|x| x.as_f64()) {
                    self.ui.faulty_cfg.chromatic_aberration = a as f32;
                }
                if let Some(c) = v.get("curvature").and_then(|x| x.as_f64()) {
                    self.ui.faulty_cfg.curvature = c as f32;
                }
                if let Some(rgb) = v
                    .get("tint")
                    .and_then(|x| x.as_str())
                    .and_then(parse_hex_rgb)
                {
                    self.ui.faulty_cfg.tint = rgb;
                }
                if let Some(b) = v.get("brightness").and_then(|x| x.as_f64()) {
                    self.ui.faulty_cfg.brightness = b as f32;
                }
            }
        }
        if let Some(raw) = &self.db.preferences.bg_opts_gridmotion {
            if let Ok(v) = serde_json::from_str::<Value>(raw) {
                if let Some(a) = v.get("autoMove").and_then(|x| x.as_bool()) {
                    self.ui.gridmotion_cfg.auto_move = a;
                }
                if let Some(s) = v.get("scale").and_then(|x| x.as_f64()) {
                    self.ui.gridmotion_cfg.scale = s as f32;
                }
                if let Some(ar) = v.get("aspectRatio").and_then(|x| x.as_str()) {
                    self.ui.gridmotion_cfg.aspect_ratio = GridAspectRatio::parse_str(ar);
                }
                if let Some(s) = v.get("speed").and_then(|x| x.as_f64()) {
                    self.ui.gridmotion_cfg.speed = s as f32;
                }
                if let Some(m) = v.get("maxMove").and_then(|x| x.as_f64()) {
                    self.ui.gridmotion_cfg.max_move = m as f32;
                }
                if let Some(rgb) = v
                    .get("gradientColor")
                    .and_then(|x| x.as_str())
                    .and_then(parse_hex_rgb)
                {
                    self.ui.gridmotion_cfg.gradient_color = rgb;
                }
            }
        }
    }

    fn apply_initial_mpv_preferences(&self, client: &MpvClient) {
        let _ = client.set_volume(self.ui.volume_pct as f64);
        let af = self.equalizer.mpv_af_value();
        let _ = client.set_property_string("af", &af);
        self.apply_subtitle_style_to_mpv(client);
    }

    fn apply_subtitle_style_to_mpv(&self, client: &MpvClient) {
        let style = &self.subtitle_style;
        let ass_override = if style.has_custom_style() {
            "force"
        } else {
            "scale"
        };
        let _ = client.set_property_string("sub-ass-override", ass_override);
        let _ = client.set_property_f64("sub-font-size", style.font_size as f64);
        let _ = client.set_property_string("sub-font", &style.font_family);
        let _ = client.set_property_string("sub-color", &style.font_color);
        let _ = client.set_property_string("sub-back-color", &style.bg_color_with_alpha());
    }

    pub fn save_db(&self) {
        let _ = save_database_atomic(&self.db, &self.db_path);
    }

    pub fn push_toast(&mut self, kind: ToastKind, message: impl Into<String>) {
        let id = self.next_toast_id;
        self.next_toast_id = self.next_toast_id.saturating_add(1);
        self.ui.push_toast(ToastItem::new(id, kind, message));
    }

    pub fn refresh_youtube_cookie_state(&mut self) {
        let cookie_path = get_cookie_file_path(&self.user_data_dir);
        let state = get_cookie_file_session_state(&cookie_path);
        self.ui.youtube_signed_in = state.logged_in;
    }

    /// Opens files or folders (`scan_media_folder` expansion), updates playlist, and starts playback.
    pub fn open_files(&mut self, inputs: &[PathBuf], replace_playlist: bool, play_first: bool) {
        let mut resolved_items: Vec<(String, bool)> = Vec::new();
        for path in inputs {
            let raw = path.to_string_lossy().into_owned();
            if is_unc_or_network_path(&raw) {
                self.push_toast(
                    ToastKind::Error,
                    format!("Rejected network/UNC path: {raw}"),
                );
                continue;
            }
            if path.is_dir() {
                match scan_folder_for_media(path) {
                    Ok(stats) => {
                        for f in stats.files {
                            resolved_items.push((f.to_string_lossy().into_owned(), false));
                        }
                    }
                    Err(err) => {
                        self.push_toast(ToastKind::Error, format!("Folder scan failed: {err}"));
                    }
                }
            } else if path.exists() {
                resolved_items.push((raw, false));
            }
        }

        if resolved_items.is_empty() {
            return;
        }

        self.yt_auto_retry_used = false;
        self.yt_retry_deadline = None;
        if replace_playlist {
            self.playlist.clear();
        }
        let was_empty = self.playlist.items.is_empty();
        let first_added = self.playlist.add_paths(resolved_items);
        self.sync_playlist_to_ui();

        if play_first || was_empty {
            let idx = if replace_playlist {
                Some(0)
            } else {
                first_added
            };
            if let Some(i) = idx {
                self.play_playlist_index(i);
            }
        }
    }

    /// Opens a single HTTP/HTTPS/RTSP/RTMP/SRT/YTDL/YouTube stream URL or media path.
    pub fn open_url_or_path(&mut self, raw: &str) {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return;
        }
        let lower = trimmed.to_ascii_lowercase();
        let normalized_url = normalize_network_media_url(trimmed);
        let is_ytdl = lower.starts_with("ytdl://") && lower.len() > "ytdl://".len();
        if normalized_url.is_some() || is_ytdl {
            self.yt_auto_retry_used = false;
            self.yt_retry_deadline = None;
            let open_target = normalized_url.unwrap_or_else(|| trimmed.to_string());
            if let Some(playlist_id) = extract_youtube_playlist_id(&open_target) {
                let _ = playlist_id;
                self.fetch_youtube_playlist_async(open_target.clone());
            }
            self.playlist.clear();
            let item = PlaylistItem::from_path(&open_target, true);
            self.playlist.add_item(item);
            self.sync_playlist_to_ui();
            self.play_playlist_index(0);
        } else if lower.contains("://") && !lower.starts_with("file://") {
            self.push_toast(
                ToastKind::Error,
                "Enter a valid HTTP, RTSP, RTMP, SRT, or ytdl:// URL",
            );
        } else {
            self.open_files(&[PathBuf::from(trimmed)], true, true);
        }
    }

    pub fn play_playlist_index(&mut self, index: usize) {
        let Some(item) = self.playlist.set_current_index(index).cloned() else {
            return;
        };
        self.sync_playlist_to_ui();
        self.load_media_item(&item);
    }

    fn formatted_window_title(&self) -> String {
        match (
            self.current_title.as_deref(),
            self.current_uploader.as_deref(),
        ) {
            (Some(title), Some(uploader)) if self.is_youtube && !uploader.trim().is_empty() => {
                format!("{title} — {uploader} — Hybrid Player")
            }
            (Some(title), _) if !title.trim().is_empty() => {
                format!("{title} — Hybrid Player")
            }
            _ => "Hybrid Player".to_string(),
        }
    }

    fn cancel_clip_recording(&mut self, notify: bool) {
        if self.clip_start_secs.take().is_some() || self.clip_started_at.is_some() {
            self.clip_started_at = None;
            self.active_clip_file = None;
            self.ui.recording_indicator = RecordingIndicatorMode::Hidden;
            if notify {
                self.push_toast(ToastKind::Info, "Clip recording cancelled");
            }
        }
    }

    fn load_media_item(&mut self, item: &PlaylistItem) {
        self.save_current_resume_position();
        self.cancel_clip_recording(true);
        self.playback_session_id = self.playback_session_id.saturating_add(1);
        self.pending_resume_seek = None;
        self.current_file = Some(item.path.clone());
        self.current_title = Some(item.title.clone());
        self.current_uploader = None;
        self.video_dimensions = None;
        self.video_fps = None;
        self.ui.video_aspect_ratio = 16.0 / 9.0;
        self.ui.welcome_active = false;
        if self.ui.active_modal == Some(ModalId::BgSettings)
            || self.ui.closing_modal == Some(ModalId::BgSettings)
        {
            self.ui.close_modals();
        }
        if self.ui.bg_accordion_open || self.ui.anim.bg_accordion_closing {
            self.ui.bg_accordion_open = false;
            self.ui.anim.bg_accordion_closing = false;
            self.ui.anim.bg_accordion_t = 1.0;
        }
        self.ui.loading_visible = true;
        self.ui.center_play_visible = false;
        self.ui.is_live_stream = false;
        self.ui.current_secs = 0.0;
        self.ui.duration_secs = 0.0;
        self.ui.buffered_secs = 0.0;
        self.ui.chapter_positions_norm.clear();
        self.ui.chapter_times_secs.clear();
        self.ui.chapter_titles.clear();
        self.ui.current_chapter = None;
        self.ui.hover_thumb_has_frame = false;
        self.ui.hover_thumb_loading = false;
        self.sub_visible = true;
        self.ui.window_title = format!("{} — Hybrid Player", item.title);
        self.ui.ab_loop = AbLoopState::Off;
        self.ui.audio_delay_secs = 0.0;
        self.audio_tracks.clear();
        self.subtitle_tracks.clear();
        self.ui.audio_tracks.clear();
        self.ui.subtitle_tracks.clear();

        let vid_id = extract_youtube_id(&item.path);
        self.is_youtube = vid_id.is_some();
        self.ui.is_youtube = self.is_youtube;
        self.youtube_heights.clear();
        self.current_youtube_quality = "auto".to_string();
        self.ui.youtube_quality_label = "Auto".to_string();

        // Only allow resume when loading a standalone item (`playlist.len() == 1`);
        // navigating prev/next in a multi-item playlist starts from 0.
        let allow_resume = self.playlist.items.len() <= 1;
        if allow_resume && self.ui.auto_resume {
            let resume_t = self.db.get_resume_position(&item.path);
            if resume_t > 5.0 {
                self.pending_resume_seek =
                    Some((self.playback_session_id, item.path.clone(), resume_t));
            }
        }

        self.db.add_history(&item.path, Some(&item.title), 0.0);
        self.ui.recent_files = self.db.history.iter().take(5).cloned().collect();

        if let Some(saved_speed) = self.db.get_speed(&item.path) {
            self.ui.speed = saved_speed;
            if let Some(mpv) = &self.mpv {
                let _ = mpv.set_speed(saved_speed);
            }
        }
        let saved_sub_delay_ms = self.db.get_subtitle_delay(&item.path);
        let sub_delay_sec = saved_sub_delay_ms / 1000.0;
        self.ui.subtitle_delay_secs = sub_delay_sec;
        if let Some(mpv) = &self.mpv {
            let _ = mpv.set_audio_delay(0.0);
            let _ = mpv.set_property_string("ytdl-format", "bestvideo+bestaudio/best");
            let _ = mpv.set_sub_delay(sub_delay_sec);
            let _ = mpv.clear_ab_loop();
            let mpv_clone = Arc::clone(mpv);
            let path_clone = item.path.clone();
            let tx = self.async_tx.clone();
            self.rt.spawn(async move {
                if let Err(err) = mpv_clone.load_file(&path_clone).await {
                    let _ = tx.send(BridgeAsyncMsg::Toast {
                        kind: ToastKind::Error,
                        message: format!("Failed to load media: {err}"),
                    });
                }
            });
        }

        if let Some(id) = vid_id {
            self.fetch_youtube_info_async(id, item.path.clone());
        }

        self.update_smtc_metadata(&item.title);
        self.save_db();
    }

    pub fn add_companion_subtitles(&mut self, subs: &[PathBuf]) {
        let Some(mpv) = &self.mpv else {
            return;
        };
        for p in subs {
            let _ = mpv.add_sub_file(&p.to_string_lossy());
        }
        if !subs.is_empty() {
            self.push_toast(
                ToastKind::Info,
                format!("Loaded {} external subtitle file(s)", subs.len()),
            );
        }
    }

    fn fetch_youtube_info_async(&self, video_id: String, url: String) {
        let Some(ytdl_bin) = self.resolve_ytdlp_bin() else {
            return;
        };
        let cookies = get_cookie_file_path(&self.user_data_dir);
        let cookies_opt = cookies.exists().then_some(cookies);
        let cache = Arc::clone(&self.youtube_cache);
        let tx = self.async_tx.clone();

        self.rt.spawn(async move {
            let res = get_youtube_video_info(&ytdl_bin, cookies_opt.as_deref(), &url, Some(&cache))
                .await
                .map(Box::new)
                .map_err(|e| e.to_string());
            let _ = tx.send(BridgeAsyncMsg::YoutubeInfoReady {
                video_id,
                result: res,
            });
        });
    }

    fn fetch_youtube_playlist_async(&self, url: String) {
        let Some(ytdl_bin) = self.resolve_ytdlp_bin() else {
            return;
        };
        let cookies = get_cookie_file_path(&self.user_data_dir);
        let cookies_opt = cookies.exists().then_some(cookies);
        let tx = self.async_tx.clone();

        self.rt.spawn(async move {
            if let Ok(pl) = get_youtube_playlist(&ytdl_bin, cookies_opt.as_deref(), &url).await {
                let urls: Vec<String> = pl.entries.into_iter().map(|e| e.url).collect();
                if !urls.is_empty() {
                    let _ = tx.send(BridgeAsyncMsg::YoutubePlaylistReady { entries: urls });
                }
            }
        });
    }

    pub fn sync_playlist_to_ui(&mut self) {
        self.ui.playlist.items = self
            .playlist
            .items
            .iter()
            .map(|item| PlaylistItemEntry {
                path: item.path.clone(),
                name: item.title.clone(),
            })
            .collect();
        self.ui.playlist.current_index = if self.playlist.current_index >= 0 {
            Some(self.playlist.current_index as usize)
        } else {
            None
        };
        self.ui.playlist.search_query = self.playlist.search_query.clone();
        self.ui.shuffle = self.playlist.shuffle;
        self.ui.repeat = match self.playlist.repeat {
            CoreRepeatMode::None => UiRepeatMode::None,
            CoreRepeatMode::All => UiRepeatMode::All,
            CoreRepeatMode::One => UiRepeatMode::One,
        };
        if let Some(smtc) = &self.smtc {
            let len = self.playlist.items.len();
            let idx = self.playlist.current_index;
            let wrap = self.playlist.repeat != CoreRepeatMode::None || self.playlist.shuffle;
            let can_next = len > 1 && (wrap || (idx >= 0 && ((idx as usize) + 1) < len));
            let can_prev = len > 1 && (wrap || idx > 0);
            let _ = smtc.set_navigation_enabled(can_next, can_prev);
        }
    }

    fn is_near_end(t: f64, dur: f64) -> bool {
        if dur <= 0.0 || !t.is_finite() || !dur.is_finite() {
            return false;
        }
        let remaining = (dur - t).max(0.0);
        if remaining <= 2.0 {
            return true;
        }
        let pct = t / dur;
        let threshold = (dur * 0.05).clamp(2.0, 30.0);
        pct >= 0.95 && remaining <= threshold
    }

    /// Saves current playback position (`autoResume`) if > 5s and not near EOF.
    /// Does not delete saved resume entries on periodic ticks (cleared on EOF only).
    pub fn save_current_resume_position(&mut self) {
        if !self.ui.auto_resume || self.ui.is_live_stream {
            return;
        }
        let Some(path) = self.current_file.as_deref() else {
            return;
        };
        let t = self.ui.current_secs;
        let dur = self.ui.duration_secs;
        if t > 5.0 && !Self::is_near_end(t, dur) {
            self.db.save_resume_position(path, t);
        }
    }

    fn try_execute_pending_resume(&mut self) {
        let Some((sess, path, resume_t)) = self.pending_resume_seek.clone() else {
            return;
        };
        if sess != self.playback_session_id || self.current_file.as_deref() != Some(path.as_str()) {
            self.pending_resume_seek = None;
            return;
        }
        self.pending_resume_seek = None;
        if let Some(mpv) = &self.mpv {
            let _ = mpv.seek(resume_t, "absolute+exact");
        }
        self.ui.current_secs = resume_t;
        self.push_toast(
            ToastKind::Info,
            format!("Resuming from {}", UiState::format_time(resume_t)),
        );
    }

    /// Drains `MpvEvent`, `SmtcCommand`, and async background task messages once per frame/tick.
    pub fn poll_events(&mut self) -> bool {
        let mut dirty = false;

        let mut mpv_events = Vec::new();
        if let Some(rx) = &mut self.mpv_rx {
            while let Ok(ev) = rx.try_recv() {
                mpv_events.push(ev);
            }
        }
        for ev in mpv_events {
            self.handle_mpv_event(ev);
            dirty = true;
        }

        let mut smtc_cmds = Vec::new();
        if let Some(rx) = &mut self.smtc_rx {
            while let Ok(cmd) = rx.try_recv() {
                smtc_cmds.push(cmd);
            }
        }
        for cmd in smtc_cmds {
            self.handle_smtc_command(cmd);
            dirty = true;
        }

        let mut async_msgs = Vec::new();
        while let Ok(msg) = self.async_rx.try_recv() {
            async_msgs.push(msg);
        }
        for msg in async_msgs {
            self.handle_async_msg(msg);
            dirty = true;
        }

        if let Some(start_secs) = self.clip_start_secs {
            let elapsed_ds = ((self.ui.current_secs - start_secs).max(0.0) * 10.0).floor() as u64;
            let next_mode = RecordingIndicatorMode::ClipActive { elapsed_ds };
            if self.ui.recording_indicator != next_mode {
                self.ui.recording_indicator = next_mode;
                dirty = true;
            }
        }

        if let Some(deadline) = self.eq_debounce_deadline {
            if Instant::now() >= deadline {
                self.flush_eq_debounce();
                dirty = true;
            }
        }

        if let Some(deadline) = self.gridmotion_zoom_save_deadline {
            if Instant::now() >= deadline {
                self.gridmotion_zoom_save_deadline = None;
                self.persist_bg_control_pref(WelcomeBackgroundKind::Gridmotion);
                dirty = true;
            }
        }

        if let Some((deadline, retry_url)) = self.yt_retry_deadline.clone() {
            if Instant::now() >= deadline {
                self.yt_retry_deadline = None;
                if self.current_file.is_none() && self.ui.welcome_active {
                    self.open_url_or_path(&retry_url);
                    dirty = true;
                }
            }
        }

        if self.ui.playing && self.last_resume_save.elapsed() >= Duration::from_secs(5) {
            self.save_current_resume_position();
            self.save_db();
            self.last_resume_save = Instant::now();
        }

        dirty
    }

    pub fn handle_mpv_event(&mut self, ev: MpvEvent) {
        match ev {
            MpvEvent::FileLoaded => {
                self.ui.loading_visible = false;
                self.try_execute_pending_resume();
            }
            MpvEvent::PlaybackRestart => {
                self.ui.loading_visible = false;
                self.try_execute_pending_resume();
            }
            MpvEvent::EndFile { reason, error } => {
                // `app.js:764`: ignore `end-file(stop)` when a replacement load is already pending.
                if matches!(reason, hybrid_mpv_engine::EndFileReason::Stop)
                    && self.ui.loading_visible
                {
                    return;
                }
                self.ui.loading_visible = false;
                match reason {
                    hybrid_mpv_engine::EndFileReason::Eof => {
                        if let Some(path) = self.current_file.as_deref() {
                            self.db.clear_resume_position(path);
                            self.save_db();
                        }
                        match self.playlist.play_next(true) {
                            NavigationAction::PlayIndex(idx) => self.play_playlist_index(idx),
                            NavigationAction::RestartCurrent => {
                                if let Some(mpv) = &self.mpv {
                                    let _ = mpv.seek(0.0, "absolute");
                                    let _ = mpv.play();
                                }
                            }
                            NavigationAction::None => {
                                self.ui.playing = false;
                                self.ui.center_play_visible = false;
                            }
                        }
                    }
                    hybrid_mpv_engine::EndFileReason::Error => {
                        let was_youtube = self.is_youtube;
                        let failed_url = self.current_file.clone();
                        let msg =
                            error.unwrap_or_else(|| "Stream or file failed to load".to_string());
                        self.reset_to_idle_state();

                        if was_youtube {
                            if let Some(url) = failed_url {
                                self.last_failed_load_url = Some(url.clone());
                                if !self.yt_auto_retry_used {
                                    self.yt_auto_retry_used = true;
                                    self.push_toast(
                                        ToastKind::Info,
                                        "YouTube blocked the request — retrying automatically...",
                                    );
                                    self.yt_retry_deadline =
                                        Some((Instant::now() + Duration::from_millis(1800), url));
                                    self.check_ytdlp_update_async();
                                    return;
                                }
                            }
                            let lower = msg.to_ascii_lowercase();
                            let needs_auth_hint = lower.contains("no audio or video data")
                                || lower.contains("sign in to confirm")
                                || lower.contains("bot")
                                || lower.contains("403");
                            let full = if needs_auth_hint {
                                format!(
                                    "YouTube playback failed: {msg}. YouTube's anti-bot check is likely blocking this request — use \"Sign in to YouTube\" in Settings -> Streaming Engine to make access reliable."
                                )
                            } else {
                                format!("YouTube playback failed: {msg}")
                            };
                            self.push_toast(ToastKind::Error, full);
                            self.check_ytdlp_update_async();
                        } else {
                            self.push_toast(ToastKind::Error, msg);
                        }
                    }
                    _ => {}
                }
            }
            MpvEvent::PropertyChange { name, value, .. } => {
                self.handle_mpv_property(&name, &value);
            }
            MpvEvent::Seek => {
                self.ui.seek_indicator.visible = false;
            }
            MpvEvent::VideoReconfig
            | MpvEvent::ClientMessage { .. }
            | MpvEvent::LogMessage { .. }
            | MpvEvent::Shutdown => {}
        }
    }

    fn handle_mpv_property(&mut self, name: &str, value: &Value) {
        match name {
            "pause" => {
                let paused = value.as_bool().unwrap_or(false);
                let has_media = self.current_file.is_some() && !self.ui.welcome_active;
                let (playing, center_play, smtc_status) = resolve_pause_ui_state(has_media, paused);
                self.ui.playing = playing;
                self.ui.center_play_visible = center_play;
                if let Some(smtc) = &self.smtc {
                    let _ = smtc.set_playback_status(smtc_status);
                }
            }
            "time-pos" => {
                if let Some(t) = value.as_f64() {
                    if !self.is_dragging_progress {
                        self.ui.current_secs = t.max(0.0);
                    }
                    if !self.ui.chapter_times_secs.is_empty() {
                        let pos = self.ui.current_secs;
                        self.ui.current_chapter = self
                            .ui
                            .chapter_times_secs
                            .iter()
                            .enumerate()
                            .rev()
                            .find(|(_, &ct)| pos + 0.05 >= ct)
                            .map(|(i, _)| i);
                    }
                    if let Some(smtc) = &self.smtc {
                        let dur_ms = (self.ui.duration_secs.max(0.0) * 1000.0).round() as i64;
                        let pos_ms = (self.ui.current_secs.max(0.0) * 1000.0).round() as i64;
                        let _ = smtc.set_timeline(SmtcTimeline {
                            start_ms: 0,
                            end_ms: dur_ms,
                            position_ms: pos_ms,
                            min_seek_ms: 0,
                            max_seek_ms: dur_ms,
                        });
                    }
                }
            }
            "duration" => {
                if let Some(d) = value.as_f64() {
                    self.ui.duration_secs = d.max(0.0);
                    self.recompute_chapter_positions();
                    if let Some(path) = self.current_file.clone() {
                        let name = self.current_title.clone();
                        self.db.add_history(&path, name.as_deref(), d);
                    }
                }
            }
            "demuxer-cache-state" => {
                if let Some(cache_end) = value.get("cache-end").and_then(|v| v.as_f64()) {
                    if cache_end.is_finite() {
                        let ahead = (cache_end - self.ui.current_secs).max(0.0);
                        self.ui.buffered_secs = cache_end.max(self.ui.current_secs);
                        self.ui.stats_overlay.cache_secs = ahead;
                    }
                }
            }
            "demuxer-cache-duration" => {
                if let Some(c) = value.as_f64() {
                    self.ui.buffered_secs = self.ui.current_secs + c.max(0.0);
                    self.ui.stats_overlay.cache_secs = c.max(0.0);
                }
            }
            "volume" => {
                if let Some(v) = value.as_f64() {
                    let max_v = if self.ui.volume_boost_enabled {
                        200.0
                    } else {
                        100.0
                    };
                    self.ui.volume_pct = v.clamp(0.0, max_v).round() as u32;
                }
            }
            "mute" => {
                if let Some(m) = value.as_bool() {
                    self.ui.muted = m;
                }
            }
            "speed" => {
                if let Some(s) = value.as_f64() {
                    self.ui.speed = s;
                }
            }
            "media-title" => {
                if let Some(title) = value.as_str() {
                    if !title.trim().is_empty() {
                        self.current_title = Some(title.to_string());
                        self.ui.window_title = self.formatted_window_title();
                        self.update_smtc_metadata(title);
                    }
                }
            }
            "paused-for-cache" => {
                if let Some(b) = value.as_bool() {
                    self.ui.loading_visible = b;
                }
            }
            "sub-visibility" => {
                if let Some(vis) = value.as_bool() {
                    self.sub_visible = vis;
                    for t in &mut self.ui.subtitle_tracks {
                        t.selected = vis && Some(t.id) == self.current_sid;
                    }
                }
            }
            "track-list" => self.parse_track_list(value),
            "chapter-list" => self.parse_chapter_list(value),
            "chapter" => {
                self.ui.current_chapter =
                    value
                        .as_i64()
                        .and_then(|c| if c >= 0 { Some(c as usize) } else { None });
            }
            "sid" => {
                self.current_sid = value.as_i64();
            }
            "aid" => {
                self.current_aid = value.as_i64();
            }
            "sub-delay" => {
                if let Some(s) = value.as_f64() {
                    self.ui.subtitle_delay_secs = s;
                }
            }
            "audio-delay" => {
                if let Some(s) = value.as_f64() {
                    self.ui.audio_delay_secs = s;
                }
            }
            "video-params" => {
                if let Some((w, h)) = extract_video_dimensions(value) {
                    self.video_dimensions = Some((w, h));
                    if h > 0 {
                        self.ui.video_aspect_ratio = ((w as f32) / (h as f32)).clamp(0.5, 2.5);
                    }
                    self.ui.stats_overlay.resolution = format!("{w}x{h}");
                }
                if let Some(codec) = value.get("codec").and_then(|v| v.as_str()) {
                    if self.ui.stats_overlay.codec.trim().is_empty() {
                        self.ui.stats_overlay.codec = codec.to_string();
                    }
                }
            }
            "estimated-vf-fps" => {
                if let Some(fps) = value.as_f64() {
                    if fps > 0.0 && fps.is_finite() {
                        self.video_fps = Some(fps.clamp(1.0, 240.0));
                    }
                    self.ui.stats_overlay.fps = format!("{fps:.1}");
                }
            }
            "video-bitrate" => {
                if let Some(bps) = value.as_f64() {
                    if bps > 0.0 {
                        let kbps = bps / 1000.0;
                        self.ui.stats_overlay.bitrate = format!("{kbps:.0} kbps");
                    } else {
                        self.ui.stats_overlay.bitrate = "-".to_string();
                    }
                }
            }
            "video-codec" => {
                if let Some(codec) = value.as_str().filter(|s| !s.is_empty()) {
                    self.ui.stats_overlay.codec = codec.to_string();
                }
            }
            "hwdec-current" => {
                self.ui.stats_overlay.hwdec = value
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("no (sw)")
                    .to_string();
            }
            "drop-frame-count" | "frame-drop-count" | "decoder-frame-drop-count" => {
                if let Some(n) = value.as_u64() {
                    self.ui.stats_overlay.dropped_frames = n;
                }
            }
            _ => {}
        }
    }

    fn parse_track_list(&mut self, value: &Value) {
        let Some(arr) = value.as_array() else {
            return;
        };
        self.audio_tracks.clear();
        self.subtitle_tracks.clear();
        self.ui.audio_tracks.clear();
        self.ui.subtitle_tracks.clear();

        let mut audio_idx = 0usize;
        for item in arr {
            let kind = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let id = item.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
            let title = item
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let lang = item
                .get("lang")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let codec = item.get("codec").and_then(|v| v.as_str()).unwrap_or("");
            let selected = item
                .get("selected")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let external = item
                .get("external")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let mt = MediaTrack {
                id,
                lang: lang.clone(),
                title: title.clone(),
                selected,
                external,
            };

            if kind == "audio" {
                audio_idx += 1;
                let lang_name = if !lang.trim().is_empty() {
                    Some(format_language_name(&lang))
                } else {
                    None
                };
                let mut label = if !title.trim().is_empty() {
                    if let Some(ln) = &lang_name {
                        if !title
                            .to_ascii_lowercase()
                            .contains(&ln.to_ascii_lowercase())
                        {
                            format!("{ln} • {title}")
                        } else {
                            title.clone()
                        }
                    } else {
                        title.clone()
                    }
                } else if let Some(ln) = lang_name {
                    ln
                } else {
                    format!("Track {audio_idx}")
                };
                if !codec.is_empty()
                    && !label
                        .to_ascii_lowercase()
                        .contains(&codec.to_ascii_lowercase())
                {
                    label.push_str(&format!(" ({})", codec.to_ascii_uppercase()));
                }
                if selected {
                    self.current_aid = Some(id);
                }
                self.audio_tracks.push(mt);
                self.ui.audio_tracks.push(TrackOption {
                    id,
                    label,
                    lang,
                    selected,
                    external,
                });
            } else if kind == "sub" {
                let base_label = if !title.trim().is_empty() {
                    title.clone()
                } else if !lang.trim().is_empty() {
                    lang.clone()
                } else {
                    format!("Sub {id}")
                };
                let label = if external {
                    format!("{base_label} (ext)")
                } else {
                    base_label
                };
                if selected {
                    self.current_sid = Some(id);
                }
                self.subtitle_tracks.push(mt);
                self.ui.subtitle_tracks.push(TrackOption {
                    id,
                    label,
                    lang,
                    selected: selected && self.sub_visible,
                    external,
                });
            }
        }
    }

    fn recompute_chapter_positions(&mut self) {
        let dur = self.ui.duration_secs;
        if dur <= 0.0 {
            self.ui.chapter_positions_norm.clear();
            return;
        }
        self.ui.chapter_positions_norm = self
            .ui
            .chapter_times_secs
            .iter()
            .map(|&t| (t / dur).clamp(0.0, 1.0) as f32)
            .collect();
    }

    fn parse_chapter_list(&mut self, value: &Value) {
        let Some(arr) = value.as_array() else {
            return;
        };
        self.ui.chapter_times_secs.clear();
        self.ui.chapter_titles.clear();
        for (idx, c) in arr.iter().enumerate() {
            let Some(t) = c.get("time").and_then(|v| v.as_f64()) else {
                continue;
            };
            if !t.is_finite() || t < 0.0 {
                continue;
            }
            let title = c
                .get("title")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| format!("Chapter {}", idx + 1));
            self.ui.chapter_times_secs.push(t);
            self.ui.chapter_titles.push(title);
        }
        self.recompute_chapter_positions();
    }

    fn handle_smtc_command(&mut self, cmd: SmtcCommand) {
        match cmd {
            SmtcCommand::Play => {
                if let Some(mpv) = &self.mpv {
                    let _ = mpv.play();
                }
            }
            SmtcCommand::Pause => {
                if let Some(mpv) = &self.mpv {
                    let _ = mpv.pause();
                }
            }
            SmtcCommand::Next => self.next_track(),
            SmtcCommand::Previous => self.previous_track(),
            SmtcCommand::FastForward => self.seek_relative(10.0),
            SmtcCommand::Rewind => self.seek_relative(-10.0),
            SmtcCommand::Stop => self.stop_playback(),
        }
    }

    fn handle_async_msg(&mut self, msg: BridgeAsyncMsg) {
        match msg {
            BridgeAsyncMsg::SingleScreenshotCaptured(res) => match res {
                Ok(path) => {
                    self.push_captured_screenshot(path);
                }
                Err(err) => self.push_toast(ToastKind::Error, format!("Screenshot failed: {err}")),
            },
            BridgeAsyncMsg::BurstFrameCaptured { session_id, result } => {
                self.burst_in_flight = false;
                if self.burst_session_id.as_deref() == Some(session_id.as_str()) && result.is_ok() {
                    self.burst_seq = self.burst_seq.saturating_add(1);
                    let count = self.burst_seq as usize;
                    self.ui.recording_indicator = RecordingIndicatorMode::BurstActive { count };
                }
            }
            BridgeAsyncMsg::ClipFinished(res) => {
                self.clip_start_secs = None;
                self.clip_started_at = None;
                self.ui.recording_indicator = RecordingIndicatorMode::Saved {
                    label_frames: false,
                };
                match res {
                    Ok(clip) => {
                        self.ui.anim.trigger_recording_saved();
                        let name = clip
                            .output_path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("clip");
                        self.push_toast(ToastKind::Info, format!("Clip saved: {name}"));
                    }
                    Err(err) => {
                        self.ui.recording_indicator = RecordingIndicatorMode::Hidden;
                        self.push_toast(ToastKind::Error, format!("Clip failed: {err}"));
                    }
                }
            }
            BridgeAsyncMsg::YoutubeInfoReady {
                video_id: _,
                result,
            } => {
                if let Ok(info) = result {
                    self.youtube_heights = info.heights;
                    self.ui.youtube_quality_options = self.youtube_quality_options();
                    self.ui.is_live_stream =
                        info.is_live || info.live_status.as_deref() == Some("is_live");
                    if let Some(uploader) = info.uploader.filter(|u| !u.trim().is_empty()) {
                        self.current_uploader = Some(uploader);
                    }
                    if let Some(title) = info.title.filter(|t| !t.trim().is_empty()) {
                        self.current_title = Some(title.clone());
                        self.update_smtc_metadata(&title);
                    }
                    self.ui.window_title = self.formatted_window_title();
                }
            }
            BridgeAsyncMsg::YoutubePlaylistReady { entries } => {
                let pairs: Vec<(String, bool)> = entries.into_iter().map(|u| (u, true)).collect();
                self.playlist.add_paths(pairs);
                self.sync_playlist_to_ui();
            }
            BridgeAsyncMsg::YtdlpVersionResolved(ver) => {
                if let Some(v) = ver.filter(|s| !s.trim().is_empty()) {
                    self.ui.ytdlp_version = v;
                }
            }
            BridgeAsyncMsg::YtdlpBackgroundUpdateChecked(check) => {
                if let Some(cur) = check
                    .current_version
                    .as_deref()
                    .filter(|s| !s.trim().is_empty())
                {
                    self.ui.ytdlp_version = cur.to_string();
                }
                self.ui.ytdlp_update_available = check.has_update;
                self.ui.ytdlp_latest_version = check.latest_version.clone();
                if check.has_update {
                    self.ui.ytdlp_status_text = format!(
                        "Update available: {} -> {}",
                        check.current_version.as_deref().unwrap_or("bundled"),
                        check.latest_version.as_deref().unwrap_or("latest")
                    );
                    let id = self.next_toast_id;
                    self.next_toast_id += 1;
                    let mut toast =
                        ToastItem::new(id, ToastKind::UpdateAvailable, "yt-dlp update available");
                    toast.version_tag = check.latest_version;
                    self.ui.push_toast(toast);
                } else if check.current_version.is_some() && check.latest_version.is_some() {
                    self.ui.ytdlp_status_text = format!(
                        "Up to date ({})",
                        check.current_version.as_deref().unwrap_or("ready")
                    );
                }
            }
            BridgeAsyncMsg::YtdlpUpdateChecked(res) => match res {
                Ok(check) => {
                    if let Some(cur) = check
                        .current_version
                        .as_deref()
                        .filter(|s| !s.trim().is_empty())
                    {
                        self.ui.ytdlp_version = cur.to_string();
                    }
                    self.ui.ytdlp_update_available = check.has_update;
                    self.ui.ytdlp_latest_version = check.latest_version.clone();
                    self.ui.ytdlp_status_text = if check.has_update {
                        format!(
                            "Update available: {} -> {}",
                            check.current_version.as_deref().unwrap_or("bundled"),
                            check.latest_version.as_deref().unwrap_or("latest")
                        )
                    } else {
                        format!(
                            "Up to date ({})",
                            check.current_version.as_deref().unwrap_or("ready")
                        )
                    };
                    if check.has_update {
                        let id = self.next_toast_id;
                        self.next_toast_id += 1;
                        let mut toast = ToastItem::new(
                            id,
                            ToastKind::UpdateAvailable,
                            "yt-dlp update available",
                        );
                        toast.version_tag = check.latest_version;
                        self.ui.push_toast(toast);
                    }
                }
                Err(err) => {
                    self.ui.ytdlp_status_text = format!("Update check failed: {err}");
                }
            },
            BridgeAsyncMsg::YtdlpUpdateInstalled(res) => match res {
                Ok(ver) => {
                    self.ui.ytdlp_update_available = false;
                    self.ui.ytdlp_latest_version = None;
                    if !ver.trim().is_empty() {
                        self.ui.ytdlp_version = ver.clone();
                    }
                    self.ui
                        .toasts
                        .retain(|t| t.kind != ToastKind::UpdateAvailable);
                    self.ui.ytdlp_status_text = format!("Updated to {ver}");
                    self.push_toast(ToastKind::UpdateSuccess, format!("yt-dlp updated to {ver}"));
                    if let Some(mpv) = &self.mpv {
                        let cookie_path = get_cookie_file_path(&self.user_data_dir);
                        let _ = mpv.refresh_ytdl_options(
                            "",
                            cookie_path.exists().then_some(cookie_path.as_path()),
                        );
                    }
                }
                Err(err) => {
                    for t in &mut self.ui.toasts {
                        if t.kind == ToastKind::UpdateAvailable {
                            t.downloading = false;
                        }
                    }
                    self.ui.ytdlp_status_text = format!("Update failed: {err}");
                    self.push_toast(ToastKind::Error, format!("yt-dlp update failed: {err}"));
                }
            },
            BridgeAsyncMsg::YoutubeAuthStatus { text, finishing } => {
                if let Some(win) = &self.auth_window {
                    win.set_finishing(finishing);
                    win.set_status(text);
                }
            }
            BridgeAsyncMsg::YoutubeAuthFinished(res) => {
                if let Some(win) = self.auth_window.take() {
                    win.close();
                }
                self.auth_controller = None;
                self.refresh_youtube_cookie_state();
                match res {
                    Ok(true) => {
                        self.push_toast(ToastKind::Info, "YouTube account connected");
                        if let Some(mpv) = &self.mpv {
                            let cookie = get_cookie_file_path(&self.user_data_dir);
                            let _ = mpv.refresh_ytdl_options("", Some(cookie.as_path()));
                        }
                    }
                    Ok(false) => {
                        self.push_toast(ToastKind::Info, "YouTube sign-in cancelled");
                    }
                    Err(err) => {
                        self.push_toast(ToastKind::Error, format!("YouTube sign-in failed: {err}"));
                    }
                }
            }
            BridgeAsyncMsg::YoutubeBrowserImportFinished(res) => {
                self.refresh_youtube_cookie_state();
                if res.success {
                    if let Some(mpv) = &self.mpv {
                        let cookie = get_cookie_file_path(&self.user_data_dir);
                        let _ = mpv.refresh_ytdl_options("", Some(cookie.as_path()));
                    }
                    let browser = res.browser.as_deref().unwrap_or("browser");
                    self.push_toast(
                        ToastKind::Info,
                        format!("Imported YouTube cookies from {browser}"),
                    );
                } else {
                    let err = res
                        .error
                        .unwrap_or_else(|| "Could not import browser cookies".to_string());
                    self.push_toast(ToastKind::Error, err);
                }
            }
            BridgeAsyncMsg::ThumbnailReady { .. } => {}
            BridgeAsyncMsg::SeekThumbnailDecoded {
                cache_key,
                width,
                height,
                rgba,
            } => {
                self.pending_seek_thumbs.remove(&cache_key);
                if width > 0 && height > 0 && rgba.len() >= (width * height * 4) as usize {
                    if !self.decoded_seek_thumbs.contains_key(&cache_key) {
                        self.seek_thumb_order.push_back(cache_key.clone());
                    }
                    self.decoded_seek_thumbs
                        .insert(cache_key, (width, height, rgba));
                    while self.decoded_seek_thumbs.len() > 64 {
                        if let Some(old) = self.seek_thumb_order.pop_front() {
                            self.decoded_seek_thumbs.remove(&old);
                        }
                    }
                    if self.has_local_seek_thumbnails() {
                        let was_has_frame = self.ui.hover_thumb_has_frame;
                        let is_hovering = self.ui.hover_thumb_norm.is_some();
                        if is_hovering {
                            self.ui.hover_thumb_has_frame = true;
                            if !was_has_frame {
                                self.ui.anim.trigger_hover_thumb_in();
                            }
                        }
                        self.ui.hover_thumb_loading = self
                            .ui
                            .hover_thumb_norm
                            .is_some_and(|n| self.is_seek_thumb_pending(n));
                    }
                }
            }
            BridgeAsyncMsg::SeekThumbnailFailed { cache_key } => {
                self.pending_seek_thumbs.remove(&cache_key);
                self.ui.hover_thumb_loading = self
                    .ui
                    .hover_thumb_norm
                    .is_some_and(|n| self.is_seek_thumb_pending(n));
            }
            BridgeAsyncMsg::ScreenshotImageDecoded {
                key,
                width,
                height,
                rgba,
            } => {
                self.pending_screenshots.remove(&key);
                if width > 0 && height > 0 && rgba.len() >= (width * height * 4) as usize {
                    if !self.decoded_screenshots.contains_key(&key) {
                        self.screenshot_order.push_back(key.clone());
                    }
                    self.decoded_screenshots.insert(key, (width, height, rgba));
                    while self.decoded_screenshots.len() > 32 {
                        if let Some(old) = self.screenshot_order.pop_front() {
                            self.decoded_screenshots.remove(&old);
                        }
                    }
                }
            }
            BridgeAsyncMsg::ScreenshotImageFailed { key } => {
                self.pending_screenshots.remove(&key);
            }
            BridgeAsyncMsg::Toast { kind, message } => self.push_toast(kind, message),
        }
    }

    fn update_smtc_metadata(&self, title: &str) {
        if let Some(smtc) = &self.smtc {
            let is_audio_only = self.current_file.as_deref().is_some_and(|p| {
                let lower = p.to_ascii_lowercase();
                [
                    ".mp3", ".flac", ".wav", ".aac", ".ogg", ".m4a", ".wma", ".opus", ".aiff",
                    ".alac",
                ]
                .iter()
                .any(|ext| lower.ends_with(ext))
            });
            let subtitle_or_artist = self
                .current_uploader
                .clone()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_default();
            let thumbnail_path = self
                .session_screenshots
                .last()
                .map(|s| s.file_path.to_string_lossy().into_owned())
                .or_else(|| {
                    let candidates = [
                        std::env::current_exe()
                            .ok()
                            .and_then(|e| e.parent().map(|d| d.join("assets").join("icon.png"))),
                        Some(PathBuf::from("hybrid-player/assets/icons/icon.png")),
                    ];
                    candidates
                        .into_iter()
                        .flatten()
                        .find(|p| p.is_file())
                        .map(|p| p.to_string_lossy().into_owned())
                });
            let _ = smtc.set_metadata(&SmtcMetadata {
                title: title.to_string(),
                subtitle_or_artist,
                thumbnail_path,
                is_audio_only,
            });
        }
    }

    fn record_captured_screenshot(&mut self, path: PathBuf) -> String {
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("screenshot")
            .to_string();
        self.single_screenshot_stack.push(path.clone());
        self.session_screenshots.push(SessionScreenshot {
            file_path: path,
            file_name: file_name.clone(),
            timestamp_ms: now_millis(),
        });
        if self.session_screenshots.len() > MAX_SESSION_SCREENSHOTS {
            self.session_screenshots.remove(0);
        }
        self.ui.carousel_total = self.session_screenshots.len();
        self.ui.carousel_index = self.session_screenshots.len().saturating_sub(1);
        file_name
    }

    fn push_captured_screenshot(&mut self, path: PathBuf) {
        let _file_name = self.record_captured_screenshot(path);
        if matches!(
            self.ui.carousel_mode,
            CarouselMode::Hidden | CarouselMode::SinglePreview
        ) {
            self.ui.carousel_mode = CarouselMode::SinglePreview;
            self.ui.screenshot_badge = None;
            self.ui.anim.trigger_screenshot_in(1.8);
        }
        self.ui.screenshot_notice = None;
    }

    // ─── Playback & Shortcut Actions ─────────────────────────────────────────

    pub fn execute_shortcut_action(&mut self, action_id: &str) {
        match action_id {
            "toggle-play" => self.toggle_play(),
            "stop" => self.stop_playback(),
            "seek-back-5" => {
                let step = self.ui.seek_step_secs as f64;
                self.seek_relative(-step);
            }
            "seek-forward-5" => {
                let step = self.ui.seek_step_secs as f64;
                self.seek_relative(step);
            }
            "volume-up" => self.adjust_volume(5),
            "volume-down" => self.adjust_volume(-5),
            "toggle-mute" => self.toggle_mute(),
            "speed-up" => self.set_speed((self.ui.speed + 0.25).min(4.0)),
            "speed-down" => self.set_speed((self.ui.speed - 0.25).max(0.25)),
            "frame-forward" => {
                if let Some(mpv) = &self.mpv {
                    let _ = mpv.frame_step();
                }
            }
            "frame-backward" => {
                if let Some(mpv) = &self.mpv {
                    let _ = mpv.frame_back_step();
                }
            }
            "toggle-stats" => {
                self.ui.stats_overlay.visible = !self.ui.stats_overlay.visible;
            }
            "toggle-subtitles" => self.toggle_modal(ModalId::Subtitles),
            "toggle-equalizer" => self.toggle_modal(ModalId::Equalizer),
            "cycle-audio" => self.cycle_audio_track(),
            "sub-delay-down" => self.adjust_subtitle_delay(-0.1),
            "sub-delay-up" => self.adjust_subtitle_delay(0.1),
            "next-track" | "next" => self.next_track(),
            "previous-track" | "previous" => self.previous_track(),
            "screenshot" => self.take_single_screenshot(None),
            "screenshot-png" => self.take_single_screenshot(Some(ScreenshotFormat::Png)),
            "delete-latest-screenshot" => self.delete_latest_screenshot(),
            "restore-latest-screenshot" => self.restore_latest_screenshot(),
            "toggle-screenshot-carousel" => self.toggle_carousel(false),
            "toggle-restore-carousel" => self.toggle_carousel(true),
            "toggle-recording" => self.toggle_clip_recording(),
            "unlock-ui" => self.set_locked(false),
            "toggle-lock" => self.set_locked(!self.ui.locked),
            "toggle-repeat" => {
                self.playlist.cycle_repeat();
                self.sync_playlist_to_ui();
            }
            "toggle-ab-loop" => self.cycle_ab_loop(),
            "toggle-time-format" => {
                self.ui.time_remaining_mode = !self.ui.time_remaining_mode;
            }
            "toggle-playlist" => self.toggle_playlist(),
            "toggle-bg-settings" if self.ui.welcome_active => {
                self.toggle_modal(ModalId::BgSettings);
            }
            "toggle-settings" => self.toggle_modal(ModalId::Settings),
            "open-network-stream" => {
                self.toggle_modal(ModalId::NetworkStream);
                if self.ui.active_modal == Some(ModalId::NetworkStream) {
                    self.set_focused_input(Some(FocusedTextField::NetworkStreamUrl));
                }
            }
            "open-file" => self.open_file_dialog(false, true),
            "open-multiple-files" => self.open_file_dialog(true, true),
            "open-folder" => self.open_folder_dialog(),
            "show-shortcuts" => {
                let binding_for = |id: &str| {
                    let raw = self
                        .ui
                        .custom_shortcuts
                        .get(id)
                        .cloned()
                        .unwrap_or_else(|| ShortcutManager::get_default_binding(id).to_string());
                    format_combo_compact(&raw)
                };
                let play_k = binding_for("toggle-play");
                let fs_k = binding_for("toggle-fullscreen");
                let mute_k = binding_for("toggle-mute");
                let shot_k = binding_for("screenshot");
                self.push_toast(
                    ToastKind::Info,
                    format!(
                        "Shortcuts: {}=Play, {}=Fullscreen, {}=Mute, {}=Screenshot",
                        if play_k.is_empty() {
                            "Space"
                        } else {
                            play_k.as_str()
                        },
                        if fs_k.is_empty() { "F" } else { fs_k.as_str() },
                        if mute_k.is_empty() {
                            "M"
                        } else {
                            mute_k.as_str()
                        },
                        if shot_k.is_empty() {
                            "S"
                        } else {
                            shot_k.as_str()
                        },
                    ),
                );
            }
            "show-about" => {
                self.push_toast(
                    ToastKind::Info,
                    "Hybrid Player v12.6.0 — Next-gen media player",
                );
            }
            "quit" => self.shutdown_for_exit(),
            seek if seek.starts_with("seek-") => {
                if let Ok(pct) = seek.trim_start_matches("seek-").parse::<f64>() {
                    if (0.0..=100.0).contains(&pct) {
                        self.seek_normalized(pct / 100.0);
                    }
                }
            }
            _ => {}
        }
    }

    /// Saves resume/DB state, shuts down mpv, and flags the desktop event loop to exit.
    pub fn shutdown_for_exit(&mut self) {
        self.save_current_resume_position();
        self.save_db();
        if let Some(mpv) = &self.mpv {
            mpv.shutdown();
        }
        self.quit_requested = true;
    }

    /// Dispatches a UI pointer/AccessKit interaction with optional normalized slider ratios (`0.0..=1.0`).
    pub fn apply_hit_with_ratio(
        &mut self,
        target: &HitTarget,
        ratio_x: Option<f32>,
        ratio_y: Option<f32>,
    ) {
        if self.ui.bg_accordion_open
            && matches!(
                target,
                HitTarget::BgControlInput(_)
                    | HitTarget::EqBandSlider(_)
                    | HitTarget::SpeedSlider
                    | HitTarget::SubDelaySlider
                    | HitTarget::SubtitleFontSizeSlider
                    | HitTarget::SubtitleBgOpacitySlider
                    | HitTarget::ColorPickerSpectrum
                    | HitTarget::ColorPickerChannel(_)
            )
        {
            self.ui.bg_accordion_open = false;
            self.ui.anim.trigger_bg_accordion_out();
        }
        let rx = ratio_x.unwrap_or(0.5).clamp(0.0, 1.0);
        let ry = ratio_y.unwrap_or(0.5).clamp(0.0, 1.0);
        match target {
            HitTarget::ProgressSeek => self.seek_normalized(rx as f64),
            HitTarget::VolumeSlider => {
                let max_vol = if self.ui.volume_boost_enabled {
                    200.0
                } else {
                    100.0
                };
                self.set_volume_pct((rx * max_vol).round() as u32);
            }
            HitTarget::EqBandSlider(idx) => {
                let db = (((1.0 - ry) as f64) * 24.0 - 12.0).clamp(-12.0, 12.0);
                self.set_eq_band_debounced(*idx, (db * 2.0).round() / 2.0);
            }
            HitTarget::SpeedSlider => {
                let speed = (0.1 + (rx as f64) * 3.9).clamp(0.1, 4.0);
                self.set_speed((speed * 20.0).round() / 20.0);
            }
            HitTarget::SubDelaySlider => {
                let secs = ((rx as f64) * 20.0 - 10.0).clamp(-10.0, 10.0);
                self.set_subtitle_delay((secs * 10.0).round() / 10.0);
            }
            HitTarget::SubtitleFontSizeSlider => {
                let px = (12.0 + rx * 52.0).round().clamp(12.0, 64.0) as u32;
                self.set_subtitle_font_size(px);
            }
            HitTarget::SubtitleBgOpacitySlider => {
                let pct = (rx * 100.0).round().clamp(0.0, 100.0) as u32;
                self.set_subtitle_bg_opacity((pct as f64) / 100.0);
            }
            HitTarget::BgControlInput(ctrl_id) => {
                if BG_CONTROLS_CATALOG.iter().any(|c| {
                    c.id == ctrl_id && matches!(c.input_type, BgControlInputType::Range { .. })
                }) {
                    self.set_bg_control_ratio(ctrl_id, rx);
                } else {
                    self.handle_hit_target(target);
                }
            }
            HitTarget::ColorPickerSpectrum => self.apply_color_picker_spectrum(rx, ry),
            HitTarget::ColorPickerChannel(ch) => self.apply_color_picker_channel(*ch, rx),
            other => self.handle_hit_target(other),
        }
    }

    /// Dispatches a UI pointer click on a [`HitTarget`].
    pub fn handle_hit_target(&mut self, target: &HitTarget) {
        if self.ui.bg_accordion_open
            && !matches!(
                target,
                HitTarget::BgAccordionSurface
                    | HitTarget::BgAccordionCard(_)
                    | HitTarget::BgEffectPickerToggle
            )
        {
            self.ui.bg_accordion_open = false;
            self.ui.anim.trigger_bg_accordion_out();
            if matches!(
                target,
                HitTarget::ModalBackdrop | HitTarget::ModalPanelSurface
            ) {
                return;
            }
        }

        match target {
            HitTarget::OpenFile => self.open_file_dialog(false, true),
            HitTarget::OpenLink => {
                self.toggle_modal(ModalId::NetworkStream);
                if self.ui.active_modal == Some(ModalId::NetworkStream) {
                    self.set_focused_input(Some(FocusedTextField::NetworkStreamUrl));
                }
            }
            HitTarget::ProgressSeek => self.seek_normalized(0.5),
            HitTarget::TimeRemainingToggle => {
                self.ui.time_remaining_mode = !self.ui.time_remaining_mode;
            }
            HitTarget::ChapterMarker(idx) => {
                if let Some(&ch_secs) = self.ui.chapter_times_secs.get(*idx) {
                    let target = if self.ui.is_live_stream && self.ui.duration_secs > 0.0 {
                        ch_secs.min(self.ui.duration_secs)
                    } else {
                        ch_secs
                    };
                    self.ui.current_secs = target.max(0.0);
                    self.ui.current_chapter = Some(*idx);
                    if let Some(mpv) = &self.mpv {
                        let _ = mpv.seek(target.max(0.0), "absolute");
                    }
                }
            }
            HitTarget::VolumeSlider => {}
            HitTarget::EqBandSlider(_) | HitTarget::SpeedSlider | HitTarget::SubDelaySlider => {}
            HitTarget::PlayPause => self.toggle_play(),
            HitTarget::PrevTrack => self.previous_track(),
            HitTarget::NextTrack => self.next_track(),
            HitTarget::MuteToggle => self.toggle_mute(),
            HitTarget::RecentFile(idx) => {
                if let Some(entry) = self.ui.recent_files.get(*idx).cloned() {
                    self.open_url_or_path(&entry.path);
                }
            }
            HitTarget::PlaylistClose | HitTarget::PlaylistToggle => self.toggle_playlist(),
            HitTarget::PlaylistAddFiles => self.open_file_dialog(true, false),
            HitTarget::PlaylistSearch => {
                self.set_focused_input(Some(FocusedTextField::PlaylistSearch));
            }
            HitTarget::PlaylistShuffle | HitTarget::ShuffleToggle => {
                self.playlist.toggle_shuffle();
                self.sync_playlist_to_ui();
            }
            HitTarget::PlaylistRepeat | HitTarget::RepeatToggle => {
                self.playlist.cycle_repeat();
                self.sync_playlist_to_ui();
            }
            HitTarget::PlaylistClear => {
                let had_items = !self.playlist.items.is_empty();
                self.playlist.clear();
                self.sync_playlist_to_ui();
                if had_items || self.current_file.is_some() {
                    self.stop_playlist_playback();
                }
            }
            HitTarget::PlaylistRow(idx) => self.play_playlist_index(*idx),
            HitTarget::PlaylistRowRemove(idx) => {
                let was_active = self.playlist.current_index == *idx as isize;
                self.playlist.remove(*idx);
                if was_active {
                    if self.playlist.current_index >= 0 {
                        let next_idx = self.playlist.current_index as usize;
                        self.play_playlist_index(next_idx);
                    } else {
                        self.sync_playlist_to_ui();
                        self.stop_playlist_playback();
                    }
                } else {
                    self.sync_playlist_to_ui();
                }
            }
            HitTarget::AbLoopToggle => self.cycle_ab_loop(),
            HitTarget::LockToggle => self.set_locked(!self.ui.locked),
            HitTarget::UnlockControls => self.set_locked(false),
            HitTarget::YoutubeQualityButton => self.toggle_dropdown(DropdownKind::YoutubeQuality),
            HitTarget::EqualizerButton => self.toggle_modal(ModalId::Equalizer),
            HitTarget::SpeedButton => self.toggle_modal(ModalId::Speed),
            HitTarget::SubtitlesButton => self.toggle_modal(ModalId::Subtitles),
            HitTarget::AudioTrackButton => self.toggle_modal(ModalId::Audio),
            HitTarget::SettingsButton => self.toggle_modal(ModalId::Settings),
            HitTarget::BgSettingsToggle => self.toggle_modal(ModalId::BgSettings),
            HitTarget::ModalClose | HitTarget::ModalBackdrop => {
                self.flush_eq_debounce();
                self.ui.close_modals_animated();
                if matches!(
                    self.focused_input,
                    Some(FocusedTextField::NetworkStreamUrl | FocusedTextField::ShortcutSearch)
                ) {
                    self.set_focused_input(None);
                }
            }
            HitTarget::EqPresetSelect => self.toggle_dropdown(DropdownKind::EqPreset),
            HitTarget::EqReset => self.reset_equalizer(),
            HitTarget::SpeedPreset(idx) => self.apply_speed_preset(*idx),
            HitTarget::SubDelayStep(ms) => {
                if *ms == 0 {
                    self.set_subtitle_delay(0.0);
                } else {
                    self.adjust_subtitle_delay((*ms as f64) * 0.001);
                }
            }
            HitTarget::AudioDelayStep(ms) => {
                if *ms == 0 {
                    self.reset_audio_delay();
                } else {
                    self.adjust_audio_delay(*ms as i64);
                }
            }
            HitTarget::SubtitleTrackSelect(id) => {
                if *id < 0 {
                    self.current_sid = None;
                    self.sub_visible = false;
                    for t in &mut self.ui.subtitle_tracks {
                        t.selected = false;
                    }
                    if let Some(mpv) = &self.mpv {
                        let _ = mpv.set_sub_visibility(false);
                    }
                } else {
                    self.current_sid = Some(*id);
                    self.sub_visible = true;
                    for t in &mut self.ui.subtitle_tracks {
                        t.selected = t.id == *id;
                    }
                    if let Some(mpv) = &self.mpv {
                        let _ = mpv.set_sub(&id.to_string());
                        let _ = mpv.set_sub_visibility(true);
                    }
                }
            }
            HitTarget::SubtitleLoadExternal => {
                let paths = pick_native_paths(FileDialogKind::Subtitle, self.owner_hwnd);
                if !paths.is_empty() {
                    self.add_companion_subtitles(&paths);
                }
            }
            HitTarget::SubtitleFontColorPicker => {
                self.toggle_color_picker(ColorPickerTarget::SubtitleFontColor);
            }
            HitTarget::SubtitleBgColorPicker => {
                self.toggle_color_picker(ColorPickerTarget::SubtitleBgColor);
            }
            HitTarget::AudioTrackSelect(id) => {
                self.current_aid = Some(*id);
                for t in &mut self.ui.audio_tracks {
                    t.selected = t.id == *id;
                }
                if let Some(mpv) = &self.mpv {
                    let _ = mpv.set_audio(&id.to_string());
                }
            }
            HitTarget::AudioPresetSubs => self.apply_subs_dubs_preset(SubsDubsMode::Subs),
            HitTarget::AudioPresetDubs => self.apply_subs_dubs_preset(SubsDubsMode::Dubs),
            HitTarget::BgEffectPickerToggle => {
                self.ui.active_dropdown = None;
                self.ui.active_color_picker = None;
                self.ui.bg_accordion_open = !self.ui.bg_accordion_open;
                if self.ui.bg_accordion_open {
                    self.ui.anim.trigger_bg_accordion_in();
                } else {
                    self.ui.anim.trigger_bg_accordion_out();
                }
            }
            HitTarget::BgAccordionCard(kind) => {
                self.set_welcome_background(*kind);
            }
            HitTarget::BgQualityRadio(tier) => {
                let q = match tier {
                    0 => QualityTier::Low,
                    2 => QualityTier::High,
                    _ => QualityTier::Medium,
                };
                self.set_welcome_quality(q);
            }
            HitTarget::BgControlInput(ctrl_id) => {
                if ctrl_id == "bgGridmotionAspectRatio" {
                    self.toggle_dropdown(DropdownKind::BgGridmotionAspectRatio);
                } else if BG_CONTROLS_CATALOG
                    .iter()
                    .any(|c| c.id == ctrl_id && matches!(c.input_type, BgControlInputType::Color))
                {
                    self.toggle_color_picker(ColorPickerTarget::BgControl(ctrl_id.clone()));
                } else {
                    self.step_bg_control(ctrl_id);
                }
            }
            HitTarget::BgGridmotionAddImages => {
                let paths = pick_native_paths(FileDialogKind::GridmotionImages, self.owner_hwnd);
                if !paths.is_empty() {
                    for p in paths {
                        if self.gridmotion_custom_pictures.len() >= 16 {
                            break;
                        }
                        self.gridmotion_custom_pictures
                            .push(p.to_string_lossy().into_owned());
                    }
                    self.ui.gridmotion_custom_pictures = self.gridmotion_custom_pictures.clone();
                    self.gridmotion_pictures_gen = self.gridmotion_pictures_gen.wrapping_add(1);
                    self.persist_gridmotion_custom_pictures();
                }
            }
            HitTarget::BgGridmotionClearImages => {
                self.gridmotion_custom_pictures.clear();
                self.ui.gridmotion_custom_pictures.clear();
                self.gridmotion_pictures_gen = self.gridmotion_pictures_gen.wrapping_add(1);
                self.persist_gridmotion_custom_pictures();
            }
            HitTarget::BgGridmotionRemoveImage(idx) => {
                if *idx < self.gridmotion_custom_pictures.len() {
                    self.gridmotion_custom_pictures.remove(*idx);
                    self.ui.gridmotion_custom_pictures = self.gridmotion_custom_pictures.clone();
                    self.gridmotion_pictures_gen = self.gridmotion_pictures_gen.wrapping_add(1);
                    self.persist_gridmotion_custom_pictures();
                }
            }
            HitTarget::BgResetCurrent => self.reset_current_bg_effect(),
            HitTarget::ShortcutSearch => {
                self.set_focused_input(Some(FocusedTextField::ShortcutSearch));
            }
            HitTarget::ShortcutRebind(action_id) => {
                self.ui.recording_shortcut_id = Some(action_id.clone());
            }
            HitTarget::ShortcutResetOne(action_id) => self.reset_single_shortcut(action_id),
            HitTarget::ShortcutResetAll => self.reset_all_shortcuts(),
            HitTarget::SettingsThemeSelect => self.toggle_dropdown(DropdownKind::SettingsTheme),
            HitTarget::SettingsAccentPicker => {
                self.toggle_color_picker(ColorPickerTarget::Accent);
            }
            HitTarget::SettingsBrandFontsToggle => {
                let next = !self.ui.font_config.brand_font_enabled;
                self.ui.font_config.brand_font_enabled = next;
                self.ui.gridmotion_cfg.brand_font_enabled = next;
                self.db.preferences.set("brandFontEnabled", &json!(next));
                self.save_db();
            }
            HitTarget::SettingsAutoResumeToggle => {
                let next = !self.ui.auto_resume;
                self.ui.auto_resume = next;
                self.db.preferences.set("autoResume", &json!(next));
                self.save_db();
            }
            HitTarget::SettingsMotionSelect => self.toggle_dropdown(DropdownKind::SettingsMotion),
            HitTarget::SettingsSeekStepSelect => {
                self.toggle_dropdown(DropdownKind::SettingsSeekStep)
            }
            HitTarget::SettingsDoubleClickToggle => {
                let next = !self.ui.double_click_fullscreen;
                self.ui.double_click_fullscreen = next;
                self.db
                    .preferences
                    .set("doubleClickFullscreen", &json!(next));
                self.save_db();
            }
            HitTarget::SettingsUnlockVisToggle => {
                let next = !self.ui.show_unlock_button;
                self.ui.show_unlock_button = next;
                self.db.preferences.set("showUnlockButton", &json!(next));
                self.save_db();
            }
            HitTarget::SettingsScreenshotDirPick => {
                if let Some(dir) = pick_native_paths(FileDialogKind::Folder, self.owner_hwnd)
                    .into_iter()
                    .next()
                {
                    let s = dir.to_string_lossy().into_owned();
                    self.ui.screenshot_dir = s.clone();
                    self.db.preferences.set("screenshotDir", &json!(s));
                    if let Some(mpv) = &self.mpv {
                        let _ = mpv.set_screenshot_dir(Some(&dir));
                    }
                    self.save_db();
                }
            }
            HitTarget::SettingsScreenshotDirOpen => self.open_recording_folder(),
            HitTarget::SettingsScreenshotDirReset => {
                self.ui.screenshot_dir.clear();
                self.db.preferences.set("screenshotDir", &json!(""));
                let fallback = self.default_screenshot_dir();
                if let Some(mpv) = &self.mpv {
                    let _ = mpv.set_screenshot_dir(Some(&fallback));
                }
                self.save_db();
            }
            HitTarget::SettingsScreenshotFormatSelect => {
                self.toggle_dropdown(DropdownKind::SettingsScreenshotFormat);
            }
            HitTarget::SettingsAutoOrganizeToggle => {
                let next = !self.ui.auto_organize_screenshots;
                self.ui.auto_organize_screenshots = next;
                self.db
                    .preferences
                    .set("autoOrganizeScreenshots", &json!(next));
                if let Some(mpv) = &self.mpv {
                    mpv.set_auto_organize_screenshots(next);
                }
                self.save_db();
            }
            HitTarget::SettingsMouseScreenshotSelect => {
                self.toggle_dropdown(DropdownKind::SettingsMouseScreenshot);
            }
            HitTarget::SettingsOpenMultiple => {
                self.ui.close_modals_animated();
                self.open_file_dialog(true, true);
            }
            HitTarget::SettingsOpenFolder => {
                self.ui.close_modals_animated();
                self.open_folder_dialog();
            }
            HitTarget::SettingsQuit => self.shutdown_for_exit(),
            HitTarget::CarouselPrev => self.navigate_carousel(-1),
            HitTarget::CarouselNext => self.navigate_carousel(1),
            HitTarget::CarouselConfirm => self.confirm_carousel_action(),
            HitTarget::SinglePreviewOpenFolder => self.open_recording_folder(),
            HitTarget::CarouselClose => {
                self.ui.carousel_mode = CarouselMode::Hidden;
                self.ui.screenshot_badge = None;
            }
            HitTarget::DropdownOption(idx) => self.apply_dropdown_option(*idx),
            HitTarget::DropdownBackdrop => {
                self.ui.active_dropdown = None;
                self.ui.active_color_picker = None;
            }
            HitTarget::ColorPickerSwatch(idx) => self.apply_color_picker_swatch(*idx as usize),
            HitTarget::ColorPickerClose => {
                self.ui.active_color_picker = None;
            }
            HitTarget::RecordingStopClip => self.toggle_clip_recording(),
            HitTarget::RecordingOpenFolder => self.open_recording_folder(),
            HitTarget::YoutubeSignIn => self.start_youtube_sign_in(false),
            HitTarget::YoutubeSwitchAccount => self.start_youtube_sign_in(true),
            HitTarget::YoutubeSignOut => self.sign_out_youtube(),
            HitTarget::YoutubeImportBrowserCookies => self.import_youtube_browser_cookies(),
            HitTarget::YoutubeImportCookiesFile => self.import_youtube_cookies_file(),
            HitTarget::YtdlpCheckUpdate => self.check_ytdlp_update_async(),
            HitTarget::YtdlpPerformUpdate => self.perform_ytdlp_update_async(),
            HitTarget::ToastDismiss(id) => {
                self.ui.toasts.retain(|t| t.id != *id);
            }
            HitTarget::ToastUpdateNow => self.perform_ytdlp_update_async(),
            HitTarget::NetworkStreamInput => {
                self.set_focused_input(Some(FocusedTextField::NetworkStreamUrl));
            }
            HitTarget::NetworkStreamOpen => {
                let raw = self.ui.network_stream_url.clone();
                if let Some(valid_url) = normalize_network_media_url(&raw) {
                    self.ui.close_modals_animated();
                    self.ui.network_stream_url.clear();
                    self.set_focused_input(None);
                    self.push_toast(ToastKind::Info, "Opening stream...");
                    self.open_url_or_path(&valid_url);
                } else {
                    self.push_toast(
                        ToastKind::Error,
                        "Enter a valid HTTP, RTSP, RTMP, or SRT URL",
                    );
                }
            }
            _ => {}
        }
    }

    pub fn set_subtitle_delay(&mut self, secs: f64) {
        let next = secs.clamp(SUBTITLE_SYNC_MIN_SEC, SUBTITLE_SYNC_MAX_SEC);
        self.ui.subtitle_delay_secs = next;
        if let Some(mpv) = &self.mpv {
            let _ = mpv.set_sub_delay(next);
        }
        if let Some(path) = self.current_file.as_deref() {
            self.db
                .save_subtitle_delay(path, subtitle_sec_to_ms(next) as f64);
            self.save_db();
        }
    }

    pub fn set_subtitle_font_size(&mut self, px: u32) {
        let px = px.clamp(12, 64);
        self.ui.subtitle_font_size = px;
        self.subtitle_style.font_size = px as u16;
        if let Some(mpv) = &self.mpv {
            self.apply_subtitle_style_to_mpv(mpv);
        }
        self.db.preferences.set("subFontSize", &json!(px));
        self.save_db();
    }

    pub fn set_subtitle_bg_opacity(&mut self, opacity: f64) {
        let clamped = opacity.clamp(0.0, 1.0);
        let pct = (clamped * 100.0).round() as u32;
        self.ui.subtitle_bg_opacity_pct = pct;
        self.subtitle_style.bg_opacity = (pct as f64) / 100.0;
        if let Some(mpv) = &self.mpv {
            self.apply_subtitle_style_to_mpv(mpv);
        }
        self.db
            .preferences
            .set("subBgOpacity", &json!(self.subtitle_style.bg_opacity));
        self.save_db();
    }

    pub fn youtube_quality_options(&self) -> Vec<String> {
        let mut options: Vec<String> = self
            .youtube_heights
            .iter()
            .filter(|h| **h > 0)
            .map(|h| format!("{h}p"))
            .collect();
        if options.is_empty() {
            options = vec![
                "auto".to_string(),
                "1080p".to_string(),
                "720p".to_string(),
                "480p".to_string(),
                "360p".to_string(),
                "audio".to_string(),
            ];
        } else {
            if !options.iter().any(|o| o.eq_ignore_ascii_case("auto")) {
                options.insert(0, "auto".to_string());
            }
            if !options.iter().any(|o| o.eq_ignore_ascii_case("audio")) {
                options.push("audio".to_string());
            }
        }
        options
    }

    pub fn cycle_youtube_quality(&mut self) {
        let options = self.youtube_quality_options();
        let pos = options
            .iter()
            .position(|o| o.eq_ignore_ascii_case(&self.current_youtube_quality))
            .unwrap_or(0);
        let next = options[(pos + 1) % options.len()].clone();
        self.apply_youtube_quality(&next);
    }

    pub fn apply_youtube_quality(&mut self, quality: &str) {
        let next = quality.to_string();
        self.current_youtube_quality = next.clone();
        self.ui.youtube_quality_label = if next.eq_ignore_ascii_case("auto") {
            "Auto".to_string()
        } else if next.eq_ignore_ascii_case("audio") {
            "Audio".to_string()
        } else {
            next.clone()
        };

        let fmt = build_ytdl_format(&next, self.current_file.as_deref(), self.video_dimensions);
        if let Some(mpv) = &self.mpv {
            let playback_time = mpv
                .get_property_json("time-pos")
                .ok()
                .and_then(|v| v.as_f64())
                .filter(|t| t.is_finite() && *t > 0.0)
                .unwrap_or(self.ui.current_secs);
            let _ = mpv.set_property_string("ytdl-format", &fmt);
            if let Some(target_url) = self.current_file.clone() {
                self.ui.loading_visible = true;
                let mpv_clone = Arc::clone(mpv);
                let tx = self.async_tx.clone();
                self.rt.spawn(async move {
                    if let Err(err) = mpv_clone
                        .load_file_at(&target_url, (playback_time > 0.5).then_some(playback_time))
                        .await
                    {
                        let _ = tx.send(BridgeAsyncMsg::Toast {
                            kind: ToastKind::Error,
                            message: format!("Failed to reload stream: {err}"),
                        });
                    }
                });
            }
        }
        self.push_toast(
            ToastKind::Info,
            format!("Quality: {}", self.ui.youtube_quality_label),
        );
    }

    pub fn cycle_eq_preset(&mut self) {
        let presets = EqualizerPreset::BUILTIN_PRESETS;
        let idx = presets
            .iter()
            .position(|p| *p == self.equalizer.preset)
            .map(|i| (i + 1) % presets.len())
            .unwrap_or(0);
        self.set_eq_preset(presets[idx]);
    }

    pub fn apply_speed_preset(&mut self, idx: usize) {
        if let Some(&rate) = SPEED_PRESETS.get(idx) {
            self.set_speed(rate);
        }
    }

    fn persist_gridmotion_custom_pictures(&mut self) {
        let aspect_str = match self.ui.gridmotion_cfg.aspect_ratio {
            GridAspectRatio::Ratio16x9 => "16:9",
            GridAspectRatio::Ratio1x1 => "1:1",
            GridAspectRatio::Ratio3x4 => "3:4",
            GridAspectRatio::Ratio4x3 => "4:3",
            GridAspectRatio::Auto => "auto",
        };
        let c = self.ui.gridmotion_cfg.gradient_color;
        let hex = format!(
            "#{:02x}{:02x}{:02x}",
            (c[0].clamp(0.0, 1.0) * 255.0).round() as u8,
            (c[1].clamp(0.0, 1.0) * 255.0).round() as u8,
            (c[2].clamp(0.0, 1.0) * 255.0).round() as u8,
        );
        let raw = json!({
            "autoMove": self.ui.gridmotion_cfg.auto_move,
            "scale": self.ui.gridmotion_cfg.scale,
            "aspectRatio": aspect_str,
            "speed": self.ui.gridmotion_cfg.speed,
            "maxMove": self.ui.gridmotion_cfg.max_move,
            "gradientColor": hex,
            "customPictures": self.gridmotion_custom_pictures,
        })
        .to_string();
        self.db.preferences.bg_opts_gridmotion = Some(raw);
        self.save_db();
    }

    pub fn commit_shortcut_rebind(&mut self, action_id: &str, raw_combo: &str) {
        let mut mgr = ShortcutManager::default();
        mgr.apply_custom_shortcuts(&self.ui.custom_shortcuts);
        if let Ok(outcome) = mgr.set_shortcut(action_id, raw_combo) {
            let overrides = mgr.get_custom_overrides();
            self.ui.custom_shortcuts = overrides.clone();
            self.db
                .preferences
                .set("customShortcuts", &json!(overrides));
            self.save_db();
            let msg = if let Some(displaced) = &outcome.displaced_action_label {
                format!(
                    "Shortcut '{}' set to {} (unbound from \"{}\")",
                    outcome.label, outcome.display_combo, displaced
                )
            } else {
                format!(
                    "Shortcut '{}' set to {}",
                    outcome.label, outcome.display_combo
                )
            };
            self.push_toast(ToastKind::Info, msg);
        }
        self.ui.recording_shortcut_id = None;
        self.ui.recording_shortcut_preview = None;
    }

    pub fn reset_single_shortcut(&mut self, action_id: &str) {
        let mut mgr = ShortcutManager::default();
        mgr.apply_custom_shortcuts(&self.ui.custom_shortcuts);
        let default_combo = ShortcutManager::get_default_binding(action_id);
        if mgr.set_shortcut(action_id, default_combo).is_ok() {
            let overrides = mgr.get_custom_overrides();
            self.ui.custom_shortcuts = overrides.clone();
            self.db
                .preferences
                .set("customShortcuts", &json!(overrides));
            self.save_db();
        }
        if self.ui.recording_shortcut_id.as_deref() == Some(action_id) {
            self.ui.recording_shortcut_id = None;
        }
    }

    pub fn reset_all_shortcuts(&mut self) {
        self.ui.custom_shortcuts.clear();
        self.ui.recording_shortcut_id = None;
        self.db.preferences.set(
            "customShortcuts",
            &json!(std::collections::BTreeMap::<String, String>::new()),
        );
        self.save_db();
        self.push_toast(ToastKind::Info, "All shortcuts reset to defaults");
    }

    pub fn reset_current_bg_effect(&mut self) {
        let q = self.ui.welcome_quality;
        match self.ui.welcome_background {
            WelcomeBackgroundKind::Dither => {
                self.ui.dither_cfg = DitherWavesConfig::for_quality(q);
                self.db.preferences.bg_opts_dither = None;
            }
            WelcomeBackgroundKind::Particles => {
                self.ui.particles_cfg = ParticlesConfig::for_quality(q);
                self.db.preferences.bg_opts_particles = None;
            }
            WelcomeBackgroundKind::Faulty => {
                self.ui.faulty_cfg = FaultyTerminalConfig::for_quality(q);
                self.db.preferences.bg_opts_faulty = None;
            }
            WelcomeBackgroundKind::Gridmotion => {
                self.ui.gridmotion_cfg = GridMotionConfig::for_quality(q);
                self.db.preferences.bg_opts_gridmotion = None;
            }
            WelcomeBackgroundKind::None => {}
        }
        self.save_db();
        self.push_toast(ToastKind::Info, "Welcome effect settings reset");
    }

    pub fn set_bg_control_ratio(&mut self, ctrl_id: &str, rx: f32) {
        let Some(desc) = BG_CONTROLS_CATALOG.iter().find(|c| c.id == ctrl_id) else {
            return;
        };
        let BgControlInputType::Range { min, max, step } = desc.input_type else {
            self.step_bg_control(ctrl_id);
            return;
        };
        let lo = min.parse::<f32>().unwrap_or(0.0);
        let hi = max.parse::<f32>().unwrap_or(1.0);
        let st = step.parse::<f32>().unwrap_or(0.1).max(1e-4);
        let steps = ((rx.clamp(0.0, 1.0) * (hi - lo)) / st).round();
        let val = (((lo + steps * st) * 1000.0).round() / 1000.0).clamp(lo, hi);

        match ctrl_id {
            "bgDitherSpeed" => self.ui.dither_cfg.wave_speed = val,
            "bgDitherFrequency" => self.ui.dither_cfg.wave_frequency = val,
            "bgDitherAmplitude" => self.ui.dither_cfg.wave_amplitude = val,
            "bgDitherPixelSize" => self.ui.dither_cfg.pixel_size = val,
            "bgDitherColorNum" => self.ui.dither_cfg.color_num = val,
            "bgParticlesCount" => self.ui.particles_cfg.particle_count = val.round() as u32,
            "bgParticlesSpeed" => self.ui.particles_cfg.speed = val,
            "bgParticlesSpread" => self.ui.particles_cfg.particle_spread = val,
            "bgParticlesSize" => self.ui.particles_cfg.particle_base_size = val,
            "bgFaultyScale" => self.ui.faulty_cfg.scale = val,
            "bgFaultyGlitch" => self.ui.faulty_cfg.glitch_amount = val,
            "bgFaultyScanlines" => self.ui.faulty_cfg.scanline_intensity = val,
            "bgFaultyFlicker" => self.ui.faulty_cfg.flicker_amount = val,
            "bgFaultyAberration" => self.ui.faulty_cfg.chromatic_aberration = val,
            "bgFaultyCurvature" => self.ui.faulty_cfg.curvature = val,
            "bgFaultyBrightness" => self.ui.faulty_cfg.brightness = val,
            "bgGridmotionScale" => self.ui.gridmotion_cfg.scale = val,
            "bgGridmotionSpeed" => self.ui.gridmotion_cfg.speed = val,
            "bgGridmotionMaxMove" => self.ui.gridmotion_cfg.max_move = val,
            _ => return,
        }
        self.persist_bg_control_pref(desc.bg);
    }

    pub fn step_bg_control(&mut self, ctrl_id: &str) {
        let Some(desc) = BG_CONTROLS_CATALOG.iter().find(|c| c.id == ctrl_id) else {
            return;
        };
        let step_range = |cur: f32| -> f32 {
            if let BgControlInputType::Range { min, max, step } = desc.input_type {
                let lo = min.parse::<f32>().unwrap_or(0.0);
                let hi = max.parse::<f32>().unwrap_or(1.0);
                let st = step.parse::<f32>().unwrap_or(0.1).max(1e-4);
                let next = cur + st;
                if next > hi + st * 0.25 {
                    lo
                } else {
                    (((next * 1000.0).round()) / 1000.0).clamp(lo, hi)
                }
            } else {
                cur
            }
        };
        let cycle_color = |cur: [f32; 3]| -> [f32; 3] {
            const PALETTE: [[f32; 3]; 5] = [
                [0.502, 0.502, 0.502],
                [1.0, 1.0, 1.0],
                [0.655, 0.937, 0.620],
                [0.231, 0.510, 0.965],
                [0.0, 0.0, 0.0],
            ];
            let idx = PALETTE
                .iter()
                .position(|c| {
                    (c[0] - cur[0]).abs() < 0.05
                        && (c[1] - cur[1]).abs() < 0.05
                        && (c[2] - cur[2]).abs() < 0.05
                })
                .map(|i| (i + 1) % PALETTE.len())
                .unwrap_or(0);
            PALETTE[idx]
        };

        match ctrl_id {
            "bgDitherSpeed" => {
                self.ui.dither_cfg.wave_speed = step_range(self.ui.dither_cfg.wave_speed)
            }
            "bgDitherFrequency" => {
                self.ui.dither_cfg.wave_frequency = step_range(self.ui.dither_cfg.wave_frequency);
            }
            "bgDitherAmplitude" => {
                self.ui.dither_cfg.wave_amplitude = step_range(self.ui.dither_cfg.wave_amplitude);
            }
            "bgDitherColor" => {
                self.ui.dither_cfg.wave_color = cycle_color(self.ui.dither_cfg.wave_color)
            }
            "bgDitherBgColor" => {
                self.ui.dither_cfg.bg_color = cycle_color(self.ui.dither_cfg.bg_color)
            }
            "bgDitherPixelSize" => {
                self.ui.dither_cfg.pixel_size = step_range(self.ui.dither_cfg.pixel_size)
            }
            "bgDitherColorNum" => {
                self.ui.dither_cfg.color_num = step_range(self.ui.dither_cfg.color_num)
            }
            "bgParticlesCount" => {
                self.ui.particles_cfg.particle_count =
                    step_range(self.ui.particles_cfg.particle_count as f32).round() as u32;
            }
            "bgParticlesSpeed" => {
                self.ui.particles_cfg.speed = step_range(self.ui.particles_cfg.speed)
            }
            "bgParticlesSpread" => {
                self.ui.particles_cfg.particle_spread =
                    step_range(self.ui.particles_cfg.particle_spread);
            }
            "bgParticlesColor" => {
                let first = self
                    .ui
                    .particles_cfg
                    .particle_colors
                    .first()
                    .copied()
                    .unwrap_or([1.0, 1.0, 1.0]);
                self.ui.particles_cfg.particle_colors = vec![cycle_color(first)];
            }
            "bgParticlesSize" => {
                self.ui.particles_cfg.particle_base_size =
                    step_range(self.ui.particles_cfg.particle_base_size);
            }
            "bgParticlesAlpha" => {
                self.ui.particles_cfg.alpha_particles = !self.ui.particles_cfg.alpha_particles;
            }
            "bgFaultyScale" => self.ui.faulty_cfg.scale = step_range(self.ui.faulty_cfg.scale),
            "bgFaultyGlitch" => {
                self.ui.faulty_cfg.glitch_amount = step_range(self.ui.faulty_cfg.glitch_amount)
            }
            "bgFaultyScanlines" => {
                self.ui.faulty_cfg.scanline_intensity =
                    step_range(self.ui.faulty_cfg.scanline_intensity);
            }
            "bgFaultyFlicker" => {
                self.ui.faulty_cfg.flicker_amount = step_range(self.ui.faulty_cfg.flicker_amount)
            }
            "bgFaultyAberration" => {
                self.ui.faulty_cfg.chromatic_aberration =
                    step_range(self.ui.faulty_cfg.chromatic_aberration);
            }
            "bgFaultyCurvature" => {
                self.ui.faulty_cfg.curvature = step_range(self.ui.faulty_cfg.curvature)
            }
            "bgFaultyTint" => self.ui.faulty_cfg.tint = cycle_color(self.ui.faulty_cfg.tint),
            "bgFaultyBrightness" => {
                self.ui.faulty_cfg.brightness = step_range(self.ui.faulty_cfg.brightness)
            }
            "bgGridmotionAutoMove" => {
                self.ui.gridmotion_cfg.auto_move = !self.ui.gridmotion_cfg.auto_move;
            }
            "bgGridmotionScale" => {
                self.ui.gridmotion_cfg.scale = step_range(self.ui.gridmotion_cfg.scale)
            }
            "bgGridmotionAspectRatio" => {
                self.ui.gridmotion_cfg.aspect_ratio = match self.ui.gridmotion_cfg.aspect_ratio {
                    GridAspectRatio::Ratio16x9 => GridAspectRatio::Ratio1x1,
                    GridAspectRatio::Ratio1x1 => GridAspectRatio::Ratio3x4,
                    GridAspectRatio::Ratio3x4 => GridAspectRatio::Ratio4x3,
                    GridAspectRatio::Ratio4x3 => GridAspectRatio::Auto,
                    GridAspectRatio::Auto => GridAspectRatio::Ratio16x9,
                };
            }
            "bgGridmotionSpeed" => {
                self.ui.gridmotion_cfg.speed = step_range(self.ui.gridmotion_cfg.speed)
            }
            "bgGridmotionMaxMove" => {
                self.ui.gridmotion_cfg.max_move = step_range(self.ui.gridmotion_cfg.max_move)
            }
            "bgGridmotionGradientColor" => {
                self.ui.gridmotion_cfg.gradient_color =
                    cycle_color(self.ui.gridmotion_cfg.gradient_color);
            }
            _ => return,
        }
        self.persist_bg_control_pref(desc.bg);
    }

    fn persist_bg_control_pref(&mut self, bg: WelcomeBackgroundKind) {
        let rgb_to_hex = |rgb: [f32; 3]| -> String {
            format!(
                "#{:02x}{:02x}{:02x}",
                (rgb[0].clamp(0.0, 1.0) * 255.0).round() as u8,
                (rgb[1].clamp(0.0, 1.0) * 255.0).round() as u8,
                (rgb[2].clamp(0.0, 1.0) * 255.0).round() as u8,
            )
        };

        if self.ui.welcome_quality != QualityTier::Custom {
            self.ui.welcome_quality = QualityTier::Custom;
            self.db.preferences.set("welcomeQuality", &json!("custom"));
        }

        match bg {
            WelcomeBackgroundKind::Dither => {
                let raw = json!({
                    "speed": self.ui.dither_cfg.wave_speed,
                    "frequency": self.ui.dither_cfg.wave_frequency,
                    "amplitude": self.ui.dither_cfg.wave_amplitude,
                    "color": rgb_to_hex(self.ui.dither_cfg.wave_color),
                    "bgColor": rgb_to_hex(self.ui.dither_cfg.bg_color),
                    "pixelSize": self.ui.dither_cfg.pixel_size.round() as u32,
                    "colorNum": self.ui.dither_cfg.color_num.round() as u32,
                })
                .to_string();
                self.db.preferences.bg_opts_dither = Some(raw);
            }
            WelcomeBackgroundKind::Particles => {
                let first_col = self
                    .ui
                    .particles_cfg
                    .particle_colors
                    .first()
                    .copied()
                    .unwrap_or([1.0, 1.0, 1.0]);
                let raw = json!({
                    "count": self.ui.particles_cfg.particle_count,
                    "speed": self.ui.particles_cfg.speed,
                    "spread": self.ui.particles_cfg.particle_spread,
                    "color": rgb_to_hex(first_col),
                    "size": self.ui.particles_cfg.particle_base_size,
                    "alpha": self.ui.particles_cfg.alpha_particles,
                })
                .to_string();
                self.db.preferences.bg_opts_particles = Some(raw);
            }
            WelcomeBackgroundKind::Faulty => {
                let raw = json!({
                    "scale": self.ui.faulty_cfg.scale,
                    "glitch": self.ui.faulty_cfg.glitch_amount,
                    "scanlines": self.ui.faulty_cfg.scanline_intensity,
                    "flicker": self.ui.faulty_cfg.flicker_amount,
                    "aberration": self.ui.faulty_cfg.chromatic_aberration,
                    "curvature": self.ui.faulty_cfg.curvature,
                    "tint": rgb_to_hex(self.ui.faulty_cfg.tint),
                    "brightness": self.ui.faulty_cfg.brightness,
                })
                .to_string();
                self.db.preferences.bg_opts_faulty = Some(raw);
            }
            WelcomeBackgroundKind::Gridmotion => {
                let aspect_str = match self.ui.gridmotion_cfg.aspect_ratio {
                    GridAspectRatio::Ratio16x9 => "16:9",
                    GridAspectRatio::Ratio1x1 => "1:1",
                    GridAspectRatio::Ratio3x4 => "3:4",
                    GridAspectRatio::Ratio4x3 => "4:3",
                    GridAspectRatio::Auto => "auto",
                };
                let raw = json!({
                    "autoMove": self.ui.gridmotion_cfg.auto_move,
                    "scale": self.ui.gridmotion_cfg.scale,
                    "aspectRatio": aspect_str,
                    "speed": self.ui.gridmotion_cfg.speed,
                    "maxMove": self.ui.gridmotion_cfg.max_move,
                    "gradientColor": rgb_to_hex(self.ui.gridmotion_cfg.gradient_color),
                    "customPictures": self.gridmotion_custom_pictures,
                })
                .to_string();
                self.db.preferences.bg_opts_gridmotion = Some(raw);
            }
            WelcomeBackgroundKind::None => {}
        }
        self.save_db();
    }

    pub fn set_focused_input(&mut self, field: Option<FocusedTextField>) {
        self.focused_input = field;
        self.ui.text_field_focus(field);
    }

    pub fn append_text_to_focused_input(&mut self, text: &str) {
        let Some(field) = self.focused_input else {
            return;
        };
        self.ui.text_field_insert(field, text);
    }

    pub fn backspace_focused_input(&mut self) {
        let Some(field) = self.focused_input else {
            return;
        };
        self.ui.text_field_backspace(field);
    }

    pub fn delete_focused_input(&mut self) {
        let Some(field) = self.focused_input else {
            return;
        };
        self.ui.text_field_delete(field);
    }

    pub fn move_focused_input_left(&mut self, select: bool) {
        let Some(field) = self.focused_input else {
            return;
        };
        self.ui.text_field_move_left(field, select);
    }

    pub fn move_focused_input_right(&mut self, select: bool) {
        let Some(field) = self.focused_input else {
            return;
        };
        self.ui.text_field_move_right(field, select);
    }

    pub fn home_focused_input(&mut self, select: bool) {
        let Some(field) = self.focused_input else {
            return;
        };
        self.ui.text_field_home(field, select);
    }

    pub fn end_focused_input(&mut self, select: bool) {
        let Some(field) = self.focused_input else {
            return;
        };
        self.ui.text_field_end(field, select);
    }

    pub fn select_all_focused_input(&mut self) {
        let Some(field) = self.focused_input else {
            return;
        };
        self.ui.text_field_select_all(field);
    }

    pub fn copy_focused_input(&self) -> bool {
        let Some(field) = self.focused_input else {
            return false;
        };
        let text = self
            .ui
            .text_field_selected_text(field)
            .unwrap_or_else(|| self.ui.text_field_parts(field).0.to_string());
        if text.is_empty() {
            return false;
        }
        hybrid_win32_shell::clipboard::set_clipboard_text(&text)
    }

    pub fn cut_focused_input(&mut self) -> bool {
        let Some(field) = self.focused_input else {
            return false;
        };
        if let Some(sel) = self.ui.text_field_selected_text(field) {
            if hybrid_win32_shell::clipboard::set_clipboard_text(&sel) {
                self.ui.text_field_backspace(field);
                return true;
            }
        }
        false
    }

    pub fn paste_focused_input(&mut self) -> bool {
        let Some(field) = self.focused_input else {
            return false;
        };
        if let Some(text) = hybrid_win32_shell::clipboard::get_clipboard_text() {
            let single_line = text.replace(['\r', '\n'], "");
            self.ui.text_field_insert(field, &single_line);
            return true;
        }
        false
    }

    pub fn open_file_dialog(&mut self, multi: bool, replace_playlist: bool) {
        let kind = if multi {
            FileDialogKind::MediaMulti
        } else {
            FileDialogKind::MediaSingle
        };
        let paths = pick_native_paths(kind, self.owner_hwnd);
        if !paths.is_empty() {
            self.open_files(&paths, replace_playlist, replace_playlist);
        }
    }

    pub fn open_folder_dialog(&mut self) {
        let paths = pick_native_paths(FileDialogKind::Folder, self.owner_hwnd);
        if !paths.is_empty() {
            self.open_files(&paths, true, true);
        }
    }

    pub fn open_recording_folder(&mut self) {
        let dir = if !self.db.preferences.screenshot_dir.trim().is_empty() {
            PathBuf::from(&self.db.preferences.screenshot_dir)
        } else {
            self.default_screenshot_dir()
        };
        let _ = std::fs::create_dir_all(&dir);
        #[cfg(windows)]
        {
            let _ = std::process::Command::new("explorer.exe").arg(&dir).spawn();
        }
    }

    pub fn toggle_play(&mut self) {
        if self.current_file.is_none() || self.ui.welcome_active {
            return;
        }
        if let Some(mpv) = &self.mpv {
            if let Ok(paused) = mpv.toggle_pause() {
                self.ui.playing = !paused;
                self.ui.center_play_visible = false;
            }
        } else {
            self.ui.playing = !self.ui.playing;
            self.ui.center_play_visible = false;
        }
    }

    pub fn set_locked(&mut self, locked: bool) {
        let was_locked = self.ui.locked;
        self.ui.locked = locked;
        if locked {
            self.flush_eq_debounce();
            self.ui.close_modals();
            self.set_focused_input(None);
            self.ui.playlist.open = false;
            self.ui.anim.playlist_closing = false;
            self.ui.anim.playlist_t = 1.0;
            if !was_locked {
                let raw_combo = self
                    .ui
                    .custom_shortcuts
                    .get("unlock-ui")
                    .cloned()
                    .unwrap_or_else(|| {
                        ShortcutManager::get_default_binding("unlock-ui").to_string()
                    });
                let raw_combo = if raw_combo.trim().is_empty() {
                    self.ui
                        .custom_shortcuts
                        .get("toggle-lock")
                        .cloned()
                        .unwrap_or_else(|| {
                            ShortcutManager::get_default_binding("toggle-lock").to_string()
                        })
                } else {
                    raw_combo
                };
                let display_combo = format_combo_compact(&raw_combo);
                let msg = if display_combo.is_empty() {
                    "Controls locked".to_string()
                } else {
                    format!("Controls locked ({display_combo} to unlock)")
                };
                self.push_toast(ToastKind::Info, msg);
            }
        } else if was_locked {
            self.push_toast(ToastKind::Info, "Controls unlocked");
        }
    }

    fn clear_playback_ui_state(&mut self, window_title: &str) {
        self.burst_session_id = None;
        self.burst_in_flight = false;
        self.burst_seq = 0;
        self.ui.ab_loop = AbLoopState::Off;
        self.ui.playing = false;
        self.ui.center_play_visible = false;
        self.ui.loading_visible = false;
        self.ui.welcome_active = true;
        self.ui.is_live_stream = false;
        self.ui.audio_delay_secs = 0.0;
        self.ui.current_secs = 0.0;
        self.ui.duration_secs = 0.0;
        self.ui.buffered_secs = 0.0;
        self.ui.video_aspect_ratio = 16.0 / 9.0;
        self.ui.window_title = window_title.to_string();
        self.ui.chapter_positions_norm.clear();
        self.ui.chapter_times_secs.clear();
        self.ui.chapter_titles.clear();
        self.ui.current_chapter = None;
        self.ui.hover_thumb_norm = None;
        self.ui.hover_thumb_has_frame = false;
        self.ui.hover_thumb_loading = false;
        self.ui.audio_tracks.clear();
        self.ui.subtitle_tracks.clear();
        self.ui.is_youtube = false;
        self.ui.youtube_quality_label = "Auto".to_string();
        self.ui.carousel_mode = CarouselMode::Hidden;
        self.ui.stats_overlay.resolution.clear();
        self.ui.stats_overlay.bitrate.clear();
        self.ui.stats_overlay.fps.clear();
        self.ui.stats_overlay.codec.clear();
        self.ui.stats_overlay.hwdec.clear();
        self.ui.stats_overlay.dropped_frames = 0;
        self.ui.stats_overlay.cache_secs = 0.0;
        self.current_file = None;
        self.current_title = None;
        self.current_uploader = None;
        self.current_aid = None;
        self.current_sid = None;
        self.audio_tracks.clear();
        self.subtitle_tracks.clear();
        self.video_dimensions = None;
        self.video_fps = None;
        self.is_youtube = false;
        self.youtube_heights.clear();
        self.current_youtube_quality = "auto".to_string();
        self.single_screenshot_stack.clear();
        self.deleted_screenshot_stack.clear();
        self.session_screenshots.clear();
        self.ui.carousel_total = 0;
        self.ui.carousel_index = 0;
        self.ui.screenshot_badge = None;
        self.ui.screenshot_notice = None;
        if let Some(smtc) = &self.smtc {
            let _ = smtc.set_playback_status(SmtcPlaybackStatus::Stopped);
        }
    }

    pub fn stop_playlist_playback(&mut self) {
        self.cancel_clip_recording(false);
        self.playback_session_id = self.playback_session_id.saturating_add(1);
        self.pending_resume_seek = None;
        self.last_skip_at = None;
        if let Some(mpv) = &self.mpv {
            let _ = mpv.clear_ab_loop();
            let _ = mpv.stop();
        }
        self.clear_playback_ui_state("No media loaded");
        self.save_db();
    }

    pub fn reset_to_idle_state(&mut self) {
        self.cancel_clip_recording(false);
        self.playback_session_id = self.playback_session_id.saturating_add(1);
        self.pending_resume_seek = None;
        self.last_skip_at = None;
        self.clear_playback_ui_state("Hybrid Player");
    }

    pub fn stop_playback(&mut self) {
        self.save_current_resume_position();
        self.cancel_clip_recording(true);
        self.playback_session_id = self.playback_session_id.saturating_add(1);
        self.pending_resume_seek = None;
        self.last_skip_at = None;
        if let Some(mpv) = &self.mpv {
            let _ = mpv.clear_ab_loop();
            let _ = mpv.stop();
        }
        self.clear_playback_ui_state("Hybrid Player");
        self.save_db();
    }

    pub fn seek_relative(&mut self, delta: f64) {
        if self.ui.is_live_stream && delta > 0.0 && self.ui.duration_secs > 0.0 {
            let target = (self.ui.current_secs + delta).min(self.ui.duration_secs);
            let clamped_delta = (target - self.ui.current_secs).max(0.0);
            if clamped_delta <= 0.01 {
                return;
            }
            if let Some(mpv) = &self.mpv {
                let _ = mpv.seek(target, "absolute");
            }
            self.ui.current_secs = target;
            self.record_skip_osd(clamped_delta);
            return;
        }
        if let Some(mpv) = &self.mpv {
            let _ = mpv.seek_relative(delta);
        }
        self.record_skip_osd(delta);
    }

    fn record_skip_osd(&mut self, delta: f64) {
        let rounded = delta.round() as i32;
        if rounded == 0 {
            return;
        }
        let is_forward = rounded > 0;
        let now = Instant::now();
        let within_window = self
            .last_skip_at
            .is_some_and(|t| now.duration_since(t) < Duration::from_millis(800));
        if within_window
            && self.ui.skip_indicator.visible
            && self.ui.skip_indicator.forward == is_forward
        {
            self.ui.skip_indicator.accumulated_secs = self
                .ui
                .skip_indicator
                .accumulated_secs
                .saturating_add(rounded.abs());
        } else {
            self.ui.skip_indicator.forward = is_forward;
            self.ui.skip_indicator.accumulated_secs = rounded.abs();
        }
        self.last_skip_at = Some(now);
        self.ui.skip_indicator.visible = true;
        self.ui.anim.trigger_skip_osd();
    }

    pub fn preview_seek_normalized(&mut self, norm: f64) {
        if self.ui.is_live_stream {
            return;
        }
        let clamped = norm.clamp(0.0, 1.0);
        if self.ui.duration_secs > 0.0 {
            self.ui.current_secs = clamped * self.ui.duration_secs;
        }
    }

    pub fn seek_normalized(&mut self, norm: f64) {
        if self.ui.is_live_stream {
            return;
        }
        let clamped = norm.clamp(0.0, 1.0);
        if self.ui.duration_secs > 0.0 {
            let target = clamped * self.ui.duration_secs;
            self.ui.current_secs = target;
            if let Some(mpv) = &self.mpv {
                let _ = mpv.seek(target, "absolute+exact");
            }
        }
    }

    pub fn adjust_volume(&mut self, delta: i32) {
        let max_vol = if self.ui.volume_boost_enabled {
            200
        } else {
            100
        };
        let next = (self.ui.volume_pct as i32 + delta).clamp(0, max_vol) as u32;
        self.set_volume_pct(next);
    }

    pub fn set_volume_pct(&mut self, pct: u32) {
        let max_vol = if self.ui.volume_boost_enabled {
            200
        } else {
            100
        };
        let clamped = pct.min(max_vol);
        self.ui.volume_pct = clamped;
        if self.current_file.is_some() {
            self.ui.volume_osd.visible = true;
            self.ui.volume_osd.percent = clamped;
            self.ui.anim.trigger_volume_osd();
        }
        if let Some(mpv) = &self.mpv {
            let _ = mpv.set_volume(clamped as f64);
        }
        let norm = (clamped as f64) / 100.0;
        self.db.preferences.set("volume", &json!(norm));
        self.save_db();
    }

    pub fn toggle_mute(&mut self) {
        let next = !self.ui.muted;
        self.ui.muted = next;
        if let Some(mpv) = &self.mpv {
            let _ = mpv.set_mute(next);
        }
    }

    pub fn set_speed(&mut self, speed: f64) {
        let clamped = speed.clamp(0.1, 16.0);
        self.ui.speed = clamped;
        if let Some(mpv) = &self.mpv {
            let _ = mpv.set_speed(clamped);
        }
        if let Some(path) = self.current_file.as_deref() {
            self.db.save_speed(path, clamped);
            self.save_db();
        }
    }

    pub fn next_track(&mut self) {
        if let NavigationAction::PlayIndex(idx) = self.playlist.play_next(false) {
            self.play_playlist_index(idx);
        }
    }

    pub fn previous_track(&mut self) {
        match self.playlist.play_previous(self.ui.current_secs) {
            NavigationAction::PlayIndex(idx) => self.play_playlist_index(idx),
            NavigationAction::RestartCurrent => {
                if let Some(mpv) = &self.mpv {
                    let _ = mpv.seek(0.0, "absolute");
                }
                self.ui.current_secs = 0.0;
            }
            NavigationAction::None => {}
        }
    }

    pub fn toggle_dropdown(&mut self, kind: DropdownKind) {
        self.ui.active_color_picker = None;
        if kind == DropdownKind::YoutubeQuality {
            self.ui.youtube_quality_options = self.youtube_quality_options();
        }
        if self.ui.active_dropdown == Some(kind) {
            self.ui.active_dropdown = None;
        } else {
            self.ui.active_dropdown = Some(kind);
        }
    }

    pub fn apply_dropdown_option(&mut self, idx: usize) {
        let Some(kind) = self.ui.active_dropdown.take() else {
            return;
        };
        match kind {
            DropdownKind::SettingsTheme => {
                const THEMES: [(ThemeMode, &str); 3] = [
                    (ThemeMode::Dark, "dark"),
                    (ThemeMode::Oled, "oled"),
                    (ThemeMode::Light, "light"),
                ];
                if let Some(&(mode, key)) = THEMES.get(idx) {
                    self.ui.theme = mode;
                    self.db.preferences.set("theme", &json!(key));
                    self.save_db();
                }
            }
            DropdownKind::SettingsMotion => {
                const PROFILES: [(MotionProfile, &str); 3] = [
                    (MotionProfile::Reduced, "reduced"),
                    (MotionProfile::Balanced, "balanced"),
                    (MotionProfile::Showcase, "showcase"),
                ];
                if let Some(&(prof, key)) = PROFILES.get(idx) {
                    self.ui.motion_profile = prof;
                    self.db.preferences.set("motionProfile", &json!(key));
                    self.save_db();
                }
            }
            DropdownKind::SettingsSeekStep => {
                const STEPS: [u32; 3] = [3, 5, 10];
                if let Some(&step) = STEPS.get(idx) {
                    self.ui.seek_step_secs = step;
                    self.db.preferences.set("seekStep", &json!(step));
                    self.save_db();
                }
            }
            DropdownKind::SettingsScreenshotFormat => {
                const FORMATS: [&str; 3] = ["jpg", "png", "webp"];
                if let Some(&fmt_str) = FORMATS.get(idx) {
                    self.ui.screenshot_format = fmt_str.to_string();
                    self.db.preferences.set("screenshotFormat", &json!(fmt_str));
                    if let Some(mpv) = &self.mpv {
                        let fmt = ScreenshotFormat::from_str_lossy(fmt_str)
                            .unwrap_or(ScreenshotFormat::Jpg);
                        let _ = mpv.set_screenshot_format(fmt);
                    }
                    self.save_db();
                }
            }
            DropdownKind::SettingsMouseScreenshot => {
                const BTNS: [i32; 4] = [3, 4, 1, -1];
                if let Some(&btn) = BTNS.get(idx) {
                    self.ui.set_screenshot_mouse_button(btn);
                    self.db
                        .preferences
                        .set("screenshotMouseButton", &json!(btn));
                    self.save_db();
                }
            }
            DropdownKind::EqPreset => {
                if let Some(&preset) = EqualizerPreset::BUILTIN_PRESETS.get(idx) {
                    self.set_eq_preset(preset);
                } else if idx == EqualizerPreset::BUILTIN_PRESETS.len() {
                    self.equalizer.preset = EqualizerPreset::Custom;
                    self.ui.equalizer = self.equalizer.clone();
                    self.apply_and_persist_equalizer();
                }
            }
            DropdownKind::YoutubeQuality => {
                let options = self.youtube_quality_options();
                if let Some(q) = options.get(idx).cloned() {
                    self.apply_youtube_quality(&q);
                }
            }
            DropdownKind::BgGridmotionAspectRatio => {
                const RATIOS: [GridAspectRatio; 5] = [
                    GridAspectRatio::Ratio16x9,
                    GridAspectRatio::Ratio1x1,
                    GridAspectRatio::Ratio3x4,
                    GridAspectRatio::Ratio4x3,
                    GridAspectRatio::Auto,
                ];
                if let Some(&ar) = RATIOS.get(idx) {
                    self.ui.gridmotion_cfg.aspect_ratio = ar;
                    self.persist_bg_control_pref(WelcomeBackgroundKind::Gridmotion);
                }
            }
        }
    }

    fn current_color_picker_rgb(&self) -> [f32; 3] {
        match self.ui.active_color_picker.as_ref() {
            Some(ColorPickerTarget::Accent) => {
                parse_hex_rgb(&self.ui.accent_hex).unwrap_or([0.231, 0.510, 0.965])
            }
            Some(ColorPickerTarget::SubtitleFontColor) => {
                parse_hex_rgb(&self.ui.subtitle_font_color).unwrap_or([1.0, 1.0, 1.0])
            }
            Some(ColorPickerTarget::SubtitleBgColor) => {
                parse_hex_rgb(&self.ui.subtitle_bg_color).unwrap_or([0.0, 0.0, 0.0])
            }
            Some(ColorPickerTarget::BgControl(id)) => match id.as_str() {
                "bgDitherColor" => self.ui.dither_cfg.wave_color,
                "bgDitherBgColor" => self.ui.dither_cfg.bg_color,
                "bgParticlesColor" => self
                    .ui
                    .particles_cfg
                    .particle_colors
                    .first()
                    .copied()
                    .unwrap_or([1.0, 1.0, 1.0]),
                "bgFaultyTint" => self.ui.faulty_cfg.tint,
                "bgGridmotionGradientColor" => self.ui.gridmotion_cfg.gradient_color,
                _ => [1.0, 1.0, 1.0],
            },
            None => [1.0, 1.0, 1.0],
        }
    }

    pub fn toggle_color_picker(&mut self, target: ColorPickerTarget) {
        self.ui.active_dropdown = None;
        if self.ui.active_color_picker.as_ref() == Some(&target) {
            self.ui.active_color_picker = None;
        } else {
            self.ui.active_color_picker = Some(target);
            let rgb = self.current_color_picker_rgb();
            let (h, s, _v) = rgb_to_hsv(rgb);
            if s > 1e-3 {
                self.ui.color_picker_hue = h;
            }
        }
    }

    fn apply_color_picker_rgb(&mut self, rgb: [f32; 3]) {
        let Some(target) = self.ui.active_color_picker.clone() else {
            return;
        };
        let clamped = [
            rgb[0].clamp(0.0, 1.0),
            rgb[1].clamp(0.0, 1.0),
            rgb[2].clamp(0.0, 1.0),
        ];
        let hex = format!(
            "#{:02x}{:02x}{:02x}",
            (clamped[0] * 255.0).round() as u8,
            (clamped[1] * 255.0).round() as u8,
            (clamped[2] * 255.0).round() as u8,
        );
        match target {
            ColorPickerTarget::Accent => {
                self.ui.accent_hex = hex.clone();
                self.db.preferences.set("accentColor", &json!(hex));
                self.save_db();
            }
            ColorPickerTarget::SubtitleFontColor => {
                self.ui.subtitle_font_color = hex.clone();
                self.subtitle_style.font_color = hex.clone();
                if let Some(mpv) = &self.mpv {
                    self.apply_subtitle_style_to_mpv(mpv);
                }
                self.db.preferences.set("subFontColor", &json!(hex));
                self.save_db();
            }
            ColorPickerTarget::SubtitleBgColor => {
                self.ui.subtitle_bg_color = hex.clone();
                self.subtitle_style.bg_color = hex.clone();
                if let Some(mpv) = &self.mpv {
                    self.apply_subtitle_style_to_mpv(mpv);
                }
                self.db.preferences.set("subBgColor", &json!(hex));
                self.save_db();
            }
            ColorPickerTarget::BgControl(id) => {
                let bg = match id.as_str() {
                    "bgDitherColor" => {
                        self.ui.dither_cfg.wave_color = clamped;
                        WelcomeBackgroundKind::Dither
                    }
                    "bgDitherBgColor" => {
                        self.ui.dither_cfg.bg_color = clamped;
                        WelcomeBackgroundKind::Dither
                    }
                    "bgParticlesColor" => {
                        self.ui.particles_cfg.particle_colors = vec![clamped];
                        WelcomeBackgroundKind::Particles
                    }
                    "bgFaultyTint" => {
                        self.ui.faulty_cfg.tint = clamped;
                        WelcomeBackgroundKind::Faulty
                    }
                    "bgGridmotionGradientColor" => {
                        self.ui.gridmotion_cfg.gradient_color = clamped;
                        WelcomeBackgroundKind::Gridmotion
                    }
                    _ => return,
                };
                self.persist_bg_control_pref(bg);
            }
        }
    }

    pub fn apply_color_picker_swatch(&mut self, idx: usize) {
        if let Some(&hex) = hybrid_ui::state::COLOR_PICKER_SWATCHES.get(idx) {
            if let Some(rgb) = parse_hex_rgb(hex) {
                let (h, s, _) = rgb_to_hsv(rgb);
                if s > 1e-3 {
                    self.ui.color_picker_hue = h;
                }
                self.apply_color_picker_rgb(rgb);
            }
        }
    }

    pub fn apply_color_picker_spectrum(&mut self, rx: f32, ry: f32) {
        let s = rx.clamp(0.0, 1.0);
        let v = (1.0 - ry).clamp(0.0, 1.0);
        let rgb = hsv_to_rgb(self.ui.color_picker_hue, s, v);
        self.apply_color_picker_rgb(rgb);
    }

    pub fn apply_color_picker_channel(&mut self, ch: u8, rx: f32) {
        let rx = rx.clamp(0.0, 1.0);
        let mut rgb = self.current_color_picker_rgb();
        if ch == 0 {
            let hue = rx * 360.0;
            self.ui.color_picker_hue = hue;
            let (_, s, v) = rgb_to_hsv(rgb);
            rgb = hsv_to_rgb(hue, s.max(0.01), v.max(0.01));
        } else {
            let idx = (ch - 1).min(2) as usize;
            rgb[idx] = rx;
            let (h, s, _) = rgb_to_hsv(rgb);
            if s > 1e-3 {
                self.ui.color_picker_hue = h;
            }
        }
        self.apply_color_picker_rgb(rgb);
    }

    pub fn toggle_playlist(&mut self) {
        self.ui.active_dropdown = None;
        self.ui.active_color_picker = None;
        self.ui.playlist.open = !self.ui.playlist.open;
        if self.ui.playlist.open {
            // The playlist drawer and the Welcome Effects drawer are independent
            // surfaces: opening the queue must not dismiss or reset background
            // settings. Both are composed side-by-side by the layout.
            self.ui.anim.trigger_playlist_in();
        } else {
            self.ui.anim.trigger_playlist_out();
        }
    }

    pub fn toggle_modal(&mut self, modal: ModalId) {
        if modal == ModalId::BgSettings && !self.ui.welcome_active {
            if self.ui.active_modal == Some(ModalId::BgSettings) {
                self.ui.close_modals_animated();
            }
            self.ui.bg_accordion_open = false;
            return;
        }
        if self.ui.active_modal == Some(modal) {
            self.ui.close_modals_animated();
        } else {
            if modal == ModalId::Settings {
                self.ui.settings_scroll_y = 0.0;
                self.ui.shortcut_scroll_y = 0.0;
            } else if modal == ModalId::BgSettings {
                self.ui.bg_settings_scroll_y = 0.0;
            }
            self.ui.open_modal_animated(modal);
        }
    }

    pub fn scroll_settings_modal(&mut self, delta_px: f32) {
        if self.ui.active_modal == Some(ModalId::Settings) && delta_px.is_finite() {
            let max_scroll = hybrid_ui::layout::settings_scroll_max(&self.ui);
            self.ui.settings_scroll_y =
                (self.ui.settings_scroll_y + delta_px).clamp(0.0, max_scroll);
        }
    }

    pub fn scroll_bg_settings_modal(&mut self, delta_px: f32) {
        if self.ui.active_modal == Some(ModalId::BgSettings) && delta_px.is_finite() {
            let max_scroll = hybrid_ui::layout::bg_settings_scroll_max(&self.ui);
            self.ui.bg_settings_scroll_y =
                (self.ui.bg_settings_scroll_y + delta_px).clamp(0.0, max_scroll);
        }
    }

    pub fn set_eq_band(&mut self, idx: usize, gain_db: f64) {
        self.eq_debounce_deadline = None;
        self.equalizer.set_band(idx, gain_db);
        self.ui.equalizer = self.equalizer.clone();
        self.apply_and_persist_equalizer();
    }

    pub fn set_eq_band_debounced(&mut self, idx: usize, gain_db: f64) {
        self.equalizer.set_band(idx, gain_db);
        self.ui.equalizer = self.equalizer.clone();
        self.eq_debounce_deadline = Some(Instant::now() + Duration::from_millis(100));
    }

    pub fn flush_eq_debounce(&mut self) {
        if self.eq_debounce_deadline.take().is_some() {
            self.apply_and_persist_equalizer();
        }
    }

    pub fn set_eq_preset(&mut self, preset: EqualizerPreset) {
        self.eq_debounce_deadline = None;
        self.equalizer.apply_preset(preset);
        self.ui.equalizer = self.equalizer.clone();
        self.apply_and_persist_equalizer();
    }

    pub fn reset_equalizer(&mut self) {
        self.eq_debounce_deadline = None;
        self.equalizer = EqualizerState::default();
        self.ui.equalizer = self.equalizer.clone();
        self.apply_and_persist_equalizer();
    }

    fn apply_and_persist_equalizer(&mut self) {
        let af = self.equalizer.mpv_af_value();
        if let Some(mpv) = &self.mpv {
            let _ = mpv.set_property_string("af", &af);
        }
        self.db
            .preferences
            .set("equalizerPreset", &json!(self.equalizer.preset.as_str()));
        self.db
            .preferences
            .set("equalizerBands", &json!(self.equalizer.bands.to_vec()));
        self.save_db();
    }

    pub fn cycle_audio_track(&mut self) {
        if let Some(next) = next_audio_track(&self.audio_tracks, self.current_aid).cloned() {
            self.current_aid = Some(next.id);
            if let Some(mpv) = &self.mpv {
                let _ = mpv.set_audio(&next.id.to_string());
            }
            let label = if !next.title.is_empty() {
                next.title
            } else if !next.lang.is_empty() {
                next.lang
            } else {
                format!("Track {}", next.id)
            };
            self.push_toast(ToastKind::Info, format!("Audio: {label}"));
        }
    }

    pub fn cycle_subtitle_track(&mut self) {
        match next_subtitle_track(&self.subtitle_tracks, self.current_sid).cloned() {
            Some(next) => {
                self.current_sid = Some(next.id);
                self.sub_visible = true;
                for t in &mut self.ui.subtitle_tracks {
                    t.selected = t.id == next.id;
                }
                if let Some(mpv) = &self.mpv {
                    let _ = mpv.set_sub(&next.id.to_string());
                    let _ = mpv.set_sub_visibility(true);
                }
            }
            None => {
                self.current_sid = None;
                self.sub_visible = false;
                for t in &mut self.ui.subtitle_tracks {
                    t.selected = false;
                }
                if let Some(mpv) = &self.mpv {
                    let _ = mpv.set_sub_visibility(false);
                }
            }
        }
    }

    pub fn apply_subs_dubs_preset(&mut self, mode: SubsDubsMode) {
        let Some(sel) = select_subs_dubs_mode(&self.audio_tracks, &self.subtitle_tracks, mode)
        else {
            return;
        };
        if let Some(aid) = sel.audio_track_id {
            self.current_aid = Some(aid);
            for t in &mut self.ui.audio_tracks {
                t.selected = t.id == aid;
            }
            if let Some(mpv) = &self.mpv {
                let _ = mpv.set_audio(&aid.to_string());
            }
        }
        self.current_sid = sel.subtitle_track_id;
        self.sub_visible = sel.subtitle_visible;
        for t in &mut self.ui.subtitle_tracks {
            t.selected = sel.subtitle_visible && Some(t.id) == sel.subtitle_track_id;
        }
        if let Some(mpv) = &self.mpv {
            if let Some(sid) = sel.subtitle_track_id {
                let _ = mpv.set_sub(&sid.to_string());
            }
            let _ = mpv.set_sub_visibility(sel.subtitle_visible);
        }
    }

    pub fn adjust_subtitle_delay(&mut self, delta_sec: f64) {
        let next = (self.ui.subtitle_delay_secs + delta_sec)
            .clamp(SUBTITLE_SYNC_MIN_SEC, SUBTITLE_SYNC_MAX_SEC);
        self.ui.subtitle_delay_secs = next;
        if let Some(mpv) = &self.mpv {
            let _ = mpv.set_sub_delay(next);
        }
        if let Some(path) = self.current_file.as_deref() {
            self.db
                .save_subtitle_delay(path, subtitle_sec_to_ms(next) as f64);
            self.save_db();
        }
    }

    pub fn reset_audio_delay(&mut self) {
        self.ui.audio_delay_secs = 0.0;
        if let Some(mpv) = &self.mpv {
            let _ = mpv.set_audio_delay(0.0);
        }
    }

    pub fn adjust_audio_delay(&mut self, delta_ms: i64) {
        let cur_ms = (self.ui.audio_delay_secs * 1000.0).round() as i64;
        let next_ms = clamp_audio_sync_ms(cur_ms + delta_ms);
        let next_sec = (next_ms as f64) / 1000.0;
        self.ui.audio_delay_secs = next_sec;
        if let Some(mpv) = &self.mpv {
            let _ = mpv.set_audio_delay(next_sec);
        }
    }

    pub fn cycle_ab_loop(&mut self) {
        let cur = self.ui.current_secs.max(0.0);
        let (next, toast) = match self.ui.ab_loop {
            AbLoopState::Off => {
                if let Some(mpv) = &self.mpv {
                    let _ = mpv.set_ab_loop_a(cur);
                }
                let a_ms = (cur * 1000.0).round() as u64;
                (
                    AbLoopState::PointASet(a_ms),
                    (
                        ToastKind::Info,
                        format!("Point A set at {}", UiState::format_time(cur)),
                    ),
                )
            }
            AbLoopState::PointASet(a_ms) => {
                let a_sec = (a_ms as f64) / 1000.0;
                if (cur - a_sec).abs() < 0.25 {
                    (
                        AbLoopState::PointASet(a_ms),
                        (
                            ToastKind::Info,
                            "Loop duration must be at least 0.25 seconds".to_string(),
                        ),
                    )
                } else {
                    let (a_out, b_out) = if cur < a_sec {
                        if let Some(mpv) = &self.mpv {
                            let _ = mpv.set_ab_loop_a(cur);
                            let _ = mpv.set_ab_loop_b(a_sec);
                        }
                        ((cur * 1000.0).round() as u64, a_ms)
                    } else {
                        if let Some(mpv) = &self.mpv {
                            let _ = mpv.set_ab_loop_b(cur);
                        }
                        (a_ms, (cur * 1000.0).round() as u64)
                    };
                    (
                        AbLoopState::Active(a_out, b_out),
                        (ToastKind::Info, "Loop A-B active".to_string()),
                    )
                }
            }
            AbLoopState::Active(_, _) => {
                if let Some(mpv) = &self.mpv {
                    let _ = mpv.clear_ab_loop();
                }
                (
                    AbLoopState::Off,
                    (ToastKind::Info, "Loop cleared".to_string()),
                )
            }
        };
        self.ui.ab_loop = next;
        self.push_toast(toast.0, toast.1);
    }

    // ─── Screenshots, Burst, Undo/Restore, Clip Recording ────────────────────

    pub fn take_single_screenshot(&mut self, fmt_override: Option<ScreenshotFormat>) {
        let Some(mpv) = &self.mpv else {
            return;
        };
        let mpv_clone = Arc::clone(mpv);
        let tx = self.async_tx.clone();
        self.rt.spawn(async move {
            let res = mpv_clone
                .screenshot(ScreenshotMode::Video, fmt_override)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(BridgeAsyncMsg::SingleScreenshotCaptured(res));
        });
    }

    pub fn start_burst_capture(&mut self) {
        if self.burst_session_id.is_some() {
            return;
        }
        let session_id = format!("{}", now_millis());
        self.burst_session_id = Some(session_id);
        self.burst_seq = 0;
        self.burst_in_flight = false;
        self.ui.recording_indicator = RecordingIndicatorMode::BurstActive { count: 0 };
        self.burst_tick_frame();
    }

    pub fn burst_tick_frame(&mut self) {
        if self.burst_in_flight {
            return;
        }
        let Some(session_id) = self.burst_session_id.clone() else {
            return;
        };
        let Some(mpv) = &self.mpv else {
            return;
        };
        self.burst_in_flight = true;
        let seq = self.burst_seq.saturating_add(1);
        let mpv_clone = Arc::clone(mpv);
        let tx = self.async_tx.clone();
        self.rt.spawn(async move {
            let res = mpv_clone
                .screenshot_burst_frame(&session_id, seq, ScreenshotMode::Video)
                .await
                .map(|info| info.frame_path)
                .map_err(|e| e.to_string());
            let _ = tx.send(BridgeAsyncMsg::BurstFrameCaptured {
                session_id,
                result: res,
            });
        });
    }

    pub fn stop_burst_capture(&mut self) {
        self.burst_in_flight = false;
        let Some(session_id) = self.burst_session_id.take() else {
            return;
        };
        let count = self.burst_seq as usize;
        if let Some(mpv) = &self.mpv {
            let result = mpv.finalize_burst_session(&session_id, count);
            for f in result.files {
                if !self
                    .session_screenshots
                    .iter()
                    .any(|s| s.file_path == f.file_path)
                {
                    self.record_captured_screenshot(f.file_path);
                }
            }
            if result.count == 0 {
                self.ui.recording_indicator = RecordingIndicatorMode::Hidden;
                self.take_single_screenshot(None);
            } else if self.clip_started_at.is_some() {
                let unit = if result.count == 1 { "frame" } else { "frames" };
                self.push_toast(ToastKind::Info, format!("Saved {} {unit}", result.count));
            } else {
                self.ui.recording_saved_frame_count = result.count;
                self.ui.recording_indicator = RecordingIndicatorMode::Saved { label_frames: true };
                self.ui.anim.trigger_recording_saved();
            }
        } else {
            self.ui.recording_indicator = RecordingIndicatorMode::Hidden;
        }
    }

    fn stage_delete_file(target: &std::path::Path, file_name: &str) -> Option<PathBuf> {
        let staged_path = std::env::temp_dir().join(format!(
            "hybrid-undo-{}-{}-{file_name}",
            std::process::id(),
            now_millis()
        ));
        let staged_ok = if target.exists() {
            std::fs::rename(target, &staged_path).is_ok()
                || (std::fs::copy(target, &staged_path).is_ok() && trash_file(target).is_ok())
        } else {
            false
        };
        staged_ok.then_some(staged_path)
    }

    pub fn delete_latest_screenshot(&mut self) {
        let Some(target) = self.single_screenshot_stack.pop() else {
            self.ui.screenshot_notice = Some("No screenshots to delete".to_string());
            self.ui.anim.trigger_screenshot_notice();
            return;
        };
        let file_name = target
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("screenshot")
            .to_string();
        if let Some(staged_path) = Self::stage_delete_file(&target, &file_name) {
            self.session_screenshots.retain(|s| s.file_path != target);
            self.deleted_screenshot_stack.push(DeletedScreenshotRecord {
                original_path: target,
                staged_path,
                file_name: file_name.clone(),
            });
            self.ui.carousel_total = self.session_screenshots.len();
            self.ui.carousel_index = self.deleted_screenshot_stack.len().saturating_sub(1);
            self.ui.carousel_mode = CarouselMode::SinglePreview;
            self.ui.screenshot_badge = Some(("DELETED".to_string(), true));
            self.ui.anim.trigger_screenshot_delete();
            self.ui.screenshot_notice = None;
        } else {
            self.push_toast(ToastKind::Error, "Failed to delete screenshot");
        }
    }

    pub fn restore_latest_screenshot(&mut self) {
        let Some(rec) = self.deleted_screenshot_stack.pop() else {
            self.ui.screenshot_notice = Some("No screenshots to restore".to_string());
            self.ui.anim.trigger_screenshot_notice();
            return;
        };
        let restored = if rec.staged_path.exists() {
            if let Some(parent) = rec.original_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::rename(&rec.staged_path, &rec.original_path).is_ok()
        } else {
            false
        };
        if restored {
            self.push_captured_screenshot(rec.original_path);
            self.ui.screenshot_badge = Some(("RESTORED".to_string(), false));
            self.ui.anim.trigger_screenshot_in(1.8);
            self.ui.screenshot_notice = None;
        } else {
            self.push_toast(ToastKind::Error, "Failed to restore screenshot");
        }
    }

    pub fn toggle_carousel(&mut self, restore_mode: bool) {
        let desired = if restore_mode {
            CarouselMode::RestoreCarousel
        } else {
            CarouselMode::DeleteCarousel
        };
        if self.ui.carousel_mode == desired {
            self.ui.carousel_mode = CarouselMode::Hidden;
            self.ui.screenshot_badge = None;
            return;
        }
        let total = if restore_mode {
            self.deleted_screenshot_stack.len()
        } else {
            self.session_screenshots.len()
        };
        if total == 0 {
            self.ui.screenshot_notice = Some(if restore_mode {
                "No screenshots to restore".to_string()
            } else {
                "No screenshots to delete".to_string()
            });
            self.ui.anim.trigger_screenshot_notice();
            return;
        }
        self.ui.carousel_total = total;
        self.ui.carousel_index = total.saturating_sub(1);
        self.ui.carousel_mode = desired;
        self.ui.screenshot_badge = None;
        self.ui.screenshot_notice = None;
        self.ui.anim.trigger_screenshot_in(0.0);
    }

    pub fn navigate_carousel(&mut self, delta: i32) {
        if self.ui.carousel_total == 0 {
            return;
        }
        let len = self.ui.carousel_total as isize;
        let cur = self.ui.carousel_index as isize;
        let next = (cur + delta as isize).rem_euclid(len) as usize;
        self.ui.carousel_index = next;
    }

    pub fn confirm_carousel_action(&mut self) {
        match self.ui.carousel_mode {
            CarouselMode::DeleteCarousel => {
                let idx = self.ui.carousel_index;
                if idx >= self.session_screenshots.len() {
                    self.ui.carousel_mode = CarouselMode::Hidden;
                    self.ui.carousel_total = 0;
                    return;
                }
                let item = self.session_screenshots[idx].clone();
                if let Some(staged_path) = Self::stage_delete_file(&item.file_path, &item.file_name)
                {
                    self.session_screenshots.remove(idx);
                    self.single_screenshot_stack
                        .retain(|p| p != &item.file_path);
                    self.deleted_screenshot_stack.push(DeletedScreenshotRecord {
                        original_path: item.file_path,
                        staged_path,
                        file_name: item.file_name.clone(),
                    });
                    self.ui.carousel_total = self.session_screenshots.len();
                    self.ui.carousel_index = self.deleted_screenshot_stack.len().saturating_sub(1);
                    self.ui.carousel_mode = CarouselMode::SinglePreview;
                    self.ui.screenshot_badge = Some(("DELETED".to_string(), true));
                    self.ui.anim.trigger_screenshot_delete();
                    self.ui.screenshot_notice = None;
                } else {
                    self.push_toast(ToastKind::Error, "Failed to delete screenshot");
                }
            }
            CarouselMode::RestoreCarousel => {
                let idx = self.ui.carousel_index;
                if idx >= self.deleted_screenshot_stack.len() {
                    self.ui.carousel_mode = CarouselMode::Hidden;
                    self.ui.carousel_total = 0;
                    return;
                }
                let rec = self.deleted_screenshot_stack.remove(idx);
                let restored = if rec.staged_path.exists() {
                    if let Some(parent) = rec.original_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    std::fs::rename(&rec.staged_path, &rec.original_path).is_ok()
                } else {
                    false
                };
                if restored {
                    self.single_screenshot_stack.push(rec.original_path.clone());
                    self.session_screenshots.push(SessionScreenshot {
                        file_path: rec.original_path,
                        file_name: rec.file_name.clone(),
                        timestamp_ms: now_millis(),
                    });
                    if self.session_screenshots.len() > MAX_SESSION_SCREENSHOTS {
                        self.session_screenshots.remove(0);
                    }
                    self.ui.carousel_total = self.deleted_screenshot_stack.len();
                    self.ui.carousel_index = self.session_screenshots.len().saturating_sub(1);
                    self.ui.carousel_mode = CarouselMode::SinglePreview;
                    self.ui.screenshot_badge = Some(("RESTORED".to_string(), false));
                    self.ui.anim.trigger_screenshot_in(0.4);
                    self.ui.screenshot_notice = None;
                } else {
                    self.deleted_screenshot_stack.insert(idx, rec);
                    self.push_toast(ToastKind::Error, "Failed to restore screenshot");
                }
            }
            CarouselMode::SinglePreview | CarouselMode::Hidden => {}
        }
    }

    pub fn toggle_clip_recording(&mut self) {
        if let Some(start_secs) = self.clip_start_secs.take() {
            self.clip_started_at = None;
            let clip_file = self
                .active_clip_file
                .take()
                .or_else(|| self.current_file.clone());
            let end_secs = self.ui.current_secs;
            let duration = end_secs - start_secs;
            let Some(file_path) = clip_file else {
                self.ui.recording_indicator = RecordingIndicatorMode::Hidden;
                return;
            };
            if duration <= 0.2 {
                self.ui.recording_indicator = RecordingIndicatorMode::Hidden;
                self.push_toast(ToastKind::Error, "Clip duration is too short");
                return;
            }
            let Some(ffmpeg) = self.resolve_ffmpeg_bin() else {
                self.ui.recording_indicator = RecordingIndicatorMode::Hidden;
                self.push_toast(ToastKind::Error, "ffmpeg binary not found");
                return;
            };
            self.ui.recording_indicator = RecordingIndicatorMode::ClipSaving;
            self.push_toast(ToastKind::Info, "Stop Recording");
            let tx = self.async_tx.clone();
            self.rt.spawn(async move {
                let res = clip_media_segment(ClipRequest {
                    file_path,
                    start_time: start_secs,
                    duration,
                    output_dir: None,
                    ffmpeg_binary: Some(ffmpeg),
                    timeout_ms: None,
                })
                .await
                .map_err(|e| e.to_string());
                let _ = tx.send(BridgeAsyncMsg::ClipFinished(res));
            });
        } else {
            let Some(path) = self.current_file.as_deref() else {
                return;
            };
            if !SeekThumbnailer::is_valid_local_media(path) {
                self.push_toast(ToastKind::Error, "Clip recording only works on local files");
                return;
            }
            if self.resolve_ffmpeg_bin().is_none() {
                self.push_toast(ToastKind::Error, "ffmpeg binary not found");
                return;
            }
            self.clip_start_secs = Some(self.ui.current_secs);
            self.active_clip_file = Some(path.to_string());
            self.clip_started_at = Some(Instant::now());
            self.ui.recording_indicator = RecordingIndicatorMode::ClipActive { elapsed_ds: 0 };
            self.push_toast(ToastKind::Info, "Recording started");
        }
    }

    // ─── Appearance, yt-dlp Updates, Option A Auth ───────────────────────────

    pub fn update_gridmotion_scale_from_wheel(&mut self, new_scale: f32) {
        let rounded = (new_scale.clamp(0.4, 1.4) * 100.0).round() / 100.0;
        if (rounded - self.ui.gridmotion_cfg.scale).abs() > 1e-4 {
            self.ui.gridmotion_cfg.scale = rounded;
            if self.ui.welcome_quality != QualityTier::Custom {
                self.ui.welcome_quality = QualityTier::Custom;
                self.db.preferences.set("welcomeQuality", &json!("custom"));
            }
            self.gridmotion_zoom_save_deadline = Some(Instant::now() + Duration::from_millis(300));
        }
    }

    pub fn set_welcome_background(&mut self, kind: WelcomeBackgroundKind) {
        if kind == WelcomeBackgroundKind::Faulty && self.ui.welcome_background != kind {
            self.ui.faulty_cfg.page_load_progress = 0.0;
        }
        self.ui.welcome_background = kind;
        let key = match kind {
            WelcomeBackgroundKind::Dither => "dither",
            WelcomeBackgroundKind::Particles => "particles",
            WelcomeBackgroundKind::Faulty => "faulty",
            WelcomeBackgroundKind::Gridmotion => "gridmotion",
            WelcomeBackgroundKind::None => "none",
        };
        self.db.preferences.set("welcomeBackground", &json!(key));
        self.save_db();
    }

    pub fn set_welcome_quality(&mut self, tier: QualityTier) {
        self.ui.welcome_quality = tier;
        let key = match tier {
            QualityTier::Low => "low",
            QualityTier::Medium => "medium",
            QualityTier::High => "high",
            QualityTier::Custom => "custom",
        };
        self.db.preferences.set("welcomeQuality", &json!(key));
        self.apply_saved_bg_configs_to_ui();
        self.save_db();
    }

    pub fn check_ytdlp_update_async(&mut self) {
        let bin = self.resolve_ytdlp_bin();
        self.ui.ytdlp_status_text = "Checking for updates...".to_string();
        let cache = Arc::clone(&self.release_cache);
        let tx = self.async_tx.clone();
        self.rt.spawn(async move {
            let res = Ok(check_for_ytdlp_update(bin.as_deref(), Some(&cache), false).await);
            let _ = tx.send(BridgeAsyncMsg::YtdlpUpdateChecked(res));
        });
    }

    pub fn perform_ytdlp_update_async(&mut self) {
        let target_dir = self.user_data_dir.join("bin");
        let _ = std::fs::create_dir_all(&target_dir);
        for t in &mut self.ui.toasts {
            if t.kind == ToastKind::UpdateAvailable {
                t.downloading = true;
            }
        }
        self.ui.ytdlp_status_text = "Downloading yt-dlp update...".to_string();
        let tx = self.async_tx.clone();
        self.rt.spawn(async move {
            let res = download_ytdlp_update(&target_dir, None, None)
                .await
                .map(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("yt-dlp.exe")
                        .to_string()
                })
                .map_err(|e| e.to_string());
            let _ = tx.send(BridgeAsyncMsg::YtdlpUpdateInstalled(res));
        });
    }

    pub fn start_youtube_sign_in(&mut self, switch_account: bool) {
        if self.auth_controller.is_some() {
            return;
        }
        let browser_name = find_installed_chromium_browser()
            .map(|b| b.browser.display_name().to_string())
            .unwrap_or_else(|| "Google Chrome".to_string());

        let controller = Arc::new(CompanionAuthController::new());
        let handle = controller.handle();
        let handle_for_win = handle.clone();

        if let Ok(win) = CompanionAuthHandle::open(
            CompanionAuthOptions {
                browser_display_name: browser_name.clone(),
                switch_account,
                initial_status: format!("Waiting for sign-in in {browser_name}..."),
            },
            move |action| match action {
                CompanionAuthAction::FinishAndImport => handle_for_win.request_finish(),
                CompanionAuthAction::Cancel | CompanionAuthAction::WindowClosed => {
                    handle_for_win.request_cancel();
                }
            },
        ) {
            self.auth_window = Some(win);
        }

        self.auth_controller = Some(Arc::clone(&controller));
        let user_data = self.user_data_dir.clone();
        let tx = self.async_tx.clone();

        self.rt.spawn(async move {
            let mut sub = handle.subscribe();
            let tx_status = tx.clone();
            let watcher = tokio::spawn(async move {
                while sub.changed().await.is_ok() {
                    let st = sub.borrow().clone();
                    let (text, finishing) = match st {
                        CompanionAuthState::Idle => continue,
                        CompanionAuthState::Launched { browser, .. } => (
                            format!("Launched {} — opening YouTube...", browser.display_name()),
                            false,
                        ),
                        CompanionAuthState::WaitingForSignIn { browser } => (
                            format!(
                                "Waiting for YouTube sign-in in {}...",
                                browser.display_name()
                            ),
                            false,
                        ),
                        CompanionAuthState::Extracting { browser } => (
                            format!("Extracting cookies from {}...", browser.display_name()),
                            true,
                        ),
                        CompanionAuthState::Complete {
                            cookie_count,
                            logged_in,
                        } => (
                            if logged_in {
                                format!("Signed in ({cookie_count} cookies imported)")
                            } else {
                                format!("Finished ({cookie_count} cookies imported)")
                            },
                            false,
                        ),
                        CompanionAuthState::Cancelled => ("Sign-in cancelled".to_string(), false),
                        CompanionAuthState::Error(msg) => (msg, false),
                    };
                    let _ = tx_status.send(BridgeAsyncMsg::YoutubeAuthStatus { text, finishing });
                }
            });

            let res = run_option_a_sign_in(&user_data, &controller)
                .await
                .map(|st| st.logged_in)
                .map_err(|e| e.to_string());
            watcher.abort();
            let _ = tx.send(BridgeAsyncMsg::YoutubeAuthFinished(res));
        });
    }

    pub fn sign_out_youtube(&mut self) {
        let _ = sign_out_youtube_in_dir(&self.user_data_dir);
        self.refresh_youtube_cookie_state();
        if let Some(mpv) = &self.mpv {
            let _ = mpv.refresh_ytdl_options("", None);
        }
        self.push_toast(ToastKind::Info, "Signed out of YouTube");
    }

    /// Option B fallback (`youtube-auth.js:1326-1443`): extract YouTube/Google cookies from
    /// installed Chromium CDP or Firefox/yt-dlp supported browsers into `cookies.txt`.
    pub fn import_youtube_browser_cookies(&mut self) {
        self.push_toast(ToastKind::Info, "Importing YouTube cookies from browser...");
        let user_data = self.user_data_dir.clone();
        let ytdlp_bin = self.resolve_ytdlp_bin();
        let tx = self.async_tx.clone();
        self.rt.spawn(async move {
            let res = import_cookies_from_browser(&user_data, ytdlp_bin.as_deref()).await;
            let _ = tx.send(BridgeAsyncMsg::YoutubeBrowserImportFinished(res));
        });
    }

    /// Imports a user-selected Netscape `cookies.txt` file into `userData/cookies.txt` (`main.js:818-850`).
    pub fn import_youtube_cookies_file(&mut self) {
        let Some(src) = pick_native_paths(FileDialogKind::CookiesTxt, self.owner_hwnd)
            .into_iter()
            .next()
        else {
            return;
        };
        let Ok(content) = std::fs::read_to_string(&src) else {
            self.push_toast(ToastKind::Error, "Failed to read selected cookies file");
            return;
        };
        let header: String = content.chars().take(512).collect();
        let header_lower = header.to_ascii_lowercase();
        let looks_like_cookies = header_lower.contains("# netscape http cookie file")
            || header_lower.contains("# http cookie file")
            || has_youtube_cookie_lines(&content);
        if !looks_like_cookies {
            self.push_toast(ToastKind::Error, "Not a valid Netscape cookies.txt file");
            return;
        }
        let _ = std::fs::create_dir_all(&self.user_data_dir);
        let dest = get_cookie_file_path(&self.user_data_dir);
        if let Err(err) = std::fs::write(&dest, content) {
            self.push_toast(
                ToastKind::Error,
                format!("Failed to save cookies.txt: {err}"),
            );
            return;
        }
        self.refresh_youtube_cookie_state();
        if let Some(mpv) = &self.mpv {
            let _ = mpv.refresh_ytdl_options("", Some(dest.as_path()));
        }
        self.push_toast(ToastKind::Info, "Imported YouTube cookies.txt");
    }

    // ─── Seekbar Hover Thumbnails & Screenshot Image Decoding ────────────────

    fn seek_thumb_cache_key(&self, norm: f32) -> Option<(String, String, f64)> {
        let media_path = self.current_file.as_deref()?;
        if self.ui.duration_secs <= 0.0 || !SeekThumbnailer::is_valid_local_media(media_path) {
            return None;
        }
        let target_time = (norm.clamp(0.0, 1.0) as f64) * self.ui.duration_secs;
        let quantized = SeekThumbnailer::quantize_time(target_time);
        let (cache_key, _) = SeekThumbnailer::cache_key_and_sha1(media_path, quantized);
        Some((cache_key, media_path.to_string(), quantized))
    }

    /// Requests async `SeekThumbnailer` capture + RGBA decode for a normalized progress bar hover position.
    pub fn request_seek_thumbnail(&mut self, norm: f32) {
        let Some(thumbnailer) = self.thumbnailer.as_ref().map(Arc::clone) else {
            return;
        };
        let Some((cache_key, media_path, quantized)) = self.seek_thumb_cache_key(norm) else {
            return;
        };
        if self.decoded_seek_thumbs.contains_key(&cache_key)
            || self.pending_seek_thumbs.contains(&cache_key)
        {
            return;
        }
        self.pending_seek_thumbs.insert(cache_key.clone());
        let tx = self.async_tx.clone();
        self.rt.spawn(async move {
            let Some(payload) = thumbnailer.capture_thumbnail(&media_path, quantized).await else {
                let _ = tx.send(BridgeAsyncMsg::SeekThumbnailFailed { cache_key });
                return;
            };
            let decoded = tokio::task::spawn_blocking(move || {
                let img = image::load_from_memory(&payload.jpeg_bytes)
                    .ok()?
                    .to_rgba8();
                let (w, h) = img.dimensions();
                if w == 0 || h == 0 {
                    return None;
                }
                Some((w, h, img.into_raw()))
            })
            .await
            .ok()
            .flatten();

            match decoded {
                Some((width, height, rgba)) => {
                    let _ = tx.send(BridgeAsyncMsg::SeekThumbnailDecoded {
                        cache_key,
                        width,
                        height,
                        rgba,
                    });
                }
                None => {
                    let _ = tx.send(BridgeAsyncMsg::SeekThumbnailFailed { cache_key });
                }
            }
        });
    }

    /// Returns cached `(width, height, rgba)` for a normalized seekbar hover position if decoded.
    pub fn get_seek_thumbnail_rgba(&self, norm: f32) -> Option<&(u32, u32, Vec<u8>)> {
        let (cache_key, _, _) = self.seek_thumb_cache_key(norm)?;
        self.decoded_seek_thumbs.get(&cache_key)
    }

    /// Returns `true` if a seekbar hover thumbnail request is in flight for `norm`.
    pub fn is_seek_thumb_pending(&self, norm: f32) -> bool {
        self.seek_thumb_cache_key(norm)
            .is_some_and(|(cache_key, _, _)| self.pending_seek_thumbs.contains(&cache_key))
    }

    /// Returns `true` if `current_file` is a valid local media file eligible for hover thumbnails.
    pub fn has_local_seek_thumbnails(&self) -> bool {
        self.current_file.as_deref().is_some_and(|p| {
            self.ui.duration_secs > 0.0 && SeekThumbnailer::is_valid_local_media(p)
        })
    }

    /// Returns cached `(width, height, rgba)` for a screenshot preview/carousel slot, or spawns async decode.
    pub fn resolve_or_request_screenshot_rgba(
        &mut self,
        slot_key: &str,
        idx: usize,
        restore_mode: bool,
    ) -> Option<(u32, u32, Vec<u8>)> {
        let path = if restore_mode {
            self.deleted_screenshot_stack
                .get(idx)
                .map(|r| r.staged_path.clone())
        } else {
            self.session_screenshots
                .get(idx)
                .map(|s| s.file_path.clone())
        }?;
        self.resolve_or_request_image_path_rgba(slot_key, path)
    }

    pub fn resolve_or_request_gridmotion_thumb_rgba(
        &mut self,
        slot_key: &str,
        idx: usize,
    ) -> Option<(u32, u32, Vec<u8>)> {
        let raw = self.gridmotion_custom_pictures.get(idx)?.clone();
        self.resolve_or_request_image_path_rgba(slot_key, PathBuf::from(raw))
    }

    fn resolve_or_request_image_path_rgba(
        &mut self,
        slot_key: &str,
        path: PathBuf,
    ) -> Option<(u32, u32, Vec<u8>)> {
        let cache_key = format!("{}|{}", slot_key, path.to_string_lossy());
        if let Some(cached) = self.decoded_screenshots.get(&cache_key) {
            return Some(cached.clone());
        }
        if !self.pending_screenshots.contains(&cache_key) {
            self.pending_screenshots.insert(cache_key.clone());
            let tx = self.async_tx.clone();
            self.rt.spawn(async move {
                let decoded = tokio::task::spawn_blocking(move || {
                    let bytes = std::fs::read(&path).ok()?;
                    let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
                    let (w, h) = img.dimensions();
                    if w == 0 || h == 0 {
                        return None;
                    }
                    Some((w, h, img.into_raw()))
                })
                .await
                .ok()
                .flatten();

                match decoded {
                    Some((width, height, rgba)) => {
                        let _ = tx.send(BridgeAsyncMsg::ScreenshotImageDecoded {
                            key: cache_key,
                            width,
                            height,
                            rgba,
                        });
                    }
                    None => {
                        let _ = tx.send(BridgeAsyncMsg::ScreenshotImageFailed { key: cache_key });
                    }
                }
            });
        }
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileDialogKind {
    MediaSingle,
    MediaMulti,
    Folder,
    Subtitle,
    GridmotionImages,
    CookiesTxt,
}

impl FileDialogKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::MediaSingle => "Open Media File",
            Self::MediaMulti => "Open Multiple Media Files",
            Self::Folder => "Open Folder",
            Self::Subtitle => "Load Subtitle File",
            Self::GridmotionImages => "Select Background Images",
            Self::CookiesTxt => "Import cookies.txt",
        }
    }

    pub fn is_folder(self) -> bool {
        matches!(self, Self::Folder)
    }

    pub fn is_multi(self) -> bool {
        matches!(self, Self::MediaMulti | Self::GridmotionImages)
    }
}

/// Returns Win32 `COMDLG_FILTERSPEC` `(name, semicolon-separated pattern)` pairs matching
/// Electron `main.js:58-63, 111-118`, `ipc-handlers.js:1089-1091`, and `settings.js:229`.
pub fn build_file_dialog_filters(kind: FileDialogKind) -> Vec<(&'static str, &'static str)> {
    match kind {
        FileDialogKind::MediaSingle | FileDialogKind::MediaMulti => vec![
            (
                "Media Files",
                "*.mp4;*.mkv;*.avi;*.mov;*.webm;*.flv;*.m4v;*.wmv;*.mpg;*.mpeg;*.ts;*.m2ts;*.mts;*.vob;*.ogv;*.3gp;*.mp3;*.flac;*.wav;*.aac;*.ogg;*.m4a;*.wma;*.opus;*.aiff;*.alac;*.m3u;*.m3u8;*.pls;*.mpd",
            ),
            ("All Files", "*.*"),
        ],
        FileDialogKind::Folder => Vec::new(),
        FileDialogKind::Subtitle => vec![
            (
                "Subtitle Files",
                "*.srt;*.vtt;*.ass;*.ssa;*.sub;*.idx;*.sup",
            ),
            ("All Files", "*.*"),
        ],
        FileDialogKind::GridmotionImages => vec![
            (
                "Image Files",
                "*.png;*.jpg;*.jpeg;*.webp;*.gif;*.bmp;*.avif;*.svg;*.ico;*.tif;*.tiff",
            ),
            ("All Files", "*.*"),
        ],
        FileDialogKind::CookiesTxt => vec![("cookies.txt", "*.txt")],
    }
}

#[cfg(windows)]
fn pick_native_paths(kind: FileDialogKind, owner_hwnd: Option<isize>) -> Vec<PathBuf> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HWND, S_FALSE};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
        COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
    };
    use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
    use windows::Win32::UI::Shell::{
        FileOpenDialog, IFileOpenDialog, FOS_ALLOWMULTISELECT, FOS_FORCEFILESYSTEM,
        FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
    };

    unsafe {
        let init_hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        let needs_uninit = init_hr.is_ok() || init_hr == S_FALSE;

        let mut out = Vec::new();
        if let Ok(dialog) =
            CoCreateInstance::<_, IFileOpenDialog>(&FileOpenDialog, None, CLSCTX_ALL)
        {
            if let Ok(mut opts) = dialog.GetOptions() {
                opts |= FOS_FORCEFILESYSTEM;
                if kind.is_folder() {
                    opts |= FOS_PICKFOLDERS;
                }
                if kind.is_multi() && !kind.is_folder() {
                    opts |= FOS_ALLOWMULTISELECT;
                }
                let _ = dialog.SetOptions(opts);
            }
            let title_wide: Vec<u16> = kind.title().encode_utf16().chain(Some(0)).collect();
            let _ = dialog.SetTitle(PCWSTR(title_wide.as_ptr()));

            // Keep wide backing strings alive across SetFileTypes and Show!
            let filter_pairs = build_file_dialog_filters(kind);
            let wide_filters: Vec<(Vec<u16>, Vec<u16>)> = filter_pairs
                .iter()
                .map(|(name, spec)| {
                    (
                        name.encode_utf16().chain(Some(0)).collect(),
                        spec.encode_utf16().chain(Some(0)).collect(),
                    )
                })
                .collect();
            let specs: Vec<COMDLG_FILTERSPEC> = wide_filters
                .iter()
                .map(|(name_w, spec_w)| COMDLG_FILTERSPEC {
                    pszName: PCWSTR(name_w.as_ptr()),
                    pszSpec: PCWSTR(spec_w.as_ptr()),
                })
                .collect();
            if !specs.is_empty() {
                let _ = dialog.SetFileTypes(&specs);
                let _ = dialog.SetFileTypeIndex(1);
            }

            let owner = owner_hwnd
                .filter(|&h| h != 0)
                .map(|h| HWND(h as *mut std::ffi::c_void));
            if dialog.Show(owner).is_ok() {
                if let Ok(results) = dialog.GetResults() {
                    if let Ok(count) = results.GetCount() {
                        for idx in 0..count {
                            if let Ok(item) = results.GetItemAt(idx) {
                                if let Ok(pwstr) = item.GetDisplayName(SIGDN_FILESYSPATH) {
                                    if !pwstr.is_null() {
                                        if let Ok(s) = pwstr.to_string() {
                                            if !s.is_empty() {
                                                out.push(PathBuf::from(s));
                                            }
                                        }
                                        CoTaskMemFree(Some(pwstr.0 as *const std::ffi::c_void));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        if needs_uninit {
            CoUninitialize();
        }
        out
    }
}

#[cfg(not(windows))]
fn pick_native_paths(_kind: FileDialogKind, _owner_hwnd: Option<isize>) -> Vec<PathBuf> {
    Vec::new()
}

fn rgb_to_hsv(rgb: [f32; 3]) -> (f32, f32, f32) {
    let r = rgb[0].clamp(0.0, 1.0);
    let g = rgb[1].clamp(0.0, 1.0);
    let b = rgb[2].clamp(0.0, 1.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d < 1e-6 {
        0.0
    } else if (max - r).abs() < 1e-6 {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if (max - g).abs() < 1e-6 {
        60.0 * (((b - r) / d) + 2.0)
    } else {
        60.0 * (((r - g) / d) + 4.0)
    };
    let s = if max < 1e-6 { 0.0 } else { d / max };
    (h, s, max)
}

fn hsv_to_rgb(h_deg: f32, s: f32, v: f32) -> [f32; 3] {
    let h = h_deg.rem_euclid(360.0) / 60.0;
    let s = s.clamp(0.0, 1.0);
    let v = v.clamp(0.0, 1.0);
    let c = v * s;
    let x = c * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
    let m = v - c;
    let (r1, g1, b1) = if h < 1.0 {
        (c, x, 0.0)
    } else if h < 2.0 {
        (x, c, 0.0)
    } else if h < 3.0 {
        (0.0, c, x)
    } else if h < 4.0 {
        (0.0, x, c)
    } else if h < 5.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    [
        (r1 + m).clamp(0.0, 1.0),
        (g1 + m).clamp(0.0, 1.0),
        (b1 + m).clamp(0.0, 1.0),
    ]
}

pub fn normalize_network_media_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > 4096 || trimmed.chars().any(char::is_whitespace) {
        return None;
    }
    let (scheme, rest) = trimmed.split_once("://")?;
    let scheme_lower = scheme.to_ascii_lowercase();
    if !matches!(
        scheme_lower.as_str(),
        "http" | "https" | "rtsp" | "rtmp" | "rtmps" | "srt" | "ytdl"
    ) {
        return None;
    }
    let host_part = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host_part.rsplit('@').next().unwrap_or("");
    if host.is_empty() || host.starts_with(':') {
        return None;
    }
    Some(format!("{scheme_lower}://{rest}"))
}

fn build_ytdl_format(
    selected: &str,
    stream_url: Option<&str>,
    video_dimensions: Option<(u32, u32)>,
) -> String {
    if selected.eq_ignore_ascii_case("audio") {
        return "bestaudio/best".to_string();
    }
    if let Some(h) = selected
        .strip_suffix('p')
        .and_then(|digits| digits.parse::<u32>().ok())
    {
        let is_shorts = stream_url.is_some_and(|u| u.contains("/shorts/"))
            || video_dimensions.is_some_and(|(w, vh)| w > 0 && vh > w);
        let dim = if is_shorts { "width" } else { "height" };
        format!("bestvideo[{dim}<=?{h}]+bestaudio/best")
    } else {
        "bestvideo+bestaudio/best".to_string()
    }
}

fn resolve_pause_ui_state(
    has_active_media: bool,
    paused: bool,
) -> (bool, bool, SmtcPlaybackStatus) {
    if !has_active_media {
        (false, false, SmtcPlaybackStatus::Stopped)
    } else if paused {
        (false, false, SmtcPlaybackStatus::Paused)
    } else {
        (true, false, SmtcPlaybackStatus::Playing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_pause_ui_state_stays_stopped_while_idle_on_welcome() {
        assert_eq!(
            resolve_pause_ui_state(false, false),
            (false, false, SmtcPlaybackStatus::Stopped)
        );
        assert_eq!(
            resolve_pause_ui_state(false, true),
            (false, false, SmtcPlaybackStatus::Stopped)
        );
        assert_eq!(
            resolve_pause_ui_state(true, false),
            (true, false, SmtcPlaybackStatus::Playing)
        );
        assert_eq!(
            resolve_pause_ui_state(true, true),
            (false, false, SmtcPlaybackStatus::Paused)
        );
    }

    #[test]
    fn normalize_network_media_url_validates_allowed_schemes_and_length() {
        assert_eq!(
            normalize_network_media_url("  HTTPS://www.youtube.com/watch?v=dQw4w9WgXcQ  ")
                .as_deref(),
            Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ")
        );
        assert!(normalize_network_media_url("rtsp://192.168.1.10:554/stream").is_some());
        assert!(normalize_network_media_url("rtmp://live.example.com/app/key").is_some());
        assert!(normalize_network_media_url("rtmps://live.example.com:443/app").is_some());
        assert!(normalize_network_media_url("srt://example.com:9000?mode=caller").is_some());

        assert!(normalize_network_media_url("").is_none());
        assert!(normalize_network_media_url("   ").is_none());
        assert!(normalize_network_media_url("ftp://example.com/file.mp4").is_none());
        assert!(normalize_network_media_url("https://").is_none());
        assert!(normalize_network_media_url("https:///missing-host").is_none());
        assert!(normalize_network_media_url("https://example.com/has space").is_none());
        let too_long = format!("https://example.com/{}", "a".repeat(4096));
        assert!(normalize_network_media_url(&too_long).is_none());
    }

    #[test]
    fn build_ytdl_format_handles_auto_audio_landscape_and_shorts() {
        assert_eq!(
            build_ytdl_format(
                "auto",
                Some("https://www.youtube.com/watch?v=abc"),
                Some((1920, 1080))
            ),
            "bestvideo+bestaudio/best"
        );
        assert_eq!(
            build_ytdl_format(
                "audio",
                Some("https://www.youtube.com/watch?v=abc"),
                Some((1920, 1080))
            ),
            "bestaudio/best"
        );
        assert_eq!(
            build_ytdl_format(
                "720p",
                Some("https://www.youtube.com/watch?v=abc"),
                Some((1920, 1080))
            ),
            "bestvideo[height<=?720]+bestaudio/best"
        );
        assert_eq!(
            build_ytdl_format("1080p", Some("https://www.youtube.com/shorts/xyz"), None),
            "bestvideo[width<=?1080]+bestaudio/best"
        );
        assert_eq!(
            build_ytdl_format(
                "720p",
                Some("https://www.youtube.com/watch?v=abc"),
                Some((1080, 1920))
            ),
            "bestvideo[width<=?720]+bestaudio/best"
        );
    }

    #[test]
    fn burst_frame_captured_does_not_push_stale_pre_finalize_paths() {
        let rt = Arc::new(Runtime::new().unwrap());
        let dir = std::env::temp_dir().join(format!(
            "hybrid-burst-unit-{}-{}",
            std::process::id(),
            now_millis()
        ));
        let mut bridge = MediaBridge::new(dir.clone(), rt);
        bridge.burst_session_id = Some("sess-1".into());
        bridge.burst_in_flight = true;

        bridge.handle_async_msg(BridgeAsyncMsg::BurstFrameCaptured {
            session_id: "sess-1".into(),
            result: Ok(dir
                .join("screenshots")
                .join("burst_sess-1")
                .join("frame_0001.jpg")),
        });

        assert!(!bridge.burst_in_flight);
        assert_eq!(bridge.burst_seq, 1);
        assert!(matches!(
            bridge.ui.recording_indicator,
            RecordingIndicatorMode::BurstActive { count: 1 }
        ));
        assert!(bridge.session_screenshots.is_empty());
        assert!(bridge.single_screenshot_stack.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn seek_thumbnail_decoded_triggers_hover_thumb_enter_animation_only_when_hovering() {
        let rt = Arc::new(Runtime::new().unwrap());
        let dir = std::env::temp_dir().join(format!(
            "hybrid-hover-thumb-unit-{}-{}",
            std::process::id(),
            now_millis()
        ));
        let _ = std::fs::create_dir_all(&dir);
        let video_path = dir.join("video.mp4");
        std::fs::write(&video_path, b"mp4").unwrap();
        let mut bridge = MediaBridge::new(dir.clone(), rt);
        bridge.current_file = Some(video_path.to_string_lossy().into_owned());
        bridge.ui.duration_secs = 120.0;
        bridge.ui.welcome_active = false;

        // Fresh MediaBridge must start with hover_thumb_has_frame == false (no blank card on initial hover).
        assert!(!bridge.ui.hover_thumb_has_frame);

        // If cursor already left seekbar before FFmpeg returned, don't reveal stale popup.
        bridge.ui.hover_thumb_norm = None;
        bridge.ui.anim.hover_thumb_t = 1.0;
        bridge.handle_async_msg(BridgeAsyncMsg::SeekThumbnailDecoded {
            cache_key: "k-stale".into(),
            width: 2,
            height: 2,
            rgba: vec![255; 16],
        });
        assert!(!bridge.ui.hover_thumb_has_frame);
        assert_eq!(bridge.ui.anim.hover_thumb_t, 1.0);

        // While hovering with no frame yet, SeekThumbnailDecoded reveals card AND restarts enter anim (`0.0`).
        bridge.ui.hover_thumb_norm = Some(0.5);
        bridge.ui.anim.hover_thumb_t = 1.0;
        bridge.handle_async_msg(BridgeAsyncMsg::SeekThumbnailDecoded {
            cache_key: "k-live".into(),
            width: 2,
            height: 2,
            rgba: vec![255; 16],
        });
        assert!(bridge.ui.hover_thumb_has_frame);
        assert_eq!(bridge.ui.anim.hover_thumb_t, 0.0);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
