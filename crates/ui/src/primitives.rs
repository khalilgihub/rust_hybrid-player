//! Render-primitive output structs decoupled from `wgpu` implementation details.
//! `crates/app` and `crates/shaders` consume `UiFrameOutput` to rasterize glass, text, SVGs, and hit-testing.

use hybrid_shaders::{
    ChameleonSkin, ClearContext, CssBackdropRule, GlassRect, WelcomeBackgroundKind,
};

use crate::fonts::FontRole;
use crate::icons::SvgVariant;
use crate::svg_catalog::SvgId;

/// Axis-aligned rectangle in logical CSS pixels (top-left origin).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RectPx {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl RectPx {
    #[inline]
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    #[inline]
    pub fn contains(self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.width && py >= self.y && py <= self.y + self.height
    }

    #[inline]
    pub fn expanded(self, pad: f32) -> Self {
        Self {
            x: self.x - pad,
            y: self.y - pad,
            width: self.width + pad * 2.0,
            height: self.height + pad * 2.0,
        }
    }
}

/// Active responsive CSS breakpoints evaluated for the current viewport and playlist state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BreakpointState {
    pub viewport_w: f32,
    pub viewport_h: f32,
    pub playlist_open: bool,
    pub max_w_1150_playlist: bool,
    /// Compact sizing tier for playlist-open layouts that still show every playback toggle.
    pub max_w_1280_playlist: bool,
    pub max_w_1000: bool,
    pub max_w_950: bool,
    pub max_w_860: bool,
    pub max_w_760: bool,
    pub max_w_720: bool,
    pub max_w_640: bool,
    pub max_h_620: bool,
}

impl BreakpointState {
    pub fn evaluate(viewport_w: f32, viewport_h: f32, playlist_open: bool) -> Self {
        Self {
            viewport_w,
            viewport_h,
            playlist_open,
            max_w_1150_playlist: playlist_open && viewport_w <= 1150.0,
            max_w_1280_playlist: playlist_open && viewport_w <= 1280.0,
            max_w_1000: viewport_w <= 1000.0,
            max_w_950: viewport_w <= 950.0,
            max_w_860: viewport_w <= 860.0,
            max_w_760: viewport_w <= 760.0,
            max_w_720: viewport_w <= 720.0,
            max_w_640: viewport_w <= 640.0,
            max_h_620: viewport_h <= 620.0,
        }
    }

    /// Titlebar button width (`46px` default, `38px` at `<=720px`).
    pub fn titlebar_btn_width(self) -> f32 {
        if self.max_w_720 {
            38.0
        } else {
            46.0
        }
    }

    /// Playlist sidebar width (`320px` default, `min(320, viewport_w)` at `<=640px`).
    pub fn playlist_width(self) -> f32 {
        if self.max_w_640 {
            self.viewport_w.min(320.0)
        } else {
            320.0
        }
    }

    pub fn ctrl_btn_size(self) -> f32 {
        self.ctrl_btn_size_for(false)
    }

    /// Control button size for the given screen context.
    ///
    /// The 1151-1280px playlist-open compaction only applies during playback: on the welcome
    /// screen Electron hides `bgSettingsToggle` when the queue opens, so the remaining controls
    /// still fit at full size.
    pub fn ctrl_btn_size_for(self, welcome_active: bool) -> f32 {
        if self.max_w_1150_playlist {
            24.0
        } else if self.max_w_1280_playlist && !welcome_active {
            // Playlist steals 320px but `show_eq_and_subs()`/`show_speed_btn()` are still true,
            // so every playback toggle must share the bar at a slightly reduced size.
            30.0
        } else if self.max_w_760 {
            28.0
        } else if self.max_w_860 {
            32.0
        } else if self.max_w_1000 {
            34.0
        } else {
            40.0
        }
    }

    pub fn ctrl_play_btn_size(self) -> f32 {
        self.ctrl_play_btn_size_for(false)
    }

    /// Play-button size for the given screen context (see [`Self::ctrl_btn_size_for`]).
    pub fn ctrl_play_btn_size_for(self, welcome_active: bool) -> f32 {
        if self.max_w_1150_playlist {
            28.0
        } else if self.max_w_1280_playlist && !welcome_active {
            32.0
        } else if self.max_w_760 {
            32.0
        } else if self.max_w_1000 {
            38.0
        } else {
            44.0
        }
    }

    pub fn volume_slider_width(self) -> f32 {
        if self.max_w_1150_playlist {
            34.0
        } else if self.max_w_760 {
            40.0
        } else if self.max_w_860 {
            50.0
        } else if self.max_w_1000 {
            60.0
        } else {
            80.0
        }
    }

    pub fn show_prev_next(self) -> bool {
        !(self.playlist_open && self.max_w_760)
    }

    pub fn show_volume_value(self) -> bool {
        if self.max_w_1150_playlist || self.max_w_760 {
            return false;
        }
        if self.playlist_open && self.max_w_640 {
            return false;
        }
        true
    }

    pub fn show_time_display(self) -> bool {
        !(self.playlist_open && self.max_w_640)
    }

    /// Secondary toggles hidden at `<=860px` (which also covers `<=760px`).
    ///
    /// Unlike `controls.css:248-256`, the playlist drawer does not suppress them: keeping
    /// repeat/shuffle/A-B loop/lock reachable while the queue is open avoids losing playback
    /// settings on narrow-but-supported widths.
    pub fn show_secondary_toggles(self) -> bool {
        !self.max_w_860
    }

    pub fn show_eq_and_subs(self) -> bool {
        !self.max_w_1150_playlist
    }

    pub fn show_speed_btn(self) -> bool {
        !self.max_w_1150_playlist
    }

    pub fn show_audio_track(self) -> bool {
        !self.max_w_1150_playlist && !self.max_w_760
    }

    pub fn show_youtube_quality(self, is_youtube: bool) -> bool {
        is_youtube && !self.max_w_760
    }

    /// `#btnScreenshot` is permanently hidden (`display:none !important` in `index.html:559`).
    pub fn show_screenshot_btn(self) -> bool {
        false
    }
}

/// SDF Glassmorphism panel primitive backed directly by `hybrid_shaders::GlassRect`.
#[derive(Debug, Clone, PartialEq)]
pub struct UiGlassPrimitive {
    pub id: String,
    pub z_index: i32,
    pub glass: GlassRect,
    pub backdrop_rule: Option<CssBackdropRule>,
    pub chameleon_skin: ChameleonSkin,
    pub clip_rect: Option<RectPx>,
}

/// Solid or linear-gradient 2D rectangle primitive (tracks, sliders, progress bar, dividers, swatches, backdrops).
#[derive(Debug, Clone, PartialEq)]
pub struct UiSolidRect {
    pub id: String,
    pub z_index: i32,
    pub rect: RectPx,
    pub color: [f32; 4],
    pub gradient: Option<([f32; 4], f32)>,
    pub corner_radii: [f32; 4],
    pub border_width: f32,
    pub border_color: [f32; 4],
    pub clip_rect: Option<RectPx>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// Shaped text run primitive rendered by `crates/app` / `crates/ui::TextEngine`.
#[derive(Debug, Clone, PartialEq)]
pub struct UiTextRun {
    pub id: String,
    pub z_index: i32,
    pub rect: RectPx,
    pub text: String,
    pub font_role: FontRole,
    pub font_size_px: f32,
    pub line_height_px: f32,
    pub weight: u16,
    pub letter_spacing_px: f32,
    pub color: [f32; 4],
    pub align: TextAlign,
    pub allow_wrap: bool,
    pub clip_rect: Option<RectPx>,
}

/// Rasterized SVG quad primitive referencing `SvgId`.
#[derive(Debug, Clone, PartialEq)]
pub struct UiSvgQuad {
    pub id: String,
    pub z_index: i32,
    pub svg: SvgId,
    pub variant: SvgVariant,
    pub rect: RectPx,
    pub tint_rgba: [u8; 4],
    pub opacity: f32,
    pub rotation_deg: f32,
    pub clip_rect: Option<RectPx>,
}

/// Welcome Shader render target slot (either full-bleed backdrop or `400x240` accordion preview).
#[derive(Debug, Clone, PartialEq)]
pub struct UiShaderSlot {
    pub id: String,
    pub z_index: i32,
    pub kind: WelcomeBackgroundKind,
    pub rect: RectPx,
    pub is_preview: bool,
    pub opacity: f32,
    pub clip_rect: Option<RectPx>,
}

/// Thumbnail / image quad slot (screenshot preview, carousel slide, hover seek thumb).
#[derive(Debug, Clone, PartialEq)]
pub struct UiImageSlot {
    pub id: String,
    pub z_index: i32,
    pub rect: RectPx,
    pub corner_radius: f32,
    pub opacity: f32,
    pub key: String,
    pub clip_rect: Option<RectPx>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CursorHint {
    Default,
    Pointer,
    Text,
    Grab,
}

/// Semantic UI hit-test action target for pointer/wheel/drag dispatch.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HitTarget {
    TitlebarDrag,
    TitlebarMinimize,
    TitlebarMaximize,
    TitlebarClose,
    OpenFile,
    OpenLink,
    RecentFile(usize),
    ProgressSeek,
    PrevTrack,
    PlayPause,
    NextTrack,
    MuteToggle,
    VolumeSlider,
    BgSettingsToggle,
    SpeedButton,
    EqualizerButton,
    AudioTrackButton,
    SubtitlesButton,
    PlaylistToggle,
    RepeatToggle,
    ShuffleToggle,
    AbLoopToggle,
    LockToggle,
    YoutubeQualityButton,
    SettingsButton,
    FullscreenToggle,
    UnlockControls,
    RecordingStopClip,
    RecordingOpenFolder,
    CarouselPrev,
    CarouselNext,
    CarouselConfirm,
    CarouselClose,
    SinglePreviewOpenFolder,
    PlaylistClose,
    PlaylistAddFiles,
    PlaylistShuffle,
    PlaylistRepeat,
    PlaylistClear,
    PlaylistSearch,
    PlaylistRow(usize),
    PlaylistRowRemove(usize),
    ModalBackdrop,
    ModalClose,
    EqPresetSelect,
    EqBandSlider(usize),
    EqReset,
    SpeedPreset(usize),
    SpeedSlider,
    SubDelayStep(i32),
    SubDelaySlider,
    SubtitleTrackSelect(i64),
    SubtitleLoadExternal,
    SubtitleFontSizeSlider,
    SubtitleFontColorPicker,
    SubtitleBgColorPicker,
    SubtitleBgOpacitySlider,
    AudioPresetSubs,
    AudioPresetDubs,
    AudioTrackSelect(i64),
    AudioDelayStep(i32),
    BgEffectPickerToggle,
    BgAccordionCard(WelcomeBackgroundKind),
    BgQualityRadio(u8),
    BgControlInput(String),
    BgGridmotionAddImages,
    BgGridmotionClearImages,
    BgGridmotionRemoveImage(usize),
    BgResetCurrent,
    // Settings modal section controls
    SettingsThemeSelect,
    SettingsAccentPicker,
    SettingsBrandFontsToggle,
    SettingsAutoResumeToggle,
    SettingsMotionSelect,
    SettingsSeekStepSelect,
    SettingsDoubleClickToggle,
    SettingsUnlockVisToggle,
    SettingsScreenshotDirPick,
    SettingsScreenshotDirOpen,
    SettingsScreenshotDirReset,
    SettingsScreenshotFormatSelect,
    SettingsAutoOrganizeToggle,
    SettingsMouseScreenshotSelect,
    SettingsOpenMultiple,
    SettingsOpenFolder,
    SettingsQuit,
    ShortcutSearch,
    ShortcutRebind(String),
    ShortcutResetOne(String),
    ShortcutResetAll,
    YoutubeSignIn,
    YoutubeSwitchAccount,
    YoutubeSignOut,
    YoutubeImportBrowserCookies,
    YoutubeImportCookiesFile,
    YtdlpCheckUpdate,
    YtdlpPerformUpdate,
    NetworkStreamInput,
    NetworkStreamOpen,
    ToastDismiss(u64),
    ToastUpdateNow,
    DropdownOption(usize),
    DropdownBackdrop,
    ModalPanelSurface,
    PopoverSurface,
    BgAccordionSurface,
    ControlsBarSurface,
    PlaylistSurface,
    WelcomeContentSurface,
    VolumeContainerSurface,
    TimeRemainingToggle,
    ChapterMarker(usize),
    ColorPickerSwatch(u8),
    ColorPickerSpectrum,
    ColorPickerChannel(u8),
    ColorPickerClose,
    ShortcutScrollArea,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HitRegion {
    pub target: HitTarget,
    pub z_index: i32,
    pub rect: RectPx,
    pub cursor: CursorHint,
}

/// Virtualization window slice calculated by `PlaylistState::virtual_window`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct VirtualSlice {
    pub is_virtualized: bool,
    pub total_items: usize,
    pub total_height_px: f32,
    pub start_index: usize,
    pub end_index: usize,
    pub offset_top_px: f32,
}

/// Complete frame output emitted by `build_ui_frame` for `crates/app` & `crates/shaders`.
#[derive(Debug, Clone)]
pub struct UiFrameOutput {
    pub clear_context: ClearContext,
    pub breakpoints: BreakpointState,
    pub playlist_slice: VirtualSlice,
    pub shader_slots: Vec<UiShaderSlot>,
    pub glass_panels: Vec<UiGlassPrimitive>,
    pub solid_rects: Vec<UiSolidRect>,
    pub image_slots: Vec<UiImageSlot>,
    pub text_runs: Vec<UiTextRun>,
    pub svg_quads: Vec<UiSvgQuad>,
    pub hit_regions: Vec<HitRegion>,
    pub a11y_tree: accesskit::TreeUpdate,
}

impl UiFrameOutput {
    /// Returns the highest-z-index `HitRegion` containing `(x, y)`.
    pub fn hit_test(&self, x: f32, y: f32) -> Option<&HitRegion> {
        self.hit_regions
            .iter()
            .enumerate()
            .filter(|(_, region)| region.rect.contains(x, y))
            .max_by_key(|(idx, region)| (region.z_index, *idx))
            .map(|(_, region)| region)
    }

    /// Returns the `CursorHint` for `(x, y)` (`CursorHint::Default` when no region matches).
    pub fn cursor_hint_at(&self, x: f32, y: f32) -> CursorHint {
        self.hit_test(x, y)
            .map(|r| r.cursor)
            .unwrap_or(CursorHint::Default)
    }
}
