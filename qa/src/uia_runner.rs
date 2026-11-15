//! Windows UIAutomation (`IUIAutomation`), `SendInput`, DWM/`HWND` verification,
//! and client-area screen capture harness for `hybrid-app`.

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::core::BOOL;
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWINDOWATTRIBUTE};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleDC, CreateDIBSection,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PW_CLIENTONLY, PrintWindow};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationInvokePattern,
    UIA_InvokePatternId,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP,
    MOUSE_EVENT_FLAGS, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN,
    MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT,
    SendInput, VIRTUAL_KEY, VK_CONTROL, VK_MENU, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClientRect, GetSystemMetrics, GetWindowLongPtrW, GetWindowRect,
    GetWindowThreadProcessId, GWL_EXSTYLE, GWL_STYLE, IsWindow, IsWindowVisible,
    PW_RENDERFULLCONTENT, SM_CXSCREEN, SM_CYSCREEN, SetForegroundWindow, WS_CAPTION,
    WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_SYSMENU, WS_THICKFRAME,
};

use crate::pixel_diff::RgbaImageBuffer;
use crate::{QaError, Result};

/// Expected frameless main window style mask (mirrors `hybrid-win32-shell::dwm`).
pub const EXPECTED_FRAMELESS_STYLE_MASK: isize = (WS_THICKFRAME.0
    | WS_CAPTION.0
    | WS_MINIMIZEBOX.0
    | WS_MAXIMIZEBOX.0
    | WS_SYSMENU.0
    | WS_CLIPCHILDREN.0
    | WS_CLIPSIBLINGS.0) as isize;

pub const DWMWA_NCRENDERING_POLICY_ID: u32 = 2;
pub const DWMNCRP_ENABLED_VAL: i32 = 2;
pub const DWMWA_USE_IMMERSIVE_DARK_MODE_ID: u32 = 20;
pub const DWMWA_WINDOW_CORNER_PREFERENCE_ID: u32 = 33;
pub const DWMWCP_DONOTROUND_VAL: i32 = 1;
pub const DWMWA_BORDER_COLOR_ID: u32 = 34;
pub const DWMWA_COLOR_NONE_VAL: u32 = 0xFFFF_FFFE;
pub const DWMWA_SYSTEMBACKDROP_TYPE_ID: u32 = 38;
pub const DWMSBT_NONE_VAL: i32 = 1;

/// Searches `HYBRID_APP_BIN`, `target/debug/hybrid-app.exe`, and `target/release/hybrid-app.exe`.
pub fn discover_hybrid_app_binary(workspace_root: &Path) -> Option<PathBuf> {
    if let Ok(env_bin) = std::env::var("HYBRID_APP_BIN") {
        let p = PathBuf::from(env_bin);
        if p.is_file() {
            return Some(p);
        }
    }
    let candidates = [
        workspace_root
            .join("target")
            .join("debug")
            .join("hybrid-app.exe"),
        workspace_root
            .join("target")
            .join("release")
            .join("hybrid-app.exe"),
    ];
    candidates.into_iter().find(|p| p.is_file())
}

#[derive(Debug, Clone)]
pub struct AppLaunchConfig {
    pub binary_path: PathBuf,
    pub args: Vec<String>,
    pub working_dir: Option<PathBuf>,
    pub mute_audio: bool,
    pub window_wait_timeout: Duration,
}

impl AppLaunchConfig {
    pub fn new(binary_path: PathBuf) -> Self {
        Self {
            binary_path,
            args: Vec::new(),
            working_dir: None,
            mute_audio: true,
            window_wait_timeout: Duration::from_secs(12),
        }
    }
}

/// RAII child process handle for `hybrid-app.exe` that kills & reaps on drop.
pub struct LaunchedApp {
    child: Child,
    hwnd: Option<isize>,
}

impl LaunchedApp {
    pub fn launch_and_wait_for_window(cfg: &AppLaunchConfig) -> Result<Self> {
        let mut cmd = Command::new(&cfg.binary_path);
        cmd.args(&cfg.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(dir) = &cfg.working_dir {
            cmd.current_dir(dir);
        }
        if cfg.mute_audio {
            cmd.env("HYBRID_MUTE_AUDIO", "1");
        }
        cmd.env("NODE_ENV", "test");

        let mut child = cmd
            .spawn()
            .map_err(|e| QaError::Process(format!("failed to spawn {}: {e}", cfg.binary_path.display())))?;
        let pid = child.id();

        let deadline = Instant::now() + cfg.window_wait_timeout;
        let mut found_hwnd = None;
        while Instant::now() < deadline {
            if let Ok(Some(status)) = child.try_wait() {
                return Err(QaError::Process(format!(
                    "app exited early with {status} before creating a visible window"
                )));
            }
            if let Some(hwnd) = find_top_level_window_for_pid(pid) {
                found_hwnd = Some(hwnd.0 as isize);
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }

        let Some(hwnd) = found_hwnd else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(QaError::Process(format!(
                "timed out after {:?} waiting for top-level HWND (pid={pid})",
                cfg.window_wait_timeout
            )));
        };

        Ok(Self {
            child,
            hwnd: Some(hwnd),
        })
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn hwnd(&self) -> HWND {
        HWND(self.hwnd.unwrap_or(0) as *mut c_void)
    }

    pub fn terminate(mut self) -> Result<()> {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.hwnd = None;
        Ok(())
    }
}

impl Drop for LaunchedApp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// CLI specification matching `hybrid-app --headless-snapshot <out.png> ...`.
#[derive(Debug, Clone, Default)]
pub struct HeadlessSnapshotSpec {
    pub out_png: PathBuf,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub dpr: Option<f32>,
    pub theme: Option<String>,
    pub welcome_bg: Option<String>,
    pub modal: Option<String>,
    pub playlist_open: bool,
    pub bg_accordion: bool,
    pub overlay: Option<String>,
    pub burst_count: Option<usize>,
    pub carousel_index: Option<usize>,
    pub clip_time: Option<f64>,
    pub playback_sample: bool,
    pub media: Option<PathBuf>,
    pub time_secs: Option<f32>,
    pub preview_time_secs: Option<f32>,
    pub app_data_dir: Option<PathBuf>,
    pub extra_args: Vec<String>,
}

/// Runs `hybrid-app.exe --headless-snapshot <out.png> [flags...]` and waits for exit.
pub fn run_headless_snapshot_cli(
    binary_path: &Path,
    spec: &HeadlessSnapshotSpec,
    timeout: Duration,
) -> Result<()> {
    if let Some(parent) = spec.out_png.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut cmd = Command::new(binary_path);
    cmd.arg("--headless-snapshot").arg(&spec.out_png);
    if let Some(w) = spec.width {
        cmd.arg("--width").arg(w.to_string());
    }
    if let Some(h) = spec.height {
        cmd.arg("--height").arg(h.to_string());
    }
    if let Some(dpr) = spec.dpr {
        cmd.arg("--dpr").arg(dpr.to_string());
    }
    if let Some(theme) = &spec.theme {
        cmd.arg("--theme").arg(theme);
    }
    if let Some(bg) = &spec.welcome_bg {
        cmd.arg("--welcome-bg").arg(bg);
    }
    if let Some(modal) = &spec.modal {
        cmd.arg("--modal").arg(modal);
    }
    if spec.playlist_open {
        cmd.arg("--playlist-open");
    }
    if spec.bg_accordion {
        cmd.arg("--bg-accordion");
    }
    if let Some(overlay) = &spec.overlay {
        cmd.arg("--overlay").arg(overlay);
    }
    if let Some(cnt) = spec.burst_count {
        cmd.arg("--burst-count").arg(cnt.to_string());
    }
    if let Some(idx) = spec.carousel_index {
        cmd.arg("--carousel-index").arg(idx.to_string());
    }
    if let Some(ct) = spec.clip_time {
        cmd.arg("--clip-time").arg(ct.to_string());
    }
    if spec.playback_sample {
        cmd.arg("--playback-sample");
    }
    if let Some(media) = &spec.media {
        cmd.arg("--media").arg(media);
    }
    if let Some(t) = spec.time_secs {
        cmd.arg("--time-secs").arg(t.to_string());
    }
    if let Some(pt) = spec.preview_time_secs {
        cmd.arg("--preview-time-secs").arg(pt.to_string());
    }
    if let Some(dir) = &spec.app_data_dir {
        cmd.arg("--app-data-dir").arg(dir);
    }
    cmd.args(&spec.extra_args)
        .env("HYBRID_MUTE_AUDIO", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| QaError::Process(format!("failed to spawn headless snapshot: {e}")))?;

    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                return Err(QaError::Process(format!(
                    "headless snapshot failed with exit status {status}"
                )));
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(QaError::Process(format!(
                    "headless snapshot timed out after {timeout:?}"
                )));
            }
            Err(e) => {
                let _ = child.kill();
                return Err(QaError::Process(format!("wait failed: {e}")));
            }
        }
    }
}

struct EnumSearch {
    target_pid: u32,
    found: Option<HWND>,
}

unsafe extern "system" fn enum_windows_for_pid(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut EnumSearch);
    let mut pid = 0u32;
    let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == ctx.target_pid && IsWindowVisible(hwnd).as_bool() {
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_ok()
            && (rect.right - rect.left) > 32
            && (rect.bottom - rect.top) > 32
        {
            ctx.found = Some(hwnd);
            return BOOL(0);
        }
    }
    BOOL(1)
}

pub fn find_top_level_window_for_pid(pid: u32) -> Option<HWND> {
    let mut search = EnumSearch {
        target_pid: pid,
        found: None,
    };
    unsafe {
        let _ = EnumWindows(
            Some(enum_windows_for_pid),
            LPARAM(&mut search as *mut EnumSearch as isize),
        );
    }
    search.found
}

#[derive(Debug, Clone)]
pub struct WindowInspection {
    pub style: isize,
    pub ex_style: isize,
    pub client_width: i32,
    pub client_height: i32,
    pub window_width: i32,
    pub window_height: i32,
    pub dpi: u32,
    pub nc_rendering_policy: Option<i32>,
    pub immersive_dark_mode: Option<i32>,
    pub corner_preference: Option<i32>,
    pub border_color: Option<u32>,
    pub system_backdrop_type: Option<i32>,
}

impl WindowInspection {
    pub fn has_frameless_aero_style_mask(&self) -> bool {
        (self.style & EXPECTED_FRAMELESS_STYLE_MASK) == EXPECTED_FRAMELESS_STYLE_MASK
    }
}

pub fn inspect_window_state(hwnd: HWND) -> Result<WindowInspection> {
    if !unsafe { IsWindow(Some(hwnd)).as_bool() } {
        return Err(QaError::Win32("invalid HWND passed to inspect_window_state".to_owned()));
    }

    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) };
    let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };

    let mut client = RECT::default();
    unsafe {
        GetClientRect(hwnd, &mut client)
            .map_err(|e| QaError::Win32(format!("GetClientRect failed: {e}")))?;
    }
    let mut win_rect = RECT::default();
    unsafe {
        GetWindowRect(hwnd, &mut win_rect)
            .map_err(|e| QaError::Win32(format!("GetWindowRect failed: {e}")))?;
    }
    let dpi = unsafe { GetDpiForWindow(hwnd) };

    let get_dwm_i32 = |attr: u32| -> Option<i32> {
        let mut val = 0i32;
        let hr = unsafe {
            DwmGetWindowAttribute(
                hwnd,
                DWMWINDOWATTRIBUTE(attr as i32),
                (&mut val as *mut i32).cast(),
                std::mem::size_of::<i32>() as u32,
            )
        };
        if hr.is_ok() { Some(val) } else { None }
    };
    let get_dwm_u32 = |attr: u32| -> Option<u32> {
        let mut val = 0u32;
        let hr = unsafe {
            DwmGetWindowAttribute(
                hwnd,
                DWMWINDOWATTRIBUTE(attr as i32),
                (&mut val as *mut u32).cast(),
                std::mem::size_of::<u32>() as u32,
            )
        };
        if hr.is_ok() { Some(val) } else { None }
    };

    Ok(WindowInspection {
        style,
        ex_style,
        client_width: client.right - client.left,
        client_height: client.bottom - client.top,
        window_width: win_rect.right - win_rect.left,
        window_height: win_rect.bottom - win_rect.top,
        dpi,
        nc_rendering_policy: get_dwm_i32(DWMWA_NCRENDERING_POLICY_ID),
        immersive_dark_mode: get_dwm_i32(DWMWA_USE_IMMERSIVE_DARK_MODE_ID),
        corner_preference: get_dwm_i32(DWMWA_WINDOW_CORNER_PREFERENCE_ID),
        border_color: get_dwm_u32(DWMWA_BORDER_COLOR_ID),
        system_backdrop_type: get_dwm_i32(DWMWA_SYSTEMBACKDROP_TYPE_ID),
    })
}

/// Captures the client area of `hwnd` via `PrintWindow(PW_CLIENTONLY | PW_RENDERFULLCONTENT)`
/// with a `BitBlt` fallback and returns an RGBA `RgbaImageBuffer`.
pub fn capture_window_rgba(hwnd: HWND) -> Result<RgbaImageBuffer> {
    if !unsafe { IsWindow(Some(hwnd)).as_bool() } {
        return Err(QaError::Win32("invalid HWND passed to capture_window_rgba".to_owned()));
    }
    let mut client = RECT::default();
    unsafe {
        GetClientRect(hwnd, &mut client)
            .map_err(|e| QaError::Win32(format!("GetClientRect failed: {e}")))?;
    }
    let width = (client.right - client.left).max(0) as u32;
    let height = (client.bottom - client.top).max(0) as u32;
    if width == 0 || height == 0 {
        return Err(QaError::Win32(format!(
            "cannot capture zero-sized client rect ({width}x{height})"
        )));
    }

    unsafe {
        let screen_dc = GetDC(Some(hwnd));
        if screen_dc.is_invalid() {
            return Err(QaError::Win32("GetDC failed".to_owned()));
        }
        let mem_dc = CreateCompatibleDC(Some(screen_dc));
        if mem_dc.is_invalid() {
            let _ = ReleaseDC(Some(hwnd), screen_dc);
            return Err(QaError::Win32("CreateCompatibleDC failed".to_owned()));
        }

        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32), // top-down DIB
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut bits_ptr: *mut c_void = std::ptr::null_mut();
        let hbm = match CreateDIBSection(Some(screen_dc), &bmi, DIB_RGB_COLORS, &mut bits_ptr, None, 0) {
            Ok(b) => b,
            Err(e) => {
                let _ = DeleteDC(mem_dc);
                let _ = ReleaseDC(Some(hwnd), screen_dc);
                return Err(QaError::Win32(format!("CreateDIBSection failed: {e}")));
            }
        };

        let old_obj = SelectObject(mem_dc, hbm.into());

        let byte_len = (width as usize) * (height as usize) * 4;
        let flags = PRINT_WINDOW_FLAGS(PW_CLIENTONLY.0 | PW_RENDERFULLCONTENT);
        let mut ok = PrintWindow(hwnd, mem_dc, flags).as_bool();
        if ok && !bits_ptr.is_null() {
            let bgra = std::slice::from_raw_parts(bits_ptr as *const u8, byte_len);
            if !bgra.chunks_exact(4).any(|px| px[0] != 0 || px[1] != 0 || px[2] != 0) {
                ok = PrintWindow(hwnd, mem_dc, PW_CLIENTONLY).as_bool();
            }
        }
        if !ok {
            let _ = BitBlt(
                mem_dc,
                0,
                0,
                width as i32,
                height as i32,
                Some(screen_dc),
                0,
                0,
                SRCCOPY,
            );
        }

        let mut rgba = vec![0u8; byte_len];
        if !bits_ptr.is_null() {
            let bgra = std::slice::from_raw_parts(bits_ptr as *const u8, byte_len);
            for (dst, src) in rgba.chunks_exact_mut(4).zip(bgra.chunks_exact(4)) {
                dst[0] = src[2];
                dst[1] = src[1];
                dst[2] = src[0];
                dst[3] = 255;
            }
        }

        let _ = SelectObject(mem_dc, old_obj);
        let _ = DeleteObject(hbm.into());
        let _ = DeleteDC(mem_dc);
        let _ = ReleaseDC(Some(hwnd), screen_dc);

        RgbaImageBuffer::from_raw_rgba(width, height, rgba)
    }
}

#[derive(Debug, Clone)]
pub struct UiaNodeInfo {
    pub name: String,
    pub automation_id: String,
    pub control_type: i32,
    pub bounding_rect: (i32, i32, i32, i32),
    pub is_enabled: bool,
    pub is_offscreen: bool,
    pub depth: usize,
}

/// Windows UI Automation (`IUIAutomation`) session for querying `accesskit` control trees.
pub struct UiaSession {
    automation: IUIAutomation,
    com_initialized: bool,
}

impl UiaSession {
    pub fn new() -> Result<Self> {
        unsafe {
            let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
            let com_initialized = hr.is_ok();
            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).map_err(|e| {
                    if com_initialized {
                        CoUninitialize();
                    }
                    QaError::Win32(format!("CoCreateInstance(CUIAutomation) failed: {e}"))
                })?;
            Ok(Self {
                automation,
                com_initialized,
            })
        }
    }

    pub fn element_from_hwnd(&self, hwnd: HWND) -> Result<IUIAutomationElement> {
        unsafe {
            self.automation
                .ElementFromHandle(hwnd)
                .map_err(|e| QaError::Win32(format!("ElementFromHandle failed: {e}")))
        }
    }

    pub fn dump_tree(
        &self,
        root: &IUIAutomationElement,
        max_depth: usize,
    ) -> Result<Vec<UiaNodeInfo>> {
        let walker = unsafe {
            self.automation
                .ControlViewWalker()
                .map_err(|e| QaError::Win32(format!("ControlViewWalker failed: {e}")))?
        };
        let mut out = Vec::new();
        self.walk_recursive(&walker, root, 0, max_depth, &mut out);
        Ok(out)
    }

    fn walk_recursive(
        &self,
        walker: &windows::Win32::UI::Accessibility::IUIAutomationTreeWalker,
        el: &IUIAutomationElement,
        depth: usize,
        max_depth: usize,
        out: &mut Vec<UiaNodeInfo>,
    ) {
        out.push(read_element_info(el, depth));
        if depth >= max_depth {
            return;
        }
        let mut child = unsafe { walker.GetFirstChildElement(el).ok() };
        while let Some(c) = child {
            self.walk_recursive(walker, &c, depth + 1, max_depth, out);
            child = unsafe { walker.GetNextSiblingElement(&c).ok() };
        }
    }

    pub fn find_by_name(
        &self,
        root: &IUIAutomationElement,
        needle: &str,
        max_depth: usize,
    ) -> Result<Option<UiaNodeInfo>> {
        let nodes = self.dump_tree(root, max_depth)?;
        Ok(nodes.into_iter().find(|n| n.name == needle))
    }

    pub fn find_by_automation_id(
        &self,
        root: &IUIAutomationElement,
        automation_id: &str,
        max_depth: usize,
    ) -> Result<Option<IUIAutomationElement>> {
        let walker = unsafe {
            self.automation
                .ControlViewWalker()
                .map_err(|e| QaError::Win32(format!("ControlViewWalker failed: {e}")))?
        };
        Ok(Self::search_id_recursive(&walker, root, automation_id, 0, max_depth))
    }

    fn search_id_recursive(
        walker: &windows::Win32::UI::Accessibility::IUIAutomationTreeWalker,
        el: &IUIAutomationElement,
        target_id: &str,
        depth: usize,
        max_depth: usize,
    ) -> Option<IUIAutomationElement> {
        let info = read_element_info(el, depth);
        if info.automation_id == target_id {
            return Some(el.clone());
        }
        if depth >= max_depth {
            return None;
        }
        let mut child = unsafe { walker.GetFirstChildElement(el).ok() };
        while let Some(c) = child {
            if let Some(hit) =
                Self::search_id_recursive(walker, &c, target_id, depth + 1, max_depth)
            {
                return Some(hit);
            }
            child = unsafe { walker.GetNextSiblingElement(&c).ok() };
        }
        None
    }

    pub fn invoke_element(&self, el: &IUIAutomationElement) -> Result<()> {
        use windows::core::Interface;
        unsafe {
            let unk = el
                .GetCurrentPattern(UIA_InvokePatternId)
                .map_err(|e| QaError::Win32(format!("GetCurrentPattern(Invoke) failed: {e}")))?;
            let pattern: IUIAutomationInvokePattern = unk
                .cast()
                .map_err(|e| QaError::Win32(format!("cast IUIAutomationInvokePattern: {e}")))?;
            pattern
                .Invoke()
                .map_err(|e| QaError::Win32(format!("InvokePattern::Invoke failed: {e}")))
        }
    }
}

impl Drop for UiaSession {
    fn drop(&mut self) {
        if self.com_initialized {
            unsafe {
                CoUninitialize();
            }
        }
    }
}

fn read_element_info(el: &IUIAutomationElement, depth: usize) -> UiaNodeInfo {
    unsafe {
        let name = el.CurrentName().map(|b| b.to_string()).unwrap_or_default();
        let automation_id = el
            .CurrentAutomationId()
            .map(|b| b.to_string())
            .unwrap_or_default();
        let control_type = el.CurrentControlType().map(|c| c.0).unwrap_or(0);
        let rect = el.CurrentBoundingRectangle().unwrap_or_default();
        let is_enabled = el.CurrentIsEnabled().map(|b| b.as_bool()).unwrap_or(false);
        let is_offscreen = el
            .CurrentIsOffscreen()
            .map(|b| b.as_bool())
            .unwrap_or(false);
        UiaNodeInfo {
            name,
            automation_id,
            control_type,
            bounding_rect: (rect.left, rect.top, rect.right, rect.bottom),
            is_enabled,
            is_offscreen,
            depth,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    X1,
    X2,
}

/// Synthesizes real keyboard & mouse events via Win32 `SendInput`.
pub struct InputInjector;

impl InputInjector {
    pub fn focus_window(hwnd: HWND) {
        unsafe {
            let _ = SetForegroundWindow(hwnd);
        }
    }

    pub fn send_key_tap(vk: VIRTUAL_KEY, mods: Modifiers) -> Result<()> {
        Self::send_key_hold(vk, mods, Duration::from_millis(20))
    }

    pub fn send_key_hold(vk: VIRTUAL_KEY, mods: Modifiers, hold: Duration) -> Result<()> {
        let mut down = Vec::with_capacity(4);
        if mods.ctrl {
            down.push(kb_input(VK_CONTROL, KEYBD_EVENT_FLAGS(0)));
        }
        if mods.shift {
            down.push(kb_input(VK_SHIFT, KEYBD_EVENT_FLAGS(0)));
        }
        if mods.alt {
            down.push(kb_input(VK_MENU, KEYBD_EVENT_FLAGS(0)));
        }
        down.push(kb_input(vk, KEYBD_EVENT_FLAGS(0)));
        Self::dispatch_inputs(&down)?;

        thread::sleep(hold);

        let mut up = Vec::with_capacity(4);
        up.push(kb_input(vk, KEYEVENTF_KEYUP));
        if mods.alt {
            up.push(kb_input(VK_MENU, KEYEVENTF_KEYUP));
        }
        if mods.shift {
            up.push(kb_input(VK_SHIFT, KEYEVENTF_KEYUP));
        }
        if mods.ctrl {
            up.push(kb_input(VK_CONTROL, KEYEVENTF_KEYUP));
        }
        Self::dispatch_inputs(&up)
    }

    pub fn send_mouse_click(screen_x: i32, screen_y: i32, btn: MouseButton) -> Result<()> {
        Self::send_mouse_hold(screen_x, screen_y, btn, Duration::from_millis(20))
    }

    pub fn send_mouse_hold(
        screen_x: i32,
        screen_y: i32,
        btn: MouseButton,
        hold: Duration,
    ) -> Result<()> {
        let (abs_x, abs_y) = to_absolute_coords(screen_x, screen_y);
        let (down_flag, up_flag, x_data) = match btn {
            MouseButton::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, 0),
            MouseButton::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, 0),
            MouseButton::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, 0),
            MouseButton::X1 => (MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, 1),
            MouseButton::X2 => (MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, 2),
        };
        Self::dispatch_inputs(&[
            mouse_input(
                abs_x,
                abs_y,
                0,
                MOUSE_EVENT_FLAGS(MOUSEEVENTF_MOVE.0 | MOUSEEVENTF_ABSOLUTE.0),
            ),
            mouse_input(abs_x, abs_y, x_data, down_flag),
        ])?;
        thread::sleep(hold);
        Self::dispatch_inputs(&[mouse_input(abs_x, abs_y, x_data, up_flag)])
    }

    pub fn send_mouse_wheel(screen_x: i32, screen_y: i32, detents: i32) -> Result<()> {
        let (abs_x, abs_y) = to_absolute_coords(screen_x, screen_y);
        Self::dispatch_inputs(&[
            mouse_input(
                abs_x,
                abs_y,
                0,
                MOUSE_EVENT_FLAGS(MOUSEEVENTF_MOVE.0 | MOUSEEVENTF_ABSOLUTE.0),
            ),
            mouse_input(abs_x, abs_y, detents * 120, MOUSEEVENTF_WHEEL),
        ])
    }

    fn dispatch_inputs(inputs: &[INPUT]) -> Result<()> {
        let sent = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
        if sent as usize != inputs.len() {
            return Err(QaError::Win32(format!(
                "SendInput dispatched {sent} of {} events",
                inputs.len()
            )));
        }
        Ok(())
    }
}

fn kb_input(vk: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn mouse_input(dx: i32, dy: i32, mouse_data: i32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: mouse_data as u32,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn to_absolute_coords(screen_x: i32, screen_y: i32) -> (i32, i32) {
    let w = unsafe { GetSystemMetrics(SM_CXSCREEN) }.max(1);
    let h = unsafe { GetSystemMetrics(SM_CYSCREEN) }.max(1);
    let ax = ((screen_x as i64 * 65535) / (w as i64 - 1).max(1)).clamp(0, 65535) as i32;
    let ay = ((screen_y as i64 * 65535) / (h as i64 - 1).max(1)).clamp(0, 65535) as i32;
    (ax, ay)
}

/// Reads an unused helper so `WPARAM`/`LPARAM` import stays clean if needed.
#[allow(dead_code)]
fn _lparam_marker(_w: WPARAM, _l: LPARAM) {}
