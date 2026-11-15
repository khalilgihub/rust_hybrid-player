//! Input routing, shortcuts, hold-to-burst, cursor idle auto-hide,
//! chrome auto-hide, mouse-wheel volume gestures, and drag-and-drop file routing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use hybrid_core::media_scanner::is_media_extension;
use hybrid_core::shortcuts::ShortcutManager;

/// Modifier state for building canonical shortcut combo strings (`Ctrl+Shift+KeyS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyModifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
}

/// Formats modifier flags and a Web-style key code (`"Space"`, `"KeyS"`, `"ArrowLeft"`) into a combo string.
pub fn build_combo_string(code: &str, mods: KeyModifiers) -> String {
    let mut parts = Vec::with_capacity(5);
    if mods.ctrl {
        parts.push("Ctrl");
    }
    if mods.alt {
        parts.push("Alt");
    }
    if mods.shift {
        parts.push("Shift");
    }
    if mods.meta {
        parts.push("Meta");
    }
    parts.push(code);
    parts.join("+")
}

pub const CURSOR_IDLE_TIMEOUT: Duration = Duration::from_millis(2000);
pub const CHROME_AUTO_HIDE_TIMEOUT: Duration = Duration::from_millis(2500);
pub const BURST_HOLD_THRESHOLD: Duration = Duration::from_millis(200);
pub const BURST_FRAME_INTERVAL: Duration = Duration::from_millis(250);

/// External subtitle file extensions supported for drag-and-drop (`controls.js:776`).
pub const SUBTITLE_EXTENSIONS: [&str; 9] =
    ["srt", "ass", "ssa", "vtt", "sub", "idx", "sup", "smi", "rt"];

/// Returns `true` when `path` has a supported subtitle file extension.
pub fn is_subtitle_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|ext| SUBTITLE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Partitioned drag-and-drop payload matching `controls.js:765-816`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DroppedFilesPlan {
    Empty,
    /// Only subtitle files dropped while media is active -> load external tracks.
    LoadSubtitlesOnly(Vec<PathBuf>),
    /// Only subtitle files dropped while no media is open -> show toast error.
    SubtitlesWithoutMedia,
    /// Media files (optionally with companion subtitles loaded right after).
    OpenMedia {
        media_files: Vec<PathBuf>,
        companion_subtitles: Vec<PathBuf>,
        replace_playlist: bool,
    },
}

/// Classifies dropped paths into media vs subtitle actions (`controls.js:765-816`).
pub fn classify_dropped_files(paths: &[PathBuf], has_active_media: bool) -> DroppedFilesPlan {
    if paths.is_empty() {
        return DroppedFilesPlan::Empty;
    }

    let mut media_files = Vec::new();
    let mut subtitle_files = Vec::new();

    for path in paths {
        if is_subtitle_file(path) {
            subtitle_files.push(path.clone());
        } else if is_media_extension(path) || path.is_dir() {
            media_files.push(path.clone());
        }
    }

    if media_files.is_empty() && !subtitle_files.is_empty() {
        if has_active_media {
            return DroppedFilesPlan::LoadSubtitlesOnly(subtitle_files);
        } else {
            return DroppedFilesPlan::SubtitlesWithoutMedia;
        }
    }

    if !media_files.is_empty() {
        let replace_playlist = !subtitle_files.is_empty();
        return DroppedFilesPlan::OpenMedia {
            media_files,
            companion_subtitles: subtitle_files,
            replace_playlist,
        };
    }

    DroppedFilesPlan::Empty
}

/// High-level commands emitted by `InputController` for `MediaBridge` / window dispatch.
#[derive(Debug, Clone, PartialEq)]
pub enum InputAction {
    ShortcutAction(String),
    TakeSingleScreenshot,
    TakePngScreenshot,
    StartBurstCapture,
    StopBurstCapture,
    BurstTickCapture,
    CarouselNavigate(i32),
    CarouselClose,
    CarouselConfirmAction,
    DismissModalOrPopover,
    TogglePlaylistDrawer,
    ExitFullscreen,
    AdjustVolumeDelta(i32),
}

/// Context needed to evaluate keyboard, mouse, wheel, idle cursor, and chrome auto-hide rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InputContext {
    pub has_active_media: bool,
    pub is_playing: bool,
    pub is_fullscreen: bool,
    pub is_locked: bool,
    pub modal_open: bool,
    pub bg_accordion_open: bool,
    pub dropdown_open: bool,
    pub playlist_open: bool,
    pub text_input_focused: bool,
    pub recording_shortcut: bool,
    pub carousel_open: bool,
    pub carousel_restore_mode: bool,
    pub hovering_controls: bool,
    pub dragging_slider: bool,
    pub in_wheel_exclusion_zone: bool,
    pub mouse_button_down: bool,
    pub interactive_control_focused: bool,
}

/// Idle cursor tracker (`CursorManager`, `2000ms` threshold).
#[derive(Debug, Clone)]
pub struct CursorIdleTracker {
    last_activity: Instant,
    hidden: bool,
}

impl Default for CursorIdleTracker {
    fn default() -> Self {
        Self::new(Instant::now())
    }
}

impl CursorIdleTracker {
    pub fn new(now: Instant) -> Self {
        Self {
            last_activity: now,
            hidden: false,
        }
    }

    pub fn record_activity(&mut self, now: Instant) -> bool {
        self.last_activity = now;
        let changed = self.hidden;
        self.hidden = false;
        changed
    }

    /// Evaluates whether cursor should currently be hidden (`isPlaying && idle >= 2000ms && !modalOpen && !carousel && !editable && !hoveringControls && !mouseDown`).
    pub fn evaluate(&mut self, now: Instant, ctx: &InputContext) -> bool {
        let can_hide = ctx.is_playing
            && ctx.has_active_media
            && !ctx.modal_open
            && !ctx.bg_accordion_open
            && !ctx.dropdown_open
            && !ctx.carousel_open
            && !ctx.text_input_focused
            && !ctx.hovering_controls
            && !ctx.dragging_slider
            && !ctx.mouse_button_down;
        let next_hidden =
            can_hide && now.saturating_duration_since(self.last_activity) >= CURSOR_IDLE_TIMEOUT;
        self.hidden = next_hidden;
        self.hidden
    }

    pub fn is_hidden(&self) -> bool {
        self.hidden
    }
}

/// Chrome auto-hide tracker (`controls.js:818-860`, `2500ms` threshold).
#[derive(Debug, Clone)]
pub struct ChromeAutoHide {
    last_activity: Instant,
    hidden: bool,
}

impl Default for ChromeAutoHide {
    fn default() -> Self {
        Self::new(Instant::now())
    }
}

impl ChromeAutoHide {
    pub fn new(now: Instant) -> Self {
        Self {
            last_activity: now,
            hidden: false,
        }
    }

    pub fn record_activity(&mut self, now: Instant) {
        self.last_activity = now;
        self.hidden = false;
    }

    /// Returns `true` when bottom chrome should be auto-hidden.
    pub fn evaluate(&mut self, now: Instant, ctx: &InputContext) -> bool {
        let blocked = !ctx.has_active_media
            || ctx.hovering_controls
            || ctx.dragging_slider
            || ctx.modal_open
            || ctx.bg_accordion_open
            || ctx.dropdown_open
            || ctx.text_input_focused
            || ctx.carousel_open
            || (!ctx.is_playing && !ctx.is_fullscreen);

        if blocked {
            self.hidden = false;
            return false;
        }

        let elapsed = now.saturating_duration_since(self.last_activity);
        self.hidden = elapsed >= CHROME_AUTO_HIDE_TIMEOUT;
        self.hidden
    }

    pub fn is_hidden(&self) -> bool {
        self.hidden
    }
}

/// Hold-to-burst screenshot state machine (`shortcuts.js:642-813`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BurstTrigger {
    Key(u32),
    Mouse(i32),
}

/// Stateful keyboard, screenshot-burst, mouse-button, wheel, and idle-timer controller.
pub struct InputController {
    pub shortcuts: ShortcutManager,
    pub cursor_tracker: CursorIdleTracker,
    pub chrome_hide: ChromeAutoHide,
    active_hold: Option<(BurstTrigger, Instant)>,
    burst_active: bool,
    last_burst_tick: Option<Instant>,
    pub burst_interval: Duration,
    pub burst_in_flight: bool,
}

impl InputController {
    pub fn new(custom_bindings: BTreeMap<String, String>, screenshot_mouse_button: i32) -> Self {
        let now = Instant::now();
        let mut shortcuts = ShortcutManager::default();
        shortcuts.apply_custom_shortcuts(&custom_bindings);
        shortcuts.set_screenshot_mouse_button(screenshot_mouse_button);
        Self {
            shortcuts,
            cursor_tracker: CursorIdleTracker::new(now),
            chrome_hide: ChromeAutoHide::new(now),
            active_hold: None,
            burst_active: false,
            last_burst_tick: None,
            burst_interval: BURST_FRAME_INTERVAL,
            burst_in_flight: false,
        }
    }

    pub fn set_burst_fps(&mut self, fps: f64) {
        let ms = (1000.0 / fps.clamp(1.0, 240.0)).floor().max(16.0) as u64;
        self.burst_interval = Duration::from_millis(ms);
    }

    pub fn is_burst_active(&self) -> bool {
        self.burst_active
    }

    /// Handles a key-down event (`code` matches Web `KeyboardEvent.code`, e.g. `"Space"`, `"KeyS"`).
    pub fn on_key_down(
        &mut self,
        now: Instant,
        code: &str,
        key_id: u32,
        mods: KeyModifiers,
        is_repeat: bool,
        ctx: &InputContext,
    ) -> Vec<InputAction> {
        self.chrome_hide.record_activity(now);
        if ctx.recording_shortcut || ctx.text_input_focused {
            if self.burst_active {
                self.cancel_burst();
                return vec![InputAction::StopBurstCapture];
            }
            self.active_hold = None;
            return Vec::new();
        }

        let combo = build_combo_string(code, mods);
        let action = self.shortcuts.resolve_action(&combo).map(str::to_string);

        // Screenshot carousel keyboard interception (`shortcuts.js:571-606`)
        if ctx.carousel_open {
            match code {
                "ArrowLeft" => return vec![InputAction::CarouselNavigate(-1)],
                "ArrowRight" => return vec![InputAction::CarouselNavigate(1)],
                "Escape" => return vec![InputAction::CarouselClose],
                "Enter" => return vec![InputAction::CarouselConfirmAction],
                "KeyZ" if ctx.carousel_restore_mode => {
                    return vec![InputAction::CarouselConfirmAction]
                }
                "KeyD" if !ctx.carousel_restore_mode => {
                    return vec![InputAction::CarouselConfirmAction]
                }
                _ => {}
            }
            if ctx.carousel_restore_mode && action.as_deref() == Some("restore-latest-screenshot") {
                return vec![InputAction::CarouselConfirmAction];
            }
            if !ctx.carousel_restore_mode && action.as_deref() == Some("delete-latest-screenshot") {
                return vec![InputAction::CarouselConfirmAction];
            }
        }

        // Escape priority cascade (`shortcuts.js:932-964`)
        if code == "Escape" || action.as_deref() == Some("exit-fullscreen") {
            return vec![self.resolve_escape_action(ctx)];
        }

        let Some(action_id) = action else {
            return Vec::new();
        };

        // UI lock guard (`shortcuts.js:619-629`)
        if ctx.is_locked && !ShortcutManager::is_action_allowed_when_locked(&action_id) {
            return Vec::new();
        }

        // Modal guard (`shortcuts.js:635-640`)
        if ctx.modal_open && (action_id != "toggle-play" || ctx.interactive_control_focused) {
            if self.burst_active {
                self.cancel_burst();
                return vec![InputAction::StopBurstCapture];
            }
            return Vec::new();
        }

        if action_id == "screenshot" {
            if !ctx.has_active_media {
                return Vec::new();
            }
            if !is_repeat {
                self.active_hold = Some((BurstTrigger::Key(key_id), now));
            } else if !self.burst_active {
                self.burst_active = true;
                self.last_burst_tick = Some(now);
                return vec![InputAction::StartBurstCapture];
            }
            return Vec::new();
        }

        if action_id == "screenshot-png" {
            if !ctx.has_active_media || is_repeat {
                return Vec::new();
            }
            return vec![InputAction::TakePngScreenshot];
        }

        let repeatable = ShortcutManager::is_repeatable_action(&action_id);
        if is_repeat && !repeatable {
            return Vec::new();
        }

        vec![InputAction::ShortcutAction(action_id)]
    }

    /// Handles a key-up event (resolves short tap screenshot vs ending burst hold).
    pub fn on_key_up(
        &mut self,
        code: &str,
        key_id: u32,
        mods: KeyModifiers,
        ctx: &InputContext,
    ) -> Vec<InputAction> {
        if ctx.recording_shortcut || ctx.text_input_focused {
            self.active_hold = None;
            if self.burst_active {
                self.cancel_burst();
                return vec![InputAction::StopBurstCapture];
            }
            return Vec::new();
        }

        let combo = build_combo_string(code, mods);
        let matches_active =
            matches!(self.active_hold, Some((BurstTrigger::Key(k), _)) if k == key_id);
        let matches_action = self.shortcuts.resolve_action(&combo) == Some("screenshot");
        if matches_active || matches_action {
            let was_burst = self.burst_active;
            let was_pressed = matches_active;
            self.active_hold = None;
            self.burst_active = false;
            self.last_burst_tick = None;
            if was_burst {
                return vec![InputAction::StopBurstCapture];
            } else if was_pressed {
                return vec![InputAction::TakeSingleScreenshot];
            }
        }
        Vec::new()
    }

    /// Handles mouse button down mapped to Chromium button indices (`1=Middle, 3=Back/X1, 4=Forward/X2`).
    pub fn on_mouse_button_down(
        &mut self,
        now: Instant,
        mouse_btn: i32,
        ctx: &InputContext,
    ) -> Vec<InputAction> {
        self.cursor_tracker.record_activity(now);
        self.chrome_hide.record_activity(now);

        let target = self.shortcuts.screenshot_mouse_button();
        if target < 0
            || mouse_btn != target
            || ctx.text_input_focused
            || ctx.modal_open
            || !ctx.has_active_media
            || ctx.carousel_open
        {
            return Vec::new();
        }

        if self.active_hold.is_none() {
            self.active_hold = Some((BurstTrigger::Mouse(mouse_btn), now));
        }
        Vec::new()
    }

    /// Handles mouse button release for `screenshotMouseButton`.
    pub fn on_mouse_button_up(&mut self, mouse_btn: i32) -> Vec<InputAction> {
        let target = self.shortcuts.screenshot_mouse_button();
        if target < 0 || mouse_btn != target {
            return Vec::new();
        }
        if matches!(self.active_hold, Some((BurstTrigger::Mouse(b), _)) if b == mouse_btn) {
            let was_burst = self.burst_active;
            self.active_hold = None;
            self.burst_active = false;
            self.last_burst_tick = None;
            if was_burst {
                return vec![InputAction::StopBurstCapture];
            }
            return vec![InputAction::TakeSingleScreenshot];
        }
        Vec::new()
    }

    /// Handles vertical scroll wheel delta on the video viewport (`gestures.js:12-50`).
    pub fn on_mouse_wheel(
        &mut self,
        now: Instant,
        delta_x: f32,
        delta_y: f32,
        ctx: &InputContext,
    ) -> Option<InputAction> {
        self.cursor_tracker.record_activity(now);
        self.chrome_hide.record_activity(now);

        if ctx.is_locked
            || !ctx.has_active_media
            || ctx.carousel_open
            || ctx.modal_open
            || ctx.bg_accordion_open
            || ctx.in_wheel_exclusion_zone
        {
            return None;
        }
        if delta_y.abs() < 1.0 || delta_x.abs() > delta_y.abs() {
            return None;
        }
        // Scroll up (`delta_y > 0` in winit LineDelta / PixelDelta) -> +5%, scroll down -> -5%
        let step = if delta_y > 0.0 { 5 } else { -5 };
        Some(InputAction::AdjustVolumeDelta(step))
    }

    /// Periodic tick (`~30-60Hz`): promotes 200ms holds into burst capture and ticks burst frames,
    /// plus updates cursor/chrome idle timers.
    pub fn tick(&mut self, now: Instant, ctx: &InputContext) -> (Vec<InputAction>, bool, bool) {
        let mut actions = Vec::new();

        if let Some((_, started_at)) = self.active_hold {
            if !self.burst_active
                && now.saturating_duration_since(started_at) >= BURST_HOLD_THRESHOLD
            {
                self.burst_active = true;
                self.last_burst_tick = Some(now);
                actions.push(InputAction::StartBurstCapture);
            } else if self.burst_active {
                let due = !self.burst_in_flight
                    && self
                        .last_burst_tick
                        .map(|t| now.saturating_duration_since(t) >= self.burst_interval)
                        .unwrap_or(true);
                if due {
                    self.last_burst_tick = Some(now);
                    actions.push(InputAction::BurstTickCapture);
                }
            }
        }

        let hide_cursor = self.cursor_tracker.evaluate(now, ctx);
        let hide_chrome = self.chrome_hide.evaluate(now, ctx);
        (actions, hide_cursor, hide_chrome)
    }

    /// Cancels any active screenshot hold/burst on window focus loss (`shortcuts.js:794-826`).
    pub fn on_focus_lost(&mut self) -> Vec<InputAction> {
        self.active_hold = None;
        if self.burst_active {
            self.burst_active = false;
            self.last_burst_tick = None;
            return vec![InputAction::StopBurstCapture];
        }
        Vec::new()
    }

    fn cancel_burst(&mut self) {
        self.active_hold = None;
        self.burst_active = false;
        self.last_burst_tick = None;
    }

    /// Escape priority: carousel -> dropdown/modal/bgPopover -> playlist -> exit fullscreen.
    pub fn resolve_escape_action(&self, ctx: &InputContext) -> InputAction {
        if ctx.carousel_open {
            InputAction::CarouselClose
        } else if ctx.dropdown_open || ctx.modal_open || ctx.bg_accordion_open {
            InputAction::DismissModalOrPopover
        } else if ctx.playlist_open {
            InputAction::TogglePlaylistDrawer
        } else {
            InputAction::ExitFullscreen
        }
    }

    /// Time until the next burst/idle state transition so the event loop can use `WaitUntil`.
    pub fn next_wake_deadline(&self, now: Instant, ctx: &InputContext) -> Option<Instant> {
        let mut earliest: Option<Instant> = None;
        let mut push_candidate = |t: Instant| {
            earliest = Some(match earliest {
                Some(cur) if cur <= t => cur,
                _ => t,
            });
        };

        if let Some((_, started_at)) = self.active_hold {
            if !self.burst_active {
                push_candidate(started_at + BURST_HOLD_THRESHOLD);
            } else if let Some(last) = self.last_burst_tick {
                push_candidate(last + self.burst_interval);
            } else {
                push_candidate(now);
            }
        }
        if ctx.is_playing
            && ctx.has_active_media
            && !ctx.modal_open
            && !ctx.bg_accordion_open
            && !ctx.dropdown_open
            && !ctx.carousel_open
            && !ctx.text_input_focused
            && !ctx.hovering_controls
            && !ctx.dragging_slider
            && !ctx.mouse_button_down
            && !self.cursor_tracker.is_hidden()
        {
            push_candidate(self.cursor_tracker.last_activity + CURSOR_IDLE_TIMEOUT);
        }
        if ctx.has_active_media && !self.chrome_hide.is_hidden() {
            push_candidate(self.chrome_hide.last_activity + CHROME_AUTO_HIDE_TIMEOUT);
        }
        earliest
    }
}
