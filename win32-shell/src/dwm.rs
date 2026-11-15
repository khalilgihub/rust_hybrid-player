//! Native Windows DWM window style & attribute helpers.
//!
//! Mirrors `hybrid-player/src/main/native-dwm.js`, `native-dwm.c`, and
//! `bin/hybrid-dwm-helper.cs` so frameless Windows keep native Aero-zoom
//! animations, hardware composition, and zero white/black resize flashes.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{
    DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMWINDOWATTRIBUTE,
};
use windows::Win32::Graphics::Gdi::{GetStockObject, SetWindowRgn, NULL_BRUSH};
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, GetWindowLongPtrW, IsWindow, SetClassLongPtrW, SetWindowLongPtrW,
    SetWindowPos, GCLP_HBRBACKGROUND, GWL_STYLE, SET_WINDOW_POS_FLAGS, WS_CAPTION, WS_CLIPCHILDREN,
    WS_CLIPSIBLINGS, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_SYSMENU, WS_THICKFRAME,
};

use crate::error::{Result, Win32ShellError};

/// Theme mode controlling DWM immersive dark mode (`DWMWA_USE_IMMERSIVE_DARK_MODE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeMode {
    #[default]
    Dark,
    Oled,
    Light,
}

impl ThemeMode {
    pub fn from_pref(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "light" => Self::Light,
            "oled" => Self::Oled,
            _ => Self::Dark,
        }
    }

    /// Returns `1` for `dark`/`oled` and `0` for `light`.
    pub fn immersive_dark_mode_value(self) -> i32 {
        match self {
            Self::Dark | Self::Oled => 1,
            Self::Light => 0,
        }
    }
}

/// Exact Win32 window style mask applied to the frameless main window.
pub const FRAMELESS_WINDOW_STYLE_MASK: isize = (WS_THICKFRAME.0
    | WS_CAPTION.0
    | WS_MINIMIZEBOX.0
    | WS_MAXIMIZEBOX.0
    | WS_SYSMENU.0
    | WS_CLIPCHILDREN.0
    | WS_CLIPSIBLINGS.0) as isize;

/// Child surface style mask (`WS_CLIPCHILDREN | WS_CLIPSIBLINGS`).
pub const CHILD_WINDOW_STYLE_MASK: isize = (WS_CLIPCHILDREN.0 | WS_CLIPSIBLINGS.0) as isize;

// Exact DWM attribute IDs & values matching `native-dwm.js` / `native-dwm.c` / `hybrid-dwm-helper.cs`.
pub const DWMWA_NCRENDERING_POLICY_ID: u32 = 2;
pub const DWMNCRP_ENABLED_VAL: i32 = 2;

pub const DWMWA_TRANSITIONS_FORCEDISABLED_ID: u32 = 3;
pub const DWMWA_TRANSITIONS_ENABLED_VAL: i32 = 0;

pub const DWMWA_USE_IMMERSIVE_DARK_MODE_LEGACY_ID: u32 = 19;
pub const DWMWA_USE_IMMERSIVE_DARK_MODE_ID: u32 = 20;

pub const DWMWA_WINDOW_CORNER_PREFERENCE_ID: u32 = 33;
pub const DWMWCP_DONOTROUND_VAL: i32 = 1;

pub const DWMWA_BORDER_COLOR_ID: u32 = 34;
pub const DWMWA_COLOR_NONE_VAL: u32 = 0xFFFF_FFFE;

pub const DWMWA_CAPTION_COLOR_ID: u32 = 35;
pub const DWMWA_TEXT_COLOR_ID: u32 = 36;

pub const DWMWA_SYSTEMBACKDROP_TYPE_ID: u32 = 38;
pub const DWMSBT_NONE_VAL: i32 = 1;

/// `SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER`
/// Synchronous frame update so immediate `ShowWindow(SW_MAXIMIZE)` / fullscreen transitions never race.
pub const SWP_REFRESH_FRAME_FLAGS: u32 = 0x0237;

/// Extract raw Win32 `HWND` from a `winit::window::Window`.
pub fn extract_hwnd_from_winit(window: &winit::window::Window) -> Result<HWND> {
    let handle = window
        .window_handle()
        .map_err(|e| Win32ShellError::WindowHandle(e.to_string()))?;
    match handle.as_raw() {
        RawWindowHandle::Win32(h) => Ok(HWND(h.hwnd.get() as *mut c_void)),
        other => Err(Win32ShellError::WindowHandle(format!(
            "expected Win32 handle, got {other:?}"
        ))),
    }
}

/// Convert an integer handle value into a Win32 `HWND`.
#[inline]
pub fn hwnd_from_raw(raw: isize) -> HWND {
    HWND(raw as *mut c_void)
}

/// Convert a Win32 `HWND` into an `isize`.
#[inline]
pub fn hwnd_to_raw(hwnd: HWND) -> isize {
    hwnd.0 as isize
}

fn set_dwm_i32(hwnd: HWND, attr: u32, value: i32) -> Result<()> {
    let hr = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWINDOWATTRIBUTE(attr as i32),
            (&value as *const i32).cast(),
            std::mem::size_of::<i32>() as u32,
        )
    };
    hr.map_err(|e| Win32ShellError::HResult {
        api: "DwmSetWindowAttribute",
        hr: e.code().0,
        message: e.message(),
    })
}

fn set_dwm_u32(hwnd: HWND, attr: u32, value: u32) -> Result<()> {
    let hr = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWINDOWATTRIBUTE(attr as i32),
            (&value as *const u32).cast(),
            std::mem::size_of::<u32>() as u32,
        )
    };
    hr.map_err(|e| Win32ShellError::HResult {
        api: "DwmSetWindowAttribute",
        hr: e.code().0,
        message: e.message(),
    })
}

/// Set the window class background brush to `NULL_BRUSH` (stock object 5)
/// to eliminate white/black GDI flashes during creation, move, or drag.
pub fn apply_null_brush(hwnd: HWND) -> Result<()> {
    if hwnd.0.is_null() || unsafe { !IsWindow(Some(hwnd)).as_bool() } {
        return Err(Win32ShellError::InvalidHwnd);
    }
    unsafe {
        let null_brush = GetStockObject(NULL_BRUSH);
        SetClassLongPtrW(hwnd, GCLP_HBRBACKGROUND, null_brush.0 as isize);
    }
    Ok(())
}

unsafe extern "system" fn enum_child_style_proc(child_hwnd: HWND, _lparam: LPARAM) -> BOOL {
    let cur_style = GetWindowLongPtrW(child_hwnd, GWL_STYLE);
    SetWindowLongPtrW(child_hwnd, GWL_STYLE, cur_style | CHILD_WINDOW_STYLE_MASK);
    let null_brush = GetStockObject(NULL_BRUSH);
    SetClassLongPtrW(child_hwnd, GCLP_HBRBACKGROUND, null_brush.0 as isize);
    BOOL(1)
}

/// Apply `WS_CLIPCHILDREN | WS_CLIPSIBLINGS` and `NULL_BRUSH` to all child HWNDs
/// (e.g. DirectX swapchain / mpv child surfaces).
pub fn style_child_surfaces(hwnd: HWND) {
    if hwnd.0.is_null() || unsafe { !IsWindow(Some(hwnd)).as_bool() } {
        return;
    }
    unsafe {
        let _ = EnumChildWindows(Some(hwnd), Some(enum_child_style_proc), LPARAM(0));
    }
}

/// Apply all exact DWM attributes and 1px frame extension without altering window styles.
pub fn apply_dwm_attributes(hwnd: HWND, theme: ThemeMode) -> Result<()> {
    if hwnd.0.is_null() || unsafe { !IsWindow(Some(hwnd)).as_bool() } {
        return Err(Win32ShellError::InvalidHwnd);
    }

    let dark_val = theme.immersive_dark_mode_value();

    // Order matches native-dwm.js & hybrid-dwm-helper.cs
    let _ = set_dwm_i32(
        hwnd,
        DWMWA_TRANSITIONS_FORCEDISABLED_ID,
        DWMWA_TRANSITIONS_ENABLED_VAL,
    );
    let _ = set_dwm_i32(hwnd, DWMWA_NCRENDERING_POLICY_ID, DWMNCRP_ENABLED_VAL);
    let _ = set_dwm_i32(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE_ID, dark_val);
    let _ = set_dwm_i32(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE_LEGACY_ID, dark_val);
    let _ = set_dwm_i32(
        hwnd,
        DWMWA_WINDOW_CORNER_PREFERENCE_ID,
        DWMWCP_DONOTROUND_VAL,
    );
    let _ = set_dwm_u32(hwnd, DWMWA_BORDER_COLOR_ID, DWMWA_COLOR_NONE_VAL);
    let _ = set_dwm_u32(hwnd, DWMWA_CAPTION_COLOR_ID, 0);
    let _ = set_dwm_u32(hwnd, DWMWA_TEXT_COLOR_ID, 0);
    let _ = set_dwm_i32(hwnd, DWMWA_SYSTEMBACKDROP_TYPE_ID, DWMSBT_NONE_VAL);

    // Extend DWM frame by 1px on all 4 borders to preserve hardware composition on resize edges.
    let margins = MARGINS {
        cxLeftWidth: 1,
        cxRightWidth: 1,
        cyTopHeight: 1,
        cyBottomHeight: 1,
    };
    let _ = unsafe { DwmExtendFrameIntoClientArea(hwnd, &margins) };
    Ok(())
}

/// Apply native Win32 window styles, `NULL_BRUSH`, child surface clipping,
/// DWM attributes, `SetWindowPos(..., 0x4237)`, and post-SWP corner preference re-application.
///
/// Mirrors `applyNativeDwmWindowStyles` in `native-dwm.js`:
/// - Skips immediately if `is_resizing` or `is_fullscreen` is true (fullscreen windows
///   must remain pure borderless popups without `WS_CAPTION`).
pub fn apply_native_dwm_window_styles(
    hwnd: HWND,
    theme: ThemeMode,
    is_resizing: bool,
    is_fullscreen: bool,
) -> Result<bool> {
    if is_resizing || is_fullscreen {
        return Ok(false);
    }
    if hwnd.0.is_null() || unsafe { !IsWindow(Some(hwnd)).as_bool() } {
        return Err(Win32ShellError::InvalidHwnd);
    }

    unsafe {
        let cur_style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        let new_style = cur_style | FRAMELESS_WINDOW_STYLE_MASK;
        if new_style != cur_style {
            SetWindowLongPtrW(hwnd, GWL_STYLE, new_style);
        }

        let null_brush = GetStockObject(NULL_BRUSH);
        SetClassLongPtrW(hwnd, GCLP_HBRBACKGROUND, null_brush.0 as isize);

        // Clear any clipping region so the window remains a sharp complete rectangle.
        let _ = SetWindowRgn(hwnd, None, true);
    }

    style_child_surfaces(hwnd);
    apply_dwm_attributes(hwnd, theme)?;

    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SET_WINDOW_POS_FLAGS(SWP_REFRESH_FRAME_FLAGS),
        );
    }

    // Re-apply corner preference after SWP_FRAMECHANGED to guarantee Win11 keeps corners square.
    let _ = set_dwm_i32(
        hwnd,
        DWMWA_WINDOW_CORNER_PREFERENCE_ID,
        DWMWCP_DONOTROUND_VAL,
    );

    Ok(true)
}

/// Guard state shared with scheduled DWM re-application timers.
#[derive(Debug, Default)]
pub struct DwmScheduleGuard {
    pub is_resizing: AtomicBool,
    pub is_fullscreen: AtomicBool,
    pub destroyed: AtomicBool,
}

/// Schedule DWM style application at `[0, 80, 200, 350]` ms (matching `scheduleNativeDwmWindowStyles`).
pub fn schedule_native_dwm_window_styles(
    hwnd: HWND,
    theme: ThemeMode,
    guard: Arc<DwmScheduleGuard>,
    delays_ms: &[u64],
) {
    let raw_hwnd = hwnd_to_raw(hwnd);
    for &delay in delays_ms {
        if delay == 0 {
            if !guard.destroyed.load(Ordering::Acquire) {
                let _ = apply_native_dwm_window_styles(
                    hwnd_from_raw(raw_hwnd),
                    theme,
                    guard.is_resizing.load(Ordering::Acquire),
                    guard.is_fullscreen.load(Ordering::Acquire),
                );
            }
        } else {
            let guard_clone = Arc::clone(&guard);
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(delay));
                if guard_clone.destroyed.load(Ordering::Acquire) {
                    return;
                }
                let _ = apply_native_dwm_window_styles(
                    hwnd_from_raw(raw_hwnd),
                    theme,
                    guard_clone.is_resizing.load(Ordering::Acquire),
                    guard_clone.is_fullscreen.load(Ordering::Acquire),
                );
            });
        }
    }
}

/// Helper for computing window rect size on a monitor (useful in tests & creation).
pub fn window_rect(hwnd: HWND) -> Option<RECT> {
    if hwnd.0.is_null() {
        return None;
    }
    let mut rc = RECT::default();
    if unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut rc) }.is_ok() {
        Some(rc)
    } else {
        None
    }
}
