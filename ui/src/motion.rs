//! Motion profiles (`balanced`, `reduced`, `showcase`), all 38 main CSS `@keyframes`
//! plus 2 companion auth keyframes (`youtube-auth.js:744,821`), and elastic volume spring math.

/// Motion intensity profile (`main.css:14-84`, `settings.js:2301-2337`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MotionProfile {
    #[default]
    Balanced,
    Reduced,
    Showcase,
}

impl MotionProfile {
    pub fn from_str_name(value: &str) -> Self {
        match value {
            "reduced" => Self::Reduced,
            "showcase" => Self::Showcase,
            _ => Self::Balanced,
        }
    }

    pub fn fast_ms(self) -> f32 {
        match self {
            Self::Balanced => 120.0,
            Self::Reduced => 90.0,
            Self::Showcase => 140.0,
        }
    }

    pub fn medium_ms(self) -> f32 {
        match self {
            Self::Balanced => 200.0,
            Self::Reduced => 120.0,
            Self::Showcase => 240.0,
        }
    }

    pub fn slow_ms(self) -> f32 {
        match self {
            Self::Balanced => 320.0,
            Self::Reduced => 180.0,
            Self::Showcase => 360.0,
        }
    }

    /// Evaluates `--motion-ease-standard: cubic-bezier(0.2, 0.7, 0.2, 1)`.
    pub fn ease_standard(t: f32) -> f32 {
        cubic_bezier_y(0.2, 0.7, 0.2, 1.0, t)
    }

    /// Evaluates `--motion-ease-emphasized: cubic-bezier(0.18, 0.84, 0.24, 1)`.
    pub fn ease_emphasized(t: f32) -> f32 {
        cubic_bezier_y(0.18, 0.84, 0.24, 1.0, t)
    }
}

/// Window lifecycle motion modifiers (`main.css:90-145`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WindowMotionState {
    pub cold_start: bool,
    pub unfocused: bool,
    pub is_animating_bounds: bool,
    pub max_transition: bool,
}

/// All 38 main CSS `@keyframes` + 2 companion auth `@keyframes` (`tokens-and-metrics.md` §4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyframeId {
    // animations.css (19)
    FadeIn,
    FadeOut,
    ModalSlideIn,
    ModalSlideOut,
    SlideUp,
    SlideDown,
    SlideInRight,
    SkipIndicatorForwardPop,
    SkipIndicatorBackwardPop,
    SkipArrowWaveForward,
    SkipArrowWaveBackward,
    Pulse,
    PulseGlow,
    Spin,
    Ripple,
    ToastIn,
    ToastOut,
    PulseToastDot,
    ToastSpin,
    // controls.css (2)
    HoverThumbIn,
    HoverThumbLoading,
    // main.css (12)
    RecordingPillIn,
    RecordingRubyBreath,
    RecordingBtnPop,
    RecordingDigitFlash,
    PreviewDeletePop,
    PreviewRestorePop,
    BadgePopIn,
    ScreenshotNoticeIn,
    FaultyPickerSweep,
    ColorBendsPickerSpin,
    WelcomeTitlebarIn,
    WelcomeControlIn,
    // settings.css (5)
    YtPulse,
    DropdownFadeIn,
    BgSectionIn,
    BgAccordionPopoverIn,
    ShortcutPulse,
    // youtube-auth.js (2 companion auth keyframes)
    AuthSpin,
    AuthPulseRing,
}

impl KeyframeId {
    pub const MAIN_CSS_COUNT: usize = 38;
    pub const COMPANION_AUTH_COUNT: usize = 2;

    pub const ALL: [KeyframeId; 40] = [
        Self::FadeIn,
        Self::FadeOut,
        Self::ModalSlideIn,
        Self::ModalSlideOut,
        Self::SlideUp,
        Self::SlideDown,
        Self::SlideInRight,
        Self::SkipIndicatorForwardPop,
        Self::SkipIndicatorBackwardPop,
        Self::SkipArrowWaveForward,
        Self::SkipArrowWaveBackward,
        Self::Pulse,
        Self::PulseGlow,
        Self::Spin,
        Self::Ripple,
        Self::ToastIn,
        Self::ToastOut,
        Self::PulseToastDot,
        Self::ToastSpin,
        Self::HoverThumbIn,
        Self::HoverThumbLoading,
        Self::RecordingPillIn,
        Self::RecordingRubyBreath,
        Self::RecordingBtnPop,
        Self::RecordingDigitFlash,
        Self::PreviewDeletePop,
        Self::PreviewRestorePop,
        Self::BadgePopIn,
        Self::ScreenshotNoticeIn,
        Self::FaultyPickerSweep,
        Self::ColorBendsPickerSpin,
        Self::WelcomeTitlebarIn,
        Self::WelcomeControlIn,
        Self::YtPulse,
        Self::DropdownFadeIn,
        Self::BgSectionIn,
        Self::BgAccordionPopoverIn,
        Self::ShortcutPulse,
        Self::AuthSpin,
        Self::AuthPulseRing,
    ];

    pub const fn is_companion_auth(self) -> bool {
        matches!(self, Self::AuthSpin | Self::AuthPulseRing)
    }

    pub const fn css_name(self) -> &'static str {
        match self {
            Self::FadeIn => "fadeIn",
            Self::FadeOut => "fadeOut",
            Self::ModalSlideIn => "modalSlideIn",
            Self::ModalSlideOut => "modalSlideOut",
            Self::SlideUp => "slideUp",
            Self::SlideDown => "slideDown",
            Self::SlideInRight => "slideInRight",
            Self::SkipIndicatorForwardPop => "skipIndicatorForwardPop",
            Self::SkipIndicatorBackwardPop => "skipIndicatorBackwardPop",
            Self::SkipArrowWaveForward => "skipArrowWaveForward",
            Self::SkipArrowWaveBackward => "skipArrowWaveBackward",
            Self::Pulse => "pulse",
            Self::PulseGlow => "pulse-glow",
            Self::Spin => "spin",
            Self::Ripple => "ripple",
            Self::ToastIn => "toastIn",
            Self::ToastOut => "toastOut",
            Self::PulseToastDot => "pulseToastDot",
            Self::ToastSpin => "toastSpin",
            Self::HoverThumbIn => "hover-thumb-in",
            Self::HoverThumbLoading => "hover-thumb-loading",
            Self::RecordingPillIn => "recordingPillIn",
            Self::RecordingRubyBreath => "recordingRubyBreath",
            Self::RecordingBtnPop => "recordingBtnPop",
            Self::RecordingDigitFlash => "recordingDigitFlash",
            Self::PreviewDeletePop => "previewDeletePop",
            Self::PreviewRestorePop => "previewRestorePop",
            Self::BadgePopIn => "badgePopIn",
            Self::ScreenshotNoticeIn => "screenshotNoticeIn",
            Self::FaultyPickerSweep => "faultyPickerSweep",
            Self::ColorBendsPickerSpin => "colorBendsPickerSpin",
            Self::WelcomeTitlebarIn => "welcomeTitlebarIn",
            Self::WelcomeControlIn => "welcomeControlIn",
            Self::YtPulse => "yt-pulse",
            Self::DropdownFadeIn => "dropdownFadeIn",
            Self::BgSectionIn => "bgSectionIn",
            Self::BgAccordionPopoverIn => "bgAccordionPopoverIn",
            Self::ShortcutPulse => "shortcutPulse",
            Self::AuthSpin => "spin",
            Self::AuthPulseRing => "pulse-ring",
        }
    }
}

/// Interpolated CSS keyframe transform/opacity sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeyframeSample {
    pub opacity: f32,
    pub translate_x: f32,
    pub translate_y: f32,
    pub scale_x: f32,
    pub scale_y: f32,
    pub rotate_deg: f32,
    pub glow: f32,
}

impl Default for KeyframeSample {
    fn default() -> Self {
        Self {
            opacity: 1.0,
            translate_x: 0.0,
            translate_y: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            rotate_deg: 0.0,
            glow: 0.0,
        }
    }
}

/// Evaluates any of the 38+2 keyframes at normalized progress `t in 0.0..=1.0`.
pub fn evaluate_keyframe(id: KeyframeId, t: f32, profile: MotionProfile) -> KeyframeSample {
    let t = t.clamp(0.0, 1.0);
    if profile == MotionProfile::Reduced
        && matches!(
            id,
            KeyframeId::WelcomeTitlebarIn | KeyframeId::WelcomeControlIn
        )
    {
        return KeyframeSample::default();
    }

    match id {
        KeyframeId::FadeIn => KeyframeSample {
            opacity: t,
            ..Default::default()
        },
        KeyframeId::FadeOut => KeyframeSample {
            opacity: 1.0 - t,
            ..Default::default()
        },
        KeyframeId::ModalSlideIn => KeyframeSample {
            opacity: t,
            translate_y: lerp(10.0, 0.0, t),
            scale_x: lerp(0.95, 1.0, t),
            scale_y: lerp(0.95, 1.0, t),
            ..Default::default()
        },
        KeyframeId::ModalSlideOut => KeyframeSample {
            opacity: 1.0 - t,
            translate_y: lerp(0.0, 10.0, t),
            scale_x: lerp(1.0, 0.95, t),
            scale_y: lerp(1.0, 0.95, t),
            ..Default::default()
        },
        KeyframeId::SlideUp | KeyframeId::ToastIn => KeyframeSample {
            opacity: t,
            translate_y: lerp(20.0, 0.0, t),
            ..Default::default()
        },
        KeyframeId::SlideDown => KeyframeSample {
            opacity: t,
            translate_y: lerp(-20.0, 0.0, t),
            ..Default::default()
        },
        KeyframeId::SlideInRight => KeyframeSample {
            opacity: t,
            translate_x: lerp(20.0, 0.0, t),
            ..Default::default()
        },
        KeyframeId::ToastOut => KeyframeSample {
            opacity: 1.0 - t,
            translate_y: lerp(0.0, -20.0, t),
            ..Default::default()
        },
        KeyframeId::SkipIndicatorForwardPop => {
            let (s, x, op) = if t < 0.6 {
                let u = t / 0.6;
                (lerp(0.86, 1.04, u), lerp(-6.0, 2.0, u), u.min(1.0))
            } else {
                let u = (t - 0.6) / 0.4;
                (lerp(1.04, 1.0, u), lerp(2.0, 0.0, u), 1.0)
            };
            KeyframeSample {
                opacity: op,
                translate_x: x,
                scale_x: s,
                scale_y: s,
                ..Default::default()
            }
        }
        KeyframeId::SkipIndicatorBackwardPop => {
            let (s, x, op) = if t < 0.6 {
                let u = t / 0.6;
                (lerp(0.86, 1.04, u), lerp(6.0, -2.0, u), u.min(1.0))
            } else {
                let u = (t - 0.6) / 0.4;
                (lerp(1.04, 1.0, u), lerp(-2.0, 0.0, u), 1.0)
            };
            KeyframeSample {
                opacity: op,
                translate_x: x,
                scale_x: s,
                scale_y: s,
                ..Default::default()
            }
        }
        KeyframeId::SkipArrowWaveForward => {
            let wave = (t * std::f32::consts::PI).sin();
            KeyframeSample {
                opacity: lerp(0.45, 1.0, wave),
                translate_x: lerp(0.0, 2.0, wave),
                ..Default::default()
            }
        }
        KeyframeId::SkipArrowWaveBackward => {
            let wave = (t * std::f32::consts::PI).sin();
            KeyframeSample {
                opacity: lerp(0.45, 1.0, wave),
                translate_x: lerp(0.0, -2.0, wave),
                ..Default::default()
            }
        }
        KeyframeId::Pulse => {
            let wave = (t * std::f32::consts::PI).sin();
            KeyframeSample {
                opacity: lerp(1.0, 0.6, wave),
                ..Default::default()
            }
        }
        KeyframeId::PulseGlow => {
            let wave = (t * std::f32::consts::PI).sin();
            let s = lerp(1.0, 1.05, wave);
            KeyframeSample {
                scale_x: s,
                scale_y: s,
                glow: wave,
                ..Default::default()
            }
        }
        KeyframeId::Spin | KeyframeId::ToastSpin | KeyframeId::AuthSpin => KeyframeSample {
            rotate_deg: 360.0 * t,
            ..Default::default()
        },
        KeyframeId::Ripple => KeyframeSample {
            opacity: lerp(0.5, 0.0, t),
            scale_x: 2.5 * t,
            scale_y: 2.5 * t,
            ..Default::default()
        },
        KeyframeId::PulseToastDot | KeyframeId::YtPulse | KeyframeId::AuthPulseRing => {
            let wave = (t * std::f32::consts::PI).sin();
            let s = lerp(1.0, 1.3, wave);
            KeyframeSample {
                opacity: lerp(1.0, 0.55, wave),
                scale_x: s,
                scale_y: s,
                ..Default::default()
            }
        }
        KeyframeId::HoverThumbIn => KeyframeSample {
            opacity: t,
            translate_y: lerp(6.0, 0.0, t),
            scale_x: lerp(0.96, 1.0, t),
            scale_y: lerp(0.96, 1.0, t),
            ..Default::default()
        },
        KeyframeId::HoverThumbLoading => KeyframeSample {
            translate_x: lerp(-1.0, 1.0, t),
            ..Default::default()
        },
        KeyframeId::RecordingPillIn => KeyframeSample {
            opacity: t,
            translate_y: lerp(-10.0, 0.0, t),
            scale_x: lerp(0.92, 1.0, t),
            scale_y: lerp(0.92, 1.0, t),
            ..Default::default()
        },
        KeyframeId::RecordingRubyBreath => {
            let wave = (t * std::f32::consts::PI).sin();
            let s = lerp(1.0, 0.94, wave);
            KeyframeSample {
                opacity: lerp(1.0, 0.68, wave),
                scale_x: s,
                scale_y: s,
                glow: lerp(1.0, 0.4, wave),
                ..Default::default()
            }
        }
        KeyframeId::RecordingBtnPop => {
            let s = if t < 0.4 {
                lerp(1.0, 0.86, t / 0.4)
            } else if t < 0.75 {
                lerp(0.86, 1.08, (t - 0.4) / 0.35)
            } else {
                lerp(1.08, 1.0, (t - 0.75) / 0.25)
            };
            KeyframeSample {
                scale_x: s,
                scale_y: s,
                ..Default::default()
            }
        }
        KeyframeId::RecordingDigitFlash => {
            let wave = (t * std::f32::consts::PI).sin();
            KeyframeSample {
                scale_y: lerp(1.0, 1.15, wave),
                glow: wave,
                ..Default::default()
            }
        }
        KeyframeId::PreviewDeletePop => {
            if t < 0.3 {
                let u = t / 0.3;
                let s = lerp(1.0, 1.04, u);
                KeyframeSample {
                    scale_x: s,
                    scale_y: s,
                    rotate_deg: lerp(0.0, -1.0, u),
                    ..Default::default()
                }
            } else {
                let u = (t - 0.3) / 0.7;
                let s = lerp(1.04, 0.75, u);
                KeyframeSample {
                    opacity: 1.0 - u,
                    translate_y: lerp(0.0, 24.0, u),
                    scale_x: s,
                    scale_y: s,
                    rotate_deg: lerp(-1.0, 2.0, u),
                    ..Default::default()
                }
            }
        }
        KeyframeId::PreviewRestorePop => {
            if t < 0.65 {
                let u = t / 0.65;
                let s = lerp(0.85, 1.02, u);
                KeyframeSample {
                    opacity: u,
                    translate_y: lerp(10.0, -2.0, u),
                    scale_x: s,
                    scale_y: s,
                    ..Default::default()
                }
            } else {
                let u = (t - 0.65) / 0.35;
                let s = lerp(1.02, 1.0, u);
                KeyframeSample {
                    opacity: 1.0,
                    translate_y: lerp(-2.0, 0.0, u),
                    scale_x: s,
                    scale_y: s,
                    ..Default::default()
                }
            }
        }
        KeyframeId::BadgePopIn => {
            let s = if t < 0.6 {
                lerp(0.6, 1.06, t / 0.6)
            } else {
                lerp(1.06, 1.0, (t - 0.6) / 0.4)
            };
            KeyframeSample {
                opacity: (t * 2.0).min(1.0),
                scale_x: s,
                scale_y: s,
                ..Default::default()
            }
        }
        KeyframeId::ScreenshotNoticeIn => KeyframeSample {
            opacity: t,
            translate_y: lerp(-8.0, 0.0, t),
            scale_x: lerp(0.94, 1.0, t),
            scale_y: lerp(0.94, 1.0, t),
            ..Default::default()
        },
        KeyframeId::FaultyPickerSweep => KeyframeSample {
            translate_y: lerp(-1.0, 1.0, t),
            ..Default::default()
        },
        KeyframeId::ColorBendsPickerSpin => KeyframeSample {
            rotate_deg: lerp(8.0, 368.0, t),
            ..Default::default()
        },
        KeyframeId::WelcomeTitlebarIn => KeyframeSample {
            opacity: t,
            translate_y: lerp(-10.0, 0.0, t),
            ..Default::default()
        },
        KeyframeId::WelcomeControlIn => KeyframeSample {
            opacity: t,
            translate_y: lerp(10.0, 0.0, t),
            ..Default::default()
        },
        KeyframeId::DropdownFadeIn => KeyframeSample {
            opacity: t,
            translate_y: lerp(-6.0, 0.0, t),
            ..Default::default()
        },
        KeyframeId::BgSectionIn => KeyframeSample {
            opacity: t,
            translate_y: lerp(6.0, 0.0, t),
            ..Default::default()
        },
        KeyframeId::BgAccordionPopoverIn => KeyframeSample {
            opacity: t,
            translate_x: lerp(20.0, 0.0, t),
            scale_x: lerp(0.96, 1.0, t),
            scale_y: lerp(0.96, 1.0, t),
            ..Default::default()
        },
        KeyframeId::ShortcutPulse => {
            let wave = (t * std::f32::consts::PI).sin();
            KeyframeSample {
                glow: wave,
                ..Default::default()
            }
        }
    }
}

/// Over-drag rubber-band region on the volume slider (`controls.js:1124-1266`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ElasticRegion {
    #[default]
    Middle,
    Left,
    Right,
}

/// Damped elastic spring for volume slider rubber-banding (`controls.js:1112-1266`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ElasticVolumeSpring {
    pub overflow: f32,
    pub velocity: f32,
    pub region: ElasticRegion,
    pub dragging: bool,
}

impl Default for ElasticVolumeSpring {
    fn default() -> Self {
        Self {
            overflow: 0.0,
            velocity: 0.0,
            region: ElasticRegion::Middle,
            dragging: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VolumeElasticTransform {
    pub scale_x: f32,
    pub scale_y: f32,
    /// `0.0` = left edge, `0.5` = center, `1.0` = right edge.
    pub origin_x: f32,
    pub btn_volume_translate_x: f32,
    pub volume_value_translate_x: f32,
    pub time_display_translate_x: f32,
}

impl ElasticVolumeSpring {
    pub const MAX_OVERFLOW: f32 = 30.0;
    pub const STIFFNESS: f32 = 0.2;
    pub const DAMPING: f32 = 0.55;
    pub const SETTLE_EPSILON: f32 = 0.05;

    /// Evaluates pointer drag at `pointer_x` against slider horizontal bounds `[left, right]`
    /// and returns `(new_volume_pct, new_overflow)`.
    pub fn handle_drag(
        &mut self,
        pointer_x: f32,
        left: f32,
        right: f32,
        boost_enabled: bool,
    ) -> u32 {
        let width = (right - left).max(1.0);
        let max_vol: f32 = if boost_enabled { 200.0 } else { 100.0 };

        if pointer_x < left {
            let diff = left - pointer_x;
            let ov = Self::MAX_OVERFLOW * (1.0 - (-diff / 30.0).exp());
            self.region = ElasticRegion::Left;
            self.overflow = ov;
            self.velocity = 0.0;
            0
        } else if pointer_x > right {
            let diff = pointer_x - right;
            let ov = Self::MAX_OVERFLOW * (1.0 - (-diff / 30.0).exp());
            self.region = ElasticRegion::Right;
            self.overflow = ov;
            self.velocity = 0.0;
            max_vol as u32
        } else {
            self.region = ElasticRegion::Middle;
            self.overflow = 0.0;
            self.velocity = 0.0;
            (((pointer_x - left) / width) * max_vol)
                .round()
                .clamp(0.0, max_vol) as u32
        }
    }

    pub fn release(&mut self) {
        self.dragging = false;
    }

    /// Advances spring physics one animation frame when released. Returns `true` if still animating.
    pub fn step(&mut self) -> bool {
        if self.dragging {
            return false;
        }
        let force = (0.0 - self.overflow) * Self::STIFFNESS;
        self.velocity = (self.velocity + force) * Self::DAMPING;
        self.overflow += self.velocity;

        if self.overflow.abs() > Self::SETTLE_EPSILON || self.velocity.abs() > Self::SETTLE_EPSILON
        {
            true
        } else {
            self.overflow = 0.0;
            self.velocity = 0.0;
            self.region = ElasticRegion::Middle;
            false
        }
    }

    /// Computes exact CSS transform equivalents (`controls.js:1126-1168`).
    pub fn transforms(&self, slider_width: f32) -> VolumeElasticTransform {
        let width = if slider_width > 0.0 {
            slider_width
        } else {
            72.0
        };
        let scale_x = 1.0 + (self.overflow / width);
        let scale_y = 1.0 - (self.overflow / Self::MAX_OVERFLOW) * 0.2;
        match self.region {
            ElasticRegion::Left => VolumeElasticTransform {
                scale_x,
                scale_y,
                origin_x: 1.0,
                btn_volume_translate_x: -self.overflow,
                volume_value_translate_x: 0.0,
                time_display_translate_x: 0.0,
            },
            ElasticRegion::Right => VolumeElasticTransform {
                scale_x,
                scale_y,
                origin_x: 0.0,
                btn_volume_translate_x: 0.0,
                volume_value_translate_x: self.overflow,
                time_display_translate_x: self.overflow,
            },
            ElasticRegion::Middle => VolumeElasticTransform {
                scale_x,
                scale_y,
                origin_x: 0.5,
                btn_volume_translate_x: 0.0,
                volume_value_translate_x: 0.0,
                time_display_translate_x: 0.0,
            },
        }
    }
}

/// Frame-timed UI animation state consumed by `crates/app` (`step_animations`) and `layout.rs`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiAnimationState {
    /// Normalized modal slide/fade progress (`0.0..=1.0`, `1.0` = settled).
    pub modal_t: f32,
    /// `true` while `closing_modal` is playing `ModalSlideOut`.
    pub modal_closing: bool,
    /// Normalized playlist drawer slide progress (`0.0..=1.0`, `1.0` = settled).
    pub playlist_t: f32,
    /// `true` while playlist drawer is animating closed (`0 -> +pw`).
    pub playlist_closing: bool,
    /// Normalized `#bgAccordionPopover` slide/fade progress (`0.0..=1.0`).
    pub bg_accordion_t: f32,
    /// `true` while `#bgAccordionPopover` is animating closed.
    pub bg_accordion_closing: bool,
    /// Normalized controls bar fade/slide visibility (`0.0` hidden .. `1.0` visible).
    pub controls_t: f32,
    /// Normalized seekbar hover thumbnail pop-in progress (`0.0..=1.0`).
    pub hover_thumb_t: f32,
    /// Normalized toast slide-in progress (`0.0..=1.0`).
    pub toast_t: f32,
    /// Normalized skip indicator pop progress (`0.0..=1.0`).
    pub skip_t: f32,
    /// Remaining visibility time (s) for skip indicator (`0.85s` in `controls.js:1616`).
    pub skip_ttl_secs: f32,
    /// Remaining visibility time (s) for top-right volume OSD (`0.90s` in `controls.js:1639`).
    pub volume_osd_ttl_secs: f32,
    /// Normalized screenshot preview / delete / restore card animation progress (`0.0..=1.0`).
    pub screenshot_t: f32,
    /// `true` when single-shot screenshot card is playing `PreviewDeletePop`.
    pub screenshot_deleting: bool,
    /// Remaining visibility time (s) for single-shot screenshot preview (`1.8s` normal/restore, `0.45s` delete).
    pub screenshot_ttl_secs: f32,
    /// Remaining visibility time (s) for `screenshot_notice` (`1.8s` in `player.js:1453`).
    pub screenshot_notice_ttl_secs: f32,
    /// Remaining visibility time (s) for `RecordingIndicatorMode::Saved` (`1.8s`).
    pub recording_saved_ttl_secs: f32,
    /// Looping loading spinner phase (`0.0..1.0`, `0.8s` period).
    pub spinner_phase: f32,
    /// Looping recording pulse phase (`0.0..1.0`, `1.8s` period).
    pub recording_pulse_phase: f32,
    /// Looping A-B loop indicator pulse phase (`0.0..1.0`, `1.5s` period).
    pub ab_pulse_phase: f32,
    /// Per-card `#bgAccordionGallery` animated flex weights (`settings.css:1680-1706`).
    pub accordion_weights_initialized: bool,
    pub accordion_card_from_w: [f32; 5],
    pub accordion_card_to_w: [f32; 5],
    pub accordion_card_t: [f32; 5],
    pub accordion_card_weights: [f32; 5],
    /// Per-card `.bg-accordion-content` reveal progress (`0.0..=1.0`).
    pub accordion_card_from_reveal: [f32; 5],
    pub accordion_card_to_reveal: [f32; 5],
    pub accordion_card_reveal_t: [f32; 5],
    pub accordion_card_reveal: [f32; 5],
}

impl Default for UiAnimationState {
    fn default() -> Self {
        Self {
            modal_t: 1.0,
            modal_closing: false,
            playlist_t: 1.0,
            playlist_closing: false,
            bg_accordion_t: 1.0,
            bg_accordion_closing: false,
            controls_t: 1.0,
            hover_thumb_t: 1.0,
            toast_t: 1.0,
            skip_t: 1.0,
            skip_ttl_secs: 0.0,
            volume_osd_ttl_secs: 0.0,
            screenshot_t: 1.0,
            screenshot_deleting: false,
            screenshot_ttl_secs: 0.0,
            screenshot_notice_ttl_secs: 0.0,
            recording_saved_ttl_secs: 0.0,
            spinner_phase: 0.0,
            recording_pulse_phase: 0.0,
            ab_pulse_phase: 0.0,
            accordion_weights_initialized: false,
            accordion_card_from_w: [1.0; 5],
            accordion_card_to_w: [1.0; 5],
            accordion_card_t: [1.0; 5],
            accordion_card_weights: [1.0; 5],
            accordion_card_from_reveal: [0.0; 5],
            accordion_card_to_reveal: [0.0; 5],
            accordion_card_reveal_t: [1.0; 5],
            accordion_card_reveal: [0.0; 5],
        }
    }
}

impl UiAnimationState {
    pub fn trigger_modal_in(&mut self) {
        self.modal_t = 0.0;
        self.modal_closing = false;
    }

    pub fn trigger_modal_out(&mut self) {
        self.modal_t = 0.0;
        self.modal_closing = true;
    }

    pub fn trigger_playlist_in(&mut self) {
        self.playlist_t = 0.0;
        self.playlist_closing = false;
    }

    pub fn trigger_playlist_out(&mut self) {
        self.playlist_t = 0.0;
        self.playlist_closing = true;
    }

    pub fn trigger_bg_accordion_in(&mut self) {
        self.bg_accordion_t = 0.0;
        self.bg_accordion_closing = false;
    }

    pub fn trigger_bg_accordion_out(&mut self) {
        self.bg_accordion_t = 0.0;
        self.bg_accordion_closing = true;
    }

    pub fn trigger_hover_thumb_in(&mut self) {
        self.hover_thumb_t = 0.0;
    }

    pub fn trigger_toast_in(&mut self) {
        self.toast_t = 0.0;
    }

    pub fn trigger_skip_osd(&mut self) {
        self.skip_t = 0.0;
        self.skip_ttl_secs = 0.85;
    }

    pub fn trigger_volume_osd(&mut self) {
        self.volume_osd_ttl_secs = 0.90;
    }

    pub fn trigger_screenshot_in(&mut self, ttl_secs: f32) {
        self.screenshot_t = 0.0;
        self.screenshot_deleting = false;
        self.screenshot_ttl_secs = ttl_secs;
    }

    pub fn trigger_screenshot_delete(&mut self) {
        self.screenshot_t = 0.0;
        self.screenshot_deleting = true;
        self.screenshot_ttl_secs = 0.45;
    }

    pub fn trigger_screenshot_notice(&mut self) {
        self.screenshot_notice_ttl_secs = 1.8;
    }

    pub fn trigger_recording_saved(&mut self) {
        self.recording_saved_ttl_secs = 1.8;
    }

    pub fn accordion_card_animating(&self) -> bool {
        self.accordion_weights_initialized
            && (self.accordion_card_t.iter().any(|&t| t < 1.0)
                || self.accordion_card_reveal_t.iter().any(|&t| t < 1.0))
    }

    /// Interpolates `#bgAccordionGallery` card flex weights (`0.35s cubic-bezier(0.25, 1, 0.5, 1)`)
    /// and `.bg-accordion-content` reveal (`0.35s cubic-bezier(0.16, 1, 0.3, 1)`).
    pub fn step_accordion_cards(
        &mut self,
        dt: f32,
        target_weights: [f32; 5],
        target_reveals: [f32; 5],
        reduced_motion: bool,
    ) -> bool {
        if !self.accordion_weights_initialized || reduced_motion {
            self.accordion_weights_initialized = !reduced_motion;
            self.accordion_card_from_w = target_weights;
            self.accordion_card_to_w = target_weights;
            self.accordion_card_t = [1.0; 5];
            self.accordion_card_weights = target_weights;
            self.accordion_card_from_reveal = target_reveals;
            self.accordion_card_to_reveal = target_reveals;
            self.accordion_card_reveal_t = [1.0; 5];
            self.accordion_card_reveal = target_reveals;
            return false;
        }

        let mut active = false;
        const DUR: f32 = 0.35;
        for i in 0..5 {
            if (self.accordion_card_to_w[i] - target_weights[i]).abs() > 0.001 {
                self.accordion_card_from_w[i] = self.accordion_card_weights[i];
                self.accordion_card_to_w[i] = target_weights[i];
                self.accordion_card_t[i] = 0.0;
            }
            if self.accordion_card_t[i] < 1.0 {
                self.accordion_card_t[i] = (self.accordion_card_t[i] + dt / DUR).min(1.0);
                let e = cubic_bezier(0.25, 1.0, 0.5, 1.0, self.accordion_card_t[i]);
                self.accordion_card_weights[i] = lerp(
                    self.accordion_card_from_w[i],
                    self.accordion_card_to_w[i],
                    e,
                );
                active = true;
            } else {
                self.accordion_card_weights[i] = self.accordion_card_to_w[i];
            }

            if (self.accordion_card_to_reveal[i] - target_reveals[i]).abs() > 0.001 {
                self.accordion_card_from_reveal[i] = self.accordion_card_reveal[i];
                self.accordion_card_to_reveal[i] = target_reveals[i];
                self.accordion_card_reveal_t[i] = 0.0;
            }
            if self.accordion_card_reveal_t[i] < 1.0 {
                self.accordion_card_reveal_t[i] =
                    (self.accordion_card_reveal_t[i] + dt / DUR).min(1.0);
                let e = cubic_bezier(0.16, 1.0, 0.3, 1.0, self.accordion_card_reveal_t[i]);
                self.accordion_card_reveal[i] = lerp(
                    self.accordion_card_from_reveal[i],
                    self.accordion_card_to_reveal[i],
                    e,
                );
                active = true;
            } else {
                self.accordion_card_reveal[i] = self.accordion_card_to_reveal[i];
            }
        }
        active
    }
}

/// Evaluates CSS `cubic-bezier(x1, y1, x2, y2)` at `t in [0.0, 1.0]`.
#[inline]
pub fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
    cubic_bezier_y(x1, y1, x2, y2, t)
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Approximates CSS `cubic-bezier(x1, y1, x2, y2)` via Newton-Raphson in `t in [0, 1]`.
fn cubic_bezier_y(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    let target = x.clamp(0.0, 1.0);
    let mut u = target;
    for _ in 0..5 {
        let inv = 1.0 - u;
        let bx = 3.0 * inv * inv * u * x1 + 3.0 * inv * u * u * x2 + u * u * u;
        let dbx = 3.0 * inv * inv * x1 + 6.0 * inv * u * (x2 - x1) + 3.0 * u * u * (1.0 - x2);
        if dbx.abs() < 1e-5 {
            break;
        }
        u = (u - (bx - target) / dbx).clamp(0.0, 1.0);
    }
    let inv = 1.0 - u;
    3.0 * inv * inv * u * y1 + 3.0 * inv * u * u * y2 + u * u * u
}
