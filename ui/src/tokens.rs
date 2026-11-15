//! Design tokens, color palettes, dynamic accent derivation, Chameleon skins,
//! font configuration, light-mode context flips, and CSS z-index stack.

use hybrid_shaders::{ChameleonSkin, ThemeMode, WelcomeBackgroundKind};

/// All CSS stacking layers across `hybrid-player/src/renderer/styles/*.css` (`-100..=10060`).
pub struct ZIndex;

impl ZIndex {
    /// `settings.css:820` — settings background fallback override (`-100 !important`).
    pub const SETTINGS_BG_FALLBACK: i32 = -100;
    /// `main.css:495, 1503` — `.mpv-container`, inactive welcome mounts.
    pub const MPV_CONTAINER: i32 = 0;
    /// `main.css:370, 502, 1517, 1737, 2024`; `settings.css:1421, 1763` — active canvas & shader mounts.
    pub const ACTIVE_CANVAS: i32 = 1;
    /// `main.css:1541`; `settings.css:1465, 1713` — `.welcome-content`, `.bg-settings-body`, `.bg-accordion-preview`.
    pub const WELCOME_CONTENT: i32 = 2;
    /// `settings.css:1847` — `.bg-accordion-check`.
    pub const ACCORDION_CHECK: i32 = 3;
    /// `settings.css:1805` — `.bg-accordion-content`.
    pub const ACCORDION_CONTENT: i32 = 5;
    /// `main.css:400, 1160, 1365, 2591`; `controls.css:525` — curtain, subtitle overlay, chapter markers, carousel nav.
    pub const VIDEO_OVERLAY_BASE: i32 = 10;
    /// `animations.css:555`; `main.css:1258, 1303` — loading spinner, carousel badge/close button.
    pub const LOADING_SPINNER: i32 = 12;
    /// `animations.css:532`; `main.css:1482` — `.center-play-overlay`, `.welcome-screen`.
    pub const WELCOME_SCREEN: i32 = 15;
    /// `main.css:385` — `.network-loading-spinner`.
    pub const NETWORK_LOADING_SPINNER: i32 = 22;
    /// `animations.css:508`; `controls.css:107` — `.seek-indicator`, `.progress-hover-thumb`.
    pub const SEEK_INDICATOR: i32 = 30;
    /// `controls.css:294`; `main.css:2064` — `.quality-dropdown`, `.welcome-bg-select` popup layer.
    pub const QUALITY_DROPDOWN: i32 = 40;
    /// `main.css:2612, 2643` — `.stats-overlay`, `.ab-loop-indicator`.
    pub const STATS_AND_AB_LOOP: i32 = 50;
    /// `main.css:639` — `.recording-indicator` (burst dynamic island pill).
    pub const RECORDING_BURST_PILL: i32 = 54;
    /// `main.css:538` — `#skip-indicator`.
    pub const SKIP_INDICATOR: i32 = 55;
    /// `main.css:1077` — `.unlock-overlay`.
    pub const UNLOCK_OVERLAY: i32 = 56;
    /// `animations.css:432` — `.top-right-volume-osd`.
    pub const TOP_RIGHT_VOLUME_OSD: i32 = 60;
    /// `controls.css:209`; `settings.css:896` — `.controls-bar`, `.custom-select-dropdown`.
    pub const CONTROLS_BAR: i32 = 1000;
    /// `playlist.css:67` — `#btnClosePlaylist`.
    pub const PLAYLIST_CLOSE_BTN: i32 = 1310;
    /// `controls.css:15` — `.controls-wrapper`.
    pub const CONTROLS_WRAPPER: i32 = 1350;
    /// `main.css:172` — `.titlebar`.
    pub const TITLEBAR: i32 = 1400;
    /// `playlist.css:26` — `.sidebar-playlist`.
    pub const PLAYLIST_SIDEBAR: i32 = 1800;
    /// `main.css:2671, 784` — `.modal-overlay`, `.recording-indicator.clip-active` (`!important`).
    pub const MODAL_OVERLAY: i32 = 2000;
    /// `settings.css:1577` — `.bg-accordion-popover` (`!important`).
    pub const BG_ACCORDION_POPOVER: i32 = 2100;
    /// `main.css:1110` — `.screenshot-preview` (standard preview).
    pub const SCREENSHOT_PREVIEW: i32 = 2200;
    /// `main.css:864` — `.recording-stop-btn` (`!important`, above titlebar `1400` & clip bezel `2000`).
    pub const RECORDING_STOP_BTN: i32 = 2501;
    /// `animations.css:183` — `.toast-container`.
    pub const TOAST_CONTAINER: i32 = 3000;
    /// `main.css:1215` — `.screenshot-preview.carousel-mode`.
    pub const SCREENSHOT_CAROUSEL: i32 = 10005;
    /// `main.css:1422` — `.screenshot-notice`.
    pub const SCREENSHOT_NOTICE: i32 = 10015;
    /// `main.css:2898` — `.drop-overlay`.
    pub const DROP_OVERLAY: i32 = 10060;
}

/// Shared `:root` layout & brand constants (`themes.css:54-70`).
pub struct RootTokens;

impl RootTokens {
    pub const BRAND_LOGO_SIZE_SM: f32 = 20.0;
    pub const BRAND_LOGO_SIZE_MD: f32 = 34.0;
    pub const BRAND_LOGO_SIZE_LG: f32 = 64.0;
    pub const BRAND_CORNER_RADIUS: f32 = 14.0;
    pub const BRAND_GLOW_STRENGTH: f32 = 0.28;

    pub const RADIUS_SM: f32 = 6.0;
    pub const RADIUS_MD: f32 = 10.0;
    pub const RADIUS_LG: f32 = 16.0;
    pub const RADIUS_XL: f32 = 20.0;
    pub const RADIUS_FULL: f32 = 9999.0;
}

/// Font family resolution honoring `body.brand-font-disabled` (`themes.css:6-57, 256-259`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontConfig {
    pub brand_font_enabled: bool,
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            brand_font_enabled: true,
        }
    }
}

impl FontConfig {
    pub fn display_family(self) -> &'static str {
        if self.brand_font_enabled {
            "Sora Local"
        } else {
            "Segoe UI"
        }
    }

    pub fn ui_family(self) -> &'static str {
        if self.brand_font_enabled {
            "IBM Plex Sans Local"
        } else {
            "Segoe UI"
        }
    }

    pub fn mono_family(self) -> &'static str {
        "Consolas"
    }
}

/// Linear gradient specification used by `--bg-controls`, `--bg-titlebar`, and Chameleon panels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinearGradientSpec {
    pub angle_deg: f32,
    pub start: [f32; 4],
    pub mid: Option<([f32; 4], f32)>,
    pub end: [f32; 4],
}

/// Resolved RGBA tokens matching `[data-theme="dark" | "oled" | "light"]` (`themes.css:73-254`).
#[derive(Debug, Clone, PartialEq)]
pub struct ThemeTokens {
    pub mode: ThemeMode,
    pub bg_primary: [f32; 4],
    pub bg_secondary: [f32; 4],
    pub bg_tertiary: [f32; 4],
    pub bg_elevated: [f32; 4],
    pub bg_glass: [f32; 4],
    pub bg_glass_heavy: [f32; 4],
    pub bg_controls: LinearGradientSpec,
    pub bg_titlebar: LinearGradientSpec,
    pub text_primary: [f32; 4],
    pub text_secondary: [f32; 4],
    pub text_tertiary: [f32; 4],
    pub border_color: [f32; 4],
    pub border_focus: [f32; 4],
    pub accent: [f32; 4],
    pub accent_rgb: [u8; 3],
    pub accent_hover: [f32; 4],
    pub accent_dim: [f32; 4],
    pub progress_bg: [f32; 4],
    pub progress_buffer: [f32; 4],
    pub scrollbar_thumb: [f32; 4],
    pub scrollbar_thumb_hover: [f32; 4],
    pub glass_blur_px: f32,
    pub glass_border: [f32; 4],
    pub brand_glass_opacity: f32,
    pub brand_text_high: [f32; 4],
    pub brand_text_mid: [f32; 4],
    pub brand_card_bg: [f32; 4],
    pub brand_card_top: [f32; 4],
    // `--ytdlp-*` status colors (`themes.css:62-66, 175-178`)
    pub ytdlp_info: [f32; 4],
    pub ytdlp_ok: [f32; 4],
    pub ytdlp_pending: [f32; 4],
    pub ytdlp_error: [f32; 4],
    // Always-dark Top-Right Volume OSD (`themes.css:227-232`, `animations.css:428-475`)
    pub volume_osd_bg: [f32; 4],
    pub volume_osd_border: [f32; 4],
    pub volume_osd_fg: [f32; 4],
    pub volume_osd_pct: [f32; 4],
}

impl ThemeTokens {
    pub fn for_mode(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Dark => Self {
                mode,
                bg_primary: rgba8(0x0a, 0x0a, 0x0f, 255),
                bg_secondary: rgba8(0x12, 0x12, 0x1a, 255),
                bg_tertiary: rgba8(0x1a, 0x1a, 0x25, 255),
                bg_elevated: rgba8(0x22, 0x22, 0x2f, 255),
                bg_glass: rgba(18, 18, 26, 0.85),
                bg_glass_heavy: rgba(10, 10, 15, 0.95),
                bg_controls: LinearGradientSpec {
                    angle_deg: 0.0,
                    start: rgba(0, 0, 0, 0.95),
                    mid: Some((rgba(0, 0, 0, 0.7), 0.5)),
                    end: rgba(0, 0, 0, 0.0),
                },
                bg_titlebar: LinearGradientSpec {
                    angle_deg: 180.0,
                    start: rgba(0, 0, 0, 0.8),
                    mid: None,
                    end: rgba(0, 0, 0, 0.0),
                },
                text_primary: rgba8(0xf0, 0xf0, 0xf5, 255),
                text_secondary: rgba8(0xb8, 0xb8, 0xcc, 255),
                text_tertiary: rgba8(0x8a, 0x8a, 0xa0, 255),
                border_color: rgba(255, 255, 255, 0.08),
                border_focus: rgba(255, 255, 255, 0.20),
                accent: rgba8(0xe5, 0x09, 0x14, 255),
                accent_rgb: [0xe5, 0x09, 0x14],
                accent_hover: rgba8(0xff, 0x1a, 0x26, 255),
                accent_dim: rgba(229, 9, 20, 0.30),
                progress_bg: rgba(255, 255, 255, 0.15),
                progress_buffer: rgba(255, 255, 255, 0.25),
                scrollbar_thumb: rgba(255, 255, 255, 0.15),
                scrollbar_thumb_hover: rgba(255, 255, 255, 0.30),
                glass_blur_px: 20.0,
                glass_border: rgba(255, 255, 255, 0.06),
                brand_glass_opacity: 0.62,
                brand_text_high: rgba8(0xe8, 0xed, 0xf6, 255),
                brand_text_mid: rgba8(0xb8, 0xc3, 0xd6, 255),
                brand_card_bg: rgba8(0x11, 0x19, 0x23, 255),
                brand_card_top: rgba8(0x1f, 0x2c, 0x3d, 255),
                ytdlp_info: rgba8(0x00, 0xf2, 0xfe, 255),
                ytdlp_ok: rgba8(0x4a, 0xde, 0x80, 255),
                ytdlp_pending: rgba8(0xfa, 0xcc, 0x15, 255),
                ytdlp_error: rgba8(0xef, 0x44, 0x44, 255),
                volume_osd_bg: rgba(0, 0, 0, 0.72),
                volume_osd_border: rgba(255, 255, 255, 0.14),
                volume_osd_fg: [1.0, 1.0, 1.0, 1.0],
                volume_osd_pct: rgba8(0x38, 0xbd, 0xf8, 255),
            },
            ThemeMode::Oled => Self {
                mode,
                bg_primary: rgba8(0x00, 0x00, 0x00, 255),
                bg_secondary: rgba8(0x05, 0x05, 0x05, 255),
                bg_tertiary: rgba8(0x0a, 0x0a, 0x0a, 255),
                bg_elevated: rgba8(0x11, 0x11, 0x11, 255),
                bg_glass: rgba(5, 5, 5, 0.90),
                bg_glass_heavy: rgba(0, 0, 0, 0.98),
                bg_controls: LinearGradientSpec {
                    angle_deg: 0.0,
                    start: rgba(0, 0, 0, 1.0),
                    mid: Some((rgba(0, 0, 0, 0.8), 0.5)),
                    end: rgba(0, 0, 0, 0.0),
                },
                bg_titlebar: LinearGradientSpec {
                    angle_deg: 180.0,
                    start: rgba(0, 0, 0, 0.9),
                    mid: None,
                    end: rgba(0, 0, 0, 0.0),
                },
                text_primary: rgba8(0xe8, 0xe8, 0xed, 255),
                text_secondary: rgba8(0xa8, 0xa8, 0xb2, 255),
                text_tertiary: rgba8(0x7a, 0x7a, 0x82, 255),
                border_color: rgba(255, 255, 255, 0.12),
                border_focus: rgba(255, 255, 255, 0.15),
                accent: rgba8(0xe5, 0x09, 0x14, 255),
                accent_rgb: [0xe5, 0x09, 0x14],
                accent_hover: rgba8(0xff, 0x1a, 0x26, 255),
                accent_dim: rgba(229, 9, 20, 0.25),
                progress_bg: rgba(255, 255, 255, 0.10),
                progress_buffer: rgba(255, 255, 255, 0.20),
                scrollbar_thumb: rgba(255, 255, 255, 0.10),
                scrollbar_thumb_hover: rgba(255, 255, 255, 0.20),
                glass_blur_px: 25.0,
                glass_border: rgba(255, 255, 255, 0.04),
                brand_glass_opacity: 0.56,
                brand_text_high: rgba8(0xe8, 0xed, 0xf6, 255),
                brand_text_mid: rgba8(0xa4, 0xb1, 0xc7, 255),
                brand_card_bg: rgba8(0x0d, 0x14, 0x1d, 255),
                brand_card_top: rgba8(0x18, 0x22, 0x31, 255),
                ytdlp_info: rgba8(0x00, 0xf2, 0xfe, 255),
                ytdlp_ok: rgba8(0x4a, 0xde, 0x80, 255),
                ytdlp_pending: rgba8(0xfa, 0xcc, 0x15, 255),
                ytdlp_error: rgba8(0xef, 0x44, 0x44, 255),
                volume_osd_bg: rgba(0, 0, 0, 0.72),
                volume_osd_border: rgba(255, 255, 255, 0.14),
                volume_osd_fg: [1.0, 1.0, 1.0, 1.0],
                volume_osd_pct: rgba8(0x38, 0xbd, 0xf8, 255),
            },
            ThemeMode::Light => Self {
                mode,
                bg_primary: rgba8(0xf5, 0xf5, 0xfa, 255),
                bg_secondary: rgba8(0xff, 0xff, 0xff, 255),
                bg_tertiary: rgba8(0xea, 0xea, 0xf0, 255),
                bg_elevated: rgba8(0xff, 0xff, 0xff, 255),
                bg_glass: rgba(255, 255, 255, 0.85),
                bg_glass_heavy: rgba(245, 245, 250, 0.95),
                bg_controls: LinearGradientSpec {
                    angle_deg: 0.0,
                    start: rgba(0, 0, 0, 0.85),
                    mid: Some((rgba(0, 0, 0, 0.5), 0.5)),
                    end: rgba(0, 0, 0, 0.0),
                },
                bg_titlebar: LinearGradientSpec {
                    angle_deg: 180.0,
                    start: rgba(255, 255, 255, 0.9),
                    mid: None,
                    end: rgba(255, 255, 255, 0.0),
                },
                text_primary: rgba8(0x1a, 0x1a, 0x2e, 255),
                text_secondary: rgba8(0x4e, 0x4e, 0x64, 255),
                text_tertiary: rgba8(0x6e, 0x6e, 0x86, 255),
                border_color: rgba(0, 0, 0, 0.08),
                border_focus: rgba(0, 0, 0, 0.20),
                accent: rgba8(0xe5, 0x09, 0x14, 255),
                accent_rgb: [0xe5, 0x09, 0x14],
                accent_hover: rgba8(0xcc, 0x08, 0x12, 255),
                accent_dim: rgba(229, 9, 20, 0.15),
                progress_bg: rgba(255, 255, 255, 0.25),
                progress_buffer: rgba(255, 255, 255, 0.40),
                scrollbar_thumb: rgba(0, 0, 0, 0.15),
                scrollbar_thumb_hover: rgba(0, 0, 0, 0.30),
                glass_blur_px: 20.0,
                glass_border: rgba(0, 0, 0, 0.06),
                brand_glass_opacity: 0.75,
                brand_text_high: rgba8(0x1d, 0x24, 0x30, 255),
                brand_text_mid: rgba8(0x42, 0x54, 0x70, 255),
                brand_card_bg: rgba8(0xf3, 0xf7, 0xfd, 255),
                brand_card_top: rgba8(0xdd, 0xe7, 0xf7, 255),
                ytdlp_info: rgba8(0x03, 0x69, 0xa1, 255),
                ytdlp_ok: rgba8(0x15, 0x80, 0x3d, 255),
                ytdlp_pending: rgba8(0x85, 0x4d, 0x0e, 255),
                ytdlp_error: rgba8(0xb9, 0x1c, 0x1c, 255),
                // Light-mode Volume OSD stays always-dark (`themes.css:227-232`)
                volume_osd_bg: rgba(15, 23, 42, 0.85),
                volume_osd_border: rgba(255, 255, 255, 0.22),
                volume_osd_fg: [1.0, 1.0, 1.0, 1.0],
                volume_osd_pct: rgba8(0x38, 0xbd, 0xf8, 255),
            },
        }
    }

    /// Applies runtime custom `--accent`, `--accent-hover` (`_lighten(hex, 15)`),
    /// and `--accent-dim` (`${hex}4D`, 77/255 alpha) matching `settings.js:615-617, 1734-1736, 2780-2786`.
    pub fn with_accent_hex(mut self, hex: &str) -> Self {
        if let Some([r, g, b]) = parse_hex_rgb_u8(hex) {
            self.accent_rgb = [r, g, b];
            self.accent = rgba8(r, g, b, 255);
            let [hr, hg, hb] = lighten_rgb(r, g, b, 15);
            self.accent_hover = rgba8(hr, hg, hb, 255);
            self.accent_dim = rgba8(r, g, b, 0x4D);
        }
        self
    }

    /// Resolves titlebar foreground color accounting for `themes.css:211-225`:
    /// In light mode over video playback, titlebar text/icons remain white for contrast;
    /// when `welcome_active || modal_open`, they flip to `text_primary` / `text_secondary`.
    pub fn titlebar_colors(&self, welcome_active: bool, modal_open: bool) -> TitlebarContextColors {
        match self.mode {
            ThemeMode::Light if !welcome_active && !modal_open => TitlebarContextColors {
                title_fg: [1.0, 1.0, 1.0, 1.0],
                btn_fg: [1.0, 1.0, 1.0, 0.9],
                has_text_shadow: true,
            },
            ThemeMode::Light => TitlebarContextColors {
                title_fg: self.text_primary,
                btn_fg: self.text_secondary,
                has_text_shadow: false,
            },
            ThemeMode::Dark | ThemeMode::Oled => TitlebarContextColors {
                title_fg: [1.0, 1.0, 1.0, 0.9],
                btn_fg: [1.0, 1.0, 1.0, 1.0],
                has_text_shadow: true,
            },
        }
    }

    /// Toast surface colors (`animations.css:187-211`).
    pub fn toast_colors(
        &self,
    ) -> (
        /*bg*/ [f32; 4],
        /*border*/ [f32; 4],
        /*fg*/ [f32; 4],
    ) {
        match self.mode {
            ThemeMode::Light => (
                rgba(255, 255, 255, 0.98),
                rgba(20, 24, 36, 0.12),
                rgba8(0x1a, 0x1a, 0x2e, 255),
            ),
            ThemeMode::Dark | ThemeMode::Oled => {
                (self.bg_glass_heavy, self.border_color, self.text_primary)
            }
        }
    }

    /// `yt-dlp` capsule toast colors (`animations.css:217-424`).
    pub fn update_pill_colors(
        &self,
    ) -> (
        /*bg*/ [f32; 4],
        /*border*/ [f32; 4],
        /*fg*/ [f32; 4],
    ) {
        match self.mode {
            ThemeMode::Light => (
                rgba(255, 255, 255, 0.94),
                rgba(15, 23, 42, 0.10),
                rgba8(0x0f, 0x17, 0x2a, 255),
            ),
            ThemeMode::Oled => (
                [0.0, 0.0, 0.0, 1.0],
                rgba(255, 255, 255, 0.16),
                rgba(255, 255, 255, 0.92),
            ),
            ThemeMode::Dark => (
                rgba(15, 23, 42, 0.90),
                rgba(255, 255, 255, 0.16),
                rgba(255, 255, 255, 0.92),
            ),
        }
    }

    /// Light/dark `.bg-settings-panel` & `.bg-accordion-popover` override (`settings.css:2282-2293`).
    pub fn bg_panel_light_override(&self) -> Option<([f32; 4], [f32; 4], [f32; 4])> {
        if self.mode == ThemeMode::Light {
            Some((
                rgba(248, 249, 252, 0.94),
                rgba(0, 0, 0, 0.12),
                rgba8(0x11, 0x13, 0x1a, 255),
            ))
        } else {
            None
        }
    }

    /// Light/dark modal close button tokens (`themes.css:247-254`).
    pub fn modal_close_colors(
        &self,
    ) -> (
        /*bg*/ [f32; 4],
        /*hover_bg*/ [f32; 4],
        /*border*/ [f32; 4],
    ) {
        match self.mode {
            ThemeMode::Light => (
                rgba(0, 0, 0, 0.04),
                rgba(0, 0, 0, 0.08),
                rgba(0, 0, 0, 0.18),
            ),
            ThemeMode::Dark | ThemeMode::Oled => (
                rgba(255, 255, 255, 0.06),
                rgba(255, 255, 255, 0.14),
                self.border_color,
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TitlebarContextColors {
    pub title_fg: [f32; 4],
    pub btn_fg: [f32; 4],
    pub has_text_shadow: bool,
}

/// Chameleon skin tokens derived from `body[data-welcome-background]` (`settings.css:1388-1459`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChameleonTokens {
    pub skin: ChameleonSkin,
    pub accent: [f32; 4],
    pub fg_dim: [f32; 4],
    pub uses_monospace_body: bool,
    pub has_sweep_overlay: bool,
    pub panel_blur_px: Option<f32>,
    pub panel_saturation: f32,
}

impl ChameleonTokens {
    pub fn resolve(
        bg: WelcomeBackgroundKind,
        theme: &ThemeTokens,
        quality_low: bool,
        quality_medium: bool,
    ) -> Self {
        let skin = ChameleonSkin::from(bg);
        let default_glass = theme.glass_blur_px;
        let (picker_blur, picker_sat) = match (bg, quality_low, quality_medium) {
            (WelcomeBackgroundKind::Dither, _, _) => (None, 1.0),
            (_, true, _) => (None, 1.0),
            (WelcomeBackgroundKind::Faulty, _, _) => (Some(4.0), 1.1),
            (WelcomeBackgroundKind::Particles, _, _) => (Some(20.0), 1.5),
            (WelcomeBackgroundKind::None, _, _) => (Some(10.0), 1.0),
            (_, false, true) => (Some(10.0), 1.25),
            (WelcomeBackgroundKind::Gridmotion, false, false) => (Some(default_glass), 1.0),
        };

        let (accent, mut fg_dim, uses_monospace_body, mut has_sweep_overlay) = match skin {
            ChameleonSkin::Dither => (
                rgba8(0xb8, 0xc4, 0xff, 255),
                rgba(232, 238, 255, 0.6),
                false,
                false,
            ),
            ChameleonSkin::Faulty => (
                rgba8(0xa7, 0xef, 0x9e, 255),
                rgba(167, 239, 158, 0.6),
                true,
                true,
            ),
            ChameleonSkin::Particles => (
                rgba8(0x7e, 0xc8, 0xff, 255),
                rgba(190, 220, 255, 0.62),
                false,
                false,
            ),
            ChameleonSkin::Gridmotion => (
                rgba8(0x4a, 0xe7, 0xd2, 255),
                rgba(180, 235, 230, 0.62),
                false,
                false,
            ),
            ChameleonSkin::None => (theme.accent, theme.text_secondary, false, false),
        };
        if theme.mode == ThemeMode::Light {
            // `[data-theme="light"] .bg-settings-panel` (`settings.css:2285-2289`) forces dark
            // readable secondary ink (`#4a4f62`) across all chameleon skins.
            fg_dim = rgba8(0x4a, 0x4f, 0x62, 255);
            has_sweep_overlay = false;
        }

        Self {
            skin,
            accent,
            fg_dim,
            uses_monospace_body,
            has_sweep_overlay,
            panel_blur_px: picker_blur,
            panel_saturation: picker_sat,
        }
    }
}

/// Exact `_lighten(hex, percent)` algorithm from `hybrid-player/src/renderer/modules/settings.js:2780-2787`.
pub fn lighten_rgb(r: u8, g: u8, b: u8, percent: i32) -> [u8; 3] {
    let amt = (2.55_f64 * (percent as f64)).round() as i32;
    [
        (r as i32 + amt).clamp(0, 255) as u8,
        (g as i32 + amt).clamp(0, 255) as u8,
        (b as i32 + amt).clamp(0, 255) as u8,
    ]
}

pub fn parse_hex_rgb_u8(hex: &str) -> Option<[u8; 3]> {
    let s = hex.trim().strip_prefix('#').unwrap_or(hex.trim());
    if s.len() != 6 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some([r, g, b])
}

#[inline]
pub const fn rgba8(r: u8, g: u8, b: u8, a: u8) -> [f32; 4] {
    [
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        a as f32 / 255.0,
    ]
}

#[inline]
pub const fn rgba(r: u8, g: u8, b: u8, a: f32) -> [f32; 4] {
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a]
}
