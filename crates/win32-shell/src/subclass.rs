//! Win32 `HWND` subclassing (`SetWindowSubclass`), non-client hit testing,
//! resize/move guards, Aero-zoom, drag-from-maximize, and multi-monitor DPI.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, ScreenToClient, MONITORINFO,
    MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, TrackMouseEvent, TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT,
};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetCursorPos, GetWindowLongPtrW, GetWindowPlacement, GetWindowRect, IsIconic,
    IsZoomed, SetWindowLongPtrW, SetWindowPlacement, SetWindowPos, ShowWindow, GWL_STYLE, HTBOTTOM,
    HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION, HTCLIENT, HTLEFT, HTMAXBUTTON, HTRIGHT, HTTOP,
    HTTOPLEFT, HTTOPRIGHT, HWND_TOP, MINMAXINFO, NCCALCSIZE_PARAMS, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOZORDER, SWP_SHOWWINDOW, SW_MAXIMIZE, SW_MINIMIZE,
    SW_RESTORE, SW_SHOWMAXIMIZED, WINDOWPLACEMENT, WM_ACTIVATE, WM_CAPTURECHANGED, WM_DPICHANGED,
    WM_ENTERSIZEMOVE, WM_ERASEBKGND, WM_EXITSIZEMOVE, WM_GETMINMAXINFO, WM_LBUTTONUP,
    WM_NCACTIVATE, WM_NCCALCSIZE, WM_NCDESTROY, WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN,
    WM_NCLBUTTONUP, WM_NCMOUSEMOVE, WM_NCPAINT, WM_SIZE, WM_SIZING, WS_THICKFRAME, WVR_REDRAW,
};

use crate::dwm::{
    apply_native_dwm_window_styles, schedule_native_dwm_window_styles, DwmScheduleGuard, ThemeMode,
};
use crate::error::{Result, Win32ShellError};
use crate::os_integration::set_high_res_timer;

pub const RESIZE_BORDER_DIP: i32 = 8;
pub const MIN_WINDOW_WIDTH_DIP: i32 = 640;
pub const MIN_WINDOW_HEIGHT_DIP: i32 = 360;

// Drag-from-maximize constants matching `ipc-handlers.js:15-18`.
pub const TITLEBAR_DRAG_VERTICAL_OFFSET: i32 = 14;
pub const TITLEBAR_DRAG_MIN_VISIBLE_HEIGHT: i32 = 80;
pub const MIN_RESTORE_WIDTH: i32 = 800;
pub const MIN_RESTORE_HEIGHT: i32 = 500;
pub const DEFAULT_WINDOWED_WIDTH: i32 = 1280;
pub const DEFAULT_WINDOWED_HEIGHT: i32 = 720;
const WM_NCMOUSELEAVE: u32 = 0x02A2;
const WM_SYSCOMMAND: u32 = 0x0112;
const SC_MAXIMIZE: u32 = 0xF030;
const SC_RESTORE: u32 = 0xF120;
const SIZE_MINIMIZED: usize = 1;
const SIZE_MAXIMIZED: usize = 2;

const SUBCLASS_UID: usize = 0x4859_4252; // "HYBR"

/// A rectangle in DIP (device-independent pixel) client coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DipRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl DipRect {
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn contains(&self, px: i32, py: i32) -> bool {
        self.width > 0
            && self.height > 0
            && px >= self.x
            && py >= self.y
            && px < self.x.saturating_add(self.width)
            && py < self.y.saturating_add(self.height)
    }
}

/// Configuration for `WM_NCHITTEST` resolution in DIP coordinates.
#[derive(Debug, Clone)]
pub struct HitTestConfig {
    pub maximize_button_rect: Option<DipRect>,
    pub exclusions: Vec<DipRect>,
    pub titlebar_drag_rect: Option<DipRect>,
    pub resize_border_dip: i32,
    pub is_maximized: bool,
    pub is_fullscreen: bool,
    pub is_minimized: bool,
    pub is_resizable: bool,
}

impl Default for HitTestConfig {
    fn default() -> Self {
        Self {
            maximize_button_rect: None,
            exclusions: Vec::new(),
            titlebar_drag_rect: None,
            resize_border_dip: RESIZE_BORDER_DIP,
            is_maximized: false,
            is_fullscreen: false,
            is_minimized: false,
            is_resizable: true,
        }
    }
}

/// Pure hit-test evaluator matching `main.js:1639-1697`.
///
/// Precedence:
/// 1. Outside window -> `None`
/// 2. `maximize_button_rect` -> `HTMAXBUTTON` (enables Windows 11 Snap Layouts flyout)
/// 3. `exclusions` (UI controls) -> `HTCLIENT`
/// 4. If maximized or fullscreen: 8-DIP edges -> `HTCLIENT` (locks resize borders);
///    titlebar drag region -> `HTCAPTION` (when maximized) or `HTCLIENT` (when fullscreen)
/// 5. If windowed & resizable: 8-DIP corners/edges (`HTTOPLEFT`..`HTBOTTOMRIGHT`)
/// 6. `titlebar_drag_rect` -> `HTCAPTION`
/// 7. Fallback -> `HTCLIENT`
pub fn resolve_nc_hit_test(
    local_dip_x: i32,
    local_dip_y: i32,
    window_dip_w: i32,
    window_dip_h: i32,
    cfg: &HitTestConfig,
) -> Option<isize> {
    if cfg.is_minimized || window_dip_w <= 0 || window_dip_h <= 0 {
        return None;
    }

    let is_max_or_fs = cfg.is_maximized || cfg.is_fullscreen;
    if !cfg.is_resizable && !is_max_or_fs {
        return None;
    }

    if local_dip_x < 0
        || local_dip_y < 0
        || local_dip_x > window_dip_w
        || local_dip_y > window_dip_h
    {
        return None;
    }

    if let Some(max_rect) = cfg.maximize_button_rect {
        if max_rect.contains(local_dip_x, local_dip_y) {
            return Some(HTMAXBUTTON as isize);
        }
    }

    if cfg
        .exclusions
        .iter()
        .any(|rect| rect.contains(local_dip_x, local_dip_y))
    {
        return Some(HTCLIENT as isize);
    }

    let border = cfg.resize_border_dip.max(1);
    let left = local_dip_x <= border;
    let right = local_dip_x >= window_dip_w - border;
    let top = local_dip_y <= border;
    let bottom = local_dip_y >= window_dip_h - border;

    if is_max_or_fs {
        if top || left || bottom || right {
            return Some(HTCLIENT as isize);
        }
        if !cfg.is_fullscreen {
            if let Some(drag_rect) = cfg.titlebar_drag_rect {
                if drag_rect.contains(local_dip_x, local_dip_y) {
                    return Some(HTCAPTION as isize);
                }
            }
        }
        return Some(HTCLIENT as isize);
    }

    if top && left {
        return Some(HTTOPLEFT as isize);
    }
    if top && right {
        return Some(HTTOPRIGHT as isize);
    }
    if bottom && left {
        return Some(HTBOTTOMLEFT as isize);
    }
    if bottom && right {
        return Some(HTBOTTOMRIGHT as isize);
    }
    if top {
        return Some(HTTOP as isize);
    }
    if left {
        return Some(HTLEFT as isize);
    }
    if bottom {
        return Some(HTBOTTOM as isize);
    }
    if right {
        return Some(HTRIGHT as isize);
    }

    if let Some(drag_rect) = cfg.titlebar_drag_rect {
        if drag_rect.contains(local_dip_x, local_dip_y) {
            return Some(HTCAPTION as isize);
        }
    }

    Some(HTCLIENT as isize)
}

/// Unpack signed 16-bit `(x, y)` coordinates from Win32 `LPARAM` (`GET_X_LPARAM` / `GET_Y_LPARAM`).
#[inline]
pub fn unpack_lparam_point(lparam: isize) -> (i32, i32) {
    let raw = lparam as u32;
    let x = (raw & 0xFFFF) as u16 as i16 as i32;
    let y = ((raw >> 16) & 0xFFFF) as u16 as i16 as i32;
    (x, y)
}

/// Compute DPI scale factor from a Win32 DPI value (`96` -> `1.0`).
#[inline]
pub fn scale_for_dpi(dpi: u32) -> f64 {
    if dpi == 0 {
        1.0
    } else {
        dpi as f64 / 96.0
    }
}

/// Convert physical pixels to DIPs using Win32 DPI.
#[inline]
pub fn physical_to_dip(px: i32, dpi: u32) -> i32 {
    (px as f64 / scale_for_dpi(dpi)).round() as i32
}

/// Convert DIPs to physical pixels using Win32 DPI.
#[inline]
pub fn dip_to_physical(dip: i32, dpi: u32) -> i32 {
    (dip as f64 * scale_for_dpi(dpi)).round() as i32
}

/// Query the DPI of a window, falling back to `96` if invalid.
pub fn dpi_for_window(hwnd: HWND) -> u32 {
    if hwnd.0.is_null() {
        return 96;
    }
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 {
        96
    } else {
        dpi
    }
}

/// Monitor work area and full bounds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonitorMetrics {
    pub monitor: RECT,
    pub work: RECT,
}

/// Query `MONITORINFO` for the monitor nearest to `hwnd`.
pub fn monitor_info_for_window(hwnd: HWND) -> Option<MonitorMetrics> {
    let hmon = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    if hmon.is_invalid() {
        return None;
    }
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if unsafe { GetMonitorInfoW(hmon, &mut mi) }.as_bool() {
        Some(MonitorMetrics {
            monitor: mi.rcMonitor,
            work: mi.rcWork,
        })
    } else {
        None
    }
}

/// Query `MONITORINFO` for the monitor nearest to a physical screen point.
pub fn monitor_info_for_point(x: i32, y: i32) -> Option<MonitorMetrics> {
    let hmon = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) };
    if hmon.is_invalid() {
        return None;
    }
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if unsafe { GetMonitorInfoW(hmon, &mut mi) }.as_bool() {
        Some(MonitorMetrics {
            monitor: mi.rcMonitor,
            work: mi.rcWork,
        })
    } else {
        None
    }
}

/// Compute the client rect for `WM_NCCALCSIZE` (`wParam != 0`) so frameless
/// windows retain zero non-client borders when normal and stay strictly within
/// `monitor_work` when maximized (eliminating Windows' 8px maximized overhang).
pub fn compute_nccalcsize_client_rect(
    proposed: RECT,
    monitor_work: RECT,
    monitor_full: RECT,
    is_maximized: bool,
    is_fullscreen: bool,
) -> RECT {
    if is_fullscreen {
        return monitor_full;
    }
    if is_maximized {
        return monitor_work;
    }
    proposed
}

/// Logical/physical bounds helper for drag-from-maximize.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowBounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Active titlebar drag-from-maximize session (`ipc-handlers.js:589-741`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TitlebarDragSession {
    pub active: bool,
    pub width: i32,
    pub height: i32,
    pub offset_x: i32,
    pub offset_y: i32,
}

/// Compute restored window size during drag-from-maximize.
pub fn compute_drag_restore_size(
    saved_restore: Option<WindowBounds>,
    work_area: RECT,
) -> (i32, i32) {
    let wa_w = (work_area.right - work_area.left).max(1);
    let wa_h = (work_area.bottom - work_area.top).max(1);
    if let Some(src) = saved_restore {
        if src.width > 0 && src.height > 0 {
            return (
                src.width.clamp(MIN_RESTORE_WIDTH.min(wa_w), wa_w),
                src.height.clamp(MIN_RESTORE_HEIGHT.min(wa_h), wa_h),
            );
        }
    }
    let w = ((wa_w as f64) * 0.72).round() as i32;
    let h = ((wa_h as f64) * 0.72).round() as i32;
    (
        w.clamp(MIN_RESTORE_WIDTH.min(wa_w), wa_w),
        h.clamp(MIN_RESTORE_HEIGHT.min(wa_h), wa_h),
    )
}

/// Clamp dragged window bounds to the target monitor work area (`ipc-handlers.js:605-622`).
pub fn clamp_dragged_bounds(desired: WindowBounds, work_area: RECT) -> WindowBounds {
    let wa_w = (work_area.right - work_area.left).max(1);
    let wa_h = (work_area.bottom - work_area.top).max(1);
    let width = desired.width.clamp(MIN_RESTORE_WIDTH.min(wa_w), wa_w);
    let height = desired.height.clamp(MIN_RESTORE_HEIGHT.min(wa_h), wa_h);
    let max_x = work_area.left + wa_w - 40;
    let min_x = work_area.left - width + 40;
    let max_y = work_area.top + wa_h - height.min(TITLEBAR_DRAG_MIN_VISIBLE_HEIGHT);
    let x = desired.x.clamp(min_x, max_x);
    let y = desired.y.clamp(work_area.top, max_y);
    WindowBounds {
        x,
        y,
        width,
        height,
    }
}

/// Compute initial drag-from-maximize bounds and cursor anchor offset.
pub fn compute_drag_from_maximize_start(
    pointer_x: i32,
    pointer_y: i32,
    display_bounds: RECT,
    work_area: RECT,
    saved_restore: Option<WindowBounds>,
) -> (WindowBounds, TitlebarDragSession, f64) {
    let disp_w = (display_bounds.right - display_bounds.left).max(1);
    let cursor_ratio_x = ((pointer_x - display_bounds.left) as f64 / disp_w as f64).clamp(0.0, 1.0);
    let (restore_w, restore_h) = compute_drag_restore_size(saved_restore, work_area);

    let desired = WindowBounds {
        x: (pointer_x as f64 - (restore_w as f64) * cursor_ratio_x).round() as i32,
        y: pointer_y - TITLEBAR_DRAG_VERTICAL_OFFSET,
        width: restore_w,
        height: restore_h,
    };
    let clamped = clamp_dragged_bounds(desired, work_area);
    let offset_x = (pointer_x - clamped.x).clamp(0, (clamped.width - 1).max(0));
    let offset_y = (pointer_y - clamped.y).clamp(0, (clamped.height - 1).max(0));

    let session = TitlebarDragSession {
        active: true,
        width: clamped.width,
        height: clamped.height,
        offset_x,
        offset_y,
    };
    (clamped, session, cursor_ratio_x)
}

/// Events emitted from the Win32 subclass to the application layer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShellWindowEvent {
    BoundsAnimating(bool),
    MaximizedChanged(bool),
    FullscreenChanged(bool),
    DpiChanged { dpi: u32, scale_factor: f64 },
    MaxButtonHovered(bool),
    MaxButtonClicked,
}

/// Callback invoked for shell window lifecycle events (DPI change, size, close, restore).
pub type ShellEventCallback = Box<dyn FnMut(ShellWindowEvent) + Send>;

/// Shared mutable state accessed by `WM_*` subclass handlers and app code.
pub struct SubclassState {
    pub hwnd: isize,
    pub theme: Mutex<ThemeMode>,
    pub hit_test: Mutex<HitTestConfig>,
    pub is_resizing: AtomicBool,
    pub is_fullscreen: AtomicBool,
    pub updating_fullscreen: AtomicBool,
    pub is_clamping_bounds: AtomicBool,
    pub was_maximized: AtomicBool,
    pub max_button_hovered: AtomicBool,
    pub max_button_pressed: AtomicBool,
    pub schedule_guard: Arc<DwmScheduleGuard>,
    pub restore_bounds: Mutex<Option<WindowBounds>>,
    pub drag_session: Mutex<Option<TitlebarDragSession>>,
    pub pre_fullscreen_maximized: AtomicBool,
    pub pre_fullscreen_bounds: Mutex<Option<WindowBounds>>,
    pub pre_fullscreen_wp: Mutex<Option<WINDOWPLACEMENT>>,
    pub on_event: Mutex<Option<ShellEventCallback>>,
}

impl SubclassState {
    pub fn new(hwnd: HWND, theme: ThemeMode) -> Self {
        Self {
            hwnd: hwnd.0 as isize,
            theme: Mutex::new(theme),
            hit_test: Mutex::new(HitTestConfig::default()),
            is_resizing: AtomicBool::new(false),
            is_fullscreen: AtomicBool::new(false),
            updating_fullscreen: AtomicBool::new(false),
            is_clamping_bounds: AtomicBool::new(false),
            was_maximized: AtomicBool::new(false),
            max_button_hovered: AtomicBool::new(false),
            max_button_pressed: AtomicBool::new(false),
            schedule_guard: Arc::new(DwmScheduleGuard::default()),
            restore_bounds: Mutex::new(None),
            drag_session: Mutex::new(None),
            pre_fullscreen_maximized: AtomicBool::new(false),
            pre_fullscreen_bounds: Mutex::new(None),
            pre_fullscreen_wp: Mutex::new(None),
            on_event: Mutex::new(None),
        }
    }

    fn emit(&self, ev: ShellWindowEvent) {
        if let Ok(mut cb) = self.on_event.lock() {
            if let Some(f) = cb.as_mut() {
                f(ev);
            }
        }
    }
}

/// Handle owning the installed `SetWindowSubclass` hook and Win32 window operations.
pub struct WindowSubclass {
    hwnd: HWND,
    state: Arc<SubclassState>,
}

impl WindowSubclass {
    /// Install `SetWindowSubclass` on `hwnd`, apply initial `NULL_BRUSH`, Win32 styles,
    /// and DWM attributes, and schedule deferred style reinforcement `[0, 80, 200, 350]`.
    pub fn install(hwnd: HWND, theme: ThemeMode) -> Result<Self> {
        if hwnd.0.is_null() {
            return Err(Win32ShellError::InvalidHwnd);
        }

        let state = Arc::new(SubclassState::new(hwnd, theme));
        let raw_ptr = Arc::into_raw(Arc::clone(&state)) as usize;

        let ok = unsafe { SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_UID, raw_ptr) };
        if !ok.as_bool() {
            // Reclaim leaked Arc on failure
            unsafe {
                let _ = Arc::from_raw(raw_ptr as *const SubclassState);
            }
            return Err(Win32ShellError::Win32 {
                api: "SetWindowSubclass",
                code: 0,
            });
        }

        // Apply styles & DWM immediately and on the Electron-proven schedule [0, 80, 200, 350]
        schedule_native_dwm_window_styles(
            hwnd,
            theme,
            Arc::clone(&state.schedule_guard),
            &[0, 80, 200, 350],
        );

        Ok(Self { hwnd, state })
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub fn set_event_callback<F>(&self, cb: F)
    where
        F: FnMut(ShellWindowEvent) + Send + 'static,
    {
        if let Ok(mut guard) = self.state.on_event.lock() {
            *guard = Some(Box::new(cb));
        }
    }

    pub fn set_theme(&self, theme: ThemeMode) -> Result<()> {
        if let Ok(mut t) = self.state.theme.lock() {
            *t = theme;
        }
        let is_resizing = self.state.is_resizing.load(Ordering::Acquire);
        let is_fs = self.state.is_fullscreen.load(Ordering::Acquire);
        let _ = apply_native_dwm_window_styles(self.hwnd, theme, is_resizing, is_fs)?;
        Ok(())
    }

    pub fn set_maximize_button_rect(&self, rect: Option<DipRect>) {
        if let Ok(mut ht) = self.state.hit_test.lock() {
            ht.maximize_button_rect = rect;
        }
    }

    pub fn set_hit_test_exclusions(&self, rects: Vec<DipRect>) {
        if let Ok(mut ht) = self.state.hit_test.lock() {
            ht.exclusions = rects.into_iter().take(24).collect();
        }
    }

    pub fn set_titlebar_drag_rect(&self, rect: Option<DipRect>) {
        if let Ok(mut ht) = self.state.hit_test.lock() {
            ht.titlebar_drag_rect = rect;
        }
    }

    pub fn is_resizing(&self) -> bool {
        self.state.is_resizing.load(Ordering::Acquire)
    }

    pub fn is_fullscreen(&self) -> bool {
        self.state.is_fullscreen.load(Ordering::Acquire)
    }

    pub fn is_maximized(&self) -> bool {
        if self.is_fullscreen() {
            return false;
        }
        unsafe { IsZoomed(self.hwnd).as_bool() }
    }

    /// Native Aero-zoom maximize.
    pub fn maximize(&self) {
        if self.is_fullscreen() {
            return;
        }
        let mon = monitor_info_for_window(self.hwnd);
        let is_normal = unsafe { !IsZoomed(self.hwnd).as_bool() && !IsIconic(self.hwnd).as_bool() };
        let current_valid = if is_normal {
            get_window_bounds(self.hwnd)
                .filter(|b| mon.is_none_or(|m| !bounds_fill_work_or_monitor(*b, m)))
        } else {
            None
        };
        let normal_valid = get_normal_window_bounds(self.hwnd)
            .filter(|b| mon.is_none_or(|m| !bounds_fill_work_or_monitor(*b, m)));
        if let Ok(mut rb) = self.state.restore_bounds.lock() {
            let existing_valid =
                rb.filter(|b| mon.is_none_or(|m| !bounds_fill_work_or_monitor(*b, m)));
            *rb = current_valid.or(normal_valid).or(existing_valid);
        }
        set_high_res_timer(true);
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_MAXIMIZE);
        }
        set_high_res_timer(false);
    }

    /// Native Aero-zoom restore (exits fullscreen to windowed if currently fullscreen).
    pub fn restore(&self) {
        if self.is_fullscreen() {
            let _ = self.exit_fullscreen_to_windowed();
            return;
        }
        let was_iconic = unsafe { IsIconic(self.hwnd).as_bool() };
        let was_zoomed = unsafe { IsZoomed(self.hwnd).as_bool() };
        set_high_res_timer(true);
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_RESTORE);
        }
        if !was_iconic {
            let now_iconic = unsafe { IsIconic(self.hwnd).as_bool() };
            let now_zoomed = unsafe { IsZoomed(self.hwnd).as_bool() };
            let fills_work = get_window_bounds(self.hwnd)
                .zip(monitor_info_for_window(self.hwnd))
                .is_some_and(|(b, m)| bounds_fill_work_or_monitor(b, m));
            if !now_iconic && !now_zoomed && (was_zoomed || fills_work) {
                apply_restored_window_bounds(self.hwnd, &self.state);
            } else {
                clamp_window_to_work_area(self.hwnd, &self.state);
            }
        }
        set_high_res_timer(false);
    }

    /// Native Aero-zoom minimize.
    pub fn minimize(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_MINIMIZE);
        }
    }

    /// Toggle maximize/restore (exits fullscreen if currently fullscreen).
    pub fn toggle_maximize(&self) -> bool {
        if self.is_fullscreen() {
            let _ = self.exit_fullscreen_to_windowed();
            return self.is_maximized();
        }
        if self.is_maximized() {
            self.restore();
            false
        } else {
            self.maximize();
            true
        }
    }

    /// Enter or exit borderless fullscreen, preserving pre-fullscreen maximized/normal bounds
    /// and ensuring `WS_CAPTION` / DWM styles are NEVER applied while fullscreen.
    pub fn set_fullscreen(&self, fullscreen: bool) -> Result<bool> {
        let was_fs = self.is_fullscreen();
        if was_fs == fullscreen {
            return Ok(fullscreen);
        }

        if fullscreen {
            let mon = monitor_info_for_window(self.hwnd);
            let was_max = unsafe { IsZoomed(self.hwnd).as_bool() };
            let was_iconic = unsafe { IsIconic(self.hwnd).as_bool() };
            self.state
                .pre_fullscreen_maximized
                .store(was_max, Ordering::Release);

            let valid_wp = capture_window_placement(self.hwnd).filter(|wp| {
                let rc = wp.rcNormalPosition;
                let w = rc.right - rc.left;
                let h = rc.bottom - rc.top;
                if w <= 0 || h <= 0 {
                    return false;
                }
                let b = WindowBounds {
                    x: rc.left,
                    y: rc.top,
                    width: w,
                    height: h,
                };
                mon.is_none_or(|m| !bounds_fill_work_or_monitor(b, m))
            });
            if let Ok(mut pfwp) = self.state.pre_fullscreen_wp.lock() {
                *pfwp = valid_wp;
            }

            let current_normal = if !was_max && !was_iconic {
                get_window_bounds(self.hwnd)
                    .filter(|b| mon.is_none_or(|m| !bounds_fill_work_or_monitor(*b, m)))
            } else {
                None
            };
            let saved_normal = current_normal
                .or_else(|| {
                    get_normal_window_bounds(self.hwnd)
                        .filter(|b| mon.is_none_or(|m| !bounds_fill_work_or_monitor(*b, m)))
                })
                .or_else(|| {
                    self.state
                        .restore_bounds
                        .lock()
                        .ok()
                        .and_then(|g| *g)
                        .filter(|b| mon.is_none_or(|m| !bounds_fill_work_or_monitor(*b, m)))
                });
            if let Ok(mut pfb) = self.state.pre_fullscreen_bounds.lock() {
                *pfb = saved_normal;
            }
            if let Some(valid) = saved_normal {
                if let Ok(mut rb) = self.state.restore_bounds.lock() {
                    *rb = Some(valid);
                }
            }
            self.state
                .updating_fullscreen
                .store(true, Ordering::Release);
            self.state.is_fullscreen.store(true, Ordering::Release);
            self.state.was_maximized.store(false, Ordering::Release);
            self.state
                .schedule_guard
                .is_fullscreen
                .store(true, Ordering::Release);
            if let Ok(mut ht) = self.state.hit_test.lock() {
                ht.is_fullscreen = true;
                ht.is_maximized = false;
            }

            if was_max {
                // Clear WS_MAXIMIZE before stretching to the monitor rect so Windows
                // does not clamp/offset a maximized frame on exit/DPI change.
                unsafe {
                    let _ = ShowWindow(self.hwnd, SW_RESTORE);
                }
            }

            // Strip WS_CAPTION while keeping WS_THICKFRAME (locked in WM_NCHITTEST),
            // then raise to HWND_TOP across the full monitor rect so Windows hides the taskbar.
            unsafe {
                let style = GetWindowLongPtrW(self.hwnd, GWL_STYLE);
                let borderless =
                    style & !(windows::Win32::UI::WindowsAndMessaging::WS_CAPTION.0 as isize);
                SetWindowLongPtrW(self.hwnd, GWL_STYLE, borderless);
            }
            if let Some(mon) = monitor_info_for_window(self.hwnd) {
                let r = mon.monitor;
                unsafe {
                    let _ = SetWindowPos(
                        self.hwnd,
                        Some(HWND_TOP),
                        r.left,
                        r.top,
                        r.right - r.left,
                        r.bottom - r.top,
                        SWP_FRAMECHANGED | SWP_SHOWWINDOW,
                    );
                }
            }
            self.state
                .updating_fullscreen
                .store(false, Ordering::Release);
            self.state.emit(ShellWindowEvent::FullscreenChanged(true));
            self.state.emit(ShellWindowEvent::MaximizedChanged(false));
            Ok(true)
        } else {
            exit_fullscreen_internal(self.hwnd, &self.state, FullscreenExitTarget::PreFullscreen);
            Ok(false)
        }
    }

    /// Exit fullscreen directly into restored windowed bounds (used by Titlebar Restore / `SC_RESTORE`).
    pub fn exit_fullscreen_to_windowed(&self) -> Result<bool> {
        if !self.is_fullscreen() {
            return Ok(false);
        }
        exit_fullscreen_internal(self.hwnd, &self.state, FullscreenExitTarget::Windowed);
        Ok(false)
    }

    /// Begin drag-from-maximize session (`ipc-handlers.js:624-701`).
    pub fn begin_titlebar_drag(&self, screen_x: i32, screen_y: i32) -> Option<WindowBounds> {
        if self.is_fullscreen() || unsafe { IsIconic(self.hwnd).as_bool() } {
            return None;
        }
        if !self.is_maximized() {
            return None;
        }

        set_high_res_timer(true);
        let mon = monitor_info_for_point(screen_x, screen_y)
            .or_else(|| monitor_info_for_window(self.hwnd))?;
        let saved = self.state.restore_bounds.lock().ok().and_then(|g| *g);
        let (bounds, session, _ratio) =
            compute_drag_from_maximize_start(screen_x, screen_y, mon.monitor, mon.work, saved);

        unsafe {
            let _ = ShowWindow(self.hwnd, SW_RESTORE);
            let _ = SetWindowPos(
                self.hwnd,
                None,
                bounds.x,
                bounds.y,
                bounds.width,
                bounds.height,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            );
        }

        if let Ok(mut rb) = self.state.restore_bounds.lock() {
            *rb = Some(bounds);
        }
        if let Ok(mut ds) = self.state.drag_session.lock() {
            *ds = Some(session);
        }
        Some(bounds)
    }

    /// Update window position during an active drag-from-maximize session.
    pub fn move_titlebar_drag(&self, screen_x: i32, screen_y: i32) -> bool {
        let session = match self.state.drag_session.lock() {
            Ok(g) => *g,
            Err(_) => return false,
        };
        let Some(session) = session else {
            return false;
        };
        if !session.active {
            return false;
        }
        let Some(mon) = monitor_info_for_point(screen_x, screen_y)
            .or_else(|| monitor_info_for_window(self.hwnd))
        else {
            return false;
        };
        let moved = clamp_dragged_bounds(
            WindowBounds {
                x: screen_x - session.offset_x,
                y: screen_y - session.offset_y,
                width: session.width,
                height: session.height,
            },
            mon.work,
        );
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                None,
                moved.x,
                moved.y,
                moved.width,
                moved.height,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            );
        }
        if let Ok(mut rb) = self.state.restore_bounds.lock() {
            *rb = Some(moved);
        }
        true
    }

    /// End an active drag-from-maximize session.
    pub fn end_titlebar_drag(&self) -> bool {
        set_high_res_timer(false);
        if let Ok(mut ds) = self.state.drag_session.lock() {
            let was_active = ds.as_ref().is_some_and(|s| s.active);
            *ds = None;
            return was_active;
        }
        false
    }
}

impl Drop for WindowSubclass {
    fn drop(&mut self) {
        self.state
            .schedule_guard
            .destroyed
            .store(true, Ordering::Release);
        set_high_res_timer(false);
        unsafe {
            let _ = RemoveWindowSubclass(self.hwnd, Some(subclass_proc), SUBCLASS_UID);
        }
    }
}

/// Clamp non-maximized, non-fullscreen window bounds into the monitor work area (`main.js:1262-1305`).
pub fn clamp_bounds_to_work_area(bounds: WindowBounds, work: RECT) -> WindowBounds {
    let area_w = (work.right - work.left).max(1);
    let area_h = (work.bottom - work.top).max(1);
    let target_w = bounds.width.min(area_w);
    let target_h = bounds.height.min(area_h);
    let max_x = work.left + (area_w - target_w).max(0);
    let max_y = work.top + (area_h - target_h).max(0);
    let clamped_x = bounds.x.clamp(work.left, max_x);
    let clamped_y = bounds.y.clamp(work.top, max_y);
    WindowBounds {
        x: clamped_x,
        y: clamped_y,
        width: target_w,
        height: target_h,
    }
}

fn clamp_window_to_work_area(hwnd: HWND, state: &SubclassState) {
    if state.is_fullscreen.load(Ordering::Acquire) {
        return;
    }
    if unsafe { IsIconic(hwnd).as_bool() || IsZoomed(hwnd).as_bool() } {
        return;
    }
    if state.is_clamping_bounds.swap(true, Ordering::AcqRel) {
        return;
    }
    if let (Some(bounds), Some(mon)) = (get_window_bounds(hwnd), monitor_info_for_window(hwnd)) {
        let clamped = clamp_bounds_to_work_area(bounds, mon.work);
        if clamped != bounds {
            unsafe {
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    clamped.x,
                    clamped.y,
                    clamped.width,
                    clamped.height,
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
                );
            }
        }
    }
    state.is_clamping_bounds.store(false, Ordering::Release);
}

fn get_window_bounds(hwnd: HWND) -> Option<WindowBounds> {
    let mut rc = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut rc) }.is_ok() {
        Some(WindowBounds {
            x: rc.left,
            y: rc.top,
            width: rc.right - rc.left,
            height: rc.bottom - rc.top,
        })
    } else {
        None
    }
}

fn get_normal_window_bounds(hwnd: HWND) -> Option<WindowBounds> {
    let wp = capture_window_placement(hwnd)?;
    let rc = wp.rcNormalPosition;
    let w = rc.right - rc.left;
    let h = rc.bottom - rc.top;
    if w > 0 && h > 0 {
        return Some(WindowBounds {
            x: rc.left,
            y: rc.top,
            width: w,
            height: h,
        });
    }
    None
}

fn capture_window_placement(hwnd: HWND) -> Option<WINDOWPLACEMENT> {
    let mut wp = WINDOWPLACEMENT {
        length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    if unsafe { GetWindowPlacement(hwnd, &mut wp) }.is_ok() {
        Some(wp)
    } else {
        None
    }
}

fn bounds_fill_work_or_monitor(bounds: WindowBounds, mon: MonitorMetrics) -> bool {
    if bounds.width <= 0 || bounds.height <= 0 {
        return true;
    }
    let wa_w = (mon.work.right - mon.work.left).max(1);
    let wa_h = (mon.work.bottom - mon.work.top).max(1);
    let rc = RECT {
        left: bounds.x,
        top: bounds.y,
        right: bounds.x + bounds.width,
        bottom: bounds.y + bounds.height,
    };
    bounds_cover_monitor(rc, mon.monitor) || (bounds.width >= wa_w - 4 && bounds.height >= wa_h - 4)
}

fn compute_default_windowed_bounds(hwnd: HWND, mon: MonitorMetrics) -> WindowBounds {
    let dpi = dpi_for_window(hwnd);
    let wa_w = (mon.work.right - mon.work.left).max(1);
    let wa_h = (mon.work.bottom - mon.work.top).max(1);
    let dip_w = dip_to_physical(DEFAULT_WINDOWED_WIDTH, dpi);
    let dip_h = dip_to_physical(DEFAULT_WINDOWED_HEIGHT, dpi);
    let (rw, rh) = if dip_w <= wa_w - 40 && dip_h <= wa_h - 40 {
        (dip_w, dip_h)
    } else {
        compute_drag_restore_size(None, mon.work)
    };
    WindowBounds {
        x: mon.work.left + (wa_w - rw) / 2,
        y: mon.work.top + (wa_h - rh) / 2,
        width: rw,
        height: rh,
    }
}

fn apply_restored_window_bounds(hwnd: HWND, state: &SubclassState) {
    if state.is_fullscreen.load(Ordering::Acquire) {
        return;
    }
    if unsafe { IsIconic(hwnd).as_bool() || IsZoomed(hwnd).as_bool() } {
        return;
    }
    if state.is_clamping_bounds.swap(true, Ordering::AcqRel) {
        return;
    }
    if let Some(mon) = monitor_info_for_window(hwnd) {
        let cur_valid = get_window_bounds(hwnd).filter(|b| !bounds_fill_work_or_monitor(*b, mon));
        let stored_valid = state
            .restore_bounds
            .lock()
            .ok()
            .and_then(|g| *g)
            .filter(|b| !bounds_fill_work_or_monitor(*b, mon));
        let target = cur_valid
            .or(stored_valid)
            .unwrap_or_else(|| compute_default_windowed_bounds(hwnd, mon));
        let clamped = clamp_bounds_to_work_area(target, mon.work);
        if get_window_bounds(hwnd) != Some(clamped) {
            unsafe {
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    clamped.x,
                    clamped.y,
                    clamped.width,
                    clamped.height,
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
                );
            }
        }
        if let Ok(mut rb) = state.restore_bounds.lock() {
            *rb = Some(clamped);
        }
    }
    state.is_clamping_bounds.store(false, Ordering::Release);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FullscreenExitTarget {
    PreFullscreen,
    Windowed,
    Maximized,
}

/// Returns `true` when `bounds` matches `monitor` within a 2px tolerance.
#[inline]
pub fn bounds_cover_monitor(bounds: RECT, monitor: RECT) -> bool {
    (bounds.left - monitor.left).abs() <= 2
        && (bounds.top - monitor.top).abs() <= 2
        && (bounds.right - monitor.right).abs() <= 2
        && (bounds.bottom - monitor.bottom).abs() <= 2
}

fn window_covers_monitor(hwnd: HWND) -> bool {
    let mut win_rc = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut win_rc) }.is_err() {
        return false;
    }
    let Some(mon) = monitor_info_for_window(hwnd) else {
        return false;
    };
    bounds_cover_monitor(win_rc, mon.monitor)
}

fn is_cursor_in_max_button(hwnd: HWND, state: &SubclassState) -> bool {
    let mut pt = POINT::default();
    if unsafe { GetCursorPos(&mut pt) }.is_err() {
        return false;
    }
    if !unsafe { ScreenToClient(hwnd, &mut pt) }.as_bool() {
        return false;
    }
    let dpi = dpi_for_window(hwnd);
    let dip_x = physical_to_dip(pt.x, dpi);
    let dip_y = physical_to_dip(pt.y, dpi);
    state
        .hit_test
        .lock()
        .ok()
        .and_then(|ht| ht.maximize_button_rect)
        .is_some_and(|r| r.contains(dip_x, dip_y))
}

fn exit_fullscreen_internal(hwnd: HWND, state: &SubclassState, target: FullscreenExitTarget) {
    state.updating_fullscreen.store(true, Ordering::Release);
    state.is_fullscreen.store(false, Ordering::Release);
    state
        .schedule_guard
        .is_fullscreen
        .store(false, Ordering::Release);
    let saved_was_max = state.pre_fullscreen_maximized.swap(false, Ordering::AcqRel);
    let go_maximized = match target {
        FullscreenExitTarget::PreFullscreen => saved_was_max,
        FullscreenExitTarget::Windowed => false,
        FullscreenExitTarget::Maximized => true,
    };
    if let Ok(mut ht) = state.hit_test.lock() {
        ht.is_fullscreen = false;
        ht.is_maximized = go_maximized;
    }

    let theme = state.theme.lock().map(|g| *g).unwrap_or_default();
    let _ = apply_native_dwm_window_styles(hwnd, theme, false, false);

    let mon = monitor_info_for_window(hwnd);
    let saved_wp = state
        .pre_fullscreen_wp
        .lock()
        .ok()
        .and_then(|mut g| g.take());
    let saved_bounds = state
        .pre_fullscreen_bounds
        .lock()
        .ok()
        .and_then(|mut g| g.take())
        .filter(|b| mon.is_none_or(|m| !bounds_fill_work_or_monitor(*b, m)))
        .or_else(|| {
            state
                .restore_bounds
                .lock()
                .ok()
                .and_then(|g| *g)
                .filter(|b| mon.is_none_or(|m| !bounds_fill_work_or_monitor(*b, m)))
        });

    if go_maximized {
        let repaired_bounds =
            saved_bounds.or_else(|| mon.map(|m| compute_default_windowed_bounds(hwnd, m)));
        if let Some(valid) = repaired_bounds {
            if let Ok(mut rb) = state.restore_bounds.lock() {
                *rb = Some(valid);
            }
        }
        state.was_maximized.store(true, Ordering::Release);
        set_high_res_timer(true);
        unsafe {
            let _ = ShowWindow(hwnd, SW_MAXIMIZE);
        }
        if let Some(mut current_wp) = capture_window_placement(hwnd) {
            if let Some(saved_wp) = saved_wp {
                current_wp.rcNormalPosition = saved_wp.rcNormalPosition;
                current_wp.showCmd = SW_SHOWMAXIMIZED.0 as u32;
                unsafe {
                    let _ = SetWindowPlacement(hwnd, &current_wp);
                }
            } else if let Some(bounds) = repaired_bounds {
                let clamped = mon
                    .map(|m| clamp_bounds_to_work_area(bounds, m.work))
                    .unwrap_or(bounds);
                let (dx, dy) = mon
                    .map(|m| (m.work.left - m.monitor.left, m.work.top - m.monitor.top))
                    .unwrap_or((0, 0));
                current_wp.rcNormalPosition = RECT {
                    left: clamped.x - dx,
                    top: clamped.y - dy,
                    right: clamped.x - dx + clamped.width,
                    bottom: clamped.y - dy + clamped.height,
                };
                current_wp.showCmd = SW_SHOWMAXIMIZED.0 as u32;
                unsafe {
                    let _ = SetWindowPlacement(hwnd, &current_wp);
                }
            }
        }
        set_high_res_timer(false);
    } else {
        state.was_maximized.store(false, Ordering::Release);
        let fallback_bounds = mon.map(|m| compute_default_windowed_bounds(hwnd, m));
        if let Some(bounds) = saved_bounds.or(fallback_bounds) {
            let clamped = mon
                .map(|m| clamp_bounds_to_work_area(bounds, m.work))
                .unwrap_or(bounds);
            unsafe {
                if IsZoomed(hwnd).as_bool() {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                }
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    clamped.x,
                    clamped.y,
                    clamped.width,
                    clamped.height,
                    SWP_FRAMECHANGED | SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
                );
            }
            if let Ok(mut rb) = state.restore_bounds.lock() {
                *rb = Some(clamped);
            }
        } else {
            unsafe {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
        }
        clamp_window_to_work_area(hwnd, state);
    }

    schedule_native_dwm_window_styles(hwnd, theme, Arc::clone(&state.schedule_guard), &[50, 150]);
    state.updating_fullscreen.store(false, Ordering::Release);
    state.emit(ShellWindowEvent::FullscreenChanged(false));
    state.emit(ShellWindowEvent::MaximizedChanged(go_maximized));
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    umsg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    uidsubclass: usize,
    dwrefdata: usize,
) -> LRESULT {
    if dwrefdata == 0 {
        return DefSubclassProc(hwnd, umsg, wparam, lparam);
    }

    // Borrow Arc without decrementing refcount; WM_NCDESTROY reclaims it.
    let state_ptr = dwrefdata as *const SubclassState;
    let state = &*state_ptr;

    match umsg {
        WM_NCCALCSIZE => {
            if wparam.0 != 0 {
                let params = &mut *(lparam.0 as *mut NCCALCSIZE_PARAMS);
                let is_fs = state.is_fullscreen.load(Ordering::Acquire);
                let is_max = !is_fs && IsZoomed(hwnd).as_bool();
                if let Some(mon) = monitor_info_for_window(hwnd) {
                    let effective_fs = is_fs
                        && (state.updating_fullscreen.load(Ordering::Acquire)
                            || bounds_cover_monitor(params.rgrc[0], mon.monitor));
                    params.rgrc[0] = compute_nccalcsize_client_rect(
                        params.rgrc[0],
                        mon.work,
                        mon.monitor,
                        is_max,
                        effective_fs,
                    );
                }
                // WVR_REDRAW (0x0300) prevents trailing black resize artifacts.
                LRESULT(WVR_REDRAW as isize)
            } else {
                LRESULT(0)
            }
        }
        WM_NCPAINT => LRESULT(0),
        WM_ERASEBKGND => LRESULT(1),
        WM_NCACTIVATE => LRESULT(1),
        WM_ENTERSIZEMOVE => {
            state.is_resizing.store(true, Ordering::Release);
            state
                .schedule_guard
                .is_resizing
                .store(true, Ordering::Release);
            state.emit(ShellWindowEvent::BoundsAnimating(true));
            DefSubclassProc(hwnd, umsg, wparam, lparam)
        }
        WM_SIZING => {
            state.is_resizing.store(true, Ordering::Release);
            state
                .schedule_guard
                .is_resizing
                .store(true, Ordering::Release);
            DefSubclassProc(hwnd, umsg, wparam, lparam)
        }
        WM_EXITSIZEMOVE => {
            state.is_resizing.store(false, Ordering::Release);
            state
                .schedule_guard
                .is_resizing
                .store(false, Ordering::Release);
            clamp_window_to_work_area(hwnd, state);
            if !state.is_fullscreen.load(Ordering::Acquire)
                && !IsZoomed(hwnd).as_bool()
                && !IsIconic(hwnd).as_bool()
            {
                if let (Some(b), Some(mon)) =
                    (get_window_bounds(hwnd), monitor_info_for_window(hwnd))
                {
                    if !bounds_fill_work_or_monitor(b, mon) {
                        if let Ok(mut rb) = state.restore_bounds.lock() {
                            *rb = Some(b);
                        }
                    }
                }
            }
            state.emit(ShellWindowEvent::BoundsAnimating(false));
            // CRITICAL: NEVER call SWP_FRAMECHANGED or apply_native_dwm_window_styles here!
            // Recomputing frames tears down the DirectX/DWM backbuffer and causes black flashes.
            DefSubclassProc(hwnd, umsg, wparam, lparam)
        }
        WM_NCHITTEST => {
            let (screen_x, screen_y) = unpack_lparam_point(lparam.0);
            let mut client_pt = POINT {
                x: screen_x,
                y: screen_y,
            };
            if ScreenToClient(hwnd, &mut client_pt).as_bool() {
                let mut client_rc = RECT::default();
                if GetClientRect(hwnd, &mut client_rc).is_ok() {
                    let dpi = dpi_for_window(hwnd);
                    let local_dip_x = physical_to_dip(client_pt.x, dpi);
                    let local_dip_y = physical_to_dip(client_pt.y, dpi);
                    let win_dip_w = physical_to_dip(client_rc.right - client_rc.left, dpi);
                    let win_dip_h = physical_to_dip(client_rc.bottom - client_rc.top, dpi);

                    let mut cfg = state.hit_test.lock().map(|g| g.clone()).unwrap_or_default();
                    cfg.is_minimized = IsIconic(hwnd).as_bool();
                    cfg.is_fullscreen = state.is_fullscreen.load(Ordering::Acquire);
                    cfg.is_maximized = !cfg.is_fullscreen && IsZoomed(hwnd).as_bool();
                    let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
                    cfg.is_resizable = (style & (WS_THICKFRAME.0 as isize)) != 0;

                    if let Some(ht) =
                        resolve_nc_hit_test(local_dip_x, local_dip_y, win_dip_w, win_dip_h, &cfg)
                    {
                        if ht != HTMAXBUTTON as isize
                            && state.max_button_hovered.swap(false, Ordering::AcqRel)
                        {
                            state.emit(ShellWindowEvent::MaxButtonHovered(false));
                        }
                        return LRESULT(ht);
                    }
                }
            }
            if state.max_button_hovered.swap(false, Ordering::AcqRel) {
                state.emit(ShellWindowEvent::MaxButtonHovered(false));
            }
            DefSubclassProc(hwnd, umsg, wparam, lparam)
        }
        WM_NCMOUSEMOVE => {
            let over_max = wparam.0 == HTMAXBUTTON as usize;
            if over_max {
                if !state.max_button_hovered.swap(true, Ordering::AcqRel) {
                    state.emit(ShellWindowEvent::MaxButtonHovered(true));
                }
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE | TME_NONCLIENT,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut tme);
            } else if state.max_button_hovered.swap(false, Ordering::AcqRel) {
                state.emit(ShellWindowEvent::MaxButtonHovered(false));
            }
            DefSubclassProc(hwnd, umsg, wparam, lparam)
        }
        WM_NCMOUSELEAVE => {
            if unsafe { GetAsyncKeyState(0x01) } >= 0 {
                state.max_button_pressed.store(false, Ordering::Release);
            }
            if state.max_button_hovered.swap(false, Ordering::AcqRel) {
                state.emit(ShellWindowEvent::MaxButtonHovered(false));
            }
            DefSubclassProc(hwnd, umsg, wparam, lparam)
        }
        WM_NCLBUTTONDOWN => {
            if wparam.0 == HTMAXBUTTON as usize {
                state.max_button_pressed.store(true, Ordering::Release);
                return LRESULT(0);
            }
            DefSubclassProc(hwnd, umsg, wparam, lparam)
        }
        WM_NCLBUTTONUP => {
            if wparam.0 == HTMAXBUTTON as usize
                || (state.max_button_pressed.load(Ordering::Acquire)
                    && is_cursor_in_max_button(hwnd, state))
            {
                if state.max_button_pressed.swap(false, Ordering::AcqRel) {
                    state.emit(ShellWindowEvent::MaxButtonClicked);
                }
                return LRESULT(0);
            }
            state.max_button_pressed.store(false, Ordering::Release);
            DefSubclassProc(hwnd, umsg, wparam, lparam)
        }
        WM_LBUTTONUP => {
            if state.max_button_pressed.swap(false, Ordering::AcqRel)
                && is_cursor_in_max_button(hwnd, state)
            {
                state.emit(ShellWindowEvent::MaxButtonClicked);
                return LRESULT(0);
            }
            DefSubclassProc(hwnd, umsg, wparam, lparam)
        }
        WM_NCLBUTTONDBLCLK => {
            if wparam.0 == HTMAXBUTTON as usize {
                state.max_button_pressed.store(true, Ordering::Release);
                return LRESULT(0);
            }
            DefSubclassProc(hwnd, umsg, wparam, lparam)
        }
        WM_CAPTURECHANGED => {
            if unsafe { GetAsyncKeyState(0x01) } >= 0 {
                state.max_button_pressed.store(false, Ordering::Release);
            }
            DefSubclassProc(hwnd, umsg, wparam, lparam)
        }
        WM_ACTIVATE => {
            if (wparam.0 & 0xFFFF) == 0 {
                state.max_button_pressed.store(false, Ordering::Release);
            }
            DefSubclassProc(hwnd, umsg, wparam, lparam)
        }
        WM_SYSCOMMAND => {
            let cmd = (wparam.0 & 0xFFF0) as u32;
            if state.is_fullscreen.load(Ordering::Acquire) {
                if cmd == SC_RESTORE && !IsIconic(hwnd).as_bool() {
                    exit_fullscreen_internal(hwnd, state, FullscreenExitTarget::Windowed);
                    return LRESULT(0);
                }
                if cmd == SC_MAXIMIZE {
                    exit_fullscreen_internal(hwnd, state, FullscreenExitTarget::Maximized);
                    return LRESULT(0);
                }
            }
            if cmd == SC_MAXIMIZE && !IsZoomed(hwnd).as_bool() && !IsIconic(hwnd).as_bool() {
                let mon = monitor_info_for_window(hwnd);
                let current_valid = get_window_bounds(hwnd)
                    .filter(|b| mon.is_none_or(|m| !bounds_fill_work_or_monitor(*b, m)));
                let normal_valid = get_normal_window_bounds(hwnd)
                    .filter(|b| mon.is_none_or(|m| !bounds_fill_work_or_monitor(*b, m)));
                if let Ok(mut rb) = state.restore_bounds.lock() {
                    let existing_valid =
                        rb.filter(|b| mon.is_none_or(|m| !bounds_fill_work_or_monitor(*b, m)));
                    *rb = current_valid.or(normal_valid).or(existing_valid);
                }
            }
            if cmd == SC_RESTORE && !IsIconic(hwnd).as_bool() {
                let was_zoomed = IsZoomed(hwnd).as_bool();
                let res = DefSubclassProc(hwnd, umsg, wparam, lparam);
                if !IsIconic(hwnd).as_bool() && !IsZoomed(hwnd).as_bool() {
                    let fills_work = get_window_bounds(hwnd)
                        .zip(monitor_info_for_window(hwnd))
                        .is_some_and(|(b, m)| bounds_fill_work_or_monitor(b, m));
                    if was_zoomed || fills_work {
                        apply_restored_window_bounds(hwnd, state);
                    }
                }
                return res;
            }
            DefSubclassProc(hwnd, umsg, wparam, lparam)
        }
        WM_DPICHANGED => {
            let new_dpi = (wparam.0 & 0xFFFF) as u32;
            let suggested = &*(lparam.0 as *const RECT);
            let _ = SetWindowPos(
                hwnd,
                None,
                suggested.left,
                suggested.top,
                suggested.right - suggested.left,
                suggested.bottom - suggested.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            state.emit(ShellWindowEvent::DpiChanged {
                dpi: new_dpi,
                scale_factor: scale_for_dpi(new_dpi),
            });
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            let mmi = &mut *(lparam.0 as *mut MINMAXINFO);
            let dpi = dpi_for_window(hwnd);
            mmi.ptMinTrackSize.x = dip_to_physical(MIN_WINDOW_WIDTH_DIP, dpi);
            mmi.ptMinTrackSize.y = dip_to_physical(MIN_WINDOW_HEIGHT_DIP, dpi);
            if let Some(mon) = monitor_info_for_window(hwnd) {
                let target = if state.is_fullscreen.load(Ordering::Acquire) {
                    mon.monitor
                } else {
                    mon.work
                };
                mmi.ptMaxPosition.x = target.left - mon.monitor.left;
                mmi.ptMaxPosition.y = target.top - mon.monitor.top;
                mmi.ptMaxSize.x = target.right - target.left;
                mmi.ptMaxSize.y = target.bottom - target.top;
            }
            LRESULT(0)
        }
        WM_SIZE => {
            if wparam.0 == SIZE_MINIMIZED {
                if let Ok(mut ht) = state.hit_test.lock() {
                    ht.is_minimized = true;
                }
                return DefSubclassProc(hwnd, umsg, wparam, lparam);
            }

            let mut exited_fullscreen = false;
            if !state.updating_fullscreen.load(Ordering::Acquire)
                && state.is_fullscreen.load(Ordering::Acquire)
            {
                let exited = wparam.0 == SIZE_MAXIMIZED || !window_covers_monitor(hwnd);
                if exited {
                    exited_fullscreen = true;
                    state.is_fullscreen.store(false, Ordering::Release);
                    state
                        .schedule_guard
                        .is_fullscreen
                        .store(false, Ordering::Release);
                    state
                        .pre_fullscreen_maximized
                        .store(false, Ordering::Release);
                    if let Ok(mut b) = state.pre_fullscreen_bounds.lock() {
                        *b = None;
                    }
                    if let Ok(mut wp) = state.pre_fullscreen_wp.lock() {
                        *wp = None;
                    }
                    if let Ok(mut ht) = state.hit_test.lock() {
                        ht.is_fullscreen = false;
                        ht.is_minimized = false;
                    }
                    let theme = state.theme.lock().map(|g| *g).unwrap_or_default();
                    let _ = apply_native_dwm_window_styles(hwnd, theme, false, false);
                    schedule_native_dwm_window_styles(
                        hwnd,
                        theme,
                        Arc::clone(&state.schedule_guard),
                        &[50, 150],
                    );
                    state.emit(ShellWindowEvent::FullscreenChanged(false));
                }
            }

            let is_fs = state.is_fullscreen.load(Ordering::Acquire);
            let is_max = !is_fs && IsZoomed(hwnd).as_bool();
            let prev_max = state.was_maximized.swap(is_max, Ordering::AcqRel);
            if let Ok(mut ht) = state.hit_test.lock() {
                ht.is_minimized = false;
                ht.is_fullscreen = is_fs;
                ht.is_maximized = is_max;
            }
            if !state.updating_fullscreen.load(Ordering::Acquire)
                && (prev_max || exited_fullscreen)
                && !is_max
                && !is_fs
            {
                let fills_work = get_window_bounds(hwnd)
                    .zip(monitor_info_for_window(hwnd))
                    .is_some_and(|(b, m)| bounds_fill_work_or_monitor(b, m));
                if fills_work {
                    apply_restored_window_bounds(hwnd, state);
                } else {
                    clamp_window_to_work_area(hwnd, state);
                }
            }
            state.emit(ShellWindowEvent::MaximizedChanged(is_max));
            DefSubclassProc(hwnd, umsg, wparam, lparam)
        }
        WM_NCDESTROY => {
            state
                .schedule_guard
                .destroyed
                .store(true, Ordering::Release);
            let _ = RemoveWindowSubclass(hwnd, Some(subclass_proc), uidsubclass);
            let res = DefSubclassProc(hwnd, umsg, wparam, lparam);
            drop(Arc::from_raw(state_ptr));
            res
        }
        _ => DefSubclassProc(hwnd, umsg, wparam, lparam),
    }
}

#[cfg(test)]
mod workstream_d_tests {
    use super::*;

    #[test]
    fn clamp_bounds_to_work_area_keeps_window_within_monitor() {
        let work = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        };
        let offscreen = WindowBounds {
            x: -400,
            y: -200,
            width: 1280,
            height: 720,
        };
        let clamped = clamp_bounds_to_work_area(offscreen, work);
        assert!(clamped.x >= work.left);
        assert!(clamped.y >= work.top);
        assert_eq!(clamped.width, 1280);
        assert_eq!(clamped.height, 720);

        let oversized = WindowBounds {
            x: 1500,
            y: 800,
            width: 2400,
            height: 1400,
        };
        let clamped_big = clamp_bounds_to_work_area(oversized, work);
        assert_eq!(clamped_big.width, 1920);
        assert_eq!(clamped_big.height, 1040);
        assert_eq!(clamped_big.x, 0);
        assert_eq!(clamped_big.y, 0);
    }
}
