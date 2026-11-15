//! Full interactive UI state model for Hybrid Player.
//! Covers Titlebar, Welcome Screen, Control Bar, Playlist virtualization (>80 items),
//! all OSDs & Clip Bezel Tab, Screenshot Preview/Carousel, and all 7 Modals + #bgAccordionPopover.

use std::collections::BTreeMap;

use hybrid_core::db::{HistoryEntry, PlaylistItemEntry};
use hybrid_core::equalizer::{EqualizerPreset, EqualizerState, EQ_BAND_COUNT};
use hybrid_core::shortcuts::{
    format_combo_for_display, SHORTCUT_CATALOG, SHORTCUT_CATEGORIES, VALID_SCREENSHOT_MOUSE_BUTTONS,
};
use hybrid_shaders::{
    ClearContext, DitherWavesConfig, FaultyTerminalConfig, GridMotionConfig, ParticlesConfig,
    QualityTier, ThemeMode, WelcomeBackgroundKind,
};

use crate::motion::{ElasticVolumeSpring, MotionProfile, UiAnimationState, WindowMotionState};
use crate::primitives::{HitTarget, VirtualSlice};
use crate::tokens::{FontConfig, ThemeTokens};

pub const PLAYLIST_ITEM_HEIGHT: f32 = 40.0;
pub const PLAYLIST_VIRTUALIZE_THRESHOLD: usize = 80;
pub const PLAYLIST_OVERSCAN_ROWS: usize = 5;
pub const MAX_VISIBLE_TOASTS: usize = 3;
pub const HOVER_TOOLTIP_DELAY_SECS: f32 = 0.45;
/// All 10 playback speed preset buttons in `#speedModal` (`index.html:708-724`).
pub const SPEED_PRESETS: [f64; 10] = [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 3.0, 4.0];

/// Active text input focused by pointer click, keyboard Tab, or AccessKit `Focus` action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusedTextField {
    PlaylistSearch,
    NetworkStreamUrl,
    ShortcutSearch,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TextFieldCursor {
    pub cursor: usize,
    pub anchor: Option<usize>,
    pub scroll_x: f32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextFieldCursors {
    pub playlist_search: TextFieldCursor,
    pub network_stream: TextFieldCursor,
    pub shortcut_search: TextFieldCursor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RepeatMode {
    None,
    All,
    One,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AbLoopState {
    Off,
    PointASet(u64),
    Active(u64, u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowMaximizeState {
    Restored,
    Maximized,
    Fullscreen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModalId {
    Equalizer,
    Speed,
    Subtitles,
    Audio,
    Settings,
    NetworkStream,
    BgSettings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecordingIndicatorMode {
    Hidden,
    BurstActive { count: usize },
    ClipActive { elapsed_ds: u64 },
    ClipSaving,
    Saved { label_frames: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CarouselMode {
    Hidden,
    SinglePreview,
    DeleteCarousel,
    RestoreCarousel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DropdownKind {
    SettingsTheme,
    SettingsMotion,
    SettingsSeekStep,
    SettingsScreenshotFormat,
    SettingsMouseScreenshot,
    EqPreset,
    YoutubeQuality,
    BgGridmotionAspectRatio,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ColorPickerTarget {
    Accent,
    SubtitleFontColor,
    SubtitleBgColor,
    BgControl(String),
}

pub const COLOR_PICKER_SWATCHES: [&str; 8] = [
    "#3b82f6", "#a7ef9e", "#ffffff", "#ffff00", "#00ffff", "#ef4444", "#f59e0b", "#000000",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToastKind {
    Info,
    Error,
    UpdateAvailable,
    UpdateSuccess,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToastItem {
    pub id: u64,
    pub kind: ToastKind,
    pub message: String,
    pub version_tag: Option<String>,
    pub downloading: bool,
    /// Remaining display lifetime in seconds (`toast.js`: Info `2.0s`, Error `12.0s`,
    /// UpdateAvailable `14.0s`, UpdateSuccess `3.0s`).
    pub remaining_secs: f32,
    /// Per-toast entrance fade/slide progress (`0.0..=1.0`).
    pub enter_t: f32,
    /// `true` once TTL expires and `.hiding` (`250ms`) exit animation is playing.
    pub closing: bool,
    /// Per-toast exit fade/slide progress (`0.0..=1.0`).
    pub exit_t: f32,
}

impl Default for ToastItem {
    fn default() -> Self {
        Self {
            id: 0,
            kind: ToastKind::Info,
            message: String::new(),
            version_tag: None,
            downloading: false,
            remaining_secs: 2.0,
            enter_t: 1.0,
            closing: false,
            exit_t: 0.0,
        }
    }
}

impl ToastItem {
    pub fn default_ttl_secs(kind: ToastKind) -> f32 {
        match kind {
            ToastKind::Info => 2.0,
            ToastKind::Error => 12.0,
            ToastKind::UpdateAvailable => 14.0,
            ToastKind::UpdateSuccess => 3.0,
        }
    }

    pub fn new(id: u64, kind: ToastKind, message: impl Into<String>) -> Self {
        Self {
            id,
            kind,
            message: message.into(),
            version_tag: None,
            downloading: false,
            remaining_secs: Self::default_ttl_secs(kind),
            enter_t: 0.0,
            closing: false,
            exit_t: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkipIndicatorState {
    pub visible: bool,
    pub forward: bool,
    pub accumulated_secs: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SeekIndicatorState {
    pub visible: bool,
    pub target_secs: f64,
    pub duration_secs: f64,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StatsOverlayState {
    pub visible: bool,
    pub codec: String,
    pub resolution: String,
    pub fps: String,
    pub bitrate: String,
    pub hwdec: String,
    pub dropped_frames: u64,
    pub cache_secs: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VolumeOsdState {
    pub visible: bool,
    pub percent: u32,
}

/// Complete playlist drawer state (`playlist.css:11-286`, `playlist.js:16-590`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlaylistUiState {
    pub open: bool,
    pub items: Vec<PlaylistItemEntry>,
    pub current_index: Option<usize>,
    pub search_query: String,
    pub scroll_top_px: f32,
}

impl PlaylistUiState {
    /// Returns filtered `(original_index, &PlaylistItemEntry)` list matching `search_query`.
    pub fn filtered_items(&self) -> Vec<(usize, &PlaylistItemEntry)> {
        let q = self.search_query.trim().to_ascii_lowercase();
        self.items
            .iter()
            .enumerate()
            .filter(|(_, item)| q.is_empty() || item.name.to_ascii_lowercase().contains(&q))
            .collect()
    }

    /// Computes exact virtualization window matching `hybrid-player/src/renderer/modules/playlist.js:476-537`:
    /// - `<= 80` filtered items -> renders full list unvirtualized
    /// - `> 80` filtered items -> `ITEM_HEIGHT = 40.0` stride with `±5` overscan rows.
    pub fn virtual_window(&self, list_client_height: f32) -> VirtualSlice {
        let filtered_len = self.filtered_items().len();
        let total_height_px = (filtered_len as f32) * PLAYLIST_ITEM_HEIGHT;
        if filtered_len <= PLAYLIST_VIRTUALIZE_THRESHOLD {
            return VirtualSlice {
                is_virtualized: false,
                total_items: filtered_len,
                total_height_px,
                start_index: 0,
                end_index: filtered_len,
                offset_top_px: 0.0,
            };
        }

        let scroll_top = self.scroll_top_px.max(0.0);
        let client_h = if list_client_height > 0.0 {
            list_client_height
        } else {
            400.0
        };
        let raw_start = (scroll_top / PLAYLIST_ITEM_HEIGHT).floor() as isize
            - (PLAYLIST_OVERSCAN_ROWS as isize);
        let start_index = raw_start.max(0) as usize;
        let raw_end = ((scroll_top + client_h) / PLAYLIST_ITEM_HEIGHT).ceil() as usize
            + PLAYLIST_OVERSCAN_ROWS;
        let end_index = raw_end.min(filtered_len);

        VirtualSlice {
            is_virtualized: true,
            total_items: filtered_len,
            total_height_px,
            start_index,
            end_index,
            offset_top_px: (start_index as f32) * PLAYLIST_ITEM_HEIGHT,
        }
    }
}

/// Kind of welcome background control (`settings.js:52-83`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BgControlInputType {
    Range {
        min: &'static str,
        max: &'static str,
        step: &'static str,
    },
    Color,
    Checkbox,
    Select,
}

/// Metadata descriptor for each of the 27 welcome effect controls (`BG_CONTROLS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BgControlDescriptor {
    pub bg: WelcomeBackgroundKind,
    pub id: &'static str,
    pub label: &'static str,
    pub input_type: BgControlInputType,
    pub default_display: &'static str,
}

/// Complete 27-entry `BG_CONTROLS` catalog matching `settings.js:52-84` & `index.html:1206-1348`.
pub const BG_CONTROLS_CATALOG: [BgControlDescriptor; 27] = [
    // Dither Waves (7, incl. dynamically injected bgDitherBgColor at settings.js:340)
    BgControlDescriptor {
        bg: WelcomeBackgroundKind::Dither,
        id: "bgDitherSpeed",
        label: "Speed",
        input_type: BgControlInputType::Range {
            min: "0.01",
            max: "0.2",
            step: "0.01",
        },
        default_display: "0.05",
    },
    BgControlDescriptor {
        bg: WelcomeBackgroundKind::Dither,
        id: "bgDitherFrequency",
        label: "Frequency",
        input_type: BgControlInputType::Range {
            min: "1",
            max: "8",
            step: "0.5",
        },
        default_display: "3",
    },
    BgControlDescriptor {
        bg: WelcomeBackgroundKind::Dither,
        id: "bgDitherAmplitude",
        label: "Amplitude",
        input_type: BgControlInputType::Range {
            min: "0.1",
            max: "0.8",
            step: "0.05",
        },
        default_display: "0.3",
    },
    BgControlDescriptor {
        bg: WelcomeBackgroundKind::Dither,
        id: "bgDitherColor",
        label: "Color",
        input_type: BgControlInputType::Color,
        default_display: "#808080",
    },
    BgControlDescriptor {
        bg: WelcomeBackgroundKind::Dither,
        id: "bgDitherBgColor",
        label: "Background Color",
        input_type: BgControlInputType::Color,
        default_display: "#000000",
    },
    BgControlDescriptor {
        bg: WelcomeBackgroundKind::Dither,
        id: "bgDitherPixelSize",
        label: "Pixel Size",
        input_type: BgControlInputType::Range {
            min: "1",
            max: "8",
            step: "1",
        },
        default_display: "1",
    },
    BgControlDescriptor {
        bg: WelcomeBackgroundKind::Dither,
        id: "bgDitherColorNum",
        label: "Color Depth",
        input_type: BgControlInputType::Range {
            min: "2",
            max: "8",
            step: "1",
        },
        default_display: "4",
    },
    // Particles (6)
    BgControlDescriptor {
        bg: WelcomeBackgroundKind::Particles,
        id: "bgParticlesCount",
        label: "Count",
        input_type: BgControlInputType::Range {
            min: "50",
            max: "800",
            step: "50",
        },
        default_display: "300",
    },
    BgControlDescriptor {
        bg: WelcomeBackgroundKind::Particles,
        id: "bgParticlesSpeed",
        label: "Speed",
        input_type: BgControlInputType::Range {
            min: "0.02",
            max: "0.5",
            step: "0.02",
        },
        default_display: "0.1",
    },
    BgControlDescriptor {
        bg: WelcomeBackgroundKind::Particles,
        id: "bgParticlesSpread",
        label: "Spread",
        input_type: BgControlInputType::Range {
            min: "2",
            max: "20",
            step: "1",
        },
        default_display: "10",
    },
    BgControlDescriptor {
        bg: WelcomeBackgroundKind::Particles,
        id: "bgParticlesColor",
        label: "Color",
        input_type: BgControlInputType::Color,
        default_display: "#ffffff",
    },
    BgControlDescriptor {
        bg: WelcomeBackgroundKind::Particles,
        id: "bgParticlesSize",
        label: "Size",
        input_type: BgControlInputType::Range {
            min: "20",
            max: "300",
            step: "10",
        },
        default_display: "100",
    },
    BgControlDescriptor {
        bg: WelcomeBackgroundKind::Particles,
        id: "bgParticlesAlpha",
        label: "Alpha",
        input_type: BgControlInputType::Checkbox,
        default_display: "false",
    },
    // Faulty Terminal (8)
    BgControlDescriptor {
        bg: WelcomeBackgroundKind::Faulty,
        id: "bgFaultyScale",
        label: "Scale",
        input_type: BgControlInputType::Range {
            min: "0.5",
            max: "8",
            step: "0.1",
        },
        default_display: "1.2",
    },
    BgControlDescriptor {
        bg: WelcomeBackgroundKind::Faulty,
        id: "bgFaultyGlitch",
        label: "Glitch",
        input_type: BgControlInputType::Range {
            min: "0",
            max: "3",
            step: "0.1",
        },
        default_display: "1",
    },
    BgControlDescriptor {
        bg: WelcomeBackgroundKind::Faulty,
        id: "bgFaultyScanlines",
        label: "Scanlines",
        input_type: BgControlInputType::Range {
            min: "0",
            max: "1.5",
            step: "0.05",
        },
        default_display: "0.7",
    },
    BgControlDescriptor {
        bg: WelcomeBackgroundKind::Faulty,
        id: "bgFaultyFlicker",
        label: "Flicker",
        input_type: BgControlInputType::Range {
            min: "0",
            max: "2",
            step: "0.1",
        },
        default_display: "1",
    },
    BgControlDescriptor {
        bg: WelcomeBackgroundKind::Faulty,
        id: "bgFaultyAberration",
        label: "Aberration",
        input_type: BgControlInputType::Range {
            min: "0",
            max: "5",
            step: "0.5",
        },
        default_display: "0",
    },
    BgControlDescriptor {
        bg: WelcomeBackgroundKind::Faulty,
        id: "bgFaultyCurvature",
        label: "Curvature",
        input_type: BgControlInputType::Range {
            min: "0",
            max: "0.3",
            step: "0.02",
        },
        default_display: "0.2",
    },
    BgControlDescriptor {
        bg: WelcomeBackgroundKind::Faulty,
        id: "bgFaultyTint",
        label: "Tint",
        input_type: BgControlInputType::Color,
        default_display: "#A7EF9E",
    },
    BgControlDescriptor {
        bg: WelcomeBackgroundKind::Faulty,
        id: "bgFaultyBrightness",
        label: "Brightness",
        input_type: BgControlInputType::Range {
            min: "0.2",
            max: "1.5",
            step: "0.05",
        },
        default_display: "0.8",
    },
    // Grid Motion (6, injected by settings.js:173)
    BgControlDescriptor {
        bg: WelcomeBackgroundKind::Gridmotion,
        id: "bgGridmotionAutoMove",
        label: "Auto Move",
        input_type: BgControlInputType::Checkbox,
        default_display: "false",
    },
    BgControlDescriptor {
        bg: WelcomeBackgroundKind::Gridmotion,
        id: "bgGridmotionScale",
        label: "Zoom",
        input_type: BgControlInputType::Range {
            min: "0.4",
            max: "1.4",
            step: "0.05",
        },
        default_display: "1.0",
    },
    BgControlDescriptor {
        bg: WelcomeBackgroundKind::Gridmotion,
        id: "bgGridmotionAspectRatio",
        label: "Card Ratio",
        input_type: BgControlInputType::Select,
        default_display: "16:9",
    },
    BgControlDescriptor {
        bg: WelcomeBackgroundKind::Gridmotion,
        id: "bgGridmotionSpeed",
        label: "Speed",
        input_type: BgControlInputType::Range {
            min: "0.1",
            max: "3.0",
            step: "0.1",
        },
        default_display: "1.0",
    },
    BgControlDescriptor {
        bg: WelcomeBackgroundKind::Gridmotion,
        id: "bgGridmotionMaxMove",
        label: "Max Distance",
        input_type: BgControlInputType::Range {
            min: "50",
            max: "600",
            step: "10",
        },
        default_display: "300",
    },
    BgControlDescriptor {
        bg: WelcomeBackgroundKind::Gridmotion,
        id: "bgGridmotionGradientColor",
        label: "Background Color",
        input_type: BgControlInputType::Color,
        default_display: "#000000",
    },
];

#[derive(Debug, Clone, PartialEq)]
pub struct TrackOption {
    pub id: i64,
    pub label: String,
    pub lang: String,
    pub selected: bool,
    pub external: bool,
}

/// Interactive UI state for all Hybrid Player views, modals, overlays, and settings.
#[derive(Debug, Clone, PartialEq)]
pub struct UiState {
    pub viewport_w: f32,
    pub viewport_h: f32,
    pub dpr: f32,
    pub window_title: String,
    pub window_maximize_state: WindowMaximizeState,
    pub theme: ThemeMode,
    pub accent_hex: String,
    pub font_config: FontConfig,
    pub motion_profile: MotionProfile,
    pub window_motion: WindowMotionState,
    pub welcome_active: bool,
    pub welcome_background: WelcomeBackgroundKind,
    pub welcome_quality: QualityTier,
    pub dither_cfg: DitherWavesConfig,
    pub particles_cfg: ParticlesConfig,
    pub faulty_cfg: FaultyTerminalConfig,
    pub gridmotion_cfg: GridMotionConfig,
    pub bg_accordion_open: bool,
    pub active_modal: Option<ModalId>,
    pub closing_modal: Option<ModalId>,
    pub active_dropdown: Option<DropdownKind>,
    pub active_color_picker: Option<ColorPickerTarget>,
    pub color_picker_hue: f32,

    // Playback & Control Bar
    pub playing: bool,
    pub is_youtube: bool,
    pub is_live_stream: bool,
    pub youtube_quality_label: String,
    pub youtube_quality_options: Vec<String>,
    pub current_secs: f64,
    pub duration_secs: f64,
    pub buffered_secs: f64,
    pub speed: f64,
    pub video_aspect_ratio: f32,
    pub volume_pct: u32,
    pub muted: bool,
    pub volume_boost_enabled: bool,
    pub volume_spring: ElasticVolumeSpring,
    pub repeat: RepeatMode,
    pub shuffle: bool,
    pub ab_loop: AbLoopState,
    pub locked: bool,
    pub show_unlock_button: bool,
    pub auto_hide_controls: bool,
    pub time_remaining_mode: bool,
    pub chapter_positions_norm: Vec<f32>,
    pub chapter_times_secs: Vec<f64>,
    pub chapter_titles: Vec<String>,
    pub current_chapter: Option<usize>,
    pub hover_thumb_norm: Option<f32>,
    pub hover_thumb_has_frame: bool,
    pub hover_thumb_loading: bool,
    pub hover_thumb_shimmer_phase: f32,
    pub is_dragging_progress: bool,

    // Recent files on welcome screen (max 5 rendered)
    pub recent_files: Vec<HistoryEntry>,

    // Playlist Drawer
    pub playlist: PlaylistUiState,

    // OSDs & Overlays
    pub volume_osd: VolumeOsdState,
    pub skip_indicator: SkipIndicatorState,
    pub seek_indicator: SeekIndicatorState,
    pub center_play_visible: bool,
    pub loading_visible: bool,
    pub stats_overlay: StatsOverlayState,
    pub recording_indicator: RecordingIndicatorMode,
    pub recording_saved_frame_count: usize,
    pub carousel_mode: CarouselMode,
    pub carousel_index: usize,
    pub carousel_total: usize,
    pub screenshot_notice: Option<String>,
    /// Optional single-shot status badge `(label, is_deleted)` shown on `SinglePreview`.
    pub screenshot_badge: Option<(String, bool)>,
    pub drop_overlay_active: bool,
    pub toasts: Vec<ToastItem>,

    // Equalizer, Subtitles, Audio, Stream & Settings UI state
    pub equalizer: EqualizerState,
    pub subtitle_tracks: Vec<TrackOption>,
    pub subtitle_delay_secs: f64,
    pub subtitle_font_size: u32,
    pub subtitle_bg_opacity_pct: u32,
    pub subtitle_font_color: String,
    pub subtitle_bg_color: String,
    pub audio_tracks: Vec<TrackOption>,
    pub audio_delay_secs: f64,
    pub network_stream_url: String,

    // Settings & Shortcuts
    pub auto_resume: bool,
    pub double_click_fullscreen: bool,
    pub seek_step_secs: u32,
    pub screenshot_dir: String,
    pub default_screenshot_dir: String,
    pub screenshot_format: String,
    pub auto_organize_screenshots: bool,
    pub screenshot_mouse_button: i32,
    pub shortcut_search_query: String,
    pub recording_shortcut_id: Option<String>,
    pub recording_shortcut_preview: Option<String>,
    pub custom_shortcuts: BTreeMap<String, String>,
    pub youtube_signed_in: bool,
    pub ytdlp_status_text: String,
    pub ytdlp_version: String,
    pub ytdlp_update_available: bool,
    pub ytdlp_latest_version: Option<String>,
    pub gridmotion_custom_pictures: Vec<String>,
    /// Scroll offset (px) in Settings modal; 0.0 shows Appearance/Behavior/Screenshots at top.
    pub settings_scroll_y: f32,
    /// Scroll offset (px) in Background Settings modal (`ModalId::BgSettings`).
    pub bg_settings_scroll_y: f32,
    /// Independent scroll offset (px) inside Settings keyboard shortcut categories list (`max-height: 290px`).
    pub shortcut_scroll_y: f32,
    /// Currently hovered hit target (updated on `CursorMoved` for `:hover` & tooltips).
    pub hovered_target: Option<HitTarget>,
    /// Elapsed hover dwell time on `hovered_target` for delayed tooltip pill visibility.
    pub hover_tooltip_elapsed_secs: f32,
    /// Currently focused text input (`PlaylistSearch`, `NetworkStreamUrl`, `ShortcutSearch`).
    pub focused_text_field: Option<FocusedTextField>,
    /// Per-field caret/selection/scroll offsets.
    pub text_cursors: TextFieldCursors,
    /// Keyboard/AccessKit focused hit target (for modal Tab trap and `:focus-visible` outlines).
    pub focused_target: Option<HitTarget>,
    /// Whether `:focus-visible` outline should render around `focused_target` (`true` on keyboard/Tab).
    pub focus_visible: bool,
    /// Focus target saved on modal open and restored on modal close.
    pub saved_focus_before_modal: Option<HitTarget>,
    /// Frame-timed UI animation & OSD countdown state.
    pub anim: UiAnimationState,
}

fn default_dither_medium() -> DitherWavesConfig {
    DitherWavesConfig::for_quality(QualityTier::Medium)
}

fn default_particles_medium() -> ParticlesConfig {
    ParticlesConfig::for_quality(QualityTier::Medium)
}

fn default_faulty_medium() -> FaultyTerminalConfig {
    FaultyTerminalConfig::for_quality(QualityTier::Medium)
}

fn default_gridmotion_medium() -> GridMotionConfig {
    GridMotionConfig::for_quality(QualityTier::Medium)
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            viewport_w: 1280.0,
            viewport_h: 720.0,
            dpr: 1.0,
            window_title: "Hybrid Player".to_string(),
            window_maximize_state: WindowMaximizeState::Restored,
            theme: ThemeMode::Dark,
            accent_hex: "#e50914".to_string(),
            font_config: FontConfig::default(),
            motion_profile: MotionProfile::Balanced,
            window_motion: WindowMotionState::default(),
            welcome_active: true,
            welcome_background: WelcomeBackgroundKind::Dither,
            welcome_quality: QualityTier::Medium,
            dither_cfg: default_dither_medium(),
            particles_cfg: default_particles_medium(),
            faulty_cfg: default_faulty_medium(),
            gridmotion_cfg: default_gridmotion_medium(),
            bg_accordion_open: false,
            active_modal: None,
            closing_modal: None,
            active_dropdown: None,
            active_color_picker: None,
            color_picker_hue: 217.0,
            playing: false,
            is_youtube: false,
            is_live_stream: false,
            youtube_quality_label: "Auto".to_string(),
            youtube_quality_options: vec![
                "auto".to_string(),
                "1080p".to_string(),
                "720p".to_string(),
                "480p".to_string(),
                "360p".to_string(),
                "audio".to_string(),
            ],
            current_secs: 0.0,
            duration_secs: 0.0,
            buffered_secs: 0.0,
            speed: 1.0,
            video_aspect_ratio: 16.0 / 9.0,
            volume_pct: 100,
            muted: false,
            volume_boost_enabled: false,
            volume_spring: ElasticVolumeSpring::default(),
            repeat: RepeatMode::None,
            shuffle: false,
            ab_loop: AbLoopState::Off,
            locked: false,
            show_unlock_button: true,
            auto_hide_controls: false,
            time_remaining_mode: false,
            chapter_positions_norm: Vec::new(),
            chapter_times_secs: Vec::new(),
            chapter_titles: Vec::new(),
            current_chapter: None,
            hover_thumb_norm: None,
            hover_thumb_has_frame: false,
            hover_thumb_loading: false,
            hover_thumb_shimmer_phase: 0.0,
            is_dragging_progress: false,
            recent_files: Vec::new(),
            playlist: PlaylistUiState::default(),
            volume_osd: VolumeOsdState {
                visible: false,
                percent: 100,
            },
            skip_indicator: SkipIndicatorState {
                visible: false,
                forward: true,
                accumulated_secs: 5,
            },
            seek_indicator: SeekIndicatorState {
                visible: false,
                target_secs: 0.0,
                duration_secs: 0.0,
                label: String::new(),
            },
            center_play_visible: false,
            loading_visible: false,
            stats_overlay: StatsOverlayState {
                visible: false,
                codec: String::new(),
                resolution: String::new(),
                fps: String::new(),
                bitrate: String::new(),
                hwdec: String::new(),
                dropped_frames: 0,
                cache_secs: 0.0,
            },
            recording_indicator: RecordingIndicatorMode::Hidden,
            recording_saved_frame_count: 0,
            carousel_mode: CarouselMode::Hidden,
            carousel_index: 0,
            carousel_total: 0,
            screenshot_notice: None,
            screenshot_badge: None,
            drop_overlay_active: false,
            toasts: Vec::new(),
            equalizer: EqualizerState::default(),
            subtitle_tracks: Vec::new(),
            subtitle_delay_secs: 0.0,
            subtitle_font_size: 28,
            subtitle_bg_opacity_pct: 60,
            subtitle_font_color: "#ffffff".to_string(),
            subtitle_bg_color: "#000000".to_string(),
            audio_tracks: Vec::new(),
            audio_delay_secs: 0.0,
            network_stream_url: String::new(),
            auto_resume: true,
            double_click_fullscreen: true,
            seek_step_secs: 5,
            screenshot_dir: String::new(),
            default_screenshot_dir: String::new(),
            screenshot_format: "jpg".to_string(),
            auto_organize_screenshots: true,
            screenshot_mouse_button: 3,
            shortcut_search_query: String::new(),
            recording_shortcut_id: None,
            recording_shortcut_preview: None,
            custom_shortcuts: BTreeMap::new(),
            youtube_signed_in: false,
            ytdlp_status_text: "Ready".to_string(),
            ytdlp_version: "Bundled".to_string(),
            ytdlp_update_available: false,
            ytdlp_latest_version: None,
            gridmotion_custom_pictures: Vec::new(),
            settings_scroll_y: 0.0,
            bg_settings_scroll_y: 0.0,
            shortcut_scroll_y: 0.0,
            hovered_target: None,
            hover_tooltip_elapsed_secs: 0.0,
            focused_text_field: None,
            text_cursors: TextFieldCursors::default(),
            focused_target: None,
            focus_visible: false,
            saved_focus_before_modal: None,
            anim: UiAnimationState::default(),
        }
    }
}

impl UiState {
    /// Applies a `QualityTier` preset (`settings.js:30-49`) to shader configs.
    pub fn apply_quality_preset(&mut self, tier: QualityTier) {
        self.welcome_quality = tier;
        match tier {
            QualityTier::Low => {
                self.dither_cfg.wave_speed = 0.05;
                self.dither_cfg.wave_frequency = 2.55;
                self.dither_cfg.wave_amplitude = 0.26;
                self.dither_cfg.pixel_size = 4.0;
                self.dither_cfg.color_num = 4.0;
                self.particles_cfg.particle_count = 135;
                self.particles_cfg.speed = 0.09;
                self.particles_cfg.particle_spread = 9.0;
                self.particles_cfg.particle_base_size = 85.0;
                self.faulty_cfg.scale = 0.8;
                self.faulty_cfg.glitch_amount = 0.65;
                self.faulty_cfg.scanline_intensity = 0.46;
                self.faulty_cfg.flicker_amount = 0.65;
                self.faulty_cfg.curvature = 0.16;
                self.gridmotion_cfg.speed = 0.5;
                self.gridmotion_cfg.max_move = 150.0;
            }
            QualityTier::Medium => {
                self.dither_cfg = default_dither_medium();
                self.particles_cfg = default_particles_medium();
                self.faulty_cfg = default_faulty_medium();
                self.gridmotion_cfg = default_gridmotion_medium();
            }
            QualityTier::High => {
                self.dither_cfg.wave_speed = 0.05;
                self.dither_cfg.wave_frequency = 3.0;
                self.dither_cfg.wave_amplitude = 0.3;
                self.dither_cfg.pixel_size = 1.0;
                self.dither_cfg.color_num = 4.0;
                self.particles_cfg.particle_count = 300;
                self.particles_cfg.speed = 0.1;
                self.particles_cfg.particle_spread = 10.0;
                self.particles_cfg.particle_base_size = 100.0;
                self.faulty_cfg.scale = 1.4;
                self.faulty_cfg.glitch_amount = 1.0;
                self.faulty_cfg.scanline_intensity = 0.7;
                self.faulty_cfg.flicker_amount = 1.0;
                self.faulty_cfg.curvature = 0.2;
                self.gridmotion_cfg.speed = 1.5;
                self.gridmotion_cfg.max_move = 450.0;
            }
            QualityTier::Custom => {}
        }
    }

    /// Advances all active UI animations, OSD countdowns, and `volume_spring` by `dt_secs`.
    /// Returns `true` if any animation or timer remains active and needs another redraw.
    pub fn step_animations(&mut self, dt_secs: f32) -> bool {
        let dt = dt_secs.clamp(0.0, 0.1);
        let mut active = false;

        if self.volume_spring.step() {
            active = true;
        }

        let fast_s = (self.motion_profile.fast_ms() / 1000.0).max(0.01);
        let med_s = (self.motion_profile.medium_ms() / 1000.0).max(0.01);
        let slow_s = (self.motion_profile.slow_ms() / 1000.0).max(0.01);

        if self.anim.modal_t < 1.0 {
            self.anim.modal_t = (self.anim.modal_t + dt / med_s).min(1.0);
            if self.anim.modal_t >= 1.0 && self.anim.modal_closing {
                self.anim.modal_closing = false;
                self.closing_modal = None;
            }
            active = true;
        } else if self.anim.modal_closing {
            self.anim.modal_closing = false;
            self.closing_modal = None;
        }

        if self.anim.playlist_t < 1.0 {
            self.anim.playlist_t = (self.anim.playlist_t + dt / 0.35).min(1.0);
            if self.anim.playlist_t >= 1.0 && self.anim.playlist_closing {
                self.anim.playlist_closing = false;
            }
            active = true;
        } else if self.anim.playlist_closing {
            self.anim.playlist_closing = false;
        }

        if self.anim.bg_accordion_t < 1.0 {
            self.anim.bg_accordion_t = (self.anim.bg_accordion_t + dt / med_s).min(1.0);
            if self.anim.bg_accordion_t >= 1.0 && self.anim.bg_accordion_closing {
                self.anim.bg_accordion_closing = false;
            }
            active = true;
        } else if self.anim.bg_accordion_closing {
            self.anim.bg_accordion_closing = false;
        }

        let controls_target = if self.auto_hide_controls || self.locked {
            0.0
        } else {
            1.0
        };
        if (self.anim.controls_t - controls_target).abs() > 0.002 {
            let step = dt / 0.25;
            if self.anim.controls_t < controls_target {
                self.anim.controls_t = (self.anim.controls_t + step).min(controls_target);
            } else {
                self.anim.controls_t = (self.anim.controls_t - step).max(controls_target);
            }
            active = true;
        } else {
            self.anim.controls_t = controls_target;
        }

        if self.bg_accordion_open || self.anim.bg_accordion_closing {
            let kinds = [
                WelcomeBackgroundKind::Dither,
                WelcomeBackgroundKind::Particles,
                WelcomeBackgroundKind::Faulty,
                WelcomeBackgroundKind::Gridmotion,
                WelcomeBackgroundKind::None,
            ];
            let mut target_w = [1.0_f32; 5];
            let mut target_r = [0.0_f32; 5];
            let hovered_card = match &self.hovered_target {
                Some(HitTarget::BgAccordionCard(k)) => Some(*k),
                _ => None,
            };
            for (i, kind) in kinds.iter().enumerate() {
                if *kind == self.welcome_background {
                    target_w[i] = 3.8;
                    target_r[i] = 1.0;
                } else if hovered_card == Some(*kind) {
                    target_w[i] = 2.0;
                    target_r[i] = 1.0;
                }
            }
            let reduced = self.motion_profile == MotionProfile::Reduced;
            if self
                .anim
                .step_accordion_cards(dt, target_w, target_r, reduced)
            {
                active = true;
            }
        } else {
            self.anim.accordion_weights_initialized = false;
        }

        if self.anim.hover_thumb_t < 1.0 {
            self.anim.hover_thumb_t = (self.anim.hover_thumb_t + dt / fast_s).min(1.0);
            active = true;
        }
        if self.anim.toast_t < 1.0 {
            self.anim.toast_t = (self.anim.toast_t + dt / med_s).min(1.0);
            active = true;
        }
        if !self.toasts.is_empty() {
            let reduced = self.motion_profile == MotionProfile::Reduced;
            for toast in &mut self.toasts {
                if toast.enter_t < 1.0 {
                    toast.enter_t = if reduced {
                        1.0
                    } else {
                        (toast.enter_t + dt / fast_s).min(1.0)
                    };
                    active = true;
                }
                if !toast.closing {
                    if !toast.downloading {
                        toast.remaining_secs = (toast.remaining_secs - dt).max(0.0);
                        if toast.remaining_secs == 0.0 {
                            toast.closing = true;
                        }
                        active = true;
                    }
                } else {
                    toast.exit_t = if reduced {
                        1.0
                    } else {
                        (toast.exit_t + dt / 0.25).min(1.0)
                    };
                    active = true;
                }
            }
            self.toasts.retain(|t| !(t.closing && t.exit_t >= 1.0));
        }
        if self.anim.skip_t < 1.0 {
            self.anim.skip_t = (self.anim.skip_t + dt / slow_s).min(1.0);
            active = true;
        }
        if self.anim.screenshot_t < 1.0 {
            let dur = if self.anim.screenshot_deleting {
                0.45
            } else {
                med_s
            };
            self.anim.screenshot_t = (self.anim.screenshot_t + dt / dur).min(1.0);
            active = true;
        }

        if self.skip_indicator.visible && self.anim.skip_ttl_secs > 0.0 {
            self.anim.skip_ttl_secs = (self.anim.skip_ttl_secs - dt).max(0.0);
            if self.anim.skip_ttl_secs == 0.0 {
                self.skip_indicator.visible = false;
                self.skip_indicator.accumulated_secs = 0;
            }
            active = true;
        }

        if self.volume_osd.visible && self.anim.volume_osd_ttl_secs > 0.0 {
            self.anim.volume_osd_ttl_secs = (self.anim.volume_osd_ttl_secs - dt).max(0.0);
            if self.anim.volume_osd_ttl_secs == 0.0 {
                self.volume_osd.visible = false;
            }
            active = true;
        }

        if self.carousel_mode == CarouselMode::SinglePreview && self.anim.screenshot_ttl_secs > 0.0
        {
            self.anim.screenshot_ttl_secs = (self.anim.screenshot_ttl_secs - dt).max(0.0);
            if self.anim.screenshot_ttl_secs == 0.0 {
                self.carousel_mode = CarouselMode::Hidden;
                self.screenshot_badge = None;
                self.anim.screenshot_deleting = false;
            }
            active = true;
        }

        if self.screenshot_notice.is_some() && self.anim.screenshot_notice_ttl_secs > 0.0 {
            self.anim.screenshot_notice_ttl_secs =
                (self.anim.screenshot_notice_ttl_secs - dt).max(0.0);
            if self.anim.screenshot_notice_ttl_secs == 0.0 {
                self.screenshot_notice = None;
            }
            active = true;
        }

        if matches!(
            self.recording_indicator,
            RecordingIndicatorMode::Saved { .. }
        ) && self.anim.recording_saved_ttl_secs > 0.0
        {
            self.anim.recording_saved_ttl_secs = (self.anim.recording_saved_ttl_secs - dt).max(0.0);
            if self.anim.recording_saved_ttl_secs == 0.0 {
                self.recording_indicator = RecordingIndicatorMode::Hidden;
            }
            active = true;
        }

        if self.hover_thumb_loading && self.hover_thumb_norm.is_some() {
            self.hover_thumb_shimmer_phase = (self.hover_thumb_shimmer_phase + dt / 0.8).fract();
            active = true;
        }
        if self.loading_visible {
            self.anim.spinner_phase = (self.anim.spinner_phase + dt / 0.8).fract();
            active = true;
        }
        if matches!(
            self.recording_indicator,
            RecordingIndicatorMode::BurstActive { .. } | RecordingIndicatorMode::ClipActive { .. }
        ) {
            self.anim.recording_pulse_phase = (self.anim.recording_pulse_phase + dt / 1.8).fract();
            active = true;
        }
        if self.ab_loop != AbLoopState::Off {
            self.anim.ab_pulse_phase = (self.anim.ab_pulse_phase + dt / 1.5).fract();
            active = true;
        } else {
            // Electron hides the indicator with `[hidden] { display: none }`, which restarts its
            // CSS animation the next time it is shown. Keep the next A-B badge fading in from
            // opacity 1 rather than resuming a stale phase.
            self.anim.ab_pulse_phase = 0.0;
        }
        if self.hovered_target.is_some()
            && self.hover_tooltip_elapsed_secs < HOVER_TOOLTIP_DELAY_SECS
        {
            self.hover_tooltip_elapsed_secs =
                (self.hover_tooltip_elapsed_secs + dt).min(HOVER_TOOLTIP_DELAY_SECS);
            if self.hover_tooltip_elapsed_secs < HOVER_TOOLTIP_DELAY_SECS {
                active = true;
            }
        }

        active
    }
    pub fn theme_tokens(&self) -> ThemeTokens {
        ThemeTokens::for_mode(self.theme).with_accent_hex(&self.accent_hex)
    }

    pub fn clear_context(&self) -> ClearContext {
        ClearContext {
            welcome_active: self.welcome_active,
            app_cold_start: self.window_motion.cold_start,
            max_transition: self.window_motion.max_transition,
        }
    }

    pub fn open_modal(&mut self, modal: ModalId) {
        if self.active_modal.is_none() {
            self.saved_focus_before_modal = self.focused_target.clone();
        }
        self.closing_modal = None;
        self.anim.modal_closing = false;
        self.anim.modal_t = 1.0;
        self.active_modal = Some(modal);
        self.bg_accordion_open = false;
        self.anim.bg_accordion_closing = false;
        self.active_dropdown = None;
        self.active_color_picker = None;
        self.hover_tooltip_elapsed_secs = 0.0;
        if modal == ModalId::NetworkStream {
            self.focused_text_field = Some(FocusedTextField::NetworkStreamUrl);
            self.focused_target = Some(HitTarget::NetworkStreamInput);
            self.focus_visible = true;
        } else {
            self.focused_text_field = None;
            self.focused_target = None;
        }
    }

    pub fn open_modal_animated(&mut self, modal: ModalId) {
        if self.active_modal.is_none() {
            self.saved_focus_before_modal = self.focused_target.clone();
        }
        self.closing_modal = None;
        self.active_modal = Some(modal);
        self.bg_accordion_open = false;
        self.anim.bg_accordion_closing = false;
        self.active_dropdown = None;
        self.active_color_picker = None;
        self.hover_tooltip_elapsed_secs = 0.0;
        if modal == ModalId::NetworkStream {
            self.focused_text_field = Some(FocusedTextField::NetworkStreamUrl);
            self.focused_target = Some(HitTarget::NetworkStreamInput);
            self.focus_visible = true;
        } else {
            self.focused_text_field = None;
            self.focused_target = None;
        }
        self.anim.trigger_modal_in();
    }

    pub fn close_modals(&mut self) {
        self.active_modal = None;
        self.closing_modal = None;
        self.anim.modal_closing = false;
        self.anim.modal_t = 1.0;
        self.bg_accordion_open = false;
        self.anim.bg_accordion_closing = false;
        self.active_dropdown = None;
        self.active_color_picker = None;
        self.hover_tooltip_elapsed_secs = 0.0;
        self.recording_shortcut_id = None;
        self.recording_shortcut_preview = None;
        if matches!(
            self.focused_text_field,
            Some(FocusedTextField::NetworkStreamUrl | FocusedTextField::ShortcutSearch)
        ) {
            self.focused_text_field = None;
        }
        if let Some(prev) = self.saved_focus_before_modal.take() {
            self.focused_target = Some(prev);
        } else {
            self.focused_target = None;
        }
    }

    pub fn close_modals_animated(&mut self) {
        if let Some(m) = self.active_modal.take() {
            self.closing_modal = Some(m);
            self.anim.trigger_modal_out();
        }
        if self.bg_accordion_open {
            self.bg_accordion_open = false;
            self.anim.trigger_bg_accordion_out();
        }
        self.active_dropdown = None;
        self.active_color_picker = None;
        self.hover_tooltip_elapsed_secs = 0.0;
        self.recording_shortcut_id = None;
        self.recording_shortcut_preview = None;
        if matches!(
            self.focused_text_field,
            Some(FocusedTextField::NetworkStreamUrl | FocusedTextField::ShortcutSearch)
        ) {
            self.focused_text_field = None;
        }
        if let Some(prev) = self.saved_focus_before_modal.take() {
            self.focused_target = Some(prev);
        } else {
            self.focused_target = None;
        }
    }

    pub fn text_field_parts(&self, field: FocusedTextField) -> (&str, &TextFieldCursor) {
        match field {
            FocusedTextField::PlaylistSearch => (
                &self.playlist.search_query,
                &self.text_cursors.playlist_search,
            ),
            FocusedTextField::NetworkStreamUrl => {
                (&self.network_stream_url, &self.text_cursors.network_stream)
            }
            FocusedTextField::ShortcutSearch => (
                &self.shortcut_search_query,
                &self.text_cursors.shortcut_search,
            ),
        }
    }

    pub fn text_field_parts_mut(
        &mut self,
        field: FocusedTextField,
    ) -> (&mut String, &mut TextFieldCursor) {
        match field {
            FocusedTextField::PlaylistSearch => (
                &mut self.playlist.search_query,
                &mut self.text_cursors.playlist_search,
            ),
            FocusedTextField::NetworkStreamUrl => (
                &mut self.network_stream_url,
                &mut self.text_cursors.network_stream,
            ),
            FocusedTextField::ShortcutSearch => (
                &mut self.shortcut_search_query,
                &mut self.text_cursors.shortcut_search,
            ),
        }
    }

    fn clamp_char_boundary(s: &str, idx: usize) -> usize {
        let mut i = idx.min(s.len());
        while i > 0 && !s.is_char_boundary(i) {
            i -= 1;
        }
        i
    }

    fn delete_selection_if_any(s: &mut String, cur: &mut TextFieldCursor) -> bool {
        if let Some(anc) = cur.anchor {
            let a = Self::clamp_char_boundary(s, anc);
            let b = Self::clamp_char_boundary(s, cur.cursor);
            if a != b {
                let (lo, hi) = if a < b { (a, b) } else { (b, a) };
                s.drain(lo..hi);
                cur.cursor = lo;
                cur.anchor = None;
                return true;
            }
        }
        cur.anchor = None;
        false
    }

    pub fn text_field_focus(&mut self, field: Option<FocusedTextField>) {
        self.focused_text_field = field;
        if let Some(f) = field {
            let (s, cur) = self.text_field_parts_mut(f);
            cur.cursor = Self::clamp_char_boundary(s, cur.cursor);
            if cur.cursor == 0 && !s.is_empty() && cur.anchor.is_none() {
                cur.cursor = s.len();
            }
        }
    }

    pub fn text_field_insert(&mut self, field: FocusedTextField, text: &str) {
        let filtered: String = text.chars().filter(|c| !c.is_control()).collect();
        if filtered.is_empty() {
            return;
        }
        let (s, cur) = self.text_field_parts_mut(field);
        Self::delete_selection_if_any(s, cur);
        let idx = Self::clamp_char_boundary(s, cur.cursor);
        s.insert_str(idx, &filtered);
        cur.cursor = idx + filtered.len();
        cur.anchor = None;
        if field == FocusedTextField::ShortcutSearch {
            self.shortcut_scroll_y = 0.0;
        }
    }

    pub fn text_field_set(&mut self, field: FocusedTextField, text: &str) {
        let filtered: String = text.chars().filter(|c| !c.is_control()).collect();
        let (s, cur) = self.text_field_parts_mut(field);
        *s = filtered;
        cur.cursor = s.len();
        cur.anchor = None;
        if field == FocusedTextField::ShortcutSearch {
            self.shortcut_scroll_y = 0.0;
        }
    }

    pub fn text_field_backspace(&mut self, field: FocusedTextField) {
        let (s, cur) = self.text_field_parts_mut(field);
        let mut changed = Self::delete_selection_if_any(s, cur);
        if !changed {
            let idx = Self::clamp_char_boundary(s, cur.cursor);
            if idx > 0 {
                if let Some((prev_idx, _)) = s[..idx].char_indices().next_back() {
                    s.drain(prev_idx..idx);
                    cur.cursor = prev_idx;
                    changed = true;
                }
            }
        }
        if changed && field == FocusedTextField::ShortcutSearch {
            self.shortcut_scroll_y = 0.0;
        }
    }

    pub fn text_field_delete(&mut self, field: FocusedTextField) {
        let (s, cur) = self.text_field_parts_mut(field);
        let mut changed = Self::delete_selection_if_any(s, cur);
        if !changed {
            let idx = Self::clamp_char_boundary(s, cur.cursor);
            if idx < s.len() {
                if let Some(ch) = s[idx..].chars().next() {
                    s.drain(idx..idx + ch.len_utf8());
                    changed = true;
                }
            }
        }
        if changed && field == FocusedTextField::ShortcutSearch {
            self.shortcut_scroll_y = 0.0;
        }
    }

    pub fn text_field_move_left(&mut self, field: FocusedTextField, select: bool) {
        let (s, cur) = self.text_field_parts_mut(field);
        let idx = Self::clamp_char_boundary(s, cur.cursor);
        if !select {
            if let Some(anc) = cur.anchor.take() {
                let a = Self::clamp_char_boundary(s, anc);
                if a != idx {
                    cur.cursor = a.min(idx);
                    return;
                }
            }
        } else if cur.anchor.is_none() {
            cur.anchor = Some(idx);
        }
        if idx > 0 {
            if let Some((prev_idx, _)) = s[..idx].char_indices().next_back() {
                cur.cursor = prev_idx;
            }
        }
    }

    pub fn text_field_move_right(&mut self, field: FocusedTextField, select: bool) {
        let (s, cur) = self.text_field_parts_mut(field);
        let idx = Self::clamp_char_boundary(s, cur.cursor);
        if !select {
            if let Some(anc) = cur.anchor.take() {
                let a = Self::clamp_char_boundary(s, anc);
                if a != idx {
                    cur.cursor = a.max(idx);
                    return;
                }
            }
        } else if cur.anchor.is_none() {
            cur.anchor = Some(idx);
        }
        if idx < s.len() {
            if let Some(ch) = s[idx..].chars().next() {
                cur.cursor = idx + ch.len_utf8();
            }
        }
    }

    pub fn text_field_home(&mut self, field: FocusedTextField, select: bool) {
        let (s, cur) = self.text_field_parts_mut(field);
        let idx = Self::clamp_char_boundary(s, cur.cursor);
        if select && cur.anchor.is_none() {
            cur.anchor = Some(idx);
        } else if !select {
            cur.anchor = None;
        }
        cur.cursor = 0;
    }

    pub fn text_field_end(&mut self, field: FocusedTextField, select: bool) {
        let (s, cur) = self.text_field_parts_mut(field);
        let idx = Self::clamp_char_boundary(s, cur.cursor);
        if select && cur.anchor.is_none() {
            cur.anchor = Some(idx);
        } else if !select {
            cur.anchor = None;
        }
        cur.cursor = s.len();
    }

    pub fn text_field_select_all(&mut self, field: FocusedTextField) {
        let (s, cur) = self.text_field_parts_mut(field);
        cur.anchor = if s.is_empty() { None } else { Some(0) };
        cur.cursor = s.len();
    }

    pub fn text_field_selected_text(&self, field: FocusedTextField) -> Option<String> {
        let (s, cur) = self.text_field_parts(field);
        let anc = cur.anchor?;
        let a = Self::clamp_char_boundary(s, anc);
        let b = Self::clamp_char_boundary(s, cur.cursor);
        if a == b {
            None
        } else {
            let (lo, hi) = if a < b { (a, b) } else { (b, a) };
            Some(s[lo..hi].to_string())
        }
    }

    pub fn push_toast(&mut self, mut toast: ToastItem) {
        if toast.remaining_secs <= 0.0 {
            toast.remaining_secs = ToastItem::default_ttl_secs(toast.kind);
        }
        toast.enter_t = 0.0;
        toast.closing = false;
        toast.exit_t = 0.0;
        self.toasts.push(toast);
        if self.toasts.len() > MAX_VISIBLE_TOASTS {
            let drain_count = self.toasts.len() - MAX_VISIBLE_TOASTS;
            self.toasts.drain(0..drain_count);
        }
        self.anim.trigger_toast_in();
    }

    pub fn set_eq_preset(&mut self, preset: EqualizerPreset) {
        if let Some(bands) = preset.bands() {
            self.equalizer.bands = bands;
        }
        self.equalizer.preset = preset;
    }

    pub fn set_eq_band(&mut self, band_idx: usize, gain_db: f64) {
        if band_idx < EQ_BAND_COUNT {
            self.equalizer.bands[band_idx] = gain_db.clamp(-12.0, 12.0);
            self.equalizer.preset = EqualizerPreset::Custom;
        }
    }

    pub fn set_screenshot_mouse_button(&mut self, button: i32) {
        self.screenshot_mouse_button = if VALID_SCREENSHOT_MOUSE_BUTTONS.contains(&button) {
            button
        } else {
            3
        };
    }

    /// Filters the 52 `SHORTCUT_CATALOG` actions for Settings UI search.
    pub fn filtered_shortcuts(&self) -> Vec<(&'static str, &'static str, &'static str, String)> {
        let q = self.shortcut_search_query.trim().to_ascii_lowercase();
        SHORTCUT_CATALOG
            .iter()
            .filter_map(|entry| {
                let combo = self
                    .custom_shortcuts
                    .get(entry.id)
                    .cloned()
                    .unwrap_or_else(|| entry.default_combo.to_string());
                let display = format_combo_for_display(&combo).to_ascii_lowercase();
                if q.is_empty()
                    || entry.id.to_ascii_lowercase().contains(&q)
                    || entry.label.to_ascii_lowercase().contains(&q)
                    || entry.category.to_ascii_lowercase().contains(&q)
                    || combo.to_ascii_lowercase().contains(&q)
                    || display.contains(&q)
                {
                    Some((entry.id, entry.category, entry.label, combo))
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn shortcut_scroll_max(&self) -> f32 {
        let filtered = self.filtered_shortcuts();
        if filtered.is_empty() {
            return 0.0;
        }
        let mut total_h = 0.0_f32;
        let mut first_cat = true;
        for cat in SHORTCUT_CATEGORIES {
            let count = filtered.iter().filter(|(_, c, _, _)| *c == cat).count();
            if count == 0 {
                continue;
            }
            if !first_cat {
                total_h += 12.0;
            }
            first_cat = false;
            total_h += 20.0 + (count as f32) * 38.0 - 4.0;
        }
        (total_h - 290.0).max(0.0)
    }

    /// Formats playback time (`MM:SS` or `H:MM:SS`) matching `controls.js`.
    pub fn format_time(secs: f64) -> String {
        if !secs.is_finite() || secs <= 0.0 {
            return "0:00".to_string();
        }
        let total = secs.floor() as u64;
        let h = total / 3600;
        let m = (total % 3600) / 60;
        let s = total % 60;
        if h > 0 {
            format!("{h}:{m:02}:{s:02}")
        } else {
            format!("{m}:{s:02}")
        }
    }
}
