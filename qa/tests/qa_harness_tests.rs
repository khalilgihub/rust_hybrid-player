use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hybrid_qa::pixel_diff::{
    GoldenCaptureManifest, IgnoreRect, PixelDiffConfig, RgbaImageBuffer, compare_png_files,
    compare_rgba, solid_rgba,
};
use hybrid_qa::uia_runner::{
    AppLaunchConfig, EXPECTED_FRAMELESS_STYLE_MASK, HeadlessSnapshotSpec, InputInjector,
    LaunchedApp, Modifiers, UiaSession, capture_window_rgba, discover_hybrid_app_binary,
    inspect_window_state, run_headless_snapshot_cli,
};
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, HDC, PAINTSTRUCT, UpdateWindow,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_S};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, PeekMessageW, PM_REMOVE,
    RegisterClassW, SW_SHOWNA, ShowWindow, TranslateMessage, WINDOW_EX_STYLE, WNDCLASSW,
    WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_POPUP, WS_VISIBLE, MSG,
};
use windows::core::PCWSTR;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("qa crate sits directly under workspace root")
        .to_path_buf()
}

#[test]
fn pixel_diff_metrics_ignore_rects_and_diff_png_generation() {
    let a = solid_rgba(32, 32, [40, 80, 120, 255]);
    let b = a.clone();

    let (identical, diff_none) =
        compare_rgba(&a, &b, &PixelDiffConfig::strict(), false).expect("compare identical");
    assert!(identical.passed);
    assert_eq!(identical.mae, 0.0);
    assert_eq!(identical.rmse, 0.0);
    assert!((identical.ssim - 1.0).abs() < 1e-9);
    assert_eq!(identical.outlier_pixels, 0);
    assert!(diff_none.is_none());

    // Introduce a 16x16 outlier patch inside (0..16, 0..16)
    let mut mutated = a.clone();
    for y in 0..16 {
        for x in 0..16 {
            let idx = ((y * 32 + x) * 4) as usize;
            mutated.data[idx] = 255;
            mutated.data[idx + 1] = 255;
            mutated.data[idx + 2] = 255;
        }
    }

    let (failed_report, diff_img) =
        compare_rgba(&a, &mutated, &PixelDiffConfig::strict(), true).expect("compare mutated");
    assert!(!failed_report.passed);
    assert_eq!(failed_report.outlier_pixels, 256);
    assert!(failed_report.mae > 10.0);
    assert!(failed_report.ssim < 0.90);

    let diff_buf = diff_img.expect("expected generated diff buffer");
    // Highlighted pixel at (2, 2) must carry magenta marker (`G == 24`, `B == 160`)
    let p_idx = ((2 * 32 + 2) * 4) as usize;
    assert_eq!(diff_buf.data[p_idx + 1], 24);
    assert_eq!(diff_buf.data[p_idx + 2], 160);

    // Mask out the mutated patch using IgnoreRect -> must pass with zero error
    let mut masked_cfg = PixelDiffConfig::strict();
    masked_cfg.ignore_rects.push(IgnoreRect {
        x: 0,
        y: 0,
        width: 16,
        height: 16,
    });
    let (masked_report, _) =
        compare_rgba(&a, &mutated, &masked_cfg, false).expect("compare masked");
    assert!(masked_report.passed);
    assert_eq!(masked_report.compared_pixels, 32 * 32 - 256);
    assert_eq!(masked_report.outlier_pixels, 0);
    assert_eq!(masked_report.mae, 0.0);

    // Verify PNG round-trip via compare_png_files
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = std::env::temp_dir().join(format!("hybrid_qa_diff_{}_{}", std::process::id(), nonce));
    let p_exp = tmp.join("exp.png");
    let p_act = tmp.join("act.png");
    let p_diff = tmp.join("diff.png");
    a.save_png(&p_exp).expect("save exp");
    mutated.save_png(&p_act).expect("save act");

    let file_report = compare_png_files(&p_exp, &p_act, Some(&p_diff), &PixelDiffConfig::strict())
        .expect("compare_png_files");
    assert!(!file_report.passed);
    assert!(p_diff.is_file());
    let loaded_diff = RgbaImageBuffer::load_png(&p_diff).expect("load diff png");
    assert_eq!(loaded_diff.width, 32);
    assert_eq!(loaded_diff.height, 32);
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn golden_manifest_and_all_27_pngs_validate_cleanly() {
    let golden_dir = workspace_root().join(".rust-migration").join("golden");
    let manifest =
        GoldenCaptureManifest::load_from_golden_dir(&golden_dir).expect("load capture-manifest.json");
    assert_eq!(manifest.captures.len(), 27);
    assert!(manifest.errors.is_empty());

    for entry in &manifest.captures {
        let png_path = GoldenCaptureManifest::resolve_png_path(&golden_dir, entry);
        assert!(png_path.is_file(), "missing golden PNG {}", png_path.display());

        let img = RgbaImageBuffer::load_png(&png_path).expect("decode golden PNG");
        assert!(img.width >= entry.inner_width);
        assert!(img.height >= entry.inner_height);

        if entry.file == "shader-particles-1280x720.png" {
            assert!(
                manifest.is_known_blank_capture(entry),
                "expected particles capture limitation flag"
            );
            // Electron WebGL particles canvas rendered #000000 behind Welcome UI, matching shader-none
            let none_img =
                RgbaImageBuffer::load_png(&golden_dir.join("shader-none-1280x720.png"))
                    .expect("load shader-none");
            let (rep, _) = compare_rgba(&none_img, &img, &PixelDiffConfig::strict(), false).unwrap();
            assert_eq!(rep.mae, 0.0);
        } else {
            assert!(
                !manifest.is_known_blank_capture(entry),
                "{} should not be marked blank",
                entry.file
            );
            assert!(
                img.non_zero_rgb_pixels() > 0,
                "{} unexpectedly had 0 non-zero pixels",
                entry.file
            );
        }

        // Self-comparison sanity check on first & modal captures
        if entry.file == "welcome-1280x720-dark.png" || entry.file == "modal-settingsModal-1280x720.png" {
            let (rep, _) = compare_rgba(&img, &img, &PixelDiffConfig::strict(), false).unwrap();
            assert!(rep.passed);
            assert_eq!(rep.mae, 0.0);
            assert!((rep.ssim - 1.0).abs() < 1e-9);
        }
    }
}

#[test]
fn uia_session_hwnd_inspection_and_screen_capture_self_test() {
    // 1. Verify UIA COM session initializes cleanly
    let uia = UiaSession::new().expect("UiaSession::new failed");

    // 2. Create a small real Win32 test window painted solid blue and verify
    //    `inspect_window_state`, `capture_window_rgba`, and `UiaSession::element_from_hwnd`
    unsafe extern "system" fn test_wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> windows::Win32::Foundation::LRESULT {
        const WM_PAINT: u32 = 0x000F;
        const WM_PRINTCLIENT: u32 = 0x0318;
        let paint_solid = |hdc: HDC| {
            let rc = RECT {
                left: 0,
                top: 0,
                right: 160,
                bottom: 90,
            };
            let brush = CreateSolidBrush(windows::Win32::Foundation::COLORREF(0x00C86420)); // BGR: R=0x20, G=0x64, B=0xC8
            let _ = FillRect(hdc, &rc, brush);
            let _ = DeleteObject(brush.into());
        };
        if msg == WM_PAINT {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            paint_solid(hdc);
            let _ = EndPaint(hwnd, &ps);
            return windows::Win32::Foundation::LRESULT(0);
        }
        if msg == WM_PRINTCLIENT {
            paint_solid(HDC(wparam.0 as *mut c_void));
            return windows::Win32::Foundation::LRESULT(0);
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }

    let hwnd = unsafe {
        let h_inst = GetModuleHandleW(PCWSTR::null()).expect("GetModuleHandleW");
        let cls: Vec<u16> = "HybridQaHarnessTestWin\0".encode_utf16().collect();
        let title: Vec<u16> = "QA Harness Test Window\0".encode_utf16().collect();
        let wc = WNDCLASSW {
            lpfnWndProc: Some(test_wnd_proc),
            hInstance: h_inst.into(),
            lpszClassName: PCWSTR(cls.as_ptr()),
            ..Default::default()
        };
        let _ = RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(cls.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_POPUP | WS_VISIBLE | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
            -2000,
            0,
            160,
            90,
            None,
            None,
            Some(h_inst.into()),
            None,
        )
        .expect("CreateWindowExW failed");
        let _ = ShowWindow(hwnd, SW_SHOWNA);
        let _ = UpdateWindow(hwnd);
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, Some(hwnd), 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        hwnd
    };

    let inspection = inspect_window_state(hwnd).expect("inspect_window_state failed");
    assert_eq!(inspection.client_width, 160);
    assert_eq!(inspection.client_height, 90);
    assert!(!inspection.has_frameless_aero_style_mask()); // WS_POPUP lacks WS_THICKFRAME/WS_CAPTION

    let captured = capture_window_rgba(hwnd).expect("capture_window_rgba failed");
    assert_eq!(captured.width, 160);
    assert_eq!(captured.height, 90);
    assert!(captured.non_zero_rgb_pixels() > 0);
    let expected = solid_rgba(160, 90, [0x20, 0x64, 0xC8, 255]);
    let (diff, _) =
        compare_rgba(&expected, &captured, &PixelDiffConfig::strict(), false).expect("diff");
    assert!(
        diff.passed,
        "captured HWND pixels did not match expected FillRect brush: mae={}, ssim={}",
        diff.mae, diff.ssim
    );

    let root_el = uia.element_from_hwnd(hwnd).expect("ElementFromHandle failed");
    let tree = uia.dump_tree(&root_el, 1).expect("dump_tree failed");
    assert!(!tree.is_empty());
    assert_eq!(tree[0].name, "QA Harness Test Window");

    unsafe {
        let _ = DestroyWindow(hwnd);
    }

    // Verify binary discovery API handles workspace root cleanly (Ok(None) or Ok(Some))
    let _maybe_bin = discover_hybrid_app_binary(&workspace_root());
    let _mask_check = EXPECTED_FRAMELESS_STYLE_MASK;
    let _no_unused: *mut c_void = std::ptr::null_mut();
}

#[test]
#[ignore = "Requires built hybrid-app.exe (`cargo build -p hybrid-app`)"]
fn live_hybrid_app_uia_sendinput_and_golden_parity() {
    let root = workspace_root();
    let bin = discover_hybrid_app_binary(&root)
        .expect("hybrid-app.exe not found; set HYBRID_APP_BIN or build hybrid-app first");

    let cfg = AppLaunchConfig::new(bin);
    let app = LaunchedApp::launch_and_wait_for_window(&cfg).expect("launch hybrid-app");
    let hwnd = app.hwnd();

    let win = inspect_window_state(hwnd).expect("inspect live hybrid-app HWND");
    assert!(
        win.has_frameless_aero_style_mask(),
        "expected frameless Aero style mask on hybrid-app HWND, got 0x{:x}",
        win.style
    );

    let uia = UiaSession::new().expect("UiaSession::new");
    let root_el = uia.element_from_hwnd(hwnd).expect("ElementFromHandle");
    let tree = uia.dump_tree(&root_el, 6).expect("dump UIA tree");
    assert!(
        !tree.is_empty(),
        "expected accesskit UIA tree entries on live hybrid-app"
    );

    InputInjector::focus_window(hwnd);
    let _ = InputInjector::send_key_tap(VK_S, Modifiers::default());
    std::thread::sleep(Duration::from_millis(150));
    let _ = InputInjector::send_key_tap(VK_ESCAPE, Modifiers::default());

    let shot = capture_window_rgba(hwnd).expect("capture live hybrid-app window");
    assert!(shot.non_zero_rgb_pixels() > 0);

    app.terminate().expect("terminate hybrid-app");
}

fn snapshot_spec_for_golden_entry(
    root: &Path,
    file: &str,
    logical_size: (u32, u32),
    theme: &str,
    bg: &str,
    out_png: PathBuf,
    app_data_dir: PathBuf,
) -> HeadlessSnapshotSpec {
    let (width, height) = logical_size;
    let mut spec = HeadlessSnapshotSpec {
        out_png,
        width: Some(width),
        height: Some(height),
        theme: Some(theme.to_string()),
        welcome_bg: Some(bg.to_string()),
        time_secs: Some(0.0),
        app_data_dir: Some(app_data_dir),
        ..Default::default()
    };

    spec.time_secs = Some(match file {
        "welcome-1280x720-dark.png" => 3.5,
        "welcome-1280x720-oled.png" => 5.25,
        "welcome-1280x720-light.png" => 6.0,
        "welcome-900x600-dark.png" => 7.0,
        "welcome-900x600-oled.png" => 7.5,
        "welcome-900x600-light.png" => 8.0,
        "welcome-640x360-dark.png" => 9.0,
        "welcome-640x360-oled.png" => 9.25,
        "welcome-640x360-light.png" => 9.75,
        "shader-dither-1280x720.png" => 11.3,
        "shader-faulty-1280x720.png" => 34.915,
        "modal-equalizerModal-1280x720.png" => 1.0,
        "modal-speedModal-1280x720.png" => 2.0,
        "modal-subtitleModal-1280x720.png" => 2.5,
        "modal-audioModal-1280x720.png" => 3.5,
        "modal-settingsModal-1280x720.png" => 4.5,
        "modal-networkStreamModal-1280x720.png" => 5.5,
        "modal-bgSettingsModal-1280x720.png" => 3.75,
        "modal-bgAccordionPopover-1280x720.png" => 4.75,
        "playlist-open-1280x720.png" => 8.5,
        "overlay-clip-bezel-tab-1280x720.png" => 9.5,
        "overlay-burst-pill-1280x720.png" => 10.3,
        "overlay-screenshot-carousel-1280x720.png" => 1.0,
        "playback-sample-1280x720.png" => 0.82,
        _ => 0.0,
    });
    if file == "modal-bgAccordionPopover-1280x720.png" {
        spec.preview_time_secs = Some(0.9);
    }

    match file {
        "modal-equalizerModal-1280x720.png" => spec.modal = Some("eq".to_string()),
        "modal-speedModal-1280x720.png" => spec.modal = Some("speed".to_string()),
        "modal-subtitleModal-1280x720.png" => spec.modal = Some("subtitles".to_string()),
        "modal-audioModal-1280x720.png" => spec.modal = Some("audio".to_string()),
        "modal-settingsModal-1280x720.png" => spec.modal = Some("settings".to_string()),
        "modal-networkStreamModal-1280x720.png" => spec.modal = Some("network".to_string()),
        "modal-bgSettingsModal-1280x720.png" => spec.modal = Some("bg".to_string()),
        "modal-bgAccordionPopover-1280x720.png" => {
            spec.modal = Some("bg".to_string());
            spec.bg_accordion = true;
        }
        "playlist-open-1280x720.png" => spec.playlist_open = true,
        "overlay-clip-bezel-tab-1280x720.png" => {
            spec.overlay = Some("clip-bezel".to_string());
            spec.clip_time = Some(0.0);
        }
        "overlay-burst-pill-1280x720.png" => {
            spec.overlay = Some("burst-pill".to_string());
            spec.burst_count = Some(1);
        }
        "overlay-screenshot-carousel-1280x720.png" => {
            spec.overlay = Some("carousel".to_string());
            spec.carousel_index = Some(1);
        }
        "playback-sample-1280x720.png" => {
            spec.playback_sample = true;
            let sample_media = root.join("hybrid-player").join("test-sample.mp4");
            if sample_media.is_file() {
                spec.media = Some(sample_media);
            }
        }
        _ => {}
    }

    spec
}

/// Computes the exact logical `(width, height)` for comparing a DPI-scaled Electron golden PNG
/// against a `dpr = 1.0` Rust headless snapshot without aspect-ratio distortion.
fn resolve_golden_logical_dimensions(
    golden_w: u32,
    golden_h: u32,
    manifest_w: u32,
    manifest_h: u32,
    session_dpr: f64,
) -> (u32, u32) {
    if (golden_w as u64) * (manifest_h as u64) == (golden_h as u64) * (manifest_w as u64) {
        (manifest_w, manifest_h)
    } else {
        let dpr = if session_dpr > 0.0 && session_dpr.is_finite() {
            session_dpr
        } else {
            1.0
        };
        let logical_w = ((golden_w as f64) / dpr).round().max(1.0) as u32;
        let logical_h = ((golden_h as f64) / dpr).round().max(1.0) as u32;
        (logical_w, logical_h)
    }
}

fn detect_session_dpr(golden_dir: &Path, manifest: &GoldenCaptureManifest) -> f64 {
    for entry in &manifest.captures {
        let golden_path = GoldenCaptureManifest::resolve_png_path(golden_dir, entry);
        if let Ok(img) = RgbaImageBuffer::load_png(&golden_path) {
            if (img.width as u64) * (entry.inner_height as u64)
                == (img.height as u64) * (entry.inner_width as u64)
                && entry.inner_width > 0
            {
                return (img.width as f64) / (entry.inner_width as f64);
            }
        }
    }
    1.0
}

#[test]
fn snapshot_spec_and_dpi_normalization_invariants() {
    let root = workspace_root();
    let bg_acc = snapshot_spec_for_golden_entry(
        &root,
        "modal-bgAccordionPopover-1280x720.png",
        (1280, 720),
        "dark",
        "dither",
        PathBuf::from("out.png"),
        PathBuf::from("app"),
    );
    assert_eq!(bg_acc.modal.as_deref(), Some("bg"));
    assert!(bg_acc.bg_accordion);

    // Matching aspect ratio at 1.25x DPI (1600x900 -> 1280x720)
    assert_eq!(
        resolve_golden_logical_dimensions(1600, 900, 1280, 720, 1.25),
        (1280, 720)
    );
    // Mismatched manifest vs actual Electron capture (1920x1020 at 1.25x DPI -> 1536x816, preserving 32:17 aspect)
    assert_eq!(
        resolve_golden_logical_dimensions(1920, 1020, 1280, 720, 1.25),
        (1536, 816)
    );
}

#[test]
#[ignore = "Requires built hybrid-app.exe (`cargo build -p hybrid-app`)"]
fn live_headless_snapshot_golden_suite() {
    let root = workspace_root();
    let bin = discover_hybrid_app_binary(&root)
        .expect("hybrid-app.exe not found; set HYBRID_APP_BIN or build hybrid-app first");

    let golden_dir = root.join(".rust-migration").join("golden");
    let manifest_raw =
        std::fs::read_to_string(golden_dir.join("capture-manifest.json")).expect("read manifest");
    let manifest: GoldenCaptureManifest =
        serde_json::from_str(&manifest_raw).expect("parse capture-manifest.json");
    assert_eq!(manifest.captures.len(), 27);

    let session_dpr = detect_session_dpr(&golden_dir, &manifest);

    let out_dir = std::env::var_os("HYBRID_QA_DIFF_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("golden-diffs"));
    let actual_dir = out_dir.join("actual");
    let diff_dir = out_dir.join("diffs");
    let app_data_dir = out_dir.join("app-data");
    std::fs::create_dir_all(&actual_dir).expect("create actual dir");
    std::fs::create_dir_all(&diff_dir).expect("create diff dir");
    std::fs::create_dir_all(&app_data_dir).expect("create isolated app_data dir");

    let cfg = PixelDiffConfig::ui_parity();
    let mut render_failures = Vec::new();
    let mut parity_passes = 0usize;
    let mut parity_failures = Vec::new();
    let mut json_rows = Vec::new();
    let mut shader_none_non_zero: Option<usize> = None;

    for entry in &manifest.captures {
        let actual_path = actual_dir.join(&entry.file);
        let golden_path = golden_dir.join(&entry.file);
        let golden_raw = RgbaImageBuffer::load_png(&golden_path).expect("load golden png");
        let (render_w, render_h) = resolve_golden_logical_dimensions(
            golden_raw.width,
            golden_raw.height,
            entry.inner_width,
            entry.inner_height,
            session_dpr,
        );

        let mut spec = snapshot_spec_for_golden_entry(
            &root,
            &entry.file,
            (render_w, render_h),
            &entry.theme,
            &entry.bg,
            actual_path.clone(),
            app_data_dir.clone(),
        );
        spec.dpr = Some(session_dpr as f32);

        if let Err(err) = run_headless_snapshot_cli(&bin, &spec, Duration::from_secs(20)) {
            eprintln!("[FAIL-CLI] {}: {err}", entry.file);
            render_failures.push(format!("{}: {err}", entry.file));
            continue;
        }

        let diff_out = diff_dir.join(&entry.file);
        let actual_img = RgbaImageBuffer::load_png(&actual_path).expect("load actual png");
        let expected_w = ((render_w as f64) * session_dpr).round().max(1.0) as u32;
        let expected_h = ((render_h as f64) * session_dpr).round().max(1.0) as u32;
        assert_eq!(actual_img.width, expected_w);
        assert_eq!(actual_img.height, expected_h);
        if entry.file == "shader-none-1280x720.png" {
            shader_none_non_zero = Some(actual_img.non_zero_rgb_pixels());
        }

        let golden_norm = if golden_raw.width != actual_img.width
            || golden_raw.height != actual_img.height
        {
            if entry.bg == "dither" {
                golden_raw
                    .resize_nearest(actual_img.width, actual_img.height)
                    .expect("normalize pixelated DPI-scaled golden png")
            } else {
                golden_raw
                    .resize_lanczos3(actual_img.width, actual_img.height)
                    .expect("normalize DPI-scaled golden png")
            }
        } else {
            golden_raw.clone()
        };

        let (mut report, diff_img) =
            compare_rgba(&golden_norm, &actual_img, &cfg, true).expect("compare golden vs actual");
        if manifest.is_known_blank_capture(entry) {
            let baseline_nz = shader_none_non_zero.unwrap_or(0);
            let actual_nz = actual_img.non_zero_rgb_pixels();
            if actual_nz <= baseline_nz {
                report.passed = false;
            }
        }
        if let Some(diff) = diff_img {
            diff.save_png(&diff_out).expect("save diff png");
        }
        eprintln!(
            "[GOLDEN] {:38} golden={}x{} logical={}x{} actual={}x{} | MAE={:6.2} RMSE={:6.2} SSIM={:.4} outliers={:>6} ({:.2}%) pass={}",
            entry.file,
            golden_raw.width,
            golden_raw.height,
            render_w,
            render_h,
            actual_img.width,
            actual_img.height,
            report.mae,
            report.rmse,
            report.ssim,
            report.outlier_pixels,
            report.outlier_ratio * 100.0,
            report.passed
        );
        json_rows.push(serde_json::json!({
            "file": entry.file,
            "golden_size": [golden_raw.width, golden_raw.height],
            "logical_size": [render_w, render_h],
            "actual_size": [actual_img.width, actual_img.height],
            "known_blank_golden": manifest.is_known_blank_capture(entry),
            "mae": report.mae,
            "rmse": report.rmse,
            "ssim": report.ssim,
            "outlier_pixels": report.outlier_pixels,
            "outlier_ratio": report.outlier_ratio,
            "passed": report.passed,
        }));
        if report.passed {
            parity_passes += 1;
        } else {
            parity_failures.push(format!(
                "{} (MAE={:.2}, RMSE={:.2}, SSIM={:.4}, diff={})",
                entry.file,
                report.mae,
                report.rmse,
                report.ssim,
                diff_out.display()
            ));
        }
    }

    let report_json = serde_json::json!({
        "total": manifest.captures.len(),
        "rendered": manifest.captures.len() - render_failures.len(),
        "parity_passes": parity_passes,
        "session_dpr": session_dpr,
        "entries": json_rows,
    });
    std::fs::write(
        out_dir.join("report.json"),
        serde_json::to_string_pretty(&report_json).expect("serialize report.json"),
    )
    .expect("write report.json");

    eprintln!(
        "[SUMMARY] rendered={}/{} parity_pass={}/{} out_dir={}",
        manifest.captures.len() - render_failures.len(),
        manifest.captures.len(),
        parity_passes,
        manifest.captures.len(),
        out_dir.display()
    );

    assert!(
        render_failures.is_empty(),
        "headless snapshot CLI failed on {} entries: {:?}",
        render_failures.len(),
        render_failures
    );
    assert!(
        parity_failures.is_empty(),
        "pixel parity threshold failed on {}/{} captures:\n{}",
        parity_failures.len(),
        manifest.captures.len(),
        parity_failures.join("\n")
    );
}
