use serde::{Deserialize, Serialize};

/// Quality tier controlling shader complexity and DPR caps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum QualityTier {
    Low,
    #[default]
    Medium,
    High,
    Custom,
}

impl QualityTier {
    /// Returns the DPR cap matching `ditherWaves.js` / `faultyTerminal.js` (`<= 0.8`, `<= 1.0`, `<= 1.5`).
    pub fn cap_dpr(self, device_dpr: f32) -> f32 {
        let base = if device_dpr.is_finite() && device_dpr > 0.0 {
            device_dpr
        } else {
            1.0
        };
        match self {
            Self::Low => base.min(0.8),
            Self::Medium => base.min(1.0),
            Self::High | Self::Custom => base.min(1.5),
        }
    }

    /// Returns the DPR cap for Particles (`particles.js:129` uses `<= 1.4` for High/Custom).
    pub fn cap_particles_dpr(self, device_dpr: f32) -> f32 {
        let base = if device_dpr.is_finite() && device_dpr > 0.0 {
            device_dpr
        } else {
            1.0
        };
        match self {
            Self::Low => base.min(0.8),
            Self::Medium => base.min(1.0),
            Self::High | Self::Custom => base.min(1.4),
        }
    }
}

/// Active Welcome Screen background selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum WelcomeBackgroundKind {
    None,
    #[default]
    Dither,
    Particles,
    Faulty,
    Gridmotion,
}

/// UI color theme used to resolve glass background/border tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    #[default]
    Dark,
    Oled,
    Light,
}

impl ThemeMode {
    /// Returns `--bg-glass-heavy` as `[r, g, b, a]` in `0.0..=1.0`.
    pub fn bg_glass_heavy(self) -> [f32; 4] {
        match self {
            Self::Dark => [10.0 / 255.0, 10.0 / 255.0, 15.0 / 255.0, 0.95],
            Self::Oled => [0.0, 0.0, 0.0, 0.98],
            Self::Light => [245.0 / 255.0, 245.0 / 255.0, 250.0 / 255.0, 0.95],
        }
    }

    /// Returns `--bg-glass` as `[r, g, b, a]` in `0.0..=1.0`.
    pub fn bg_glass(self) -> [f32; 4] {
        match self {
            Self::Dark => [18.0 / 255.0, 18.0 / 255.0, 26.0 / 255.0, 0.85],
            Self::Oled => [5.0 / 255.0, 5.0 / 255.0, 5.0 / 255.0, 0.90],
            Self::Light => [1.0, 1.0, 1.0, 0.85],
        }
    }

    /// Returns `--glass-blur` radius in pixels (`20.0` for dark/light, `25.0` for oled).
    pub fn glass_blur_px(self) -> f32 {
        match self {
            Self::Dark | Self::Light => 20.0,
            Self::Oled => 25.0,
        }
    }

    /// Returns `--glass-border` color as `[r, g, b, a]`.
    pub fn glass_border_color(self) -> [f32; 4] {
        match self {
            Self::Dark => [1.0, 1.0, 1.0, 0.06],
            Self::Oled => [1.0, 1.0, 1.0, 0.04],
            Self::Light => [0.0, 0.0, 0.0, 0.06],
        }
    }
}

/// State flags that determine Pass 0 (`SceneTex`) clear color (`main.css:98,107-111`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ClearContext {
    pub welcome_active: bool,
    pub app_cold_start: bool,
    pub max_transition: bool,
}

impl ClearContext {
    /// Returns `#000000` when `welcome_active || app_cold_start || max_transition`, else `#0c0d12`.
    pub fn clear_color(self) -> wgpu::Color {
        if self.welcome_active || self.app_cold_start || self.max_transition {
            wgpu::Color::BLACK
        } else {
            wgpu::Color {
                r: 12.0 / 255.0,
                g: 13.0 / 255.0,
                b: 18.0 / 255.0,
                a: 1.0,
            }
        }
    }

    /// Returns exact `[u8; 4]` RGBA bytes for CPU-side assertions.
    pub fn clear_rgba8(self) -> [u8; 4] {
        if self.welcome_active || self.app_cold_start || self.max_transition {
            [0, 0, 0, 255]
        } else {
            [12, 13, 18, 255]
        }
    }
}

/// Sizing and frame-pacing configuration for either full-bleed or `400x240 @ 30fps` accordion preview.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectInstanceConfig {
    /// Logical width in CSS pixels (`400` for accordion previews).
    pub width: u32,
    /// Logical height in CSS pixels (`240` for accordion previews).
    pub height: u32,
    /// Effective device pixel ratio (`1.0` for accordion previews).
    pub dpr: f32,
    /// Optional FPS cap (`Some(30.0)` for accordion previews, `None` for uncapped/vsync).
    pub fps_limit: Option<f32>,
    /// True when mounted inside `.bg-accordion-live-mount`.
    pub is_preview: bool,
    /// Auto-pause when window loses focus or visibility.
    pub pause_when_unfocused: bool,
}

impl Default for EffectInstanceConfig {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
            dpr: 1.0,
            fps_limit: None,
            is_preview: false,
            pause_when_unfocused: true,
        }
    }
}

impl EffectInstanceConfig {
    /// Standard `400x240 @ 30fps` accordion preview instance configuration (`settings.js:2487-2532`).
    pub fn preview_400x240() -> Self {
        Self {
            width: 400,
            height: 240,
            dpr: 1.0,
            fps_limit: Some(30.0),
            is_preview: true,
            pause_when_unfocused: true,
        }
    }

    /// Physical pixel dimensions `(floor(width * dpr), floor(height * dpr))`, clamped >= 1.
    pub fn pixel_size(&self) -> (u32, u32) {
        let dpr = if self.dpr.is_finite() && self.dpr > 0.0 {
            self.dpr
        } else {
            1.0
        };
        let pw = ((self.width as f32) * dpr).floor().max(1.0) as u32;
        let ph = ((self.height as f32) * dpr).floor().max(1.0) as u32;
        (pw, ph)
    }
}

/// Lifecycle state tracking auto-pause when unfocused, hidden, playing video, or motion-paused.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectLifecycle {
    pub window_focused: bool,
    pub window_visible: bool,
    pub video_playing: bool,
    pub motion_paused: bool,
    pub pause_when_unfocused: bool,
    pub fps_limit: Option<f32>,
    last_render_secs: Option<f64>,
}

impl Default for EffectLifecycle {
    fn default() -> Self {
        Self {
            window_focused: true,
            window_visible: true,
            video_playing: false,
            motion_paused: false,
            pause_when_unfocused: true,
            fps_limit: None,
            last_render_secs: None,
        }
    }
}

impl EffectLifecycle {
    /// Creates a lifecycle state from an `EffectInstanceConfig`.
    pub fn from_instance(cfg: &EffectInstanceConfig) -> Self {
        Self {
            pause_when_unfocused: cfg.pause_when_unfocused,
            fps_limit: cfg.fps_limit,
            ..Default::default()
        }
    }

    /// Returns true when the effect is paused by visibility, focus, playback, or motion-pause state.
    pub fn is_paused(&self) -> bool {
        if !self.window_visible || self.video_playing || self.motion_paused {
            return true;
        }
        if self.pause_when_unfocused && !self.window_focused {
            return true;
        }
        false
    }

    /// Checks whether a frame should be rendered at `now_secs`, enforcing `fps_limit` if set.
    pub fn should_render(&mut self, now_secs: f64) -> bool {
        if self.is_paused() {
            return false;
        }
        if let Some(fps) = self.fps_limit {
            if fps > 0.0 {
                let min_dt = 1.0 / (fps as f64);
                if let Some(prev) = self.last_render_secs {
                    if now_secs - prev + 1e-6 < min_dt {
                        return false;
                    }
                }
            }
        }
        self.last_render_secs = Some(now_secs);
        true
    }
}

/// Configuration for `dither_waves.wgsl` (`ditherWaves.js:131-149`, `settings.js:23`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DitherWavesConfig {
    pub wave_speed: f32,
    pub wave_frequency: f32,
    pub wave_amplitude: f32,
    pub wave_color: [f32; 3],
    pub bg_color: [f32; 3],
    pub color_num: f32,
    pub pixel_size: f32,
    pub enable_mouse_interaction: bool,
    pub mouse_radius: f32,
    pub mouse_intensity: f32,
    /// Normalized or pixel mouse position `(x_px, y_px)` with top-left origin.
    pub mouse_pos_px: [f32; 2],
}

impl Default for DitherWavesConfig {
    fn default() -> Self {
        Self {
            wave_speed: 0.05,
            wave_frequency: 3.0,
            wave_amplitude: 0.3,
            wave_color: [0.5, 0.5, 0.5],
            bg_color: [0.0, 0.0, 0.0],
            color_num: 4.0,
            pixel_size: 2.0,
            enable_mouse_interaction: true,
            mouse_radius: 0.22,
            mouse_intensity: 0.35,
            mouse_pos_px: [0.0, 0.0],
        }
    }
}

impl DitherWavesConfig {
    /// Quality preset matching `BG_QUALITY_PRESETS` in `settings.js:32-44`.
    pub fn for_quality(tier: QualityTier) -> Self {
        match tier {
            QualityTier::Low => Self {
                wave_frequency: 2.55,
                wave_amplitude: 0.26,
                pixel_size: 4.0,
                ..Default::default()
            },
            QualityTier::Medium => Self {
                wave_frequency: 2.85,
                wave_amplitude: 0.29,
                pixel_size: 3.0,
                ..Default::default()
            },
            QualityTier::High | QualityTier::Custom => Self {
                pixel_size: 1.0,
                ..Default::default()
            },
        }
    }

    /// Accordion `400x240 @ 30fps` preview preset (`settings.js:2487`).
    pub fn preview_preset() -> Self {
        Self {
            wave_speed: 0.1,
            wave_frequency: 2.2,
            wave_amplitude: 0.35,
            pixel_size: 2.0,
            enable_mouse_interaction: false,
            ..Default::default()
        }
    }
}

/// Configuration for `particles.wgsl` (`particles.js:92-107`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParticlesConfig {
    pub particle_count: u32,
    pub particle_spread: f32,
    pub speed: f32,
    pub particle_colors: Vec<[f32; 3]>,
    pub move_particles_on_hover: bool,
    pub particle_hover_factor: f32,
    pub alpha_particles: bool,
    pub particle_base_size: f32,
    pub size_randomness: f32,
    pub camera_distance: f32,
    pub fov_deg: f32,
    pub disable_rotation: bool,
    /// Normalized pointer position in `[-1.0, 1.0]` (`x` right, `y` up).
    pub mouse_ndc: [f32; 2],
    /// Deterministic RNG seed for reproducible particle placement.
    pub seed: u64,
}

impl Default for ParticlesConfig {
    fn default() -> Self {
        Self {
            particle_count: 300,
            particle_spread: 10.0,
            speed: 0.1,
            particle_colors: vec![[1.0, 1.0, 1.0]],
            move_particles_on_hover: true,
            particle_hover_factor: 1.0,
            alpha_particles: false,
            particle_base_size: 100.0,
            size_randomness: 1.0,
            camera_distance: 20.0,
            fov_deg: 15.0,
            disable_rotation: false,
            mouse_ndc: [0.0, 0.0],
            seed: 0x5041_5254_4943_4C45,
        }
    }
}

impl ParticlesConfig {
    /// Quality preset matching `BG_QUALITY_PRESETS` in `settings.js:33-45`.
    pub fn for_quality(tier: QualityTier) -> Self {
        match tier {
            QualityTier::Low => Self {
                particle_count: 135,
                speed: 0.09,
                particle_spread: 9.0,
                particle_base_size: 85.0,
                ..Default::default()
            },
            QualityTier::Medium => Self {
                particle_count: 210,
                speed: 0.1,
                particle_spread: 10.0,
                particle_base_size: 95.0,
                ..Default::default()
            },
            QualityTier::High | QualityTier::Custom => Self::default(),
        }
    }

    /// Accordion `400x240 @ 30fps` preview preset (`settings.js:2491`).
    pub fn preview_preset() -> Self {
        Self {
            particle_base_size: 32.0,
            particle_count: 45,
            particle_spread: 7.0,
            speed: 0.4,
            camera_distance: 13.0,
            move_particles_on_hover: false,
            ..Default::default()
        }
    }
}

/// Configuration for `faulty_terminal.wgsl` (`faultyTerminal.js:233-254`, `settings.js:25`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FaultyTerminalConfig {
    pub scale: f32,
    pub grid_mul: [f32; 2],
    pub digit_size: f32,
    pub time_scale: f32,
    pub pause: bool,
    pub scanline_intensity: f32,
    pub glitch_amount: f32,
    pub flicker_amount: f32,
    pub noise_amp: f32,
    pub chromatic_aberration: f32,
    pub dither: f32,
    pub curvature: f32,
    pub tint: [f32; 3],
    pub mouse_react: bool,
    pub mouse_strength: f32,
    /// Normalized bottom-up mouse position `[0..1, 0..1]`.
    pub mouse_uv: [f32; 2],
    pub page_load_animation: bool,
    /// Progress `0.0..=1.0` over the 2000ms load fade-in (`faultyTerminal.js:640-643`).
    pub page_load_progress: f32,
    pub brightness: f32,
    pub time_offset: f32,
}

impl Default for FaultyTerminalConfig {
    fn default() -> Self {
        Self {
            scale: 1.2,
            grid_mul: [2.0, 1.0],
            digit_size: 1.5,
            time_scale: 0.8,
            pause: false,
            scanline_intensity: 0.3,
            glitch_amount: 1.0,
            flicker_amount: 1.0,
            noise_amp: 1.0,
            chromatic_aberration: 0.0,
            dither: 0.0,
            curvature: 0.2,
            tint: parse_hex_rgb("#A7EF9E").unwrap_or([0.655, 0.937, 0.620]),
            mouse_react: true,
            mouse_strength: 0.2,
            mouse_uv: [0.5, 0.5],
            page_load_animation: true,
            page_load_progress: 0.0,
            brightness: 1.0,
            time_offset: 0.0,
        }
    }
}

impl FaultyTerminalConfig {
    /// Quality preset matching `BG_QUALITY_PRESETS` in `settings.js:34-46`.
    pub fn for_quality(tier: QualityTier) -> Self {
        match tier {
            QualityTier::Low => Self {
                scale: 0.8,
                glitch_amount: 0.65,
                scanline_intensity: 0.46,
                flicker_amount: 0.65,
                curvature: 0.16,
                brightness: 0.8,
                ..Default::default()
            },
            QualityTier::Medium => Self {
                scale: 1.2,
                glitch_amount: 0.85,
                scanline_intensity: 0.6,
                flicker_amount: 0.85,
                curvature: 0.18,
                brightness: 0.8,
                ..Default::default()
            },
            QualityTier::High | QualityTier::Custom => Self {
                scale: 1.4,
                glitch_amount: 1.0,
                scanline_intensity: 0.7,
                flicker_amount: 1.0,
                curvature: 0.2,
                brightness: 0.8,
                ..Default::default()
            },
        }
    }

    /// Accordion `400x240 @ 30fps` preview preset (`settings.js:2498-2515`).
    pub fn preview_preset() -> Self {
        Self {
            page_load_animation: false,
            page_load_progress: 1.0,
            mouse_react: false,
            scale: 1.4,
            digit_size: 1.25,
            grid_mul: [1.0, 1.0],
            curvature: 0.14,
            glitch_amount: 0.6,
            scanline_intensity: 0.45,
            flicker_amount: 0.6,
            chromatic_aberration: 0.0,
            brightness: 0.9,
            ..Default::default()
        }
    }
}

/// Supported GridMotion card aspect ratios (`gridMotion.js:22-28`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum GridAspectRatio {
    #[default]
    Ratio16x9,
    Ratio1x1,
    Ratio3x4,
    Ratio4x3,
    Auto,
}

impl GridAspectRatio {
    /// Width / height ratio matching `ASPECT_RATIO_MAP` in `gridMotion.js:22-28`.
    pub fn as_f32(self) -> f32 {
        match self {
            Self::Ratio16x9 => 16.0 / 9.0,
            Self::Ratio1x1 | Self::Auto => 1.0,
            Self::Ratio3x4 => 3.0 / 4.0,
            Self::Ratio4x3 => 4.0 / 3.0,
        }
    }

    /// Parses `"16:9"`, `"1:1"`, `"3:4"`, `"4:3"`, or `"auto"`.
    pub fn parse_str(s: &str) -> Self {
        match s.trim() {
            "16:9" => Self::Ratio16x9,
            "1:1" => Self::Ratio1x1,
            "3:4" => Self::Ratio3x4,
            "4:3" => Self::Ratio4x3,
            "auto" => Self::Auto,
            _ => Self::Ratio16x9,
        }
    }
}

/// Configuration for `GridMotion` (`gridMotion.js:10-20, 95-98`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GridMotionConfig {
    pub gradient_color: [f32; 3],
    pub speed: f32,
    pub max_move: f32,
    pub scale: f32,
    pub aspect_ratio: GridAspectRatio,
    pub auto_move: bool,
    pub rows: u32,
    pub cols: u32,
    pub miniature: bool,
    /// Horizontal pointer coordinate in pixels (`0..viewport_width`).
    pub pointer_x: f32,
    /// Optional hovered card index `(row, col)` for 1.06x zoom + 1.08x brightness.
    pub hovered_card: Option<(u32, u32)>,
    /// Uses bundled `IBM Plex Sans` when `true`, falling back to `Segoe UI` when `false`.
    pub brand_font_enabled: bool,
}

impl Default for GridMotionConfig {
    fn default() -> Self {
        Self {
            gradient_color: [0.0, 0.0, 0.0],
            speed: 1.0,
            max_move: 300.0,
            scale: 1.0,
            aspect_ratio: GridAspectRatio::Ratio16x9,
            auto_move: false,
            rows: 10,
            cols: 18,
            miniature: false,
            pointer_x: 640.0,
            hovered_card: None,
            brand_font_enabled: true,
        }
    }
}

impl GridMotionConfig {
    /// Quality preset matching `BG_QUALITY_PRESETS` in `settings.js:35-47`.
    pub fn for_quality(tier: QualityTier) -> Self {
        match tier {
            QualityTier::Low => Self {
                speed: 0.5,
                max_move: 150.0,
                ..Default::default()
            },
            QualityTier::Medium => Self::default(),
            QualityTier::High | QualityTier::Custom => Self {
                speed: 1.5,
                max_move: 450.0,
                ..Default::default()
            },
        }
    }

    /// Accordion `400x240 @ 30fps` preview preset (`settings.js:2522-2532`).
    pub fn preview_preset() -> Self {
        Self {
            max_move: 40.0,
            speed: 1.2,
            auto_move: true,
            miniature: true,
            rows: 6,
            cols: 10,
            pointer_x: 200.0,
            ..Default::default()
        }
    }
}

/// Video aspect-fitting mode inside Pass 0 (`SceneTex`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VideoFitMode {
    #[default]
    Contain,
    Cover,
    Stretch,
}

/// Video frame source dimensions and fit parameters for Pass 0 (`SceneTex`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoFrameTransform {
    pub video_width: u32,
    pub video_height: u32,
    pub fit_mode: VideoFitMode,
    pub opacity: f32,
}

impl VideoFrameTransform {
    /// Computes normalized UV rect `[scale_x, scale_y, offset_x, offset_y]` within viewport.
    pub fn uv_rect(self, viewport_w: u32, viewport_h: u32) -> [f32; 4] {
        let vw = viewport_w.max(1) as f32;
        let vh = viewport_h.max(1) as f32;
        let iw = self.video_width.max(1) as f32;
        let ih = self.video_height.max(1) as f32;

        let vp_aspect = vw / vh;
        let vid_aspect = iw / ih;

        match self.fit_mode {
            VideoFitMode::Stretch => [1.0, 1.0, 0.0, 0.0],
            VideoFitMode::Contain => {
                if vid_aspect > vp_aspect {
                    let sy = vp_aspect / vid_aspect;
                    [1.0, sy, 0.0, (1.0 - sy) * 0.5]
                } else {
                    let sx = vid_aspect / vp_aspect;
                    [sx, 1.0, (1.0 - sx) * 0.5, 0.0]
                }
            }
            VideoFitMode::Cover => {
                if vid_aspect > vp_aspect {
                    let sx = vid_aspect / vp_aspect;
                    [sx, 1.0, (1.0 - sx) * 0.5, 0.0]
                } else {
                    let sy = vp_aspect / vid_aspect;
                    [1.0, sy, 0.0, (1.0 - sy) * 0.5]
                }
            }
        }
    }
}

/// Parses a `#RGB`, `#RRGGBB`, or `#RRGGBBAA` hex string into normalized RGB floats.
pub fn parse_hex_rgb(hex: &str) -> Option<[f32; 3]> {
    let rgba = parse_hex_rgba(hex)?;
    Some([rgba[0], rgba[1], rgba[2]])
}

/// Parses `#RGB`, `#RGBA`, `#RRGGBB`, or `#RRGGBBAA` into normalized RGBA floats.
pub fn parse_hex_rgba(hex: &str) -> Option<[f32; 4]> {
    let h = hex.trim().trim_start_matches('#');
    match h.len() {
        3 => {
            let r = u8::from_str_radix(&h[0..1], 16).ok()? * 17;
            let g = u8::from_str_radix(&h[1..2], 16).ok()? * 17;
            let b = u8::from_str_radix(&h[2..3], 16).ok()? * 17;
            Some([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0])
        }
        4 => {
            let r = u8::from_str_radix(&h[0..1], 16).ok()? * 17;
            let g = u8::from_str_radix(&h[1..2], 16).ok()? * 17;
            let b = u8::from_str_radix(&h[2..3], 16).ok()? * 17;
            let a = u8::from_str_radix(&h[3..4], 16).ok()? * 17;
            Some([
                r as f32 / 255.0,
                g as f32 / 255.0,
                b as f32 / 255.0,
                a as f32 / 255.0,
            ])
        }
        6 => {
            let r = u8::from_str_radix(&h[0..2], 16).ok()?;
            let g = u8::from_str_radix(&h[2..4], 16).ok()?;
            let b = u8::from_str_radix(&h[4..6], 16).ok()?;
            Some([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0])
        }
        8 => {
            let r = u8::from_str_radix(&h[0..2], 16).ok()?;
            let g = u8::from_str_radix(&h[2..4], 16).ok()?;
            let b = u8::from_str_radix(&h[4..6], 16).ok()?;
            let a = u8::from_str_radix(&h[6..8], 16).ok()?;
            Some([
                r as f32 / 255.0,
                g as f32 / 255.0,
                b as f32 / 255.0,
                a as f32 / 255.0,
            ])
        }
        _ => None,
    }
}
