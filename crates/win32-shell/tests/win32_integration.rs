use std::ffi::c_void;
use std::fs;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, SystemTime};

use hybrid_win32_shell::{
    apply_native_dwm_window_styles, clamp_dragged_bounds, cleanup_temp_files,
    compute_drag_from_maximize_start, compute_nccalcsize_client_rect, decode_pipe_args,
    dip_to_physical, dpi_for_window, encode_pipe_args, is_process_running, physical_to_dip,
    render_companion_vector_layer, resolve_nc_hit_test, set_current_process_app_user_model_id,
    set_high_res_timer, trash_file, unpack_lparam_point, CompanionAuthHandle, CompanionAuthOptions,
    CompanionAuthState, CompanionSvgAssets, DipRect, HighResTimerGuard, HitTestConfig,
    ShellWindowEvent, SingleInstanceOutcome, SingleInstanceServer, ThemeMode, WindowBounds,
    WindowSubclass, APP_ID, COMPANION_HEIGHT_DIP, COMPANION_WIDTH_DIP, DWMNCRP_ENABLED_VAL,
    DWMSBT_NONE_VAL, DWMWA_BORDER_COLOR_ID, DWMWA_COLOR_NONE_VAL, DWMWA_NCRENDERING_POLICY_ID,
    DWMWA_SYSTEMBACKDROP_TYPE_ID, DWMWA_USE_IMMERSIVE_DARK_MODE_ID,
    DWMWA_WINDOW_CORNER_PREFERENCE_ID, DWMWCP_DONOTROUND_VAL, FRAMELESS_WINDOW_STYLE_MASK,
};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DwmSetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS,
    DWMWA_NCRENDERING_ENABLED, DWMWINDOWATTRIBUTE,
};
use windows::Win32::Graphics::Gdi::{ClientToScreen, GetStockObject, NULL_BRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClassLongPtrW, GetClientRect,
    GetWindowLongPtrW, GetWindowTextW, RegisterClassExW, SendMessageW, GCLP_HBRBACKGROUND,
    GWL_STYLE, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION, HTCLIENT, HTLEFT, HTMAXBUTTON,
    HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, NCCALCSIZE_PARAMS, WM_ENTERSIZEMOVE, WM_ERASEBKGND,
    WM_EXITSIZEMOVE, WM_NCACTIVATE, WM_NCCALCSIZE, WM_NCHITTEST, WM_NCPAINT, WNDCLASSEXW,
    WS_CAPTION, WS_OVERLAPPEDWINDOW, WVR_REDRAW,
};

fn to_wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe extern "system" fn test_wnd_proc(
    hwnd: HWND,
    umsg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    DefWindowProcW(hwnd, umsg, wparam, lparam)
}

fn create_test_hwnd(class_suffix: &str) -> HWND {
    let class_name = to_wide_null(&format!("HybridWin32ShellTest_{class_suffix}"));
    let title = to_wide_null("Hybrid Test Window");
    let hinst = unsafe { GetModuleHandleW(None) }.unwrap_or_default();

    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(test_wnd_proc),
        hInstance: hinst.into(),
        lpszClassName: PCWSTR(class_name.as_ptr()),
        ..Default::default()
    };
    unsafe {
        let _ = RegisterClassExW(&wc);
        CreateWindowExW(
            Default::default(),
            PCWSTR(class_name.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPEDWINDOW,
            100,
            100,
            1024,
            640,
            None,
            None,
            Some(hinst.into()),
            None,
        )
        .expect("CreateWindowExW test window")
    }
}

fn get_dwm_bool(hwnd: HWND, attr: DWMWINDOWATTRIBUTE) -> bool {
    let mut val = 0i32;
    unsafe {
        DwmGetWindowAttribute(
            hwnd,
            attr,
            (&mut val as *mut i32).cast(),
            std::mem::size_of::<i32>() as u32,
        )
        .expect("DwmGetWindowAttribute");
    }
    val != 0
}

fn try_set_dwm_u32(hwnd: HWND, attr_id: u32, value: u32) -> windows::core::Result<()> {
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWINDOWATTRIBUTE(attr_id as i32),
            (&value as *const u32).cast(),
            std::mem::size_of::<u32>() as u32,
        )
    }
}

#[test]
fn hit_test_precedence_and_8dip_borders() {
    let cfg = HitTestConfig {
        maximize_button_rect: Some(DipRect::new(1180, 0, 46, 32)),
        exclusions: vec![DipRect::new(1134, 0, 146, 32)],
        titlebar_drag_rect: Some(DipRect::new(0, 0, 1280, 32)),
        resize_border_dip: 8,
        is_maximized: false,
        is_fullscreen: false,
        is_minimized: false,
        is_resizable: true,
    };

    // 1. HTMAXBUTTON takes priority even if inside exclusions & top edge
    assert_eq!(
        resolve_nc_hit_test(1200, 16, 1280, 720, &cfg),
        Some(HTMAXBUTTON as isize)
    );

    // 2. Exclusions (minimize / close buttons) return HTCLIENT, taking priority over 8-DIP edges
    assert_eq!(
        resolve_nc_hit_test(1150, 16, 1280, 720, &cfg),
        Some(HTCLIENT as isize)
    );
    assert_eq!(
        resolve_nc_hit_test(1250, 16, 1280, 720, &cfg),
        Some(HTCLIENT as isize)
    );
    assert_eq!(
        resolve_nc_hit_test(1276, 4, 1280, 720, &cfg),
        Some(HTCLIENT as isize)
    );

    // 3. 8-DIP corners & edges when windowed (both unexcluded top-right & standard edges)
    let mut unexcluded_cfg = cfg.clone();
    unexcluded_cfg.exclusions.clear();
    unexcluded_cfg.maximize_button_rect = None;
    assert_eq!(
        resolve_nc_hit_test(1276, 4, 1280, 720, &unexcluded_cfg),
        Some(HTTOPRIGHT as isize)
    );
    assert_eq!(
        resolve_nc_hit_test(1200, 4, 1280, 720, &unexcluded_cfg),
        Some(HTTOP as isize)
    );
    assert_eq!(
        resolve_nc_hit_test(4, 4, 1280, 720, &cfg),
        Some(HTTOPLEFT as isize)
    );
    assert_eq!(
        resolve_nc_hit_test(4, 716, 1280, 720, &cfg),
        Some(HTBOTTOMLEFT as isize)
    );
    assert_eq!(
        resolve_nc_hit_test(1276, 716, 1280, 720, &cfg),
        Some(HTBOTTOMRIGHT as isize)
    );
    assert_eq!(
        resolve_nc_hit_test(500, 4, 1280, 720, &cfg),
        Some(HTTOP as isize)
    );
    assert_eq!(
        resolve_nc_hit_test(4, 300, 1280, 720, &cfg),
        Some(HTLEFT as isize)
    );
    assert_eq!(
        resolve_nc_hit_test(500, 716, 1280, 720, &cfg),
        Some(HTBOTTOM as isize)
    );
    assert_eq!(
        resolve_nc_hit_test(1276, 300, 1280, 720, &cfg),
        Some(HTRIGHT as isize)
    );

    // 4. Titlebar drag region outside 8-DIP edge & exclusions -> HTCAPTION
    assert_eq!(
        resolve_nc_hit_test(400, 16, 1280, 720, &cfg),
        Some(HTCAPTION as isize)
    );

    // 5. Client body -> HTCLIENT
    assert_eq!(
        resolve_nc_hit_test(400, 200, 1280, 720, &cfg),
        Some(HTCLIENT as isize)
    );

    // 6. Maximized locks all 8-DIP edges to HTCLIENT while preserving HTMAXBUTTON & HTCAPTION
    let mut max_cfg = cfg.clone();
    max_cfg.is_maximized = true;
    assert_eq!(
        resolve_nc_hit_test(4, 4, 1280, 720, &max_cfg),
        Some(HTCLIENT as isize)
    );
    assert_eq!(
        resolve_nc_hit_test(1200, 16, 1280, 720, &max_cfg),
        Some(HTMAXBUTTON as isize)
    );
    assert_eq!(
        resolve_nc_hit_test(400, 16, 1280, 720, &max_cfg),
        Some(HTCAPTION as isize)
    );

    // 7. Fullscreen locks edges AND disables HTCAPTION drag
    let mut fs_cfg = cfg;
    fs_cfg.is_fullscreen = true;
    assert_eq!(
        resolve_nc_hit_test(400, 16, 1280, 720, &fs_cfg),
        Some(HTCLIENT as isize)
    );
}

#[test]
fn nccalcsize_and_drag_from_maximize_math() {
    let proposed = RECT {
        left: 100,
        top: 100,
        right: 1380,
        bottom: 820,
    };
    let work = RECT {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1032,
    };
    let full = RECT {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1080,
    };

    assert_eq!(
        compute_nccalcsize_client_rect(proposed, work, full, false, false),
        proposed
    );
    assert_eq!(
        compute_nccalcsize_client_rect(proposed, work, full, true, false),
        work
    );
    assert_eq!(
        compute_nccalcsize_client_rect(proposed, work, full, true, true),
        full
    );

    // Drag-from-maximize anchor & multi-monitor clamping
    let saved = Some(WindowBounds {
        x: 200,
        y: 120,
        width: 1280,
        height: 720,
    });
    let (bounds, session, ratio) = compute_drag_from_maximize_start(960, 14, full, work, saved);
    assert!((ratio - 0.5).abs() < 0.01);
    assert_eq!(bounds.width, 1280);
    assert_eq!(bounds.height, 720);
    assert_eq!(bounds.y, 0);
    assert!(session.active);

    // Offscreen drag clamped so >=40px horizontal and >=80px vertical stay visible
    let clamped = clamp_dragged_bounds(
        WindowBounds {
            x: 5000,
            y: 2000,
            width: 1280,
            height: 720,
        },
        work,
    );
    assert_eq!(clamped.x, work.right - 40);
    assert_eq!(clamped.y, work.bottom - 80);
}

#[test]
fn lparam_unpack_and_dpi_scaling() {
    let packed = ((-25i16 as u16 as u32) << 16) | (120i16 as u16 as u32);
    assert_eq!(unpack_lparam_point(packed as isize), (120, -25));
    assert_eq!(dip_to_physical(100, 144), 150);
    assert_eq!(physical_to_dip(150, 144), 100);
}

#[test]
fn real_hwnd_dwm_styles_and_subclass_messages() {
    let hwnd = create_test_hwnd("dwm_subclass");
    let subclass = WindowSubclass::install(hwnd, ThemeMode::Dark).expect("install subclass");

    let events: Arc<Mutex<Vec<ShellWindowEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_cb = Arc::clone(&events);
    subclass.set_event_callback(move |ev| {
        events_cb.lock().unwrap().push(ev);
    });

    // Verify exact frameless Win32 styles + NULL_BRUSH
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) };
    assert_eq!(
        style & FRAMELESS_WINDOW_STYLE_MASK,
        FRAMELESS_WINDOW_STYLE_MASK
    );
    let hbr = unsafe { GetClassLongPtrW(hwnd, GCLP_HBRBACKGROUND) };
    let null_brush = unsafe { GetStockObject(NULL_BRUSH) }.0 as usize;
    assert_eq!(hbr, null_brush);

    // Verify readable DWM attributes and that exact write-only DWM attributes accept our values.
    assert!(get_dwm_bool(hwnd, DWMWA_NCRENDERING_ENABLED));
    let mut ext_bounds = RECT::default();
    unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&mut ext_bounds as *mut RECT).cast(),
            std::mem::size_of::<RECT>() as u32,
        )
        .expect("DWMWA_EXTENDED_FRAME_BOUNDS");
    }
    assert!(ext_bounds.right > ext_bounds.left);

    try_set_dwm_u32(
        hwnd,
        DWMWA_NCRENDERING_POLICY_ID,
        DWMNCRP_ENABLED_VAL as u32,
    )
    .expect("DWMWA_NCRENDERING_POLICY");
    try_set_dwm_u32(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE_ID, 1)
        .expect("DWMWA_USE_IMMERSIVE_DARK_MODE dark");
    assert_eq!(ThemeMode::Dark.immersive_dark_mode_value(), 1);
    assert_eq!(ThemeMode::Oled.immersive_dark_mode_value(), 1);
    assert_eq!(ThemeMode::Light.immersive_dark_mode_value(), 0);

    // Win11-specific attributes (33, 34, 38)
    let hr_corner = try_set_dwm_u32(
        hwnd,
        DWMWA_WINDOW_CORNER_PREFERENCE_ID,
        DWMWCP_DONOTROUND_VAL as u32,
    );
    if hr_corner.is_ok() {
        try_set_dwm_u32(hwnd, DWMWA_BORDER_COLOR_ID, DWMWA_COLOR_NONE_VAL)
            .expect("DWMWA_BORDER_COLOR");
        try_set_dwm_u32(hwnd, DWMWA_SYSTEMBACKDROP_TYPE_ID, DWMSBT_NONE_VAL as u32)
            .expect("DWMWA_SYSTEMBACKDROP_TYPE");
    }

    // Switch theme to Light and back to OLED cleanly without error.
    subclass
        .set_theme(ThemeMode::Light)
        .expect("set light theme");
    subclass.set_theme(ThemeMode::Oled).expect("set oled theme");

    // Verify message handlers: WM_NCCALCSIZE, WM_NCPAINT, WM_ERASEBKGND, WM_NCACTIVATE
    let mut ncc = NCCALCSIZE_PARAMS::default();
    let r_calc_true = unsafe {
        SendMessageW(
            hwnd,
            WM_NCCALCSIZE,
            Some(WPARAM(1)),
            Some(LPARAM(&mut ncc as *mut _ as isize)),
        )
    };
    assert_eq!(r_calc_true.0, WVR_REDRAW as isize);

    let r_calc_false =
        unsafe { SendMessageW(hwnd, WM_NCCALCSIZE, Some(WPARAM(0)), Some(LPARAM(0))) };
    assert_eq!(r_calc_false.0, 0);

    let r_ncpaint = unsafe { SendMessageW(hwnd, WM_NCPAINT, Some(WPARAM(1)), Some(LPARAM(0))) };
    assert_eq!(r_ncpaint.0, 0);

    let r_erase = unsafe { SendMessageW(hwnd, WM_ERASEBKGND, Some(WPARAM(0)), Some(LPARAM(0))) };
    assert_eq!(r_erase.0, 1);

    let r_ncact = unsafe { SendMessageW(hwnd, WM_NCACTIVATE, Some(WPARAM(1)), Some(LPARAM(0))) };
    assert_eq!(r_ncact.0, 1);

    // Verify WM_NCHITTEST on live HWND (HTMAXBUTTON + HTCAPTION)
    subclass.set_maximize_button_rect(Some(DipRect::new(400, 0, 46, 32)));
    subclass.set_titlebar_drag_rect(Some(DipRect::new(0, 0, 800, 32)));
    let dpi = dpi_for_window(hwnd);
    let mut pt = POINT {
        x: dip_to_physical(420, dpi),
        y: dip_to_physical(16, dpi),
    };
    unsafe {
        let _ = ClientToScreen(hwnd, &mut pt);
    }
    let lparam_pt = ((pt.y as i16 as u16 as u32) << 16) | (pt.x as i16 as u16 as u32);
    let ht = unsafe {
        SendMessageW(
            hwnd,
            WM_NCHITTEST,
            Some(WPARAM(0)),
            Some(LPARAM(lparam_pt as isize)),
        )
    };
    assert_eq!(ht.0, HTMAXBUTTON as isize);

    // Verify WM_ENTERSIZEMOVE / WM_EXITSIZEMOVE resize guard
    unsafe {
        let _ = SendMessageW(hwnd, WM_ENTERSIZEMOVE, Some(WPARAM(0)), Some(LPARAM(0)));
    }
    assert!(subclass.is_resizing());
    // While resizing, apply_native_dwm_window_styles must skip (return Ok(false))
    assert!(
        !apply_native_dwm_window_styles(hwnd, ThemeMode::Dark, subclass.is_resizing(), false)
            .unwrap()
    );
    unsafe {
        let _ = SendMessageW(hwnd, WM_EXITSIZEMOVE, Some(WPARAM(0)), Some(LPARAM(0)));
    }
    assert!(!subclass.is_resizing());
    {
        let evs = events.lock().unwrap();
        assert!(evs.contains(&ShellWindowEvent::BoundsAnimating(true)));
        assert!(evs.contains(&ShellWindowEvent::BoundsAnimating(false)));
    }

    // Verify fullscreen guard strips WS_CAPTION, blocks DWM caption re-application,
    // reconciles SC_RESTORE / shrinking WM_SIZE, and exit_fullscreen_to_windowed restores windowed state.
    subclass.maximize();
    subclass
        .set_fullscreen(true)
        .expect("enter fullscreen from maximized");
    assert!(subclass.is_fullscreen());
    assert!(events
        .lock()
        .unwrap()
        .contains(&ShellWindowEvent::FullscreenChanged(true)));
    let fs_style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) };
    assert_eq!(fs_style & (WS_CAPTION.0 as isize), 0);
    assert!(!apply_native_dwm_window_styles(
        hwnd,
        ThemeMode::Dark,
        false,
        subclass.is_fullscreen()
    )
    .unwrap());
    // Pressing TitlebarMaximize / Restore while fullscreen (even if pre_fullscreen_maximized was true)
    // must restore directly to windowed rather than snapping back to maximized.
    subclass
        .exit_fullscreen_to_windowed()
        .expect("exit fullscreen to windowed");
    assert!(!subclass.is_fullscreen());
    assert!(!subclass.is_maximized());
    assert!(events
        .lock()
        .unwrap()
        .contains(&ShellWindowEvent::FullscreenChanged(false)));

    // External WM_SYSCOMMAND SC_RESTORE while fullscreen must reconcile is_fullscreen -> false.
    events.lock().unwrap().clear();
    subclass.set_fullscreen(true).expect("re-enter fullscreen");
    assert!(subclass.is_fullscreen());
    unsafe {
        let _ = SendMessageW(
            hwnd,
            windows::Win32::UI::WindowsAndMessaging::WM_SYSCOMMAND,
            Some(WPARAM(
                windows::Win32::UI::WindowsAndMessaging::SC_RESTORE as usize,
            )),
            Some(LPARAM(0)),
        );
    }
    assert!(!subclass.is_fullscreen());
    assert!(events
        .lock()
        .unwrap()
        .contains(&ShellWindowEvent::FullscreenChanged(false)));

    drop(subclass);
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

#[test]
fn single_instance_mutex_and_named_pipe_roundtrip() {
    let unique = format!(
        "{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let mutex_name = format!("Local\\HybridPlayerTestMutex_{unique}");
    let pipe_name = format!("\\\\.\\pipe\\hybrid-player-test-pipe-{unique}");

    // Packet codec unit check
    let sample_args = vec![
        "C:\\Videos\\My Movie (4K).mkv".to_string(),
        "https://www.youtube.com/watch?v=dQw4w9WgXcQ".to_string(),
    ];
    let encoded = encode_pipe_args(&sample_args);
    assert_eq!(decode_pipe_args(&encoded), Some(sample_args.clone()));

    // Primary acquires mutex and starts pipe listener
    let primary = match SingleInstanceServer::acquire(&mutex_name, &pipe_name, &[]).unwrap() {
        SingleInstanceOutcome::Primary(srv) => srv,
        SingleInstanceOutcome::SecondaryForwarded => panic!("expected primary"),
    };

    let (tx, rx) = mpsc::channel();
    primary.start_listener(None, move |args| {
        let _ = tx.send(args);
    });

    // Give listener thread a brief moment to bind CreateNamedPipeW
    std::thread::sleep(Duration::from_millis(40));

    // Secondary sees mutex already exists and forwards CLI args over the named pipe
    let secondary = SingleInstanceServer::acquire(&mutex_name, &pipe_name, &sample_args).unwrap();
    assert!(matches!(
        secondary,
        SingleInstanceOutcome::SecondaryForwarded
    ));

    let received = rx
        .recv_timeout(Duration::from_secs(3))
        .expect("primary should receive forwarded args");
    assert_eq!(received, sample_args);

    drop(primary);
}

#[test]
fn os_integration_timer_aumid_cleanup_and_trash() {
    // High-res timer guard + imperative toggle
    {
        let _guard = HighResTimerGuard::acquire();
        set_high_res_timer(true);
        set_high_res_timer(false);
    }

    // Process AppUserModelId
    set_current_process_app_user_model_id(APP_ID).expect("SetCurrentProcessExplicitAppUserModelID");

    // Process running check
    assert!(is_process_running(std::process::id()));
    assert!(!is_process_running(0));

    // PID-scoped temp cleanup
    let base = std::env::temp_dir().join(format!("hybrid_win32_shell_test_{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();

    let dead_thumbs = base.join("hybrid-player-thumbs-99999999");
    let live_thumbs = base.join(format!("hybrid-player-thumbs-{}", std::process::id()));
    let dead_undo = base.join("hybrid_screenshot_undo_99999999");
    let paused_dir = base.join("hybrid-player-paused-frames");
    fs::create_dir_all(&dead_thumbs).unwrap();
    fs::create_dir_all(&live_thumbs).unwrap();
    fs::create_dir_all(&dead_undo).unwrap();
    fs::create_dir_all(&paused_dir).unwrap();
    fs::write(dead_thumbs.join("thumb.jpg"), b"x").unwrap();
    fs::write(live_thumbs.join("thumb.jpg"), b"x").unwrap();
    fs::write(paused_dir.join("pid-99999999-frame.png"), b"x").unwrap();

    // Startup cleanup removes dead PID artifacts, keeps current PID artifacts
    cleanup_temp_files(&base, std::process::id(), false);
    assert!(!dead_thumbs.exists());
    assert!(live_thumbs.exists());
    assert!(!dead_undo.exists());

    // Shutdown cleanup removes current PID artifacts too
    cleanup_temp_files(&base, std::process::id(), true);
    assert!(!live_thumbs.exists());

    // Recycle Bin / Shell trash helper
    let trash_target = base.join("screenshot_undo_test.png");
    fs::write(&trash_target, b"fake png payload").unwrap();
    assert!(trash_target.exists());
    trash_file(&trash_target).expect("SHFileOperationW FO_DELETE FOF_ALLOWUNDO");
    assert!(!trash_target.exists());

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn companion_auth_svgs_render_pulse_and_native_window() {
    let assets = CompanionSvgAssets::parse().expect("parse all 3 companion auth SVGs");
    let mut state = CompanionAuthState::from_options(&CompanionAuthOptions::default());

    state.elapsed_secs = 0.0;
    let frame_a = render_companion_vector_layer(
        &state,
        &assets,
        COMPANION_WIDTH_DIP as u32,
        COMPANION_HEIGHT_DIP as u32,
        1.0,
    )
    .expect("render frame A");

    state.elapsed_secs = 0.8;
    let frame_b = render_companion_vector_layer(
        &state,
        &assets,
        COMPANION_WIDTH_DIP as u32,
        COMPANION_HEIGHT_DIP as u32,
        1.0,
    )
    .expect("render frame B");

    // Verify non-blank & animated pulse ring changes pixels between t=0.0 and t=0.8
    assert_ne!(frame_a.data(), frame_b.data());
    assert!(frame_a.data().iter().any(|&b| b != 0));

    state.finishing = true;
    state.elapsed_secs = 0.25;
    let frame_finishing = render_companion_vector_layer(
        &state,
        &assets,
        COMPANION_WIDTH_DIP as u32,
        COMPANION_HEIGHT_DIP as u32,
        1.0,
    )
    .expect("render finishing spinner frame");
    assert_ne!(frame_b.data(), frame_finishing.data());

    // Real native Win32 Companion Auth window (pure Rust, zero WebView)
    let handle = CompanionAuthHandle::open(CompanionAuthOptions::default(), |_act| {})
        .expect("open native companion auth window");
    let hwnd = handle.hwnd();
    assert!(!hwnd.0.is_null());

    // Verify title "Sign in to YouTube"
    let mut buf = [0u16; 64];
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) } as usize;
    let title = String::from_utf16_lossy(&buf[..len]);
    assert_eq!(title, "Sign in to YouTube");

    // Verify client area matches 500x405 DIPs scaled by window DPI
    let mut client = RECT::default();
    unsafe {
        GetClientRect(hwnd, &mut client).unwrap();
    }
    let dpi = dpi_for_window(hwnd);
    assert_eq!(
        client.right - client.left,
        dip_to_physical(COMPANION_WIDTH_DIP, dpi)
    );
    assert_eq!(
        client.bottom - client.top,
        dip_to_physical(COMPANION_HEIGHT_DIP, dpi)
    );

    handle.set_status("Extracting cookies from browser...");
    handle.set_finishing(true);
    std::thread::sleep(Duration::from_millis(40));
    handle.close();
}

#[test]
fn fullscreen_restore_and_toggle_maximize_exit_cleanly() {
    let hwnd = create_test_hwnd("fs_restore");
    let subclass = WindowSubclass::install(hwnd, ThemeMode::Dark).expect("install subclass");

    // 1. Start maximized -> enter fullscreen -> toggle_maximize must exit to windowed (NOT re-maximize)
    subclass.maximize();
    assert!(subclass.is_maximized());
    subclass.set_fullscreen(true).expect("enter fullscreen");
    assert!(subclass.is_fullscreen());
    assert!(!subclass.is_maximized());

    let now_max = subclass.toggle_maximize();
    assert!(
        !now_max,
        "toggle_maximize from fullscreen must exit to windowed"
    );
    assert!(!subclass.is_fullscreen());
    assert!(!subclass.is_maximized());

    let mut wr = RECT::default();
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut wr).unwrap();
        let mon = windows::Win32::Graphics::Gdi::MonitorFromWindow(
            hwnd,
            windows::Win32::Graphics::Gdi::MONITOR_DEFAULTTONEAREST,
        );
        let mut mi = windows::Win32::Graphics::Gdi::MONITORINFO {
            cbSize: std::mem::size_of::<windows::Win32::Graphics::Gdi::MONITORINFO>() as u32,
            ..Default::default()
        };
        assert!(windows::Win32::Graphics::Gdi::GetMonitorInfoW(mon, &mut mi).as_bool());
        assert!(wr.left >= mi.rcWork.left && wr.top >= mi.rcWork.top);
        assert!(wr.right <= mi.rcWork.right && wr.bottom <= mi.rcWork.bottom);
    }

    // 2. WM_SYSCOMMAND(SC_RESTORE) while fullscreen clears fullscreen state and restores windowed bounds
    subclass.set_fullscreen(true).expect("re-enter fullscreen");
    assert!(subclass.is_fullscreen());
    unsafe {
        let _ = SendMessageW(
            hwnd,
            windows::Win32::UI::WindowsAndMessaging::WM_SYSCOMMAND,
            Some(WPARAM(
                windows::Win32::UI::WindowsAndMessaging::SC_RESTORE as usize,
            )),
            Some(LPARAM(0)),
        );
    }
    assert!(
        !subclass.is_fullscreen(),
        "WM_SYSCOMMAND(SC_RESTORE) while fullscreen must clear is_fullscreen"
    );
    assert!(!subclass.is_maximized());

    // 3. subclass.restore() and external non-monitor-covering resize while fullscreen both exit cleanly
    subclass.set_fullscreen(true).expect("enter fullscreen 3");
    assert!(subclass.is_fullscreen());
    subclass.restore();
    assert!(!subclass.is_fullscreen());

    subclass.set_fullscreen(true).expect("enter fullscreen 4");
    assert!(subclass.is_fullscreen());
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowPos(
            hwnd,
            None,
            40,
            40,
            900,
            600,
            windows::Win32::UI::WindowsAndMessaging::SWP_NOZORDER
                | windows::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE,
        );
    }
    assert!(
        !subclass.is_fullscreen(),
        "external resize smaller than monitor while fullscreen must clear is_fullscreen"
    );
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

fn assert_strictly_windowed_bounds(hwnd: windows::Win32::Foundation::HWND, context: &str) {
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, IsIconic, IsZoomed};

    let mut wr = RECT::default();
    unsafe {
        assert!(!IsIconic(hwnd).as_bool(), "{context}: must not be iconic");
        assert!(
            !IsZoomed(hwnd).as_bool(),
            "{context}: must not remain zoomed"
        );
        GetWindowRect(hwnd, &mut wr).expect("GetWindowRect");
        let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        assert!(GetMonitorInfoW(mon, &mut mi).as_bool());
        let wa = mi.rcWork;
        let w = wr.right - wr.left;
        let h = wr.bottom - wr.top;
        let wa_w = wa.right - wa.left;
        let wa_h = wa.bottom - wa.top;
        assert!(
            wr.left >= wa.left
                && wr.top >= wa.top
                && wr.right <= wa.right
                && wr.bottom <= wa.bottom,
            "{context}: restored bounds {wr:?} must stay inside work area {wa:?}"
        );
        assert!(
            !(w >= wa_w - 4 && h >= wa_h - 4),
            "{context}: restored bounds ({w}x{h}) must be strictly smaller than work area ({wa_w}x{wa_h})"
        );
        assert!(
            w >= 640 && h >= 360,
            "{context}: restored bounds ({w}x{h}) too small"
        );
    }
}

#[test]
fn fullscreen_then_restore_and_subsequent_restores_stay_windowed() {
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowPlacement, SetWindowPlacement, SC_RESTORE, SW_SHOWMAXIMIZED, WINDOWPLACEMENT,
        WM_SYSCOMMAND,
    };

    let hwnd = create_test_hwnd("fs_restore_loop");
    let subclass = WindowSubclass::install(hwnd, ThemeMode::Dark).expect("install subclass");

    // 1. Maximized -> fullscreen (f) -> unfullscreen (f) -> restore() -> subsequent max/restore
    subclass.maximize();
    assert!(subclass.is_maximized());
    subclass.set_fullscreen(true).expect("enter fullscreen 1");
    assert!(subclass.is_fullscreen());
    assert!(!subclass.is_maximized());
    subclass.set_fullscreen(false).expect("exit fullscreen 1");
    assert!(!subclass.is_fullscreen());
    assert!(
        subclass.is_maximized(),
        "exiting fullscreen after starting maximized must return to maximized"
    );

    subclass.restore();
    assert!(!subclass.is_fullscreen());
    assert!(!subclass.is_maximized());
    assert_strictly_windowed_bounds(hwnd, "max -> fs -> unfs -> restore");

    // Subsequent maximize/restore cycle must keep working cleanly
    subclass.maximize();
    assert!(subclass.is_maximized());
    subclass.restore();
    assert!(!subclass.is_maximized());
    assert_strictly_windowed_bounds(hwnd, "subsequent maximize -> restore");

    // 2. Maximized -> fullscreen (f) -> direct restore (titlebar restore) -> subsequent max/restore
    subclass.maximize();
    assert!(subclass.is_maximized());
    subclass.set_fullscreen(true).expect("enter fullscreen 2");
    assert!(subclass.is_fullscreen());
    subclass
        .exit_fullscreen_to_windowed()
        .expect("exit fullscreen to windowed");
    assert!(!subclass.is_fullscreen());
    assert!(!subclass.is_maximized());
    assert_strictly_windowed_bounds(hwnd, "max -> fs -> exit_fullscreen_to_windowed");

    subclass.maximize();
    assert!(subclass.is_maximized());
    subclass.restore();
    assert!(!subclass.is_maximized());
    assert_strictly_windowed_bounds(hwnd, "after exit_fullscreen_to_windowed -> max -> restore");

    // 3. Maximized -> fullscreen -> unfullscreen -> native WM_SYSCOMMAND(SC_RESTORE)
    subclass.maximize();
    subclass.set_fullscreen(true).expect("enter fullscreen 3");
    subclass.set_fullscreen(false).expect("exit fullscreen 3");
    assert!(subclass.is_maximized());
    unsafe {
        let _ = SendMessageW(
            hwnd,
            WM_SYSCOMMAND,
            Some(WPARAM(SC_RESTORE as usize)),
            Some(LPARAM(0)),
        );
    }
    assert!(!subclass.is_fullscreen());
    assert!(!subclass.is_maximized());
    assert_strictly_windowed_bounds(hwnd, "max -> fs -> unfs -> WM_SYSCOMMAND(SC_RESTORE)");

    // 4. Recovery even if WINDOWPLACEMENT.rcNormalPosition was externally corrupted to monitor rect
    subclass.maximize();
    assert!(subclass.is_maximized());
    unsafe {
        let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        assert!(GetMonitorInfoW(mon, &mut mi).as_bool());
        let mut wp = WINDOWPLACEMENT {
            length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
            ..Default::default()
        };
        assert!(GetWindowPlacement(hwnd, &mut wp).is_ok());
        wp.rcNormalPosition = mi.rcMonitor;
        wp.showCmd = SW_SHOWMAXIMIZED.0 as u32;
        assert!(SetWindowPlacement(hwnd, &wp).is_ok());
    }
    subclass.restore();
    assert!(!subclass.is_maximized());
    assert_strictly_windowed_bounds(hwnd, "corrupted rcNormalPosition restore recovery");

    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

#[test]
fn fullscreen_exit_repairs_placement_and_external_restore_recovers() {
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowPlacement, IsZoomed, SetWindowPlacement, SetWindowPos, ShowWindow, SC_MAXIMIZE,
        SC_RESTORE, SWP_NOACTIVATE, SWP_NOZORDER, SW_MAXIMIZE, SW_RESTORE, SW_SHOWMAXIMIZED,
        WINDOWPLACEMENT, WM_SYSCOMMAND,
    };

    let hwnd = create_test_hwnd("fs_repair_wp_external_restore");
    let mon = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    assert!(unsafe { GetMonitorInfoW(mon, &mut mi).as_bool() });
    let wa = mi.rcWork;
    let wa_w = wa.right - wa.left;
    let wa_h = wa.bottom - wa.top;

    // 1. Simulate a window whose rcNormalPosition was monitor-sized before fullscreen
    //    with no saved restore_bounds, then maximize -> fullscreen -> unfullscreen.
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            wa.left,
            wa.top,
            wa_w,
            wa_h,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
    let subclass = WindowSubclass::install(hwnd, ThemeMode::Dark).expect("install subclass");
    unsafe {
        let _ = ShowWindow(hwnd, SW_MAXIMIZE);
        let mut wp = WINDOWPLACEMENT {
            length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
            ..Default::default()
        };
        assert!(GetWindowPlacement(hwnd, &mut wp).is_ok());
        wp.rcNormalPosition = mi.rcMonitor;
        wp.showCmd = SW_SHOWMAXIMIZED.0 as u32;
        assert!(SetWindowPlacement(hwnd, &wp).is_ok());
    }
    assert!(subclass.is_maximized());

    subclass.set_fullscreen(true).expect("enter fullscreen");
    assert!(subclass.is_fullscreen());
    subclass
        .set_fullscreen(false)
        .expect("exit fullscreen to max");
    assert!(subclass.is_maximized());

    // WINDOWPLACEMENT.rcNormalPosition must now be repaired to strictly smaller than work area
    unsafe {
        let mut wp = WINDOWPLACEMENT {
            length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
            ..Default::default()
        };
        assert!(GetWindowPlacement(hwnd, &mut wp).is_ok());
        let np_w = wp.rcNormalPosition.right - wp.rcNormalPosition.left;
        let np_h = wp.rcNormalPosition.bottom - wp.rcNormalPosition.top;
        assert!(
            !(np_w >= wa_w - 4 && np_h >= wa_h - 4),
            "rcNormalPosition ({np_w}x{np_h}) must be repaired below work area ({wa_w}x{wa_h})"
        );
    }

    // External Win32 ShowWindow(SW_RESTORE) must restore cleanly to strictly windowed bounds
    unsafe {
        let _ = ShowWindow(hwnd, SW_RESTORE);
    }
    assert!(!subclass.is_maximized());
    assert_strictly_windowed_bounds(hwnd, "external ShowWindow(SW_RESTORE) after f -> f");

    // 2. Native WM_SYSCOMMAND(SC_MAXIMIZE) -> fullscreen -> unfullscreen -> external SW_RESTORE
    unsafe {
        let _ = SendMessageW(
            hwnd,
            WM_SYSCOMMAND,
            Some(WPARAM(SC_MAXIMIZE as usize)),
            Some(LPARAM(0)),
        );
        assert!(IsZoomed(hwnd).as_bool());
    }
    subclass.set_fullscreen(true).expect("enter fullscreen 2");
    subclass.set_fullscreen(false).expect("exit fullscreen 2");
    assert!(subclass.is_maximized());
    unsafe {
        let _ = ShowWindow(hwnd, SW_RESTORE);
    }
    assert!(!subclass.is_maximized());
    assert_strictly_windowed_bounds(hwnd, "SC_MAXIMIZE -> f -> f -> external SW_RESTORE");

    // 3. Recovery from a stuck non-zoomed work-area state via WM_SYSCOMMAND(SC_RESTORE)
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            wa.left,
            wa.top,
            wa_w,
            wa_h,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        assert!(!IsZoomed(hwnd).as_bool());
        let _ = SendMessageW(
            hwnd,
            WM_SYSCOMMAND,
            Some(WPARAM(SC_RESTORE as usize)),
            Some(LPARAM(0)),
        );
    }
    assert_strictly_windowed_bounds(hwnd, "stuck work-area -> WM_SYSCOMMAND(SC_RESTORE)");

    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

#[test]
fn keep_c_void_import_clean() {
    let _ = std::ptr::null::<c_void>();
}
