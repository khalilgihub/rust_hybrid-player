use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::blur::DualKawaseBlur;
use crate::types::{ThemeMode, WelcomeBackgroundKind};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GlassGlobals {
    // xy = viewport_size (px), z = time (s), w = unused
    viewport_time: [f32; 4],
}

/// GPU per-instance layout matching `GlassInstanceInput` in `sdf_glass.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct GlassInstanceGpu {
    pub rect: [f32; 4],
    pub radii: [f32; 4],
    pub glass_params: [f32; 4],
    pub fill_color_0: [f32; 4],
    pub fill_color_1: [f32; 4],
    pub border_color_0: [f32; 4],
    pub border_color_1: [f32; 4],
    pub border_color_2: [f32; 4],
    pub shadow_geom: [f32; 4],
    pub shadow_color: [f32; 4],
    pub inset_highlight: [f32; 4],
    pub extra: [f32; 4],
    pub ring_color: [f32; 4],
    pub clip_rect: [f32; 4],
}

/// All 23 active CSS `backdrop-filter` rules and 7 `none` overrides across the stylesheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CssBackdropRule {
    // Active 23 rules:
    ToastUpdatePill,        // animations.css:224 — blur(28px) saturate(190%)
    TopRightVolumeOsd,      // animations.css:446 — blur(10px)
    SpeedPanel,             // controls.css:748 — blur(28px)
    SubtitlePanel,          // controls.css:900 — blur(28px)
    RecordingIndicator,     // main.css:633 — blur(32px) saturate(190%)
    RecordingClipActive,    // main.css:780 — blur(12px) saturate(150%)
    CarouselMode,           // main.css:1217 — blur(14px)
    CarouselBadge,          // main.css:1265 — blur(8px)
    CarouselCloseBtn,       // main.css:1309 — blur(6px)
    CarouselNavBtn,         // main.css:1357 — blur(8px)
    ScreenshotNotice,       // main.css:1430 — blur(14px)
    WelcomeBgQualityMedium, // main.css:1723 — blur(10px) saturate(125%)
    WelcomeBgFaulty,        // main.css:1842 — blur(4px) saturate(110%)
    WelcomeBgParticles,     // main.css:1881 — blur(20px) saturate(150%)
    WelcomeBgNone,          // main.css:1948 — blur(10px)
    WelcomeRecent,          // main.css:2389 — blur(8px)
    StatsOverlay,           // main.css:2614 — var(--glass-blur) (20/25px)
    ModalPanel,             // main.css:2691 — var(--glass-blur) (20/25px)
    DropOverlay,            // main.css:2900 — blur(10px)
    SettingsModal,          // settings.css:15 — blur(28px)
    BgSettingsPanel,        // settings.css:684 — var(--glass-blur) (20/25px)
    CustomSelectDropdown,   // settings.css:891 — blur(16px)
    BgAccordionPopover,     // settings.css:1574 — var(--glass-blur) (20/25px)
    // 7 `none` overrides:
    TitlebarNone,               // main.css:163
    TitlebarDragNone,           // main.css:196
    WelcomeBgQualityLowNone,    // main.css:1727
    WelcomeBgDitherNone,        // main.css:1812
    ModalOverlayNone,           // main.css:2667
    BgSettingsModalOverlayNone, // main.css:2979
    BgAccordionItemNone,        // settings.css:1683
}

/// Backdrop blur + saturation parameters matching a CSS `backdrop-filter` rule.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BackdropFilterSpec {
    pub blur_px: f32,
    pub saturation: f32,
    pub is_none_override: bool,
}

impl CssBackdropRule {
    /// All 23 active CSS `backdrop-filter` rules.
    pub const ACTIVE_RULES: [CssBackdropRule; 23] = [
        Self::ToastUpdatePill,
        Self::TopRightVolumeOsd,
        Self::SpeedPanel,
        Self::SubtitlePanel,
        Self::RecordingIndicator,
        Self::RecordingClipActive,
        Self::CarouselMode,
        Self::CarouselBadge,
        Self::CarouselCloseBtn,
        Self::CarouselNavBtn,
        Self::ScreenshotNotice,
        Self::WelcomeBgQualityMedium,
        Self::WelcomeBgFaulty,
        Self::WelcomeBgParticles,
        Self::WelcomeBgNone,
        Self::WelcomeRecent,
        Self::StatsOverlay,
        Self::ModalPanel,
        Self::DropOverlay,
        Self::SettingsModal,
        Self::BgSettingsPanel,
        Self::CustomSelectDropdown,
        Self::BgAccordionPopover,
    ];

    /// All 7 CSS `backdrop-filter: none` overrides.
    pub const NONE_OVERRIDES: [CssBackdropRule; 7] = [
        Self::TitlebarNone,
        Self::TitlebarDragNone,
        Self::WelcomeBgQualityLowNone,
        Self::WelcomeBgDitherNone,
        Self::ModalOverlayNone,
        Self::BgSettingsModalOverlayNone,
        Self::BgAccordionItemNone,
    ];

    /// Resolves blur radius (px) and saturation multiplier (`1.0` = 100%, `1.9` = 190%).
    pub fn spec(self, theme: ThemeMode) -> BackdropFilterSpec {
        let glass = theme.glass_blur_px();
        match self {
            Self::ToastUpdatePill => BackdropFilterSpec {
                blur_px: 28.0,
                saturation: 1.9,
                is_none_override: false,
            },
            Self::TopRightVolumeOsd => BackdropFilterSpec {
                blur_px: 10.0,
                saturation: 1.0,
                is_none_override: false,
            },
            Self::SpeedPanel => BackdropFilterSpec {
                blur_px: 28.0,
                saturation: 1.0,
                is_none_override: false,
            },
            Self::SubtitlePanel => BackdropFilterSpec {
                blur_px: 28.0,
                saturation: 1.0,
                is_none_override: false,
            },
            Self::RecordingIndicator => BackdropFilterSpec {
                blur_px: 32.0,
                saturation: 1.9,
                is_none_override: false,
            },
            Self::RecordingClipActive => BackdropFilterSpec {
                blur_px: 12.0,
                saturation: 1.5,
                is_none_override: false,
            },
            Self::CarouselMode => BackdropFilterSpec {
                blur_px: 14.0,
                saturation: 1.0,
                is_none_override: false,
            },
            Self::CarouselBadge => BackdropFilterSpec {
                blur_px: 8.0,
                saturation: 1.0,
                is_none_override: false,
            },
            Self::CarouselCloseBtn => BackdropFilterSpec {
                blur_px: 6.0,
                saturation: 1.0,
                is_none_override: false,
            },
            Self::CarouselNavBtn => BackdropFilterSpec {
                blur_px: 8.0,
                saturation: 1.0,
                is_none_override: false,
            },
            Self::ScreenshotNotice => BackdropFilterSpec {
                blur_px: 14.0,
                saturation: 1.0,
                is_none_override: false,
            },
            Self::WelcomeBgQualityMedium => BackdropFilterSpec {
                blur_px: 10.0,
                saturation: 1.25,
                is_none_override: false,
            },
            Self::WelcomeBgFaulty => BackdropFilterSpec {
                blur_px: 4.0,
                saturation: 1.1,
                is_none_override: false,
            },
            Self::WelcomeBgParticles => BackdropFilterSpec {
                blur_px: 20.0,
                saturation: 1.5,
                is_none_override: false,
            },
            Self::WelcomeBgNone => BackdropFilterSpec {
                blur_px: 10.0,
                saturation: 1.0,
                is_none_override: false,
            },
            Self::WelcomeRecent => BackdropFilterSpec {
                blur_px: 8.0,
                saturation: 1.0,
                is_none_override: false,
            },
            Self::StatsOverlay => BackdropFilterSpec {
                blur_px: glass,
                saturation: 1.0,
                is_none_override: false,
            },
            Self::ModalPanel => BackdropFilterSpec {
                blur_px: glass,
                saturation: 1.0,
                is_none_override: false,
            },
            Self::DropOverlay => BackdropFilterSpec {
                blur_px: 10.0,
                saturation: 1.0,
                is_none_override: false,
            },
            Self::SettingsModal => BackdropFilterSpec {
                blur_px: 28.0,
                saturation: 1.0,
                is_none_override: false,
            },
            Self::BgSettingsPanel => BackdropFilterSpec {
                blur_px: glass,
                saturation: 1.0,
                is_none_override: false,
            },
            Self::CustomSelectDropdown => BackdropFilterSpec {
                blur_px: 16.0,
                saturation: 1.0,
                is_none_override: false,
            },
            Self::BgAccordionPopover => BackdropFilterSpec {
                blur_px: glass,
                saturation: 1.0,
                is_none_override: false,
            },
            Self::TitlebarNone
            | Self::TitlebarDragNone
            | Self::WelcomeBgQualityLowNone
            | Self::WelcomeBgDitherNone
            | Self::ModalOverlayNone
            | Self::BgSettingsModalOverlayNone
            | Self::BgAccordionItemNone => BackdropFilterSpec {
                blur_px: 0.0,
                saturation: 1.0,
                is_none_override: true,
            },
        }
    }
}

/// Chameleon skin variant matching `settings.css:1388-1459`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ChameleonSkin {
    #[default]
    None,
    Dither,
    Faulty,
    Particles,
    Gridmotion,
}

impl From<WelcomeBackgroundKind> for ChameleonSkin {
    fn from(kind: WelcomeBackgroundKind) -> Self {
        match kind {
            WelcomeBackgroundKind::None => Self::None,
            WelcomeBackgroundKind::Dither => Self::Dither,
            WelcomeBackgroundKind::Faulty => Self::Faulty,
            WelcomeBackgroundKind::Particles => Self::Particles,
            WelcomeBackgroundKind::Gridmotion => Self::Gridmotion,
        }
    }
}

impl ChameleonSkin {
    /// Skin ID passed to `sdf_glass.wgsl`.
    pub fn shader_id(self) -> f32 {
        match self {
            Self::None => 0.0,
            Self::Dither => 1.0,
            Self::Faulty => 2.0,
            Self::Particles => 3.0,
            Self::Gridmotion => 4.0,
        }
    }

    /// `--bgp-accent` hex RGB color (`settings.css:1389-1450`).
    pub fn accent_rgba(self) -> [f32; 4] {
        match self {
            Self::Dither => [184.0 / 255.0, 196.0 / 255.0, 1.0, 1.0], // #b8c4ff
            Self::Faulty => [167.0 / 255.0, 239.0 / 255.0, 158.0 / 255.0, 1.0], // #A7EF9E
            Self::Particles => [126.0 / 255.0, 200.0 / 255.0, 1.0, 1.0], // #7ec8ff
            Self::Gridmotion => [74.0 / 255.0, 231.0 / 255.0, 210.0 / 255.0, 1.0], // #4ae7d2
            Self::None => [229.0 / 255.0, 9.0 / 255.0, 20.0 / 255.0, 1.0], // #e50914
        }
    }
}

/// High-level SDF glass surface descriptor (Pass 2).
#[derive(Debug, Clone, PartialEq)]
pub struct GlassRect {
    /// Top-left X in pixels.
    pub x: f32,
    /// Top-left Y in pixels.
    pub y: f32,
    /// Width in pixels.
    pub width: f32,
    /// Height in pixels.
    pub height: f32,
    /// Corner radii `[top_left, top_right, bottom_right, bottom_left]` in pixels.
    pub corner_radii: [f32; 4],
    /// Backdrop blur radius in px (`0.0` for `none` override, `4.0..=32.0` for `Blur4..Blur32`).
    pub blur_px: f32,
    /// Backdrop saturation (`1.0` = 100%, `1.9` = 190%).
    pub saturation: f32,
    /// Border thickness in px.
    pub border_width: f32,
    /// Chameleon skin overlay selector.
    pub skin: ChameleonSkin,
    /// `padding-box` linear gradient stops `(color0, color1)` and angle in degrees.
    pub fill_color_0: [f32; 4],
    pub fill_color_1: [f32; 4],
    pub fill_angle_deg: f32,
    /// `border-box` linear gradient stops `(color0, color1, color2)` and angle in degrees.
    pub border_color_0: [f32; 4],
    pub border_color_1: [f32; 4],
    pub border_color_2: [f32; 4],
    pub border_angle_deg: f32,
    /// Drop shadow geometry: offset `(x, y)`, blur (px), spread (px), color.
    pub shadow_offset: [f32; 2],
    pub shadow_blur: f32,
    pub shadow_spread: f32,
    pub shadow_color: [f32; 4],
    /// Optional `inset 0 1px 0` top rim highlight.
    pub inset_highlight: [f32; 4],
    /// Optional outer 1px ring (`0 0 0 1px ...`).
    pub outer_ring_width: f32,
    pub outer_ring_color: [f32; 4],
    /// Overall surface opacity (`0.0..=1.0`).
    pub opacity: f32,
    /// Optional fragment clip rectangle `[x, y, width, height]` in target pixel coordinates.
    pub clip_rect: Option<[f32; 4]>,
}

impl GlassRect {
    /// Creates a standard glass rect with uniform corner radius and `ThemeMode` defaults.
    pub fn new(x: f32, y: f32, width: f32, height: f32, radius: f32, theme: ThemeMode) -> Self {
        let fill = theme.bg_glass_heavy();
        let border = theme.glass_border_color();
        Self {
            x,
            y,
            width,
            height,
            corner_radii: [radius; 4],
            blur_px: theme.glass_blur_px(),
            saturation: 1.0,
            border_width: 1.0,
            skin: ChameleonSkin::None,
            fill_color_0: fill,
            fill_color_1: fill,
            fill_angle_deg: 180.0,
            border_color_0: border,
            border_color_1: border,
            border_color_2: border,
            border_angle_deg: 130.0,
            shadow_offset: [0.0, 4.0],
            shadow_blur: 20.0,
            shadow_spread: 0.0,
            shadow_color: [0.0, 0.0, 0.0, 0.5],
            inset_highlight: [0.0; 4],
            outer_ring_width: 0.0,
            outer_ring_color: [0.0; 4],
            opacity: 1.0,
            clip_rect: None,
        }
    }

    /// Sets an optional clip rectangle `[x, y, width, height]`.
    pub fn with_clip_rect(mut self, clip_rect: [f32; 4]) -> Self {
        self.clip_rect = Some(clip_rect);
        self
    }

    /// Applies a `CssBackdropRule`'s exact blur radius and saturation multiplier.
    pub fn with_backdrop_rule(mut self, rule: CssBackdropRule, theme: ThemeMode) -> Self {
        let spec = rule.spec(theme);
        self.blur_px = spec.blur_px;
        self.saturation = spec.saturation;
        self
    }

    /// Applies the exact Chameleon skin colors, dual `padding-box`/`border-box` gradients,
    /// drop shadows, and inset highlights from `settings.css:1388-1459`.
    pub fn with_chameleon_skin(mut self, skin: ChameleonSkin, theme: ThemeMode) -> Self {
        self.skin = skin;
        self.border_width = 1.0;
        self.fill_angle_deg = 155.0;
        self.border_angle_deg = 130.0;

        match (skin, theme) {
            (
                ChameleonSkin::Dither
                | ChameleonSkin::Faulty
                | ChameleonSkin::Particles
                | ChameleonSkin::Gridmotion,
                ThemeMode::Light,
            ) => {
                // Light-theme override (`settings.css:2282-2293`):
                // `[data-theme="light"] .bg-settings-panel, body[data-theme="light"][data-welcome-background] .bg-settings-panel`
                // forces `background: rgba(248, 249, 252, 0.94) !important` and `border-color: rgba(0, 0, 0, 0.12) !important`
                // across ALL chameleon skins so Light mode never renders dark text on dark glass.
                let fill = [248.0 / 255.0, 249.0 / 255.0, 252.0 / 255.0, 0.94];
                let border = [0.0, 0.0, 0.0, 0.12];
                self.border_width = 1.0;
                self.fill_color_0 = fill;
                self.fill_color_1 = fill;
                self.border_color_0 = border;
                self.border_color_1 = border;
                self.border_color_2 = border;
                self.shadow_offset = [0.0, 8.0];
                self.shadow_blur = 24.0;
                self.shadow_spread = 0.0;
                self.shadow_color = [0.0, 0.0, 0.0, 0.16];
                self.inset_highlight = [0.0; 4];
            }
            (ChameleonSkin::Dither, ThemeMode::Dark | ThemeMode::Oled) => {
                // settings.css:1388-1398: in Dark/Oled Chromium drops `--welcome-picker-panel-bg` (IACVT)
                // and `--welcome-picker-panel-border: 1px solid transparent`, leaving backdrop blur
                // over a transparent interior and zero shadow (`--welcome-picker-panel-shadow: none`).
                self.border_width = 0.0;
                self.fill_color_0 = [0.0; 4];
                self.fill_color_1 = [0.0; 4];
                self.border_color_0 = [0.0; 4];
                self.border_color_1 = [0.0; 4];
                self.border_color_2 = [0.0; 4];
                self.shadow_offset = [0.0, 0.0];
                self.shadow_blur = 0.0;
                self.shadow_spread = 0.0;
                self.shadow_color = [0.0; 4];
                self.inset_highlight = [0.0; 4];
            }
            (ChameleonSkin::Faulty, ThemeMode::Dark | ThemeMode::Oled) => {
                // settings.css:1400-1411
                self.fill_color_0 = [7.0 / 255.0, 18.0 / 255.0, 11.0 / 255.0, 0.95];
                self.fill_color_1 = [10.0 / 255.0, 26.0 / 255.0, 17.0 / 255.0, 0.86];
                self.border_color_0 = [94.0 / 255.0, 1.0, 153.0 / 255.0, 0.44];
                self.border_color_1 = [190.0 / 255.0, 1.0, 153.0 / 255.0, 0.40];
                self.border_color_2 = [208.0 / 255.0, 1.0, 218.0 / 255.0, 0.22];
                self.shadow_offset = [0.0, 20.0];
                self.shadow_blur = 40.0;
                self.shadow_spread = 0.0;
                self.shadow_color = [4.0 / 255.0, 14.0 / 255.0, 8.0 / 255.0, 0.56];
                self.inset_highlight = [205.0 / 255.0, 1.0, 220.0 / 255.0, 0.16];
            }
            (ChameleonSkin::Particles, ThemeMode::Dark | ThemeMode::Oled) => {
                // settings.css:1429-1439
                self.fill_color_0 = [8.0 / 255.0, 14.0 / 255.0, 31.0 / 255.0, 0.94];
                self.fill_color_1 = [8.0 / 255.0, 18.0 / 255.0, 39.0 / 255.0, 0.82];
                self.border_color_0 = [95.0 / 255.0, 219.0 / 255.0, 1.0, 0.46];
                self.border_color_1 = [114.0 / 255.0, 171.0 / 255.0, 1.0, 0.40];
                self.border_color_2 = [225.0 / 255.0, 244.0 / 255.0, 1.0, 0.24];
                self.shadow_offset = [0.0, 22.0];
                self.shadow_blur = 42.0;
                self.shadow_spread = 0.0;
                self.shadow_color = [2.0 / 255.0, 8.0 / 255.0, 22.0 / 255.0, 0.56];
                self.inset_highlight = [212.0 / 255.0, 240.0 / 255.0, 1.0, 0.12];
            }
            (ChameleonSkin::Gridmotion, ThemeMode::Dark | ThemeMode::Oled) => {
                // settings.css:1449-1459
                self.fill_color_0 = [8.0 / 255.0, 18.0 / 255.0, 20.0 / 255.0, 0.95];
                self.fill_color_1 = [6.0 / 255.0, 12.0 / 255.0, 14.0 / 255.0, 0.85];
                self.border_color_0 = [74.0 / 255.0, 231.0 / 255.0, 210.0 / 255.0, 0.52];
                self.border_color_1 = [0.0, 195.0 / 255.0, 1.0, 0.48];
                self.border_color_2 = [166.0 / 255.0, 202.0 / 255.0, 1.0, 0.24];
                self.shadow_offset = [0.0, 20.0];
                self.shadow_blur = 40.0;
                self.shadow_spread = 0.0;
                self.shadow_color = [4.0 / 255.0, 10.0 / 255.0, 12.0 / 255.0, 0.55];
                self.inset_highlight = [180.0 / 255.0, 235.0 / 255.0, 230.0 / 255.0, 0.15];
            }
            (ChameleonSkin::None, _) => {
                // settings.css:1441-1447
                let heavy = theme.bg_glass_heavy();
                let border = theme.glass_border_color();
                self.fill_color_0 = heavy;
                self.fill_color_1 = heavy;
                self.border_color_0 = border;
                self.border_color_1 = border;
                self.border_color_2 = border;
                self.shadow_offset = [0.0, 4.0];
                self.shadow_blur = 20.0;
                self.shadow_spread = 0.0;
                self.shadow_color = [0.0, 0.0, 0.0, 0.5];
                self.inset_highlight = [0.0; 4];
            }
        }
        self
    }

    /// Adjusts padding-box fills (and translucent borders when present) to account for an
    /// underlying modal backdrop scrim `rgba(0, 0, 0, dim)` composited over the blur pyramid.
    pub fn apply_backdrop_dim(&mut self, dim: f32) -> &mut Self {
        let dim = dim.clamp(0.0, 1.0);
        if dim <= 1e-4 {
            return self;
        }
        let dim_stop = |c: &mut [f32; 4]| {
            let a = c[3].clamp(0.0, 1.0);
            let eff_a = 1.0 - (1.0 - a) * (1.0 - dim);
            if eff_a > 1e-4 {
                let s = a / eff_a;
                c[0] *= s;
                c[1] *= s;
                c[2] *= s;
                c[3] = eff_a;
            }
        };
        dim_stop(&mut self.fill_color_0);
        dim_stop(&mut self.fill_color_1);
        if self.border_width > 0.0 {
            dim_stop(&mut self.border_color_0);
            dim_stop(&mut self.border_color_1);
            dim_stop(&mut self.border_color_2);
        }
        self
    }

    /// Scales pixel-space geometry and shadow/blur dimensions by a DPI factor.
    pub fn scaled(mut self, factor: f32) -> Self {
        if (factor - 1.0).abs() <= 1e-5 {
            return self;
        }
        self.x *= factor;
        self.y *= factor;
        self.width *= factor;
        self.height *= factor;
        self.blur_px *= factor;
        for r in &mut self.corner_radii {
            *r *= factor;
        }
        self.border_width *= factor;
        self.shadow_offset[0] *= factor;
        self.shadow_offset[1] *= factor;
        self.shadow_blur *= factor;
        self.shadow_spread *= factor;
        self.outer_ring_width *= factor;
        if let Some(ref mut cr) = self.clip_rect {
            for v in cr {
                *v *= factor;
            }
        }
        self
    }

    /// Converts to GPU instance struct (`radii` reordered `[TR, BR, TL, BL]` for `sd_rounded_box_4`).
    pub fn to_gpu(&self) -> GlassInstanceGpu {
        let half_w = (self.width * 0.5).max(0.5);
        let half_h = (self.height * 0.5).max(0.5);
        let [tl, tr, br, bl] = self.corner_radii;
        GlassInstanceGpu {
            rect: [self.x + half_w, self.y + half_h, half_w, half_h],
            radii: [tr, br, tl, bl],
            glass_params: [
                self.blur_px.max(0.0),
                self.saturation.max(0.0),
                self.border_width.max(0.0),
                self.skin.shader_id(),
            ],
            fill_color_0: self.fill_color_0,
            fill_color_1: self.fill_color_1,
            border_color_0: self.border_color_0,
            border_color_1: self.border_color_1,
            border_color_2: self.border_color_2,
            shadow_geom: [
                self.shadow_offset[0],
                self.shadow_offset[1],
                self.shadow_blur.max(0.0),
                self.shadow_spread,
            ],
            shadow_color: self.shadow_color,
            inset_highlight: self.inset_highlight,
            extra: [
                self.fill_angle_deg,
                self.border_angle_deg,
                self.outer_ring_width.max(0.0),
                self.opacity.clamp(0.0, 1.0),
            ],
            ring_color: self.outer_ring_color,
            clip_rect: self.clip_rect.unwrap_or([0.0, 0.0, -1.0, -1.0]),
        }
    }
}

/// Positioned & z-ordered SDF Glassmorphism rectangle with optional scissor clip for [`crate::FrameCompositor`].
#[derive(Debug, Clone, PartialEq)]
pub struct SdfGlassRect {
    /// Stacking order matching CSS z-index.
    pub z_index: i32,
    /// Glassmorphism geometry, blur/saturation, border, shadow, and Chameleon skin config.
    pub glass: GlassRect,
    /// Optional scissor clip rectangle `[x, y, width, height]` in target pixel coordinates.
    pub clip_rect: Option<[f32; 4]>,
}

impl SdfGlassRect {
    /// Wraps a [`GlassRect`] with explicit `z_index` and no clip rect.
    pub fn new(z_index: i32, glass: GlassRect) -> Self {
        Self {
            z_index,
            glass,
            clip_rect: None,
        }
    }

    /// Restricts rendering to `[x, y, width, height]`.
    pub fn with_clip_rect(mut self, clip_rect: [f32; 4]) -> Self {
        self.clip_rect = Some(clip_rect);
        self
    }

    /// Scales glass geometry and clip rectangle by DPI factor.
    pub fn scaled(mut self, factor: f32) -> Self {
        if (factor - 1.0).abs() <= 1e-5 {
            return self;
        }
        self.glass = self.glass.scaled(factor);
        if let Some(ref mut cr) = self.clip_rect {
            for v in cr {
                *v *= factor;
            }
        }
        self
    }
}

impl From<GlassRect> for SdfGlassRect {
    fn from(glass: GlassRect) -> Self {
        Self::new(0, glass)
    }
}

/// Pass 2: SDF Glassmorphism & Chameleon compositing pipeline.
pub struct SdfGlassPass {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    globals_buf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    instance_buf: wgpu::Buffer,
    instance_capacity: usize,
}

impl SdfGlassPass {
    const INITIAL_CAPACITY: usize = 64;

    /// Creates the Pass 2 SDF Glass & Chameleon pipeline targeting `target_format`.
    pub fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        scene_view: &wgpu::TextureView,
        blur: &DualKawaseBlur,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sdf-glass-wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/sdf_glass.wgsl").into()),
        });

        let tex_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sdf-glass-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                tex_entry(1),
                tex_entry(2),
                tex_entry(3),
                tex_entry(4),
                tex_entry(5),
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sdf-glass-pl"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GlassInstanceGpu>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 16,
                    shader_location: 1,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 32,
                    shader_location: 2,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 48,
                    shader_location: 3,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 64,
                    shader_location: 4,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 80,
                    shader_location: 5,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 96,
                    shader_location: 6,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 112,
                    shader_location: 7,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 128,
                    shader_location: 8,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 144,
                    shader_location: 9,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 160,
                    shader_location: 10,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 176,
                    shader_location: 11,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 192,
                    shader_location: 12,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 208,
                    shader_location: 13,
                },
            ],
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sdf-glass-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_sdf_glass"),
                buffers: &[Some(instance_layout)],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_sdf_glass"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sdf-glass-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let globals = GlassGlobals {
            viewport_time: [1280.0, 720.0, 0.0, 0.0],
        };
        let globals_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("sdf-glass-globals"),
            contents: bytemuck::bytes_of(&globals),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group = Self::create_bind_group(
            device,
            &bind_group_layout,
            &globals_buf,
            scene_view,
            blur,
            &sampler,
        );

        let instance_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sdf-glass-instances"),
            size: (Self::INITIAL_CAPACITY * std::mem::size_of::<GlassInstanceGpu>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            bind_group_layout,
            sampler,
            globals_buf,
            bind_group,
            instance_buf,
            instance_capacity: Self::INITIAL_CAPACITY,
        }
    }

    fn create_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        globals_buf: &wgpu::Buffer,
        scene_view: &wgpu::TextureView,
        blur: &DualKawaseBlur,
        sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sdf-glass-bg"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(scene_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(blur.blur4_view()),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(blur.blur8_view()),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(blur.blur16_view()),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(blur.blur32_view()),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        })
    }

    /// Refreshes texture bindings after Pass 0 / Pass 1 resize.
    pub fn rebind_textures(
        &mut self,
        device: &wgpu::Device,
        scene_view: &wgpu::TextureView,
        blur: &DualKawaseBlur,
    ) {
        self.bind_group = Self::create_bind_group(
            device,
            &self.bind_group_layout,
            &self.globals_buf,
            scene_view,
            blur,
            &self.sampler,
        );
    }

    pub(crate) fn pipeline(&self) -> &wgpu::RenderPipeline {
        &self.pipeline
    }

    pub(crate) fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }

    pub(crate) fn instance_buffer(&self) -> &wgpu::Buffer {
        &self.instance_buf
    }

    pub(crate) fn upload_instances(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        viewport_w: u32,
        viewport_h: u32,
        time_secs: f32,
        gpu_instances: &[GlassInstanceGpu],
    ) {
        let globals = GlassGlobals {
            viewport_time: [
                viewport_w.max(1) as f32,
                viewport_h.max(1) as f32,
                time_secs,
                0.0,
            ],
        };
        queue.write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&globals));

        if gpu_instances.is_empty() {
            return;
        }

        if gpu_instances.len() > self.instance_capacity {
            self.instance_capacity = gpu_instances.len().next_power_of_two();
            self.instance_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("sdf-glass-instances"),
                size: (self.instance_capacity * std::mem::size_of::<GlassInstanceGpu>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }

        queue.write_buffer(&self.instance_buf, 0, bytemuck::cast_slice(gpu_instances));
    }

    /// Encodes all SDF glass rectangles onto `target_view` (`LoadOp::Load`, preserving background).
    #[allow(clippy::too_many_arguments)]
    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target_view: &wgpu::TextureView,
        viewport_w: u32,
        viewport_h: u32,
        time_secs: f32,
        rects: &[GlassRect],
    ) {
        if rects.is_empty() {
            return;
        }

        let gpu_instances: Vec<GlassInstanceGpu> = rects.iter().map(GlassRect::to_gpu).collect();
        self.upload_instances(
            device,
            queue,
            viewport_w,
            viewport_h,
            time_secs,
            &gpu_instances,
        );

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("sdf-glass-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.instance_buf.slice(..));
        pass.draw(0..6, 0..rects.len() as u32);
    }
}
