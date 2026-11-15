//! Native Pure-Rust Companion Auth Status Window for YouTube Option A.
//!
//! Renders the 3 companion auth SVGs (`YOUTUBE_LOGO_SVG`, `VERIFIED_SHIELD_SVG`,
//! `STEP_CHECK_SVG`), radial backdrop, 3-step flow card, animated pulse ring
//! (`2.0s`, `cubic-bezier(0.215, 0.61, 0.355, 1)`), spinner, and interactive
//! Cancel / Finish & Import buttons using `resvg` (`usvg` + `tiny_skia`) +
//! Win32 GDI double-buffered DIB + `DrawTextW` — zero WebView, HTML, or JS.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Instant;

use resvg::tiny_skia::{
    Color, FillRule, GradientStop, LineCap, LinearGradient, Paint, PathBuilder, Pixmap, Point,
    RadialGradient, Rect, SpreadMode, Stroke, Transform,
};
use resvg::usvg;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWINDOWATTRIBUTE};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject,
    DrawTextW, EndPaint, InvalidateRect, SelectObject, SetBkMode, SetTextColor, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET,
    DEFAULT_PITCH, DIB_RGB_COLORS, DT_CENTER, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER,
    DT_WORDBREAK, FF_SWISS, OUT_DEFAULT_PRECIS, PAINTSTRUCT, SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_ESCAPE, VK_RETURN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW,
    KillTimer, LoadCursorW, PostMessageW, PostQuitMessage, RegisterClassExW, SetCursor, SetTimer,
    ShowWindow, TranslateMessage, CW_USEDEFAULT, IDC_ARROW, IDC_HAND, MSG, SW_SHOWNOACTIVATE,
    WM_APP, WM_CLOSE, WM_CREATE, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_KEYDOWN,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCDESTROY, WM_PAINT, WM_SETCURSOR, WM_TIMER,
    WNDCLASSEXW, WS_CAPTION, WS_EX_APPWINDOW, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU,
};

use crate::dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE_ID, DWMWA_USE_IMMERSIVE_DARK_MODE_LEGACY_ID};
use crate::error::{Result, Win32ShellError};
use crate::subclass::{
    dip_to_physical, physical_to_dip, scale_for_dpi, unpack_lparam_point, DipRect,
};

/// Logical client width in DIPs (`youtube-auth.js:569`).
pub const COMPANION_WIDTH_DIP: i32 = 500;
/// Logical client height in DIPs (`youtube-auth.js:570`).
pub const COMPANION_HEIGHT_DIP: i32 = 405;

const TIMER_ANIM_ID: usize = 1;
const TIMER_INTERVAL_MS: u32 = 16;
const WM_MOUSELEAVE: u32 = 0x02A3;
const WM_APP_UPDATE_STATE: u32 = WM_APP + 0x41;
const WM_APP_CLOSE_WINDOW: u32 = WM_APP + 0x42;

/// Companion Auth SVG #1: YouTube logo (`youtube-auth.js:888-891`).
pub const YOUTUBE_LOGO_SVG: &str = r##"<svg viewBox="0 0 28 20" xmlns="http://www.w3.org/2000/svg" fill="none">
  <path d="M27.438 3.125C27.117 1.93 26.176 0.988 24.98 0.668 22.809 0.082 14 0.082 14 0.082s-8.809 0-10.98.586C1.824 0.988 0.883 1.93 0.562 3.125 0 5.309 0 9.844 0 9.844s0 4.535.562 6.719c.321 1.195 1.262 2.137 2.458 2.457C5.191 19.605 14 19.605 14 19.605s8.809 0 10.98-.586c1.196-.32 2.137-1.262 2.458-2.457.562-2.184.562-6.719.562-6.719s0-4.535-.562-6.718z" fill="#FF0000"/>
  <polygon points="11.2,14.07 18.48,9.844 11.2,5.617" fill="#FFFFFF"/>
</svg>"##;

/// Companion Auth SVG #2: Verified shield badge (`youtube-auth.js:899`).
pub const VERIFIED_SHIELD_SVG: &str = r##"<svg viewBox="0 0 20 20" xmlns="http://www.w3.org/2000/svg" fill="#4ade80">
  <path fill-rule="evenodd" d="M10 1.944A11.954 11.954 0 012.166 5C2.056 5.649 2 6.319 2 7c0 5.225 3.34 9.67 8 11.317C14.66 16.67 18 12.225 18 7c0-.682-.057-1.35-.166-2.001A11.954 11.954 0 0110 1.944zM11 14a1 1 0 11-2 0 1 1 0 012 0zm0-7a1 1 0 10-2 0v3a1 1 0 102 0V7z" clip-rule="evenodd"/>
</svg>"##;

/// Companion Auth SVG #3: Step 1 green checkmark (`youtube-auth.js:908`).
pub const STEP_CHECK_SVG: &str = r##"<svg viewBox="0 0 16 16" xmlns="http://www.w3.org/2000/svg" fill="none" stroke="#4ade80" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
  <polyline points="3 8 6.5 11.5 13 4.5"/>
</svg>"##;

/// Actions emitted by the Companion Auth Status Window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompanionAuthAction {
    FinishAndImport,
    Cancel,
    WindowClosed,
}

/// Configuration for opening the native Companion Auth Status Window.
#[derive(Debug, Clone)]
pub struct CompanionAuthOptions {
    pub browser_display_name: String,
    pub switch_account: bool,
    pub initial_status: String,
}

impl Default for CompanionAuthOptions {
    fn default() -> Self {
        Self {
            browser_display_name: "Google Chrome".to_string(),
            switch_account: false,
            initial_status: "Waiting for sign-in in Google Chrome...".to_string(),
        }
    }
}

/// Interactive/animation state for rendering the companion auth UI.
#[derive(Debug, Clone)]
pub struct CompanionAuthState {
    pub browser_display_name: String,
    pub switch_account: bool,
    pub status_text: String,
    pub finishing: bool,
    pub hover_cancel: bool,
    pub hover_finish: bool,
    pub pressed_cancel: bool,
    pub pressed_finish: bool,
    /// Animation elapsed seconds (drives 2.0s pulse ring & 0.7s spinner).
    pub elapsed_secs: f32,
}

impl CompanionAuthState {
    pub fn from_options(opts: &CompanionAuthOptions) -> Self {
        Self {
            browser_display_name: opts.browser_display_name.clone(),
            switch_account: opts.switch_account,
            status_text: opts.initial_status.clone(),
            finishing: false,
            hover_cancel: false,
            hover_finish: false,
            pressed_cancel: false,
            pressed_finish: false,
            elapsed_secs: 0.0,
        }
    }
}

/// Cached parsed `usvg::Tree` instances for the 3 companion auth SVGs.
pub struct CompanionSvgAssets {
    pub youtube_logo: usvg::Tree,
    pub verified_shield: usvg::Tree,
    pub step_check: usvg::Tree,
}

impl CompanionSvgAssets {
    pub fn parse() -> Result<Self> {
        let opts = usvg::Options::default();
        let parse_one = |svg: &str, name: &str| {
            usvg::Tree::from_str(svg, &opts)
                .map_err(|e| Win32ShellError::Render(format!("SVG `{name}` parse error: {e}")))
        };
        Ok(Self {
            youtube_logo: parse_one(YOUTUBE_LOGO_SVG, "youtube_logo")?,
            verified_shield: parse_one(VERIFIED_SHIELD_SVG, "verified_shield")?,
            step_check: parse_one(STEP_CHECK_SVG, "step_check")?,
        })
    }
}

/// Layout geometry in DIPs (`500 x 405`).
pub struct CompanionLayout {
    pub cancel_btn: DipRect,
    pub finish_btn: DipRect,
    pub verified_pill: DipRect,
    pub card: DipRect,
    pub status_box: DipRect,
}

pub fn companion_layout(finishing: bool) -> CompanionLayout {
    let finish_w = if finishing { 112 } else { 136 };
    let finish_x = COMPANION_WIDTH_DIP - 22 - finish_w;
    let cancel_w = 78;
    let cancel_x = finish_x - 10 - cancel_w;
    let btn_y = COMPANION_HEIGHT_DIP - 20 - 32;

    CompanionLayout {
        verified_pill: DipRect::new(COMPANION_WIDTH_DIP - 22 - 74, 24, 74, 22),
        card: DipRect::new(22, 70, COMPANION_WIDTH_DIP - 44, 260),
        status_box: DipRect::new(37, 278, COMPANION_WIDTH_DIP - 74, 36),
        cancel_btn: DipRect::new(cancel_x, btn_y, cancel_w, 32),
        finish_btn: DipRect::new(finish_x, btn_y, finish_w, 32),
    }
}

/// Ease-out curve approximating CSS `cubic-bezier(0.215, 0.61, 0.355, 1)`.
pub fn pulse_ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

fn rounded_rect_path(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<resvg::tiny_skia::Path> {
    let r = r.min(w * 0.5).min(h * 0.5).max(0.0);
    let mut pb = PathBuilder::new();
    if r <= 0.01 {
        pb.push_rect(Rect::from_xywh(x, y, w, h)?);
        return pb.finish();
    }
    let k = r * 0.552_284_8;
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish()
}

#[allow(clippy::too_many_arguments)]
fn fill_rounded_rect(
    pixmap: &mut Pixmap,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    r: f32,
    color: Color,
    stroke_color: Option<Color>,
    stroke_width: f32,
) {
    let Some(path) = rounded_rect_path(x, y, w, h, r) else {
        return;
    };
    let mut paint = Paint::default();
    paint.set_color(color);
    paint.anti_alias = true;
    pixmap.fill_path(
        &path,
        &paint,
        FillRule::Winding,
        Transform::identity(),
        None,
    );

    if let Some(sc) = stroke_color {
        let mut spaint = Paint::default();
        spaint.set_color(sc);
        spaint.anti_alias = true;
        let stroke = Stroke {
            width: stroke_width,
            ..Default::default()
        };
        pixmap.stroke_path(&path, &spaint, &stroke, Transform::identity(), None);
    }
}

fn render_svg_fit(
    pixmap: &mut Pixmap,
    tree: &usvg::Tree,
    dst_x: f32,
    dst_y: f32,
    dst_w: f32,
    dst_h: f32,
) {
    let sz = tree.size();
    if sz.width() <= 0.0 || sz.height() <= 0.0 {
        return;
    }
    let sx = dst_w / sz.width();
    let sy = dst_h / sz.height();
    let transform = Transform::from_translate(dst_x, dst_y).pre_scale(sx, sy);
    let mut pm = pixmap.as_mut();
    resvg::render(tree, transform, &mut pm);
}

/// Render the vector graphics, background gradient, 3 SVGs, pulse animation,
/// and button shells into a `tiny_skia::Pixmap` (RGBA premultiplied).
pub fn render_companion_vector_layer(
    state: &CompanionAuthState,
    assets: &CompanionSvgAssets,
    width_px: u32,
    height_px: u32,
    dpi_scale: f32,
) -> Option<Pixmap> {
    let mut pixmap = Pixmap::new(width_px.max(1), height_px.max(1))?;
    let w = width_px as f32;
    let h = height_px as f32;
    let s = dpi_scale.max(0.5);

    // 1. Radial top gradient `#171822` -> `#090a0f`
    pixmap.fill(Color::from_rgba8(9, 10, 15, 255));
    if let Some(bg_rect) = Rect::from_xywh(0.0, 0.0, w, h) {
        let mut pb = PathBuilder::new();
        pb.push_rect(bg_rect);
        if let Some(bg_path) = pb.finish() {
            if let Some(grad) = RadialGradient::new(
                Point::from_xy(w * 0.5, -h * 0.1),
                0.0,
                Point::from_xy(w * 0.5, -h * 0.1),
                w.max(h) * 0.85,
                vec![
                    GradientStop::new(0.0, Color::from_rgba8(23, 24, 34, 255)),
                    GradientStop::new(1.0, Color::from_rgba8(9, 10, 15, 255)),
                ],
                SpreadMode::Pad,
                Transform::identity(),
            ) {
                let paint = Paint {
                    shader: grad,
                    anti_alias: false,
                    ..Default::default()
                };
                pixmap.fill_path(
                    &bg_path,
                    &paint,
                    FillRule::Winding,
                    Transform::identity(),
                    None,
                );
            }
        }
    }

    let layout = companion_layout(state.finishing);

    // 2. `.yt-icon-container` (36x36 at x=22, y=20) + YouTube logo SVG (24x17)
    fill_rounded_rect(
        &mut pixmap,
        22.0 * s,
        20.0 * s,
        36.0 * s,
        36.0 * s,
        10.0 * s,
        Color::from_rgba8(255, 255, 255, 10),
        Some(Color::from_rgba8(255, 255, 255, 20)),
        1.0 * s,
    );
    render_svg_fit(
        &mut pixmap,
        &assets.youtube_logo,
        28.0 * s,
        29.5 * s,
        24.0 * s,
        17.0 * s,
    );

    // 3. `.shield-badge` pill + Verified Shield SVG (12x12)
    let vp = layout.verified_pill;
    fill_rounded_rect(
        &mut pixmap,
        vp.x as f32 * s,
        vp.y as f32 * s,
        vp.width as f32 * s,
        vp.height as f32 * s,
        11.0 * s,
        Color::from_rgba8(34, 197, 94, 23),
        Some(Color::from_rgba8(34, 197, 94, 64)),
        1.0 * s,
    );
    render_svg_fit(
        &mut pixmap,
        &assets.verified_shield,
        (vp.x as f32 + 9.0) * s,
        (vp.y as f32 + 5.0) * s,
        12.0 * s,
        12.0 * s,
    );

    // 4. `.card` container
    let card = layout.card;
    fill_rounded_rect(
        &mut pixmap,
        card.x as f32 * s,
        card.y as f32 * s,
        card.width as f32 * s,
        card.height as f32 * s,
        12.0 * s,
        Color::from_rgba8(255, 255, 255, 6),
        Some(Color::from_rgba8(255, 255, 255, 18)),
        1.0 * s,
    );

    // 5. Step markers + connector lines + Checkmark SVG
    let marker_cx = 47.0;
    let step1_cy = 96.0;
    let step2_cy = 160.0;
    let step3_cy = 224.0;

    // Vertical connector lines
    fill_rounded_rect(
        &mut pixmap,
        (marker_cx - 0.5) * s,
        (step1_cy + 13.0) * s,
        1.0 * s,
        (step2_cy - step1_cy - 26.0) * s,
        0.0,
        Color::from_rgba8(255, 255, 255, 28),
        None,
        0.0,
    );
    fill_rounded_rect(
        &mut pixmap,
        (marker_cx - 0.5) * s,
        (step2_cy + 13.0) * s,
        1.0 * s,
        (step3_cy - step2_cy - 26.0) * s,
        0.0,
        Color::from_rgba8(255, 255, 255, 28),
        None,
        0.0,
    );

    // Step 1 done circle (20x20) + STEP_CHECK_SVG (10x10)
    fill_rounded_rect(
        &mut pixmap,
        (marker_cx - 10.0) * s,
        (step1_cy - 10.0) * s,
        20.0 * s,
        20.0 * s,
        10.0 * s,
        Color::from_rgba8(34, 197, 94, 38),
        Some(Color::from_rgba8(34, 197, 94, 102)),
        1.0 * s,
    );
    render_svg_fit(
        &mut pixmap,
        &assets.step_check,
        (marker_cx - 5.0) * s,
        (step1_cy - 5.0) * s,
        10.0 * s,
        10.0 * s,
    );

    // Step 2 active blue circle
    fill_rounded_rect(
        &mut pixmap,
        (marker_cx - 10.0) * s,
        (step2_cy - 10.0) * s,
        20.0 * s,
        20.0 * s,
        10.0 * s,
        Color::from_rgba8(59, 130, 246, 38),
        Some(Color::from_rgba8(59, 130, 246, 102)),
        1.0 * s,
    );

    // Step 3 pending circle
    fill_rounded_rect(
        &mut pixmap,
        (marker_cx - 10.0) * s,
        (step3_cy - 10.0) * s,
        20.0 * s,
        20.0 * s,
        10.0 * s,
        Color::from_rgba8(255, 255, 255, 13),
        Some(Color::from_rgba8(255, 255, 255, 31)),
        1.0 * s,
    );

    // 6. `.status-area` + Pulse Ring animation (`2.0s` period)
    let sb = layout.status_box;
    fill_rounded_rect(
        &mut pixmap,
        sb.x as f32 * s,
        sb.y as f32 * s,
        sb.width as f32 * s,
        sb.height as f32 * s,
        9.0 * s,
        Color::from_rgba8(0, 0, 0, 89),
        Some(Color::from_rgba8(255, 255, 255, 15)),
        1.0 * s,
    );

    let pulse_cx = (sb.x as f32 + 17.0) * s;
    let pulse_cy = (sb.y as f32 + sb.height as f32 * 0.5) * s;
    let phase = (state.elapsed_secs / 2.0).fract();
    let eased = pulse_ease_out(phase);
    let ring_scale = 0.6 + 1.0 * eased;
    let ring_alpha = ((0.9 * (1.0 - eased)) * 153.0).round().clamp(0.0, 255.0) as u8;
    let ring_radius = 6.0 * ring_scale * s;

    if let Some(ring_path) = PathBuilder::from_circle(pulse_cx, pulse_cy, ring_radius) {
        let mut rpaint = Paint::default();
        rpaint.set_color(Color::from_rgba8(34, 197, 94, ring_alpha));
        rpaint.anti_alias = true;
        let rstroke = Stroke {
            width: 1.5 * s,
            ..Default::default()
        };
        pixmap.stroke_path(&ring_path, &rpaint, &rstroke, Transform::identity(), None);
    }

    // Pulse glow + inner 6px dot (`#22c55e`)
    if let Some(glow_path) = PathBuilder::from_circle(pulse_cx, pulse_cy, 5.5 * s) {
        let mut gpaint = Paint::default();
        gpaint.set_color(Color::from_rgba8(34, 197, 94, 55));
        gpaint.anti_alias = true;
        pixmap.fill_path(
            &glow_path,
            &gpaint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
    if let Some(dot_path) = PathBuilder::from_circle(pulse_cx, pulse_cy, 3.0 * s) {
        let mut dpaint = Paint::default();
        dpaint.set_color(Color::from_rgba8(34, 197, 94, 255));
        dpaint.anti_alias = true;
        pixmap.fill_path(
            &dot_path,
            &dpaint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }

    // 7. Cancel secondary button
    let cb = layout.cancel_btn;
    let (c_fill, c_border) = if state.pressed_cancel {
        (
            Color::from_rgba8(255, 255, 255, 16),
            Color::from_rgba8(255, 255, 255, 36),
        )
    } else if state.hover_cancel {
        (
            Color::from_rgba8(255, 255, 255, 20),
            Color::from_rgba8(255, 255, 255, 41),
        )
    } else {
        (
            Color::from_rgba8(255, 255, 255, 10),
            Color::from_rgba8(255, 255, 255, 23),
        )
    };
    fill_rounded_rect(
        &mut pixmap,
        cb.x as f32 * s,
        cb.y as f32 * s,
        cb.width as f32 * s,
        cb.height as f32 * s,
        8.0 * s,
        c_fill,
        Some(c_border),
        1.0 * s,
    );

    // 8. Finish & Import primary red gradient button
    let fb = layout.finish_btn;
    let (top_col, bot_col, border_col) = if state.hover_finish && !state.finishing {
        (
            Color::from_rgba8(255, 26, 68, 255),
            Color::from_rgba8(220, 0, 0, 255),
            Color::from_rgba8(255, 255, 255, 71),
        )
    } else {
        (
            Color::from_rgba8(255, 0, 51, 255),
            Color::from_rgba8(204, 0, 0, 255),
            Color::from_rgba8(255, 255, 255, 46),
        )
    };
    if let Some(btn_path) = rounded_rect_path(
        fb.x as f32 * s,
        fb.y as f32 * s,
        fb.width as f32 * s,
        fb.height as f32 * s,
        8.0 * s,
    ) {
        if let Some(grad) = LinearGradient::new(
            Point::from_xy(0.0, fb.y as f32 * s),
            Point::from_xy(0.0, (fb.y + fb.height) as f32 * s),
            vec![
                GradientStop::new(0.0, top_col),
                GradientStop::new(1.0, bot_col),
            ],
            SpreadMode::Pad,
            Transform::identity(),
        ) {
            let paint = Paint {
                shader: grad,
                anti_alias: true,
                ..Default::default()
            };
            pixmap.fill_path(
                &btn_path,
                &paint,
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
        let mut spaint = Paint::default();
        spaint.set_color(border_col);
        spaint.anti_alias = true;
        pixmap.stroke_path(
            &btn_path,
            &spaint,
            &Stroke {
                width: 1.0 * s,
                ..Default::default()
            },
            Transform::identity(),
            None,
        );
    }

    // 9. Spinner arc inside Finish button when `finishing == true` (`0.7s` per revolution)
    if state.finishing {
        let spin_cx = (fb.x as f32 + 19.0) * s;
        let spin_cy = (fb.y as f32 + fb.height as f32 * 0.5) * s;
        let spin_r = 5.5 * s;

        if let Some(track) = PathBuilder::from_circle(spin_cx, spin_cy, spin_r) {
            let mut tpaint = Paint::default();
            tpaint.set_color(Color::from_rgba8(255, 255, 255, 77));
            tpaint.anti_alias = true;
            pixmap.stroke_path(
                &track,
                &tpaint,
                &Stroke {
                    width: 2.0 * s,
                    ..Default::default()
                },
                Transform::identity(),
                None,
            );
        }

        let start_angle = (state.elapsed_secs / 0.7) * std::f32::consts::TAU;
        let mut arc_pb = PathBuilder::new();
        let segments = 10usize;
        for i in 0..=segments {
            let a = start_angle + (i as f32 / segments as f32) * std::f32::consts::FRAC_PI_2;
            let px = spin_cx + a.cos() * spin_r;
            let py = spin_cy + a.sin() * spin_r;
            if i == 0 {
                arc_pb.move_to(px, py);
            } else {
                arc_pb.line_to(px, py);
            }
        }
        if let Some(arc_path) = arc_pb.finish() {
            let mut apaint = Paint::default();
            apaint.set_color(Color::from_rgba8(255, 255, 255, 255));
            apaint.anti_alias = true;
            pixmap.stroke_path(
                &arc_path,
                &apaint,
                &Stroke {
                    width: 2.0 * s,
                    line_cap: LineCap::Round,
                    ..Default::default()
                },
                Transform::identity(),
                None,
            );
        }
    }

    Some(pixmap)
}

/// Handle controlling a running native Companion Auth Status Window.
pub struct CompanionAuthHandle {
    hwnd: isize,
    shared: Arc<Mutex<CompanionAuthState>>,
    closed: Arc<AtomicBool>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl CompanionAuthHandle {
    /// Open the native pure-Rust Companion Auth Status Window on its own Win32 thread.
    pub fn open<F>(options: CompanionAuthOptions, on_action: F) -> Result<Self>
    where
        F: FnMut(CompanionAuthAction) + Send + 'static,
    {
        let shared = Arc::new(Mutex::new(CompanionAuthState::from_options(&options)));
        let closed = Arc::new(AtomicBool::new(false));
        let assets = Arc::new(CompanionSvgAssets::parse()?);

        let (ready_tx, ready_rx) = mpsc::channel::<std::result::Result<isize, Win32ShellError>>();
        let shared_clone = Arc::clone(&shared);
        let closed_clone = Arc::clone(&closed);

        let thread = std::thread::Builder::new()
            .name("hybrid-companion-auth".to_string())
            .spawn(move || {
                run_companion_window_loop(
                    shared_clone,
                    closed_clone,
                    assets,
                    Box::new(on_action),
                    ready_tx,
                );
            })?;

        let raw_hwnd = ready_rx
            .recv()
            .map_err(|_| Win32ShellError::Render("companion auth thread terminated".into()))??;

        Ok(Self {
            hwnd: raw_hwnd,
            shared,
            closed,
            thread: Mutex::new(Some(thread)),
        })
    }

    pub fn hwnd(&self) -> HWND {
        HWND(self.hwnd as *mut c_void)
    }

    pub fn set_status(&self, text: impl Into<String>) {
        if let Ok(mut s) = self.shared.lock() {
            s.status_text = text.into();
        }
        if !self.closed.load(Ordering::Acquire) && self.hwnd != 0 {
            unsafe {
                let _ = PostMessageW(Some(self.hwnd()), WM_APP_UPDATE_STATE, WPARAM(0), LPARAM(0));
            }
        }
    }

    pub fn set_finishing(&self, finishing: bool) {
        if let Ok(mut s) = self.shared.lock() {
            s.finishing = finishing;
            if finishing {
                s.status_text = "Finishing and importing cookies...".to_string();
            }
        }
        if !self.closed.load(Ordering::Acquire) && self.hwnd != 0 {
            unsafe {
                let _ = PostMessageW(Some(self.hwnd()), WM_APP_UPDATE_STATE, WPARAM(0), LPARAM(0));
            }
        }
    }

    pub fn close(&self) {
        if !self.closed.swap(true, Ordering::AcqRel) && self.hwnd != 0 {
            unsafe {
                let _ = PostMessageW(Some(self.hwnd()), WM_APP_CLOSE_WINDOW, WPARAM(0), LPARAM(0));
            }
        }
        if let Ok(mut guard) = self.thread.lock() {
            if let Some(handle) = guard.take() {
                let _ = handle.join();
            }
        }
    }
}

impl Drop for CompanionAuthHandle {
    fn drop(&mut self) {
        self.close();
    }
}

struct CompanionWindowData {
    shared: Arc<Mutex<CompanionAuthState>>,
    closed: Arc<AtomicBool>,
    assets: Arc<CompanionSvgAssets>,
    on_action: Box<dyn FnMut(CompanionAuthAction) + Send>,
    started_at: Instant,
    tracking_mouse_leave: bool,
}

fn to_wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn run_companion_window_loop(
    shared: Arc<Mutex<CompanionAuthState>>,
    closed: Arc<AtomicBool>,
    assets: Arc<CompanionSvgAssets>,
    on_action: Box<dyn FnMut(CompanionAuthAction) + Send>,
    ready_tx: mpsc::Sender<std::result::Result<isize, Win32ShellError>>,
) {
    let class_name = to_wide_null("HybridPlayerCompanionAuthWindow");
    let window_title = to_wide_null("Sign in to YouTube");
    let hinstance = unsafe { GetModuleHandleW(None) }.unwrap_or_default();

    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(companion_wnd_proc),
        hInstance: hinstance.into(),
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(),
        lpszClassName: PCWSTR(class_name.as_ptr()),
        ..Default::default()
    };

    let atom = unsafe { RegisterClassExW(&wc) };
    // ERROR_CLASS_ALREADY_EXISTS (1410) is fine if another companion window ran earlier.
    if atom == 0 {
        let last = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if last != 1410 {
            let _ = ready_tx.send(Err(Win32ShellError::Win32 {
                api: "RegisterClassExW",
                code: last as u32,
            }));
            return;
        }
    }

    let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
    let ex_style = WS_EX_APPWINDOW;
    let sys_dpi = unsafe { GetDpiForSystem() }.max(96);
    let mut win_rect = RECT {
        left: 0,
        top: 0,
        right: dip_to_physical(COMPANION_WIDTH_DIP, sys_dpi),
        bottom: dip_to_physical(COMPANION_HEIGHT_DIP, sys_dpi),
    };
    unsafe {
        let _ = AdjustWindowRectExForDpi(&mut win_rect, style, false, ex_style, sys_dpi);
    }

    let data = Box::new(CompanionWindowData {
        shared,
        closed: Arc::clone(&closed),
        assets,
        on_action,
        started_at: Instant::now(),
        tracking_mouse_leave: false,
    });
    let data_ptr = Box::into_raw(data);

    let hwnd_res = unsafe {
        CreateWindowExW(
            ex_style,
            PCWSTR(class_name.as_ptr()),
            PCWSTR(window_title.as_ptr()),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            win_rect.right - win_rect.left,
            win_rect.bottom - win_rect.top,
            None,
            None,
            Some(hinstance.into()),
            Some(data_ptr.cast()),
        )
    };

    let hwnd = match hwnd_res {
        Ok(h) if !h.0.is_null() => h,
        Ok(_) | Err(_) => {
            unsafe {
                drop(Box::from_raw(data_ptr));
            }
            let _ = ready_tx.send(Err(Win32ShellError::Win32 {
                api: "CreateWindowExW",
                code: std::io::Error::last_os_error().raw_os_error().unwrap_or(0) as u32,
            }));
            return;
        }
    };

    // Dark titlebar (Win10/11)
    let dark_mode: i32 = 1;
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWINDOWATTRIBUTE(DWMWA_USE_IMMERSIVE_DARK_MODE_ID as i32),
            (&dark_mode as *const i32).cast(),
            4,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWINDOWATTRIBUTE(DWMWA_USE_IMMERSIVE_DARK_MODE_LEGACY_ID as i32),
            (&dark_mode as *const i32).cast(),
            4,
        );
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }

    let _ = ready_tx.send(Ok(hwnd.0 as isize));

    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    closed.store(true, Ordering::Release);
}

fn rgb_colorref(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF((r as u32) | ((g as u32) << 8) | ((b as u32) << 16))
}

#[allow(clippy::too_many_arguments)]
unsafe fn draw_gdi_text(
    hdc: windows::Win32::Graphics::Gdi::HDC,
    text: &str,
    rect_dip: DipRect,
    dpi: u32,
    height_dip: i32,
    weight: i32,
    color: COLORREF,
    flags: windows::Win32::Graphics::Gdi::DRAW_TEXT_FORMAT,
) {
    let font_name = to_wide_null("Segoe UI");
    let font = CreateFontW(
        -dip_to_physical(height_dip, dpi),
        0,
        0,
        0,
        weight,
        0,
        0,
        0,
        DEFAULT_CHARSET,
        OUT_DEFAULT_PRECIS,
        CLIP_DEFAULT_PRECIS,
        CLEARTYPE_QUALITY,
        (DEFAULT_PITCH.0 | FF_SWISS.0) as u32,
        PCWSTR(font_name.as_ptr()),
    );
    let old_font = SelectObject(hdc, font.into());
    SetTextColor(hdc, color);

    let mut rc = RECT {
        left: dip_to_physical(rect_dip.x, dpi),
        top: dip_to_physical(rect_dip.y, dpi),
        right: dip_to_physical(rect_dip.x + rect_dip.width, dpi),
        bottom: dip_to_physical(rect_dip.y + rect_dip.height, dpi),
    };
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    if !wide.is_empty() {
        DrawTextW(hdc, &mut wide, &mut rc, flags | DT_NOPREFIX);
    }
    SelectObject(hdc, old_font);
    let _ = DeleteObject(font.into());
}

unsafe fn paint_companion_window(hwnd: HWND, data: &CompanionWindowData) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);

    let mut client = RECT::default();
    if GetClientRect(hwnd, &mut client).is_err() {
        let _ = EndPaint(hwnd, &ps);
        return;
    }

    let w_px = (client.right - client.left).max(1) as u32;
    let h_px = (client.bottom - client.top).max(1) as u32;
    let dpi = GetDpiForWindow(hwnd).max(96);
    let dpi_scale = scale_for_dpi(dpi) as f32;

    let state = {
        let mut guard = data.shared.lock().unwrap_or_else(|e| e.into_inner());
        guard.elapsed_secs = data.started_at.elapsed().as_secs_f32();
        guard.clone()
    };

    if let Some(pixmap) = render_companion_vector_layer(&state, &data.assets, w_px, h_px, dpi_scale)
    {
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w_px as i32,
                biHeight: -(h_px as i32), // top-down DIB
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut bits: *mut c_void = std::ptr::null_mut();
        let mem_dc = CreateCompatibleDC(Some(hdc));
        let dib = CreateDIBSection(Some(mem_dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0);
        if let Ok(hbmp) = dib {
            if !bits.is_null() {
                let src = pixmap.data();
                let dst = std::slice::from_raw_parts_mut(bits as *mut u8, src.len());
                // Convert RGBA -> BGRA for GDI 32bpp DIB
                for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                    d[0] = s[2];
                    d[1] = s[1];
                    d[2] = s[0];
                    d[3] = s[3];
                }
            }

            let old_bmp = SelectObject(mem_dc, hbmp.into());
            SetBkMode(mem_dc, TRANSPARENT);

            let layout = companion_layout(state.finishing);
            // Header text
            draw_gdi_text(
                mem_dc,
                "Sign in to YouTube",
                DipRect::new(70, 21, 280, 20),
                dpi,
                15,
                600,
                rgb_colorref(255, 255, 255),
                DT_LEFT | DT_SINGLELINE | DT_VCENTER,
            );
            draw_gdi_text(
                mem_dc,
                "Secure authentication via genuine browser",
                DipRect::new(70, 41, 290, 16),
                dpi,
                11,
                400,
                rgb_colorref(140, 145, 160),
                DT_LEFT | DT_SINGLELINE | DT_VCENTER,
            );
            draw_gdi_text(
                mem_dc,
                "Verified",
                DipRect::new(layout.verified_pill.x + 22, layout.verified_pill.y, 46, 22),
                dpi,
                11,
                500,
                rgb_colorref(74, 222, 128),
                DT_LEFT | DT_SINGLELINE | DT_VCENTER,
            );

            // Step 1
            draw_gdi_text(
                mem_dc,
                "Browser Launched",
                DipRect::new(69, 86, 360, 18),
                dpi,
                12,
                600,
                rgb_colorref(241, 245, 249),
                DT_LEFT | DT_SINGLELINE | DT_VCENTER,
            );
            draw_gdi_text(
                mem_dc,
                &format!(
                    "A clean, genuine {} window has been opened.",
                    state.browser_display_name
                ),
                DipRect::new(69, 105, 385, 34),
                dpi,
                11,
                400,
                rgb_colorref(160, 168, 184),
                DT_LEFT | DT_WORDBREAK,
            );

            // Step 2
            draw_gdi_text(
                mem_dc,
                "2",
                DipRect::new(37, 150, 20, 20),
                dpi,
                10,
                600,
                rgb_colorref(147, 197, 253),
                DT_CENTER | DT_SINGLELINE | DT_VCENTER,
            );
            draw_gdi_text(
                mem_dc,
                "Account Sign-In",
                DipRect::new(69, 150, 360, 18),
                dpi,
                12,
                600,
                rgb_colorref(241, 245, 249),
                DT_LEFT | DT_SINGLELINE | DT_VCENTER,
            );
            let step2_desc = if state.switch_account {
                "Sign in or switch to your preferred Google / YouTube account in that window."
            } else {
                "Sign in to your Google / YouTube account in that window."
            };
            draw_gdi_text(
                mem_dc,
                step2_desc,
                DipRect::new(69, 169, 385, 34),
                dpi,
                11,
                400,
                rgb_colorref(160, 168, 184),
                DT_LEFT | DT_WORDBREAK,
            );

            // Step 3
            draw_gdi_text(
                mem_dc,
                "3",
                DipRect::new(37, 214, 20, 20),
                dpi,
                10,
                600,
                rgb_colorref(170, 176, 190),
                DT_CENTER | DT_SINGLELINE | DT_VCENTER,
            );
            draw_gdi_text(
                mem_dc,
                "Automatic Sync",
                DipRect::new(69, 214, 360, 18),
                dpi,
                12,
                600,
                rgb_colorref(241, 245, 249),
                DT_LEFT | DT_SINGLELINE | DT_VCENTER,
            );
            draw_gdi_text(
                mem_dc,
                &format!(
                    "When finished, close the {} window (or click Finish below).",
                    state.browser_display_name
                ),
                DipRect::new(69, 233, 385, 34),
                dpi,
                11,
                400,
                rgb_colorref(160, 168, 184),
                DT_LEFT | DT_WORDBREAK,
            );

            // Status bar text
            draw_gdi_text(
                mem_dc,
                &state.status_text,
                DipRect::new(
                    layout.status_box.x + 32,
                    layout.status_box.y,
                    layout.status_box.width - 42,
                    layout.status_box.height,
                ),
                dpi,
                11,
                500,
                rgb_colorref(195, 202, 215),
                DT_LEFT | DT_SINGLELINE | DT_VCENTER,
            );

            // Buttons
            draw_gdi_text(
                mem_dc,
                "Cancel",
                layout.cancel_btn,
                dpi,
                12,
                500,
                if state.hover_cancel {
                    rgb_colorref(255, 255, 255)
                } else {
                    rgb_colorref(195, 200, 212)
                },
                DT_CENTER | DT_SINGLELINE | DT_VCENTER,
            );

            let finish_label = if state.finishing {
                "Syncing..."
            } else {
                "Finish & Import"
            };
            let finish_text_rect = if state.finishing {
                DipRect::new(
                    layout.finish_btn.x + 22,
                    layout.finish_btn.y,
                    layout.finish_btn.width - 24,
                    layout.finish_btn.height,
                )
            } else {
                layout.finish_btn
            };
            draw_gdi_text(
                mem_dc,
                finish_label,
                finish_text_rect,
                dpi,
                12,
                600,
                rgb_colorref(255, 255, 255),
                DT_CENTER | DT_SINGLELINE | DT_VCENTER,
            );

            let _ = BitBlt(
                hdc,
                0,
                0,
                w_px as i32,
                h_px as i32,
                Some(mem_dc),
                0,
                0,
                SRCCOPY,
            );
            SelectObject(mem_dc, old_bmp);
            let _ = DeleteObject(hbmp.into());
        }
        let _ = DeleteDC(mem_dc);
    }

    let _ = EndPaint(hwnd, &ps);
}

unsafe extern "system" fn companion_wnd_proc(
    hwnd: HWND,
    umsg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if umsg == WM_CREATE {
        let cs = &*(lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW);
        windows::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW(
            hwnd,
            windows::Win32::UI::WindowsAndMessaging::GWLP_USERDATA,
            cs.lpCreateParams as isize,
        );
        SetTimer(Some(hwnd), TIMER_ANIM_ID, TIMER_INTERVAL_MS, None);
        return LRESULT(0);
    }

    let data_ptr = windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(
        hwnd,
        windows::Win32::UI::WindowsAndMessaging::GWLP_USERDATA,
    ) as *mut CompanionWindowData;

    if data_ptr.is_null() {
        return DefWindowProcW(hwnd, umsg, wparam, lparam);
    }
    let data = &mut *data_ptr;

    match umsg {
        WM_ERASEBKGND => LRESULT(1),
        WM_TIMER => {
            if wparam.0 == TIMER_ANIM_ID {
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            LRESULT(0)
        }
        WM_APP_UPDATE_STATE => {
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_APP_CLOSE_WINDOW => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_PAINT => {
            paint_companion_window(hwnd, data);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if !data.tracking_mouse_leave {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                if TrackMouseEvent(&mut tme).is_ok() {
                    data.tracking_mouse_leave = true;
                }
            }
            let (px, py) = unpack_lparam_point(lparam.0);
            let dpi = GetDpiForWindow(hwnd).max(96);
            let dip_x = physical_to_dip(px, dpi);
            let dip_y = physical_to_dip(py, dpi);
            let mut changed = false;
            if let Ok(mut s) = data.shared.lock() {
                let layout = companion_layout(s.finishing);
                let hc = layout.cancel_btn.contains(dip_x, dip_y);
                let hf = !s.finishing && layout.finish_btn.contains(dip_x, dip_y);
                if s.hover_cancel != hc || s.hover_finish != hf {
                    s.hover_cancel = hc;
                    s.hover_finish = hf;
                    changed = true;
                }
            }
            if changed {
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            data.tracking_mouse_leave = false;
            if let Ok(mut s) = data.shared.lock() {
                s.hover_cancel = false;
                s.hover_finish = false;
                s.pressed_cancel = false;
                s.pressed_finish = false;
            }
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_SETCURSOR => {
            let hit_btn = data
                .shared
                .lock()
                .map(|s| s.hover_cancel || s.hover_finish)
                .unwrap_or(false);
            if hit_btn {
                if let Ok(hand) = LoadCursorW(None, IDC_HAND) {
                    SetCursor(Some(hand));
                    return LRESULT(1);
                }
            }
            DefWindowProcW(hwnd, umsg, wparam, lparam)
        }
        WM_LBUTTONDOWN => {
            let (px, py) = unpack_lparam_point(lparam.0);
            let dpi = GetDpiForWindow(hwnd).max(96);
            let dip_x = physical_to_dip(px, dpi);
            let dip_y = physical_to_dip(py, dpi);
            if let Ok(mut s) = data.shared.lock() {
                let layout = companion_layout(s.finishing);
                s.pressed_cancel = layout.cancel_btn.contains(dip_x, dip_y);
                s.pressed_finish = !s.finishing && layout.finish_btn.contains(dip_x, dip_y);
            }
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (px, py) = unpack_lparam_point(lparam.0);
            let dpi = GetDpiForWindow(hwnd).max(96);
            let dip_x = physical_to_dip(px, dpi);
            let dip_y = physical_to_dip(py, dpi);
            let mut action = None;
            if let Ok(mut s) = data.shared.lock() {
                let layout = companion_layout(s.finishing);
                let do_cancel = s.pressed_cancel && layout.cancel_btn.contains(dip_x, dip_y);
                let do_finish =
                    s.pressed_finish && !s.finishing && layout.finish_btn.contains(dip_x, dip_y);
                s.pressed_cancel = false;
                s.pressed_finish = false;
                if do_cancel {
                    action = Some(CompanionAuthAction::Cancel);
                } else if do_finish {
                    s.finishing = true;
                    s.status_text = "Finishing and importing cookies...".to_string();
                    action = Some(CompanionAuthAction::FinishAndImport);
                }
            }
            let _ = InvalidateRect(Some(hwnd), None, false);
            if let Some(act) = action {
                (data.on_action)(act);
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let vk = wparam.0 as u16;
            if vk == VK_ESCAPE.0 {
                (data.on_action)(CompanionAuthAction::Cancel);
            } else if vk == VK_RETURN.0 {
                let mut should_fire = false;
                if let Ok(mut s) = data.shared.lock() {
                    if !s.finishing {
                        s.finishing = true;
                        s.status_text = "Finishing and importing cookies...".to_string();
                        should_fire = true;
                    }
                }
                if should_fire {
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    (data.on_action)(CompanionAuthAction::FinishAndImport);
                }
            }
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let suggested = &*(lparam.0 as *const RECT);
            let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowPos(
                hwnd,
                None,
                suggested.left,
                suggested.top,
                suggested.right - suggested.left,
                suggested.bottom - suggested.top,
                windows::Win32::UI::WindowsAndMessaging::SWP_NOZORDER
                    | windows::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE,
            );
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_CLOSE => {
            (data.on_action)(CompanionAuthAction::WindowClosed);
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = KillTimer(Some(hwnd), TIMER_ANIM_ID);
            data.closed.store(true, Ordering::Release);
            PostQuitMessage(0);
            LRESULT(0)
        }
        WM_NCDESTROY => {
            windows::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW(
                hwnd,
                windows::Win32::UI::WindowsAndMessaging::GWLP_USERDATA,
                0,
            );
            drop(Box::from_raw(data_ptr));
            DefWindowProcW(hwnd, umsg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, umsg, wparam, lparam),
    }
}
