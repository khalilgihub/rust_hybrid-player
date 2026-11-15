use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hybrid_ui::primitives::HitTarget;
use hybrid_ui::state::{UiState, WindowMaximizeState};
use hybrid_ui::HybridUi;
use hybrid_win32_shell::{
    apply_null_brush, extract_hwnd_from_winit, DipRect, ShellWindowEvent, ThemeMode, WindowSubclass,
};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowPlacement, GetWindowRect, IsZoomed, SendMessageW, HTCAPTION, HTMAXBUTTON, SC_RESTORE,
    WINDOWPLACEMENT, WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP,
    WM_NCMOUSEMOVE, WM_SYSCOMMAND,
};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::platform::windows::EventLoopBuilderExtWindows;
use winit::window::{Window, WindowAttributes, WindowId};

#[derive(Debug, Clone)]
enum TestStep {
    Record(&'static str),
    EnterFullscreen,
    RealClickMaxButton,
    ExitFullscreenViaKey,
    DoubleClickCaption,
    SysCommandRestore,
    VerifyAndFinish,
}

#[derive(Debug, Clone)]
struct Snapshot {
    label: &'static str,
    is_fs: bool,
    is_max: bool,
    is_zoomed: bool,
    rect: (i32, i32, i32, i32),
    work: (i32, i32, i32, i32),
    monitor: (i32, i32, i32, i32),
    ui_state: WindowMaximizeState,
}

struct LiveTestApp {
    proxy: EventLoopProxy<ShellWindowEvent>,
    window: Option<Arc<Window>>,
    hwnd: Option<HWND>,
    subclass: Option<WindowSubclass>,
    ui_engine: HybridUi,
    ui: UiState,
    max_btn_rect_dip: Option<DipRect>,
    caption_pt_dip: Option<(i32, i32)>,
    steps: Vec<(Instant, TestStep)>,
    next_step_idx: usize,
    results: Arc<Mutex<Vec<Snapshot>>>,
}

impl LiveTestApp {
    fn sync_ui_and_regions(&mut self) {
        let Some(window) = &self.window else { return };
        let Some(subclass) = &self.subclass else {
            return;
        };
        let dpr = window.scale_factor() as f32;
        let size = window.inner_size();
        self.ui.viewport_w = (size.width as f32 / dpr).max(320.0);
        self.ui.viewport_h = (size.height as f32 / dpr).max(240.0);
        self.ui.dpr = dpr;
        self.ui.window_maximize_state = if subclass.is_fullscreen() {
            WindowMaximizeState::Fullscreen
        } else if subclass.is_maximized() {
            WindowMaximizeState::Maximized
        } else {
            WindowMaximizeState::Restored
        };
        let frame = self.ui_engine.build_frame(&self.ui);
        let mut max_rect = None;
        let mut drag_rect = None;
        let mut exclusions = Vec::new();
        for region in &frame.hit_regions {
            let dr = DipRect::new(
                region.rect.x.round() as i32,
                region.rect.y.round() as i32,
                region.rect.width.round() as i32,
                region.rect.height.round() as i32,
            );
            match region.target {
                HitTarget::TitlebarMaximize => max_rect = Some(dr),
                HitTarget::TitlebarDrag => drag_rect = Some(dr),
                HitTarget::TitlebarMinimize | HitTarget::TitlebarClose => exclusions.push(dr),
                _ if region.rect.y < 40.0 => exclusions.push(dr),
                _ => {}
            }
        }
        self.max_btn_rect_dip = max_rect;
        self.caption_pt_dip = drag_rect.map(|r| (r.x + r.width / 2, r.y + r.height / 2));
        subclass.set_maximize_button_rect(max_rect);
        subclass.set_titlebar_drag_rect(drag_rect);
        subclass.set_hit_test_exclusions(exclusions);
    }

    fn record(&mut self, label: &'static str) {
        self.sync_ui_and_regions();
        let Some(hwnd) = self.hwnd else { return };
        let Some(subclass) = &self.subclass else {
            return;
        };
        let mut win_rc = RECT::default();
        let _ = unsafe { GetWindowRect(hwnd, &mut win_rc) };
        let mon = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = unsafe { GetMonitorInfoW(mon, &mut mi) };
        let mut wp = WINDOWPLACEMENT {
            length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
            ..Default::default()
        };
        let _ = unsafe { GetWindowPlacement(hwnd, &mut wp) };
        let snap = Snapshot {
            label,
            is_fs: subclass.is_fullscreen(),
            is_max: subclass.is_maximized(),
            is_zoomed: unsafe { IsZoomed(hwnd).as_bool() },
            rect: (
                win_rc.left,
                win_rc.top,
                win_rc.right - win_rc.left,
                win_rc.bottom - win_rc.top,
            ),
            work: (
                mi.rcWork.left,
                mi.rcWork.top,
                mi.rcWork.right - mi.rcWork.left,
                mi.rcWork.bottom - mi.rcWork.top,
            ),
            monitor: (
                mi.rcMonitor.left,
                mi.rcMonitor.top,
                mi.rcMonitor.right - mi.rcMonitor.left,
                mi.rcMonitor.bottom - mi.rcMonitor.top,
            ),
            ui_state: self.ui.window_maximize_state,
        };
        eprintln!(
            "[SNAP {:32}] fs={} max={} zoomed={} ui={:?} rect={:?} work={:?} normal={:?}",
            snap.label,
            snap.is_fs,
            snap.is_max,
            snap.is_zoomed,
            snap.ui_state,
            snap.rect,
            snap.work,
            (
                wp.rcNormalPosition.left,
                wp.rcNormalPosition.top,
                wp.rcNormalPosition.right - wp.rcNormalPosition.left,
                wp.rcNormalPosition.bottom - wp.rcNormalPosition.top,
            )
        );
        self.results.lock().unwrap().push(snap);
    }

    fn handle_max_btn_click(&mut self) {
        if let Some(st) = &self.subclass {
            if st.is_fullscreen() {
                let _ = st.exit_fullscreen_to_windowed();
            } else {
                st.toggle_maximize();
            }
        }
        self.sync_ui_and_regions();
    }
}

impl ApplicationHandler<ShellWindowEvent> for LiveTestApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = WindowAttributes::default()
            .with_title("Hybrid Live Fullscreen Test")
            .with_decorations(false)
            .with_transparent(false)
            .with_inner_size(LogicalSize::new(1280.0, 720.0))
            .with_min_inner_size(LogicalSize::new(640.0, 360.0))
            .with_visible(false);
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let hwnd = extract_hwnd_from_winit(&window).expect("hwnd");
        let _ = apply_null_brush(hwnd);
        let subclass = WindowSubclass::install(hwnd, ThemeMode::Dark).expect("install subclass");
        let proxy = self.proxy.clone();
        subclass.set_event_callback(move |ev| {
            let _ = proxy.send_event(ev);
        });
        self.window = Some(Arc::clone(&window));
        self.hwnd = Some(hwnd);
        self.subclass = Some(subclass);
        self.sync_ui_and_regions();
        window.set_visible(true);
        if let Some(sub) = &self.subclass {
            sub.maximize();
        }
        self.sync_ui_and_regions();

        let t0 = Instant::now();
        self.steps = vec![
            (
                t0 + Duration::from_millis(250),
                TestStep::Record("initial_maximized"),
            ),
            (t0 + Duration::from_millis(300), TestStep::EnterFullscreen),
            (
                t0 + Duration::from_millis(550),
                TestStep::Record("in_fullscreen"),
            ),
            (
                t0 + Duration::from_millis(600),
                TestStep::RealClickMaxButton,
            ),
            (
                t0 + Duration::from_millis(950),
                TestStep::Record("settled_after_f_click_restore"),
            ),
            (
                t0 + Duration::from_millis(1000),
                TestStep::RealClickMaxButton,
            ),
            (
                t0 + Duration::from_millis(1350),
                TestStep::Record("settled_after_restore_click_max"),
            ),
            (
                t0 + Duration::from_millis(1400),
                TestStep::RealClickMaxButton,
            ),
            (
                t0 + Duration::from_millis(1750),
                TestStep::Record("settled_after_max_click_restore2"),
            ),
            (
                t0 + Duration::from_millis(1800),
                TestStep::RealClickMaxButton,
            ),
            (
                t0 + Duration::from_millis(2150),
                TestStep::Record("settled_after_restore2_click_max2"),
            ),
            (t0 + Duration::from_millis(2200), TestStep::EnterFullscreen),
            (
                t0 + Duration::from_millis(2450),
                TestStep::ExitFullscreenViaKey,
            ),
            (
                t0 + Duration::from_millis(2750),
                TestStep::Record("settled_after_f_f_max"),
            ),
            (
                t0 + Duration::from_millis(2800),
                TestStep::RealClickMaxButton,
            ),
            (
                t0 + Duration::from_millis(3150),
                TestStep::Record("settled_after_f_f_click_restore"),
            ),
            (
                t0 + Duration::from_millis(3200),
                TestStep::DoubleClickCaption,
            ),
            (
                t0 + Duration::from_millis(3550),
                TestStep::Record("settled_after_dblclick_max"),
            ),
            (t0 + Duration::from_millis(3600), TestStep::EnterFullscreen),
            (
                t0 + Duration::from_millis(3850),
                TestStep::SysCommandRestore,
            ),
            (
                t0 + Duration::from_millis(4200),
                TestStep::Record("settled_after_f_sc_restore"),
            ),
            (t0 + Duration::from_millis(4250), TestStep::VerifyAndFinish),
        ];
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.steps[0].0));
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: ShellWindowEvent) {
        eprintln!("[SHELL_EVENT] {:?}", event);
        match event {
            ShellWindowEvent::MaxButtonClicked => {
                self.handle_max_btn_click();
            }
            ShellWindowEvent::MaximizedChanged(_) | ShellWindowEvent::FullscreenChanged(_) => {
                self.sync_ui_and_regions();
            }
            _ => {}
        }
    }

    fn window_event(&mut self, _event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if matches!(
            event,
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }
        ) {
            self.sync_ui_and_regions();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        while self.next_step_idx < self.steps.len() && now >= self.steps[self.next_step_idx].0 {
            let step = self.steps[self.next_step_idx].1.clone();
            self.next_step_idx += 1;
            match step {
                TestStep::Record(label) => {
                    self.record(label);
                }
                TestStep::EnterFullscreen => {
                    if let Some(sub) = &self.subclass {
                        let _ = sub.set_fullscreen(true);
                    }
                    self.sync_ui_and_regions();
                }
                TestStep::RealClickMaxButton => {
                    self.sync_ui_and_regions();
                    if let (Some(hwnd), Some(dr)) = (self.hwnd, self.max_btn_rect_dip) {
                        unsafe {
                            let dpi = GetDpiForWindow(hwnd);
                            let scale = dpi as f64 / 96.0;
                            let mut pt = POINT {
                                x: ((dr.x as f64 + dr.width as f64 * 0.5) * scale).round() as i32,
                                y: ((dr.y as f64 + dr.height as f64 * 0.5) * scale).round() as i32,
                            };
                            let _ = ClientToScreen(hwnd, &mut pt);
                            let lp = ((pt.y as isize) << 16) | ((pt.x as u16) as isize);
                            let ht =
                                SendMessageW(hwnd, WM_NCHITTEST, Some(WPARAM(0)), Some(LPARAM(lp)))
                                    .0;
                            assert_eq!(ht, HTMAXBUTTON as isize, "expected HTMAXBUTTON at {pt:?}");
                            let _ = SendMessageW(
                                hwnd,
                                WM_NCMOUSEMOVE,
                                Some(WPARAM(HTMAXBUTTON as usize)),
                                Some(LPARAM(lp)),
                            );
                            let _ = SendMessageW(
                                hwnd,
                                WM_NCLBUTTONDOWN,
                                Some(WPARAM(HTMAXBUTTON as usize)),
                                Some(LPARAM(lp)),
                            );
                            let _ = SendMessageW(
                                hwnd,
                                WM_NCLBUTTONUP,
                                Some(WPARAM(HTMAXBUTTON as usize)),
                                Some(LPARAM(lp)),
                            );
                        }
                    }
                }
                TestStep::ExitFullscreenViaKey => {
                    if let Some(sub) = &self.subclass {
                        let _ = sub.set_fullscreen(false);
                    }
                    self.sync_ui_and_regions();
                }
                TestStep::DoubleClickCaption => {
                    if let (Some(hwnd), Some((cx, cy))) = (self.hwnd, self.caption_pt_dip) {
                        unsafe {
                            let dpi = GetDpiForWindow(hwnd);
                            let scale = dpi as f64 / 96.0;
                            let mut pt = POINT {
                                x: ((cx as f64) * scale).round() as i32,
                                y: ((cy as f64) * scale).round() as i32,
                            };
                            let _ = ClientToScreen(hwnd, &mut pt);
                            let lp = ((pt.y as isize) << 16) | ((pt.x as u16) as isize);
                            let _ = SendMessageW(
                                hwnd,
                                WM_NCLBUTTONDBLCLK,
                                Some(WPARAM(HTCAPTION as usize)),
                                Some(LPARAM(lp)),
                            );
                        }
                    }
                }
                TestStep::SysCommandRestore => {
                    if let Some(hwnd) = self.hwnd {
                        unsafe {
                            let _ = SendMessageW(
                                hwnd,
                                WM_SYSCOMMAND,
                                Some(WPARAM(SC_RESTORE as usize)),
                                Some(LPARAM(0)),
                            );
                        }
                    }
                }
                TestStep::VerifyAndFinish => {
                    event_loop.exit();
                    return;
                }
            }
        }
        if let Some((next_t, _)) = self.steps.get(self.next_step_idx) {
            event_loop.set_control_flow(ControlFlow::WaitUntil(*next_t));
        }
    }
}

#[test]
fn live_winit_fullscreen_and_real_click_restore_roundtrip() {
    let event_loop = EventLoop::<ShellWindowEvent>::with_user_event()
        .with_any_thread(true)
        .build()
        .expect("build event loop");
    let proxy = event_loop.create_proxy();
    let results = Arc::new(Mutex::new(Vec::new()));
    let mut app = LiveTestApp {
        proxy,
        window: None,
        hwnd: None,
        subclass: None,
        ui_engine: HybridUi::new(),
        ui: UiState::default(),
        max_btn_rect_dip: None,
        caption_pt_dip: None,
        steps: Vec::new(),
        next_step_idx: 0,
        results: Arc::clone(&results),
    };
    event_loop.run_app(&mut app).expect("run event loop");

    let snaps = results.lock().unwrap();
    assert_eq!(snaps.len(), 10);
    for s in snaps.iter() {
        match s.label {
            "initial_maximized"
            | "settled_after_restore_click_max"
            | "settled_after_restore2_click_max2"
            | "settled_after_f_f_max"
            | "settled_after_dblclick_max" => {
                assert!(
                    !s.is_fs
                        && s.is_max
                        && s.is_zoomed
                        && s.ui_state == WindowMaximizeState::Maximized,
                    "{} should be maximized: {s:?}",
                    s.label
                );
            }
            "in_fullscreen" => {
                assert!(
                    s.is_fs
                        && !s.is_max
                        && !s.is_zoomed
                        && s.ui_state == WindowMaximizeState::Fullscreen,
                    "{} should be fullscreen: {s:?}",
                    s.label
                );
                assert_eq!(s.rect, s.monitor, "fullscreen rect must match monitor");
            }
            "settled_after_f_click_restore"
            | "settled_after_max_click_restore2"
            | "settled_after_f_f_click_restore"
            | "settled_after_f_sc_restore" => {
                assert!(
                    !s.is_fs
                        && !s.is_max
                        && !s.is_zoomed
                        && s.ui_state == WindowMaximizeState::Restored,
                    "{} should be restored windowed: {s:?}",
                    s.label
                );
                assert!(
                    s.rect.2 < s.work.2 - 4 && s.rect.3 < s.work.3 - 4,
                    "{} should be strictly smaller than work area {:?}, got {:?}",
                    s.label,
                    s.work,
                    s.rect
                );
            }
            _ => {}
        }
    }
}
