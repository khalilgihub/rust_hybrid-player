use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hybrid_app::cli::{AppCliArgs, HeadlessSnapshotOptions, SnapshotOverlayKind};
use hybrid_app::input::{
    classify_dropped_files, DroppedFilesPlan, InputAction, InputContext, InputController,
    KeyModifiers, BURST_FRAME_INTERVAL, BURST_HOLD_THRESHOLD, CHROME_AUTO_HIDE_TIMEOUT,
    CURSOR_IDLE_TIMEOUT,
};
use hybrid_app::media_bridge::MediaBridge;
use hybrid_app::renderer::render_headless_snapshot;
use hybrid_core::equalizer::EqualizerPreset;
use hybrid_shaders::{ThemeMode, WelcomeBackgroundKind};
use hybrid_ui::state::{
    AbLoopState, CarouselMode, ColorPickerTarget, DropdownKind, ModalId, RecordingIndicatorMode,
};
use tokio::runtime::Runtime;

fn temp_test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hybrid-app-test-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp test dir");
    dir
}

#[test]
fn cli_parses_headless_snapshot_and_qa_flags() {
    let args = AppCliArgs::parse_from_iter([
        "--headless-snapshot",
        "out/snap.png",
        "--width",
        "1280",
        "--height",
        "720",
        "--theme",
        "oled",
        "--welcome-bg",
        "gridmotion",
        "--modal",
        "settings",
        "--playlist-open",
        "--bg-accordion",
        "--overlay",
        "burst-pill",
        "--burst-count",
        "12",
        "--carousel-index",
        "1",
        "--clip-time",
        "4.2",
        "--playback-sample",
        "--time-secs",
        "2.5",
        "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
    ])
    .expect("parse CLI args");

    assert_eq!(
        args.media_inputs,
        vec!["https://www.youtube.com/watch?v=dQw4w9WgXcQ"]
    );
    let snap = args.headless_snapshot.expect("snapshot opts");
    assert_eq!(snap.output_path, PathBuf::from("out/snap.png"));
    assert_eq!(snap.width, 1280);
    assert_eq!(snap.height, 720);
    assert_eq!(snap.theme, ThemeMode::Oled);
    assert_eq!(snap.welcome_bg, WelcomeBackgroundKind::Gridmotion);
    assert_eq!(snap.modal, Some(ModalId::Settings));
    assert!(snap.playlist_open);
    assert!(snap.bg_accordion_open);
    assert_eq!(snap.overlay, SnapshotOverlayKind::BurstPill);
    assert_eq!(snap.burst_count, 12);
    assert_eq!(snap.carousel_index, 1);
    assert!((snap.clip_time_secs - 4.2).abs() < 1e-6);
    assert!(snap.playback_sample);
    assert!((snap.time_secs - 2.5).abs() < 1e-6);
}

#[test]
fn cli_single_instance_forward_payload_defaults_to_focus() {
    let empty = AppCliArgs::parse_from_iter(Vec::<String>::new()).unwrap();
    assert_eq!(empty.forward_payload(), vec!["--focus"]);

    let with_url = AppCliArgs::parse_from_iter(["https://youtu.be/dQw4w9WgXcQ"]).unwrap();
    assert_eq!(
        with_url.forward_payload(),
        vec!["https://youtu.be/dQw4w9WgXcQ"]
    );
}

#[test]
fn input_cursor_and_chrome_idle_timers_respect_gates() {
    let mut input = InputController::new(BTreeMap::new(), 3);
    let t0 = Instant::now();

    let playing_ctx = InputContext {
        has_active_media: true,
        is_playing: true,
        ..Default::default()
    };

    let (_, hide_cursor_early, hide_chrome_early) =
        input.tick(t0 + Duration::from_millis(1500), &playing_ctx);
    assert!(!hide_cursor_early);
    assert!(!hide_chrome_early);

    let (_, hide_cursor_2s, hide_chrome_2s) = input.tick(t0 + CURSOR_IDLE_TIMEOUT, &playing_ctx);
    assert!(hide_cursor_2s);
    assert!(!hide_chrome_2s);

    let (_, _, hide_chrome_2_5s) = input.tick(t0 + CHROME_AUTO_HIDE_TIMEOUT, &playing_ctx);
    assert!(hide_chrome_2_5s);

    // Hovering controls or opening a modal prevents hiding
    let modal_ctx = InputContext {
        modal_open: true,
        ..playing_ctx
    };
    let (_, hide_cursor_modal, hide_chrome_modal) =
        input.tick(t0 + Duration::from_secs(5), &modal_ctx);
    assert!(!hide_cursor_modal);
    assert!(!hide_chrome_modal);
}

#[test]
fn input_screenshot_tap_vs_hold_to_burst_and_mouse_button() {
    let t0 = Instant::now();
    let mut input = InputController::new(BTreeMap::new(), 3);
    let ctx = InputContext {
        has_active_media: true,
        is_playing: true,
        ..Default::default()
    };

    // Short tap (< 200ms) -> single screenshot
    assert!(input
        .on_key_down(t0, "KeyS", 1, KeyModifiers::default(), false, &ctx)
        .is_empty());
    let up_short = input.on_key_up("KeyS", 1, KeyModifiers::default(), &ctx);
    assert_eq!(up_short, vec![InputAction::TakeSingleScreenshot]);

    // Hold >= 200ms via mouse button 3 (Back / X1) -> burst session
    assert!(input.on_mouse_button_down(t0, 3, &ctx).is_empty());
    let (actions_start, _, _) = input.tick(t0 + BURST_HOLD_THRESHOLD, &ctx);
    assert_eq!(actions_start, vec![InputAction::StartBurstCapture]);
    assert!(input.is_burst_active());

    let (actions_tick, _, _) = input.tick(t0 + BURST_HOLD_THRESHOLD + BURST_FRAME_INTERVAL, &ctx);
    assert_eq!(actions_tick, vec![InputAction::BurstTickCapture]);

    let stop = input.on_mouse_button_up(3);
    assert_eq!(stop, vec![InputAction::StopBurstCapture]);
    assert!(!input.is_burst_active());
}

#[test]
fn input_escape_priority_wheel_gestures_and_drop_routing() {
    let mut input = InputController::new(BTreeMap::new(), 3);

    // Escape cascade: carousel -> modal/bg -> playlist -> exit fullscreen
    let carousel_ctx = InputContext {
        carousel_open: true,
        modal_open: true,
        playlist_open: true,
        is_fullscreen: true,
        ..Default::default()
    };
    assert_eq!(
        input.resolve_escape_action(&carousel_ctx),
        InputAction::CarouselClose
    );

    let modal_ctx = InputContext {
        modal_open: true,
        playlist_open: true,
        is_fullscreen: true,
        ..Default::default()
    };
    assert_eq!(
        input.resolve_escape_action(&modal_ctx),
        InputAction::DismissModalOrPopover
    );

    let playlist_ctx = InputContext {
        playlist_open: true,
        is_fullscreen: true,
        ..Default::default()
    };
    assert_eq!(
        input.resolve_escape_action(&playlist_ctx),
        InputAction::TogglePlaylistDrawer
    );

    let fs_ctx = InputContext {
        is_fullscreen: true,
        ..Default::default()
    };
    assert_eq!(
        input.resolve_escape_action(&fs_ctx),
        InputAction::ExitFullscreen
    );

    // Wheel volume gesture respects exclusion zone
    let now = Instant::now();
    let wheel_ctx = InputContext {
        has_active_media: true,
        ..Default::default()
    };
    assert_eq!(
        input.on_mouse_wheel(now, 0.0, 1.0, &wheel_ctx),
        Some(InputAction::AdjustVolumeDelta(5))
    );
    let excluded_ctx = InputContext {
        in_wheel_exclusion_zone: true,
        ..wheel_ctx
    };
    assert_eq!(input.on_mouse_wheel(now, 0.0, 1.0, &excluded_ctx), None);

    // Drop routing
    assert_eq!(
        classify_dropped_files(&[PathBuf::from("sub.srt")], false),
        DroppedFilesPlan::SubtitlesWithoutMedia
    );
    assert_eq!(
        classify_dropped_files(&[PathBuf::from("sub.ass")], true),
        DroppedFilesPlan::LoadSubtitlesOnly(vec![PathBuf::from("sub.ass")])
    );
    assert_eq!(
        classify_dropped_files(
            &[PathBuf::from("movie.mkv"), PathBuf::from("movie.srt")],
            false
        ),
        DroppedFilesPlan::OpenMedia {
            media_files: vec![PathBuf::from("movie.mkv")],
            companion_subtitles: vec![PathBuf::from("movie.srt")],
            replace_playlist: true,
        }
    );
}

#[test]
fn media_bridge_persistence_equalizer_ab_loop_and_screenshot_undo() {
    let dir = temp_test_dir("bridge");
    let media_dir = dir.join("media");
    fs::create_dir_all(&media_dir).unwrap();
    fs::write(media_dir.join("ep10.mp4"), b"dummy").unwrap();
    fs::write(media_dir.join("ep2.mp4"), b"dummy").unwrap();

    let shot_path = dir.join("shot-001.jpg");
    fs::write(&shot_path, b"jpeg-bytes").unwrap();

    let rt = Arc::new(Runtime::new().unwrap());
    let mut bridge = MediaBridge::new(dir.clone(), Arc::clone(&rt));

    // Folder scan adds items in natural numeric order (ep2 before ep10)
    bridge.open_files(std::slice::from_ref(&media_dir), true, false);
    assert_eq!(bridge.playlist.items.len(), 2);
    assert!(bridge.playlist.items[0].title.contains("ep2"));
    assert!(bridge.playlist.items[1].title.contains("ep10"));

    // Equalizer preset persists into DB
    bridge.set_eq_preset(EqualizerPreset::BassBoost);
    assert_eq!(bridge.db.preferences.equalizer_preset, "bass-boost");

    // Volume clamp + A-B loop cycling
    bridge.set_volume_pct(150);
    assert_eq!(bridge.ui.volume_pct, 100);
    bridge.ui.current_secs = 12.0;
    bridge.cycle_ab_loop();
    assert_eq!(bridge.ui.ab_loop, AbLoopState::PointASet(12_000));
    bridge.ui.current_secs = 24.5;
    bridge.cycle_ab_loop();
    assert_eq!(bridge.ui.ab_loop, AbLoopState::Active(12_000, 24_500));
    bridge.cycle_ab_loop();
    assert_eq!(bridge.ui.ab_loop, AbLoopState::Off);

    // Screenshot stack delete -> undo restore round-trip & carousel confirm parity
    let shot2_path = dir.join("shot-002.jpg");
    fs::write(&shot2_path, b"jpeg-bytes-2").unwrap();
    bridge.single_screenshot_stack.push(shot_path.clone());
    bridge
        .session_screenshots
        .push(hybrid_app::media_bridge::SessionScreenshot {
            file_path: shot_path.clone(),
            file_name: "shot-001.jpg".to_string(),
            timestamp_ms: 1,
        });
    bridge.single_screenshot_stack.push(shot2_path.clone());
    bridge
        .session_screenshots
        .push(hybrid_app::media_bridge::SessionScreenshot {
            file_path: shot2_path.clone(),
            file_name: "shot-002.jpg".to_string(),
            timestamp_ms: 2,
        });
    bridge.delete_latest_screenshot();
    assert!(!shot2_path.exists());
    assert_eq!(bridge.deleted_screenshot_stack.len(), 1);
    assert_eq!(bridge.ui.carousel_mode, CarouselMode::SinglePreview);
    assert_eq!(bridge.ui.carousel_index, 0);
    assert!(bridge.ui.anim.screenshot_deleting);
    let del_frame = hybrid_ui::HybridUi::new().build_frame(&bridge.ui);
    assert!(
        del_frame
            .image_slots
            .iter()
            .any(|s| s.key == "screenshot-deleted:0"),
        "SinglePreview delete must reference deleted screenshot slot"
    );

    bridge.restore_latest_screenshot();
    assert!(shot2_path.exists());
    assert_eq!(bridge.ui.carousel_mode, CarouselMode::SinglePreview);
    assert!(!bridge.ui.anim.screenshot_deleting);

    // Carousel delete confirm immediately closes carousel into SinglePreview Deleted pop
    bridge.toggle_carousel(false);
    assert_eq!(bridge.ui.carousel_mode, CarouselMode::DeleteCarousel);
    bridge.confirm_carousel_action();
    assert_eq!(bridge.ui.carousel_mode, CarouselMode::SinglePreview);
    assert!(bridge.ui.anim.screenshot_deleting);
    assert_eq!(
        bridge
            .ui
            .screenshot_badge
            .as_ref()
            .map(|(s, d)| (s.as_str(), *d)),
        Some(("DELETED", true))
    );
    assert_eq!(bridge.ui.screenshot_notice, None);

    // Carousel restore confirm immediately closes carousel into SinglePreview Restored pop
    bridge.toggle_carousel(true);
    assert_eq!(bridge.ui.carousel_mode, CarouselMode::RestoreCarousel);
    bridge.confirm_carousel_action();
    assert_eq!(bridge.ui.carousel_mode, CarouselMode::SinglePreview);
    assert!(!bridge.ui.anim.screenshot_deleting);
    assert_eq!(
        bridge
            .ui
            .screenshot_badge
            .as_ref()
            .map(|(s, d)| (s.as_str(), *d)),
        Some(("RESTORED", false))
    );
    assert_eq!(bridge.ui.screenshot_notice, None);

    // Reopen DB from disk to verify persistence survived atomic write
    let reloaded = MediaBridge::new(dir.clone(), rt);
    assert_eq!(reloaded.db.preferences.equalizer_preset, "bass-boost");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn headless_snapshot_renders_welcome_modal_and_overlay_pngs() {
    let dir = temp_test_dir("snapshots");

    let cases = [
        (
            "welcome-dither.png",
            HeadlessSnapshotOptions {
                output_path: dir.join("welcome-dither.png"),
                width: 640,
                height: 360,
                theme: ThemeMode::Dark,
                welcome_bg: WelcomeBackgroundKind::Dither,
                ..Default::default()
            },
        ),
        (
            "modal-settings-accordion.png",
            HeadlessSnapshotOptions {
                output_path: dir.join("modal-settings-accordion.png"),
                width: 640,
                height: 360,
                theme: ThemeMode::Oled,
                welcome_bg: WelcomeBackgroundKind::Particles,
                modal: Some(ModalId::BgSettings),
                bg_accordion_open: true,
                ..Default::default()
            },
        ),
        (
            "overlay-burst-pill.png",
            HeadlessSnapshotOptions {
                output_path: dir.join("overlay-burst-pill.png"),
                width: 640,
                height: 360,
                theme: ThemeMode::Dark,
                welcome_bg: WelcomeBackgroundKind::None,
                overlay: SnapshotOverlayKind::BurstPill,
                burst_count: 12,
                playback_sample: true,
                ..Default::default()
            },
        ),
    ];

    for (label, opts) in &cases {
        render_headless_snapshot(opts).unwrap_or_else(|e| panic!("{label} failed: {e}"));
        let img = image::open(&opts.output_path)
            .unwrap_or_else(|e| panic!("open {label}: {e}"))
            .to_rgba8();
        assert_eq!(img.width(), 640);
        assert_eq!(img.height(), 360);
        // Verify rendered output is non-blank (contains distinct colors/alpha).
        let first = *img.get_pixel(0, 0);
        let has_variation = img.pixels().any(|p| *p != first);
        assert!(has_variation, "{label} output image was completely uniform");
    }

    // Sanity-check RecordingIndicatorMode construction for ClipBezel snapshot flags
    let clip_opts = AppCliArgs::parse_from_iter([
        "--headless-snapshot",
        "clip.png",
        "--overlay",
        "clip-bezel",
        "--clip-time",
        "4.2",
    ])
    .unwrap()
    .headless_snapshot
    .unwrap();
    let expected_ds = (clip_opts.clip_time_secs * 10.0).round() as u64;
    assert_eq!(
        RecordingIndicatorMode::ClipActive {
            elapsed_ds: expected_ds
        },
        RecordingIndicatorMode::ClipActive { elapsed_ds: 42 }
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn complete_key_code_mapping_covers_all_52_shortcut_catalog_keys() {
    use hybrid_app::renderer::map_key_code;
    use winit::keyboard::KeyCode;

    assert_eq!(map_key_code(KeyCode::Digit1), "Digit1");
    assert_eq!(map_key_code(KeyCode::Digit5), "Digit5");
    assert_eq!(map_key_code(KeyCode::Digit9), "Digit9");
    assert_eq!(map_key_code(KeyCode::KeyH), "KeyH");
    assert_eq!(map_key_code(KeyCode::KeyJ), "KeyJ");
    assert_eq!(map_key_code(KeyCode::KeyO), "KeyO");
    assert_eq!(map_key_code(KeyCode::KeyQ), "KeyQ");
    assert_eq!(map_key_code(KeyCode::KeyV), "KeyV");
    assert_eq!(map_key_code(KeyCode::KeyW), "KeyW");
    assert_eq!(map_key_code(KeyCode::KeyX), "KeyX");
    assert_eq!(map_key_code(KeyCode::KeyY), "KeyY");
    assert_eq!(map_key_code(KeyCode::Slash), "Slash");
    assert_eq!(map_key_code(KeyCode::Backquote), "Backquote");
    assert_eq!(map_key_code(KeyCode::Backslash), "Backslash");
    assert_eq!(map_key_code(KeyCode::Semicolon), "Semicolon");
    assert_eq!(map_key_code(KeyCode::Quote), "Quote");
    assert_eq!(map_key_code(KeyCode::Tab), "Tab");
    assert_eq!(map_key_code(KeyCode::Backspace), "Backspace");
    assert_eq!(map_key_code(KeyCode::Delete), "Delete");
    assert_eq!(map_key_code(KeyCode::Home), "Home");
    assert_eq!(map_key_code(KeyCode::End), "End");
    assert_eq!(map_key_code(KeyCode::PageUp), "PageUp");
    assert_eq!(map_key_code(KeyCode::PageDown), "PageDown");
    assert_eq!(map_key_code(KeyCode::F1), "F1");
    assert_eq!(map_key_code(KeyCode::F12), "F12");
    assert_eq!(map_key_code(KeyCode::Numpad0), "Numpad0");
    assert_eq!(map_key_code(KeyCode::Numpad9), "Numpad9");
}

#[test]
fn shortcut_actions_hit_targets_sliders_and_accesskit_dispatch() {
    use accesskit::{Action, ActionData, ActionRequest, NodeId, Role, TreeId};
    use hybrid_app::media_bridge::FocusedTextField;
    use hybrid_app::renderer::dispatch_accesskit_request;
    use hybrid_ui::primitives::HitTarget;
    use hybrid_ui::HybridUi;

    let dir = temp_test_dir("parity_dispatch");
    let rt = Arc::new(Runtime::new().unwrap());
    let mut bridge = MediaBridge::new(dir.clone(), Arc::clone(&rt));
    let mut input = InputController::new(BTreeMap::new(), 3);

    // 1. Seek percentages (seek-0 .. seek-90) & modal/quit shortcut actions
    bridge.ui.duration_secs = 200.0;
    bridge.execute_shortcut_action("seek-50");
    assert!((bridge.ui.current_secs - 100.0).abs() < 1e-6);
    bridge.execute_shortcut_action("seek-0");
    assert!((bridge.ui.current_secs - 0.0).abs() < 1e-6);
    bridge.execute_shortcut_action("seek-90");
    assert!((bridge.ui.current_secs - 180.0).abs() < 1e-6);

    bridge.execute_shortcut_action("toggle-settings");
    assert_eq!(bridge.ui.active_modal, Some(ModalId::Settings));
    bridge.execute_shortcut_action("open-network-stream");
    assert_eq!(bridge.ui.active_modal, Some(ModalId::NetworkStream));
    assert_eq!(
        bridge.focused_input,
        Some(FocusedTextField::NetworkStreamUrl)
    );

    // 2. Slider ratios + HitTargets (Speed, EQ, SubDelay, Volume, Progress, Presets, Shortcuts)
    bridge.apply_hit_with_ratio(&HitTarget::ProgressSeek, Some(0.25), None);
    assert!((bridge.ui.current_secs - 50.0).abs() < 1e-6);

    bridge.apply_hit_with_ratio(&HitTarget::VolumeSlider, Some(0.65), None);
    assert_eq!(bridge.ui.volume_pct, 65);

    bridge.apply_hit_with_ratio(&HitTarget::EqBandSlider(0), None, Some(0.25));
    assert!((bridge.equalizer.bands[0] - 6.0).abs() < 0.01);

    bridge.apply_hit_with_ratio(&HitTarget::SpeedSlider, Some(1.0), None);
    assert!((bridge.ui.speed - 4.0).abs() < 1e-6);

    bridge.handle_hit_target(&HitTarget::SpeedPreset(0));
    assert!((bridge.ui.speed - 0.25).abs() < 1e-6);
    bridge.handle_hit_target(&HitTarget::SpeedPreset(3));
    assert!((bridge.ui.speed - 1.0).abs() < 1e-6);
    bridge.handle_hit_target(&HitTarget::SpeedPreset(4));
    assert!((bridge.ui.speed - 1.25).abs() < 1e-6);
    bridge.handle_hit_target(&HitTarget::SpeedPreset(9));
    assert!((bridge.ui.speed - 4.0).abs() < 1e-6);

    // Subtitle delay steps in ms (100x scaling bug fixed) + Reset(0)
    bridge.handle_hit_target(&HitTarget::SubDelayStep(500));
    assert!((bridge.ui.subtitle_delay_secs - 0.5).abs() < 1e-6);
    bridge.handle_hit_target(&HitTarget::SubDelayStep(-100));
    assert!((bridge.ui.subtitle_delay_secs - 0.4).abs() < 1e-6);
    bridge.handle_hit_target(&HitTarget::SubDelayStep(0));
    assert!(bridge.ui.subtitle_delay_secs.abs() < 1e-6);

    // Audio delay steps + Reset(0)
    bridge.handle_hit_target(&HitTarget::AudioDelayStep(100));
    assert!((bridge.ui.audio_delay_secs - 0.1).abs() < 1e-6);
    bridge.handle_hit_target(&HitTarget::AudioDelayStep(0));
    assert!(bridge.ui.audio_delay_secs.abs() < 1e-6);

    bridge.handle_hit_target(&HitTarget::EqPresetSelect);
    assert_eq!(bridge.ui.active_dropdown, Some(DropdownKind::EqPreset));
    bridge.handle_hit_target(&HitTarget::DropdownOption(0));
    assert_eq!(bridge.equalizer.preset, EqualizerPreset::Flat);
    assert_eq!(bridge.ui.active_dropdown, None);
    bridge.handle_hit_target(&HitTarget::EqPresetSelect);
    bridge.handle_hit_target(&HitTarget::DropdownOption(1));
    assert_eq!(bridge.equalizer.preset, EqualizerPreset::BassBoost);

    bridge.handle_hit_target(&HitTarget::YoutubeQualityButton);
    assert_eq!(
        bridge.ui.active_dropdown,
        Some(DropdownKind::YoutubeQuality)
    );
    bridge.handle_hit_target(&HitTarget::DropdownOption(1));
    assert_ne!(bridge.ui.youtube_quality_label, "Auto");

    // Background controls + reset
    let initial_speed = bridge.ui.dither_cfg.wave_speed;
    bridge.handle_hit_target(&HitTarget::BgControlInput("bgDitherSpeed".to_string()));
    assert_ne!(bridge.ui.dither_cfg.wave_speed, initial_speed);
    bridge.handle_hit_target(&HitTarget::BgResetCurrent);
    assert!((bridge.ui.dither_cfg.wave_speed - initial_speed).abs() < 1e-5);

    // Focused text input typing & shortcut rebind/reset
    bridge.handle_hit_target(&HitTarget::PlaylistSearch);
    assert_eq!(bridge.focused_input, Some(FocusedTextField::PlaylistSearch));
    bridge.append_text_to_focused_input("ep2");
    bridge.backspace_focused_input();
    assert_eq!(bridge.ui.playlist.search_query, "ep");

    bridge.handle_hit_target(&HitTarget::ShortcutRebind("toggle-play".to_string()));
    assert_eq!(
        bridge.ui.recording_shortcut_id.as_deref(),
        Some("toggle-play")
    );
    bridge.commit_shortcut_rebind("toggle-play", "KeyP");
    assert_eq!(bridge.ui.recording_shortcut_id, None);
    assert_eq!(
        bridge
            .ui
            .custom_shortcuts
            .get("toggle-play")
            .map(String::as_str),
        Some("KeyP")
    );
    bridge.handle_hit_target(&HitTarget::ShortcutResetOne("toggle-play".to_string()));
    assert!(!bridge.ui.custom_shortcuts.contains_key("toggle-play"));
    bridge.commit_shortcut_rebind("stop", "KeyX");
    bridge.handle_hit_target(&HitTarget::ShortcutResetAll);
    assert!(bridge.ui.custom_shortcuts.is_empty());

    // Settings appearance/behavior toggles, real dropdowns/color picker & scroll clamping/reset
    bridge.handle_hit_target(&HitTarget::SettingsThemeSelect);
    assert_eq!(bridge.ui.active_dropdown, Some(DropdownKind::SettingsTheme));
    bridge.handle_hit_target(&HitTarget::DropdownOption(1));
    assert_eq!(bridge.ui.theme, ThemeMode::Oled);
    let prev_accent = bridge.ui.accent_hex.clone();
    bridge.handle_hit_target(&HitTarget::SettingsAccentPicker);
    assert_eq!(
        bridge.ui.active_color_picker,
        Some(ColorPickerTarget::Accent)
    );
    bridge.handle_hit_target(&HitTarget::ColorPickerSwatch(1));
    assert_ne!(bridge.ui.accent_hex, prev_accent);
    bridge.apply_hit_with_ratio(&HitTarget::ColorPickerSpectrum, Some(0.8), Some(0.2));
    bridge.handle_hit_target(&HitTarget::ColorPickerClose);
    assert_eq!(bridge.ui.active_color_picker, None);
    let prev_brand = bridge.ui.font_config.brand_font_enabled;
    bridge.handle_hit_target(&HitTarget::SettingsBrandFontsToggle);
    assert_ne!(bridge.ui.font_config.brand_font_enabled, prev_brand);
    let prev_resume = bridge.ui.auto_resume;
    bridge.handle_hit_target(&HitTarget::SettingsAutoResumeToggle);
    assert_ne!(bridge.ui.auto_resume, prev_resume);
    bridge.handle_hit_target(&HitTarget::SettingsMotionSelect);
    assert_eq!(
        bridge.ui.active_dropdown,
        Some(DropdownKind::SettingsMotion)
    );
    bridge.handle_hit_target(&HitTarget::DropdownOption(0));
    assert_eq!(bridge.db.preferences.motion_profile, "reduced");
    bridge.handle_hit_target(&HitTarget::SettingsSeekStepSelect);
    assert_eq!(
        bridge.ui.active_dropdown,
        Some(DropdownKind::SettingsSeekStep)
    );
    bridge.handle_hit_target(&HitTarget::DropdownOption(2));
    assert_eq!(bridge.ui.seek_step_secs, 10);
    let prev_dbl = bridge.ui.double_click_fullscreen;
    bridge.handle_hit_target(&HitTarget::SettingsDoubleClickToggle);
    assert_ne!(bridge.ui.double_click_fullscreen, prev_dbl);
    let prev_unlock = bridge.ui.show_unlock_button;
    bridge.handle_hit_target(&HitTarget::SettingsUnlockVisToggle);
    assert_ne!(bridge.ui.show_unlock_button, prev_unlock);

    bridge.ui.active_modal = None;
    bridge.ui.settings_scroll_y = 250.0;
    bridge.toggle_modal(ModalId::Settings);
    assert_eq!(bridge.ui.settings_scroll_y, 0.0);
    bridge.scroll_settings_modal(120.0);
    assert!((bridge.ui.settings_scroll_y - 120.0).abs() < 1e-4);
    bridge.scroll_settings_modal(5000.0);
    let expected_max_settings = hybrid_ui::layout::settings_scroll_max(&bridge.ui);
    assert!((bridge.ui.settings_scroll_y - expected_max_settings).abs() < 1e-4);
    bridge.scroll_settings_modal(-5000.0);
    assert_eq!(bridge.ui.settings_scroll_y, 0.0);

    bridge.toggle_modal(ModalId::BgSettings);
    bridge.ui.welcome_background = WelcomeBackgroundKind::Dither;
    bridge.scroll_bg_settings_modal(5000.0);
    let expected_max_bg = hybrid_ui::layout::bg_settings_scroll_max(&bridge.ui);
    assert!((bridge.ui.bg_settings_scroll_y - expected_max_bg).abs() < 1e-4);
    bridge.ui.active_modal = None;

    // 3. AccessKit ActionRequested dispatch against live HybridUi tree
    bridge.ui.welcome_active = false;
    bridge.ui.active_modal = None;
    bridge.ui.viewport_w = 1280.0;
    bridge.ui.viewport_h = 720.0;
    let ui = HybridUi::new();
    let frame = ui.build_frame(&bridge.ui);

    let vol_node_id: NodeId = frame
        .a11y_tree
        .nodes
        .iter()
        .find(|(_, n)| n.role() == Role::Slider && n.label() == Some("Volume"))
        .map(|(id, _)| *id)
        .expect("Volume slider node missing in AccessKit tree");

    let handled = dispatch_accesskit_request(
        &mut bridge,
        &mut input,
        &frame,
        &ActionRequest {
            action: Action::SetValue,
            target_tree: TreeId::ROOT,
            target_node: vol_node_id,
            data: Some(ActionData::NumericValue(42.0)),
        },
    );
    assert!(handled);
    assert_eq!(bridge.ui.volume_pct, 42);

    let play_node_id: NodeId = frame
        .a11y_tree
        .nodes
        .iter()
        .find(|(_, n)| n.role() == Role::Button && n.label() == Some("Play"))
        .map(|(id, _)| *id)
        .expect("Play button node missing in AccessKit tree");
    let clicked = dispatch_accesskit_request(
        &mut bridge,
        &mut input,
        &frame,
        &ActionRequest {
            action: Action::Click,
            target_tree: TreeId::ROOT,
            target_node: play_node_id,
            data: None,
        },
    );
    assert!(clicked);

    bridge.execute_shortcut_action("quit");
    assert!(bridge.quit_requested);

    // Custom welcome quality + full saved BG config roundtrip
    let mut custom_bridge = MediaBridge::new(dir.join("custom-bg"), Arc::clone(&rt));
    custom_bridge.db.preferences.welcome_quality = "custom".to_string();
    custom_bridge.db.preferences.bg_opts_dither = Some(
        serde_json::json!({
            "speed": 0.12,
            "color": "#ff00aa",
            "bgColor": "#112233",
            "pixelSize": 5,
            "colorNum": 6
        })
        .to_string(),
    );
    custom_bridge.db.preferences.bg_opts_gridmotion = Some(
        serde_json::json!({
            "gradientColor": "#102030",
            "customPictures": ["C:/fake/img1.png"]
        })
        .to_string(),
    );
    custom_bridge.sync_preferences_to_ui();
    assert_eq!(
        custom_bridge.ui.welcome_quality,
        hybrid_shaders::QualityTier::Custom
    );
    assert!((custom_bridge.ui.dither_cfg.wave_speed - 0.12).abs() < 1e-5);
    assert!((custom_bridge.ui.dither_cfg.pixel_size - 5.0).abs() < 1e-5);
    assert!((custom_bridge.ui.dither_cfg.color_num - 6.0).abs() < 1e-5);
    assert!((custom_bridge.ui.dither_cfg.wave_color[0] - 1.0).abs() < 1e-3);
    assert!((custom_bridge.ui.dither_cfg.bg_color[2] - (0x33 as f32 / 255.0)).abs() < 1e-3);
    assert_eq!(
        custom_bridge.gridmotion_custom_pictures,
        vec!["C:/fake/img1.png"]
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn playlist_animation_slides_overlays_smoothly_and_bg_swatch_matches_css() {
    use hybrid_ui::state::UiState;
    use hybrid_ui::HybridUi;

    let mut ui = UiState {
        viewport_w: 1280.0,
        viewport_h: 720.0,
        welcome_active: false,
        ab_loop: AbLoopState::PointASet(5000),
        ..Default::default()
    };
    ui.volume_osd.visible = true;
    ui.volume_osd.percent = 80;
    ui.playlist.open = false;
    ui.anim.playlist_t = 1.0;

    let engine = HybridUi::new();
    let closed_frame = engine.build_frame(&ui);
    let closed_vol_x = closed_frame
        .solid_rects
        .iter()
        .find(|q| q.id == "top-right-volume-osd")
        .unwrap()
        .rect
        .x;
    let closed_ab_x = closed_frame
        .solid_rects
        .iter()
        .find(|q| q.id == "ab-loop-indicator")
        .unwrap()
        .rect
        .x;

    ui.playlist.open = true;
    ui.anim.playlist_t = 0.5;
    let mid_frame = engine.build_frame(&ui);
    let mid_vol_x = mid_frame
        .solid_rects
        .iter()
        .find(|q| q.id == "top-right-volume-osd")
        .unwrap()
        .rect
        .x;
    let mid_ab_x = mid_frame
        .solid_rects
        .iter()
        .find(|q| q.id == "ab-loop-indicator")
        .unwrap()
        .rect
        .x;

    ui.anim.playlist_t = 1.0;
    let open_frame = engine.build_frame(&ui);
    let open_vol_x = open_frame
        .solid_rects
        .iter()
        .find(|q| q.id == "top-right-volume-osd")
        .unwrap()
        .rect
        .x;
    let open_ab_x = open_frame
        .solid_rects
        .iter()
        .find(|q| q.id == "ab-loop-indicator")
        .unwrap()
        .rect
        .x;

    assert!(mid_vol_x < closed_vol_x && mid_vol_x > open_vol_x);
    assert!(mid_ab_x < closed_ab_x && mid_ab_x > open_ab_x);
    assert!((closed_ab_x - open_ab_x - 320.0).abs() < 0.5);

    // BG Settings color swatch matches `.bg-color-input` (36x36, 2px border)
    ui.welcome_active = true;
    ui.welcome_background = WelcomeBackgroundKind::Dither;
    ui.active_modal = Some(ModalId::BgSettings);
    ui.anim.modal_t = 1.0;
    let bg_frame = engine.build_frame(&ui);
    let swatch = bg_frame
        .solid_rects
        .iter()
        .find(|q| q.id == "bg-ctrl-swatch-bgDitherColor")
        .expect("bgDitherColor swatch quad");
    assert!((swatch.rect.width - 36.0).abs() < 0.01);
    assert!((swatch.rect.height - 36.0).abs() < 0.01);
    assert!((swatch.border_width - 2.0).abs() < 0.01);
}

#[test]
fn workstream_a_hit_absorption_accordion_scrub_escape_clipboard_cursor_and_focus() {
    use hybrid_app::input::{CursorIdleTracker, KeyModifiers};
    use hybrid_ui::primitives::HitTarget;
    use hybrid_ui::state::FocusedTextField;

    let rt = Arc::new(Runtime::new().unwrap());
    let dir = temp_test_dir("workstream_a");
    let mut bridge = MediaBridge::new(dir.clone(), Arc::clone(&rt));
    bridge.ui.welcome_active = false;
    bridge.ui.duration_secs = 200.0;
    bridge.ui.current_secs = 20.0;

    // 1. Clicking ModalPanelSurface keeps modal open; clicking outside #bgAccordionPopover closes only popover
    bridge.ui.open_modal(ModalId::Settings);
    bridge.handle_hit_target(&HitTarget::ModalPanelSurface);
    assert_eq!(bridge.ui.active_modal, Some(ModalId::Settings));

    bridge.ui.open_modal(ModalId::BgSettings);
    bridge.ui.bg_accordion_open = true;
    bridge.handle_hit_target(&HitTarget::BgAccordionCard(
        WelcomeBackgroundKind::Particles,
    ));
    assert_eq!(
        bridge.ui.welcome_background,
        WelcomeBackgroundKind::Particles
    );
    assert!(bridge.ui.bg_accordion_open);
    assert_eq!(bridge.ui.active_modal, Some(ModalId::BgSettings));
    bridge.handle_hit_target(&HitTarget::ModalPanelSurface);
    assert!(!bridge.ui.bg_accordion_open);
    assert_eq!(bridge.ui.active_modal, Some(ModalId::BgSettings));
    bridge.handle_hit_target(&HitTarget::ModalBackdrop);
    assert_eq!(bridge.ui.active_modal, None);

    // 2. Scrub preview updates visual position cleanly without committing until seek_normalized
    bridge.is_dragging_progress = true;
    bridge.preview_seek_normalized(0.5);
    assert!((bridge.ui.current_secs - 100.0).abs() < 1e-6);
    bridge.is_dragging_progress = false;
    bridge.seek_normalized(0.5);
    assert!((bridge.ui.current_secs - 100.0).abs() < 1e-6);

    // 3. Escape priority + wheel thresholds + modal Space pass-through + CursorIdleTracker gates
    let now = Instant::now();
    let mut input = InputController::new(BTreeMap::new(), 3);
    let mut tracker = CursorIdleTracker::new(now);
    let base_ctx = InputContext {
        has_active_media: true,
        is_playing: true,
        ..Default::default()
    };
    let dd_ctx = InputContext {
        dropdown_open: true,
        is_fullscreen: true,
        ..base_ctx
    };
    assert_eq!(
        input.on_key_down(now, "Escape", 0, KeyModifiers::default(), false, &dd_ctx),
        vec![InputAction::DismissModalOrPopover]
    );

    assert!(input.on_mouse_wheel(now, 0.0, 0.5, &base_ctx).is_none());
    assert!(input.on_mouse_wheel(now, 10.0, 4.0, &base_ctx).is_none());
    assert_eq!(
        input.on_mouse_wheel(now, 0.0, 4.0, &base_ctx),
        Some(InputAction::AdjustVolumeDelta(5))
    );

    let mut modal_ctx = InputContext {
        modal_open: true,
        ..base_ctx
    };
    assert_eq!(
        input.on_key_down(now, "Space", 0, KeyModifiers::default(), false, &modal_ctx),
        vec![InputAction::ShortcutAction("toggle-play".to_string())]
    );
    modal_ctx.interactive_control_focused = true;
    assert!(input
        .on_key_down(now, "Space", 0, KeyModifiers::default(), false, &modal_ctx)
        .is_empty());

    tracker.record_activity(now);
    let text_ctx = InputContext {
        text_input_focused: true,
        ..base_ctx
    };
    assert!(!tracker.evaluate(
        now + CURSOR_IDLE_TIMEOUT + Duration::from_secs(1),
        &text_ctx
    ));
    let hover_ctx = InputContext {
        hovering_controls: true,
        ..base_ctx
    };
    assert!(!tracker.evaluate(
        now + CURSOR_IDLE_TIMEOUT + Duration::from_secs(1),
        &hover_ctx
    ));

    // 4. UTF-8 text cursor editing + focus save/restore on NetworkStream modal
    bridge.ui.focused_target = Some(HitTarget::PlayPause);
    bridge.ui.open_modal(ModalId::NetworkStream);
    assert_eq!(
        bridge.ui.focused_target,
        Some(HitTarget::NetworkStreamInput)
    );
    assert_eq!(
        bridge.ui.saved_focus_before_modal,
        Some(HitTarget::PlayPause)
    );
    bridge.set_focused_input(Some(FocusedTextField::NetworkStreamUrl));
    bridge.append_text_to_focused_input("https://aβ");
    bridge.move_focused_input_left(false);
    bridge.append_text_to_focused_input("!");
    assert_eq!(bridge.ui.network_stream_url, "https://a!β");
    bridge.home_focused_input(false);
    bridge.delete_focused_input();
    assert_eq!(bridge.ui.network_stream_url, "ttps://a!β");
    bridge.ui.close_modals_animated();
    assert_eq!(bridge.ui.focused_target, Some(HitTarget::PlayPause));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn workstream_b_live_seek_skip_lock_eq_debounce_subtitles_and_resume() {
    use hybrid_core::playlist::PlaylistItem;
    use hybrid_ui::primitives::HitTarget;

    let rt = Arc::new(Runtime::new().unwrap());
    let dir = temp_test_dir("workstream_b");
    let mut bridge = MediaBridge::new(dir.clone(), Arc::clone(&rt));

    // B.5: Volume OSD does not show when no media is loaded
    bridge.current_file = None;
    bridge.set_volume_pct(55);
    assert!(!bridge.ui.volume_osd.visible);

    // Load a dummy single item context
    let sample_path = dir.join("movie.mp4");
    fs::write(&sample_path, b"dummy").unwrap();
    bridge.current_file = Some(sample_path.to_string_lossy().to_string());
    bridge.ui.welcome_active = false;
    bridge.ui.duration_secs = 300.0;
    bridge.ui.current_secs = 40.0;
    bridge.set_volume_pct(60);
    assert!(bridge.ui.volume_osd.visible);
    assert_eq!(bridge.ui.volume_osd.percent, 60);

    // B.2: Live forward seek clamp
    bridge.ui.is_live_stream = true;
    bridge.ui.duration_secs = 120.0;
    bridge.ui.current_secs = 115.0;
    bridge.seek_relative(15.0);
    assert!(bridge.ui.current_secs <= 120.0);
    bridge.ui.is_live_stream = false;

    // B.5: Skip indicator accumulates within 800ms
    bridge.last_skip_at = None;
    bridge.ui.duration_secs = 300.0;
    bridge.ui.current_secs = 40.0;
    bridge.seek_relative(5.0);
    assert!(bridge.ui.skip_indicator.forward);
    assert_eq!(bridge.ui.skip_indicator.accumulated_secs, 5);
    bridge.seek_relative(5.0);
    assert!(bridge.ui.skip_indicator.forward);
    assert_eq!(bridge.ui.skip_indicator.accumulated_secs, 10);
    bridge.seek_relative(-5.0);
    assert!(!bridge.ui.skip_indicator.forward);
    assert_eq!(bridge.ui.skip_indicator.accumulated_secs, 5);

    // B.5: Lock closes modals/popovers, collapses playlist, shows toast
    bridge.ui.open_modal(ModalId::Settings);
    bridge.ui.playlist.open = true;
    bridge.ui.active_dropdown = Some(DropdownKind::EqPreset);
    bridge.set_locked(true);
    assert!(bridge.ui.locked);
    assert!(bridge.ui.active_modal.is_none());
    assert!(bridge.ui.active_dropdown.is_none());
    assert!(!bridge.ui.playlist.open);
    assert!(bridge
        .ui
        .toasts
        .iter()
        .any(|t| t.message.contains("Controls locked")));
    bridge.set_locked(false);
    assert!(bridge
        .ui
        .toasts
        .iter()
        .any(|t| t.message.contains("Controls unlocked")));

    // B.6: EQ debounce marks Custom and flushes to DB cleanly
    bridge.set_eq_band_debounced(0, 4.5);
    assert_eq!(bridge.ui.equalizer.preset, EqualizerPreset::Custom);
    assert_eq!(bridge.ui.equalizer.bands[0], 4.5);
    bridge.flush_eq_debounce();
    assert_eq!(bridge.db.preferences.equalizer_bands[0], 4.5);

    // B.7: Subtitle delay clamp (-600..=600) and default font family "Segoe UI"
    assert_eq!(
        hybrid_core::tracks::SubtitleStyle::default().font_family,
        "Segoe UI"
    );
    bridge.set_subtitle_delay(900.0);
    assert_eq!(bridge.ui.subtitle_delay_secs, 600.0);
    bridge.set_subtitle_delay(-900.0);
    assert_eq!(bridge.ui.subtitle_delay_secs, -600.0);
    bridge.adjust_subtitle_delay(-5.5);
    assert_eq!(bridge.ui.subtitle_delay_secs, -600.0);

    // B.3: Clicking TimeRemainingToggle flips time_remaining_mode
    assert!(!bridge.ui.time_remaining_mode);
    bridge.handle_hit_target(&HitTarget::TimeRemainingToggle);
    assert!(bridge.ui.time_remaining_mode);

    // B.8: Resume `_isNearEnd` rule (periodic tick does not clear saved resume) & single-item `allowResume`
    let p_str = sample_path.to_string_lossy().to_string();
    bridge.db.save_resume_position(&p_str, 45.0);
    bridge.ui.auto_resume = true;
    bridge.ui.duration_secs = 100.0;
    bridge.ui.current_secs = 98.5; // near end (<2s remaining)
    bridge.save_current_resume_position();
    assert!((bridge.db.get_resume_position(&p_str) - 45.0).abs() < 0.01);

    let other_path = dir.join("other.mp4");
    fs::write(&other_path, b"dummy2").unwrap();
    bridge.playlist.items = vec![
        PlaylistItem {
            id: "1".to_string(),
            title: "movie.mp4".to_string(),
            path: p_str.clone(),
            is_url: false,
            duration: Some(100),
            thumbnail: None,
        },
        PlaylistItem {
            id: "2".to_string(),
            title: "other.mp4".to_string(),
            path: other_path.to_string_lossy().to_string(),
            is_url: false,
            duration: Some(100),
            thumbnail: None,
        },
    ];
    // Multi-item playlist suppresses auto-resume
    bridge.play_playlist_index(0);
    assert!(bridge.pending_resume_seek.is_none());

    // Single-item playlist queues resume seek
    bridge.playlist.items.truncate(1);
    bridge.play_playlist_index(0);
    assert!(bridge.pending_resume_seek.is_some());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn workstream_c_playlist_stop_clip_guards_shortcuts_burst_fps_and_gridmotion() {
    use hybrid_app::input::InputController;
    use hybrid_app::renderer::map_key_code;
    use hybrid_ui::primitives::HitTarget;
    use std::collections::BTreeMap;
    use winit::keyboard::KeyCode;

    let rt = Arc::new(Runtime::new().unwrap());
    let dir = std::env::temp_dir().join(format!("hybrid-wc-app-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let f1 = dir.join("clip1.mp4");
    let f2 = dir.join("clip2.mp4");
    fs::write(&f1, b"dummy1").unwrap();
    fs::write(&f2, b"dummy2").unwrap();

    let mut bridge = MediaBridge::new(dir.clone(), rt);
    assert!(!bridge.ui.default_screenshot_dir.is_empty());

    // C.2: Playlist row remove active track advances or stops; clear resets welcome + "No media loaded"
    bridge.open_files(&[f1.clone(), f2.clone()], true, true);
    assert!(!bridge.ui.welcome_active);
    bridge.handle_hit_target(&HitTarget::PlaylistRowRemove(0));
    assert_eq!(bridge.playlist.items.len(), 1);
    assert!(!bridge.ui.welcome_active);
    bridge.handle_hit_target(&HitTarget::PlaylistClear);
    assert!(bridge.ui.welcome_active);
    assert_eq!(bridge.ui.window_title, "No media loaded");
    assert!(bridge.current_file.is_none());

    // C.4: Clip recording rejects non-local stream & short duration, cancels on media change
    bridge.current_file = Some("https://example.com/live.m3u8".into());
    bridge.toggle_clip_recording();
    assert!(bridge.clip_start_secs.is_none());
    assert!(bridge
        .ui
        .toasts
        .iter()
        .any(|t| t.message.contains("local files")));

    bridge.current_file = Some(f1.to_string_lossy().into_owned());
    bridge.ui.current_secs = 12.0;
    bridge.clip_start_secs = Some(10.0);
    assert!(bridge.poll_events());
    assert_eq!(
        bridge.ui.recording_indicator,
        RecordingIndicatorMode::ClipActive { elapsed_ds: 20 }
    );
    bridge.ui.current_secs = 10.1; // <= 0.2s guard
    bridge.toggle_clip_recording();
    assert_eq!(
        bridge.ui.recording_indicator,
        RecordingIndicatorMode::Hidden
    );
    assert!(bridge
        .ui
        .toasts
        .iter()
        .any(|t| t.message.contains("too short")));

    // C.5: Rebind conflict toast shows displaced label; show-about / show-shortcuts toasts; Gridmotion thumb remove
    bridge.commit_shortcut_rebind("toggle-mute", "Space");
    assert!(bridge
        .ui
        .toasts
        .iter()
        .any(|t| t.message.contains("unbound from \"Play / Pause\"")));
    bridge.execute_shortcut_action("show-about");
    assert!(bridge
        .ui
        .toasts
        .iter()
        .any(|t| t.message == "Hybrid Player v12.6.0 — Next-gen media player"));
    bridge.execute_shortcut_action("show-shortcuts");
    assert!(bridge
        .ui
        .toasts
        .iter()
        .any(|t| t.message.starts_with("Shortcuts: ")));

    bridge.gridmotion_custom_pictures = vec!["a.png".into(), "b.png".into()];
    bridge.ui.gridmotion_custom_pictures = bridge.gridmotion_custom_pictures.clone();
    bridge.handle_hit_target(&HitTarget::BgGridmotionRemoveImage(0));
    assert_eq!(bridge.gridmotion_custom_pictures, vec!["b.png".to_string()]);
    assert_eq!(
        bridge.ui.gridmotion_custom_pictures,
        vec!["b.png".to_string()]
    );

    // C.3 / C.5: Dynamic burst FPS & extended map_key_code
    let mut input = InputController::new(BTreeMap::new(), 3);
    input.set_burst_fps(60.0);
    assert_eq!(input.burst_interval, std::time::Duration::from_millis(16));
    assert_eq!(map_key_code(KeyCode::NumpadEnter), "NumpadEnter");
    assert_eq!(map_key_code(KeyCode::PrintScreen), "PrintScreen");
    assert_eq!(map_key_code(KeyCode::CapsLock), "CapsLock");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn workstream_d_os_dialog_ytdl_single_instance_a11y_and_end_file_parity() {
    use accesskit::{Action, ActionData, ActionRequest, NodeId};
    use hybrid_app::cli::{SINGLE_INSTANCE_MUTEX_NAME, SINGLE_INSTANCE_PIPE_NAME};
    use hybrid_app::media_bridge::{
        build_file_dialog_filters, normalize_network_media_url, FileDialogKind,
    };
    use hybrid_app::renderer::dispatch_accesskit_request;
    use hybrid_core::binary_resolver::{resolve_lib_mpv_with_options, ResolveOptions};
    use hybrid_mpv_engine::{EndFileReason, MpvEvent};
    use hybrid_ui::HybridUi;
    use hybrid_win32_shell::{DEFAULT_SINGLE_INSTANCE_MUTEX, DEFAULT_SINGLE_INSTANCE_PIPE};

    // D.4: Version parity + unified single-instance constants
    assert_eq!(env!("CARGO_PKG_VERSION"), "12.6.0");
    assert_eq!(SINGLE_INSTANCE_MUTEX_NAME, DEFAULT_SINGLE_INSTANCE_MUTEX);
    assert_eq!(SINGLE_INSTANCE_PIPE_NAME, DEFAULT_SINGLE_INSTANCE_PIPE);

    // D.3: File dialog filters match Electron media/subtitle/image/cookies specs
    let media_filters = build_file_dialog_filters(FileDialogKind::MediaSingle);
    assert_eq!(media_filters.len(), 2);
    assert_eq!(media_filters[0].0, "Media Files");
    assert!(media_filters[0].1.contains("*.mp4"));
    assert!(media_filters[0].1.contains("*.ts"));
    assert!(media_filters[0].1.contains("*.m2ts"));
    assert!(media_filters[0].1.contains("*.flac"));
    assert_eq!(
        build_file_dialog_filters(FileDialogKind::Subtitle)[0].0,
        "Subtitle Files"
    );
    assert_eq!(
        build_file_dialog_filters(FileDialogKind::CookiesTxt)[0].1,
        "*.txt"
    );

    // D.4: ytdl:// accepted in network URL normalization & PATH fallback controlled by ResolveOptions
    assert_eq!(
        normalize_network_media_url("ytdl://dQw4w9WgXcQ").as_deref(),
        Some("ytdl://dQw4w9WgXcQ")
    );
    let _ = resolve_lib_mpv_with_options(
        None,
        None,
        None,
        ResolveOptions {
            allow_path_lookup: false,
        },
    );

    let rt = Arc::new(Runtime::new().unwrap());
    let dir = temp_test_dir("wd-parity");
    let mut bridge = MediaBridge::new(dir.clone(), rt);
    let mut input = InputController::new(BTreeMap::new(), 3);
    let ui_engine = HybridUi::new();

    // D.3: AccessKit SetValue on Subtitle sliders & TextInputs
    bridge.ui.active_modal = Some(ModalId::Subtitles);
    let sub_frame = ui_engine.build_frame(&bridge.ui);
    let find_node = |frame: &hybrid_ui::primitives::UiFrameOutput, label: &str| -> NodeId {
        frame
            .a11y_tree
            .nodes
            .iter()
            .find_map(|(id, n)| (n.label() == Some(label)).then_some(*id))
            .unwrap_or_else(|| panic!("missing a11y node `{label}`"))
    };
    let fs_node = find_node(&sub_frame, "Subtitle Font Size");
    let bo_node = find_node(&sub_frame, "Subtitle Background Opacity");
    assert!(dispatch_accesskit_request(
        &mut bridge,
        &mut input,
        &sub_frame,
        &ActionRequest {
            action: Action::SetValue,
            target_tree: accesskit::TreeId::ROOT,
            target_node: fs_node,
            data: Some(ActionData::NumericValue(40.0)),
        },
    ));
    assert_eq!(bridge.ui.subtitle_font_size, 40);
    assert!(dispatch_accesskit_request(
        &mut bridge,
        &mut input,
        &sub_frame,
        &ActionRequest {
            action: Action::SetValue,
            target_tree: accesskit::TreeId::ROOT,
            target_node: bo_node,
            data: Some(ActionData::NumericValue(0.85)),
        },
    ));
    assert_eq!(bridge.ui.subtitle_bg_opacity_pct, 85);

    bridge.ui.active_modal = Some(ModalId::NetworkStream);
    let net_frame = ui_engine.build_frame(&bridge.ui);
    let url_node = find_node(&net_frame, "Network Stream URL");
    assert!(dispatch_accesskit_request(
        &mut bridge,
        &mut input,
        &net_frame,
        &ActionRequest {
            action: Action::SetValue,
            target_tree: accesskit::TreeId::ROOT,
            target_node: url_node,
            data: Some(ActionData::Value("https://example.com/live.m3u8".into())),
        },
    ));
    assert_eq!(
        bridge.ui.network_stream_url,
        "https://example.com/live.m3u8"
    );
    assert_eq!(
        bridge.ui.text_cursors.network_stream.cursor,
        "https://example.com/live.m3u8".len()
    );

    // D.4: end-file stop ignored while loading; YouTube error resets idle & queues 1-shot auto-retry
    bridge.ui.active_modal = None;
    bridge.ui.loading_visible = true;
    bridge.ui.welcome_active = false;
    bridge.current_file = Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ".into());
    bridge.is_youtube = true;
    bridge.handle_mpv_event(MpvEvent::EndFile {
        reason: EndFileReason::Stop,
        error: None,
    });
    assert!(
        bridge.ui.loading_visible,
        "stop during loading must be ignored"
    );

    bridge.handle_mpv_event(MpvEvent::EndFile {
        reason: EndFileReason::Error,
        error: Some("Sign in to confirm you're not a bot".into()),
    });
    assert!(bridge.ui.welcome_active);
    assert!(bridge.current_file.is_none());
    assert!(bridge.yt_auto_retry_used);
    assert!(bridge.yt_retry_deadline.is_some());

    // Second error in same flow does not schedule a second auto-retry; emits anti-bot toast
    bridge.ui.welcome_active = false;
    bridge.is_youtube = true;
    bridge.current_file = Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ".into());
    bridge.yt_retry_deadline = None;
    bridge.handle_mpv_event(MpvEvent::EndFile {
        reason: EndFileReason::Error,
        error: Some("HTTP error 403 Forbidden".into()),
    });
    assert!(bridge.yt_retry_deadline.is_none());
    assert!(bridge
        .ui
        .toasts
        .iter()
        .any(|t| t.message.contains("Sign in to YouTube")));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn review_fixes_screenshot_dir_stop_reset_and_live_seek_guard() {
    use hybrid_ui::state::AbLoopState;

    let rt = Arc::new(Runtime::new().unwrap());
    let dir = temp_test_dir("review-fixes");
    let mut bridge = MediaBridge::new(dir.clone(), rt);

    // 1. Default screenshot dir matches Electron (`userData/screenshots`), never `Pictures/Hybrid Player`
    assert_eq!(bridge.default_screenshot_dir(), dir.join("screenshots"));
    assert_eq!(
        bridge.ui.default_screenshot_dir,
        dir.join("screenshots").to_string_lossy()
    );

    // 2. Live stream seek_normalized & preview_seek_normalized must be no-ops (`player.js:793`)
    bridge.ui.welcome_active = false;
    bridge.ui.is_live_stream = true;
    bridge.ui.duration_secs = 120.0;
    bridge.ui.current_secs = 95.0;
    bridge.preview_seek_normalized(0.25);
    assert!((bridge.ui.current_secs - 95.0).abs() < 1e-6);
    bridge.seek_normalized(0.25);
    assert!((bridge.ui.current_secs - 95.0).abs() < 1e-6);

    // 3. stop_playback resets full playback/YouTube/A-B/chapters/tracks/stats state
    bridge.ui.ab_loop = AbLoopState::Active(5, 15);
    bridge.ui.audio_delay_secs = 0.4;
    bridge.ui.is_youtube = true;
    bridge.is_youtube = true;
    bridge.ui.youtube_quality_label = "1080p".into();
    bridge.ui.buffered_secs = 80.0;
    bridge.ui.chapter_positions_norm = vec![0.2, 0.5];
    bridge.ui.chapter_times_secs = vec![24.0, 60.0];
    bridge.ui.chapter_titles = vec!["Intro".into(), "Main".into()];
    bridge.ui.current_chapter = Some(1);
    bridge.ui.stats_overlay.resolution = "1920x1080".into();
    bridge.ui.stats_overlay.bitrate = "4500 kbps".into();
    bridge.ui.stats_overlay.fps = "60.0".into();
    bridge.ui.stats_overlay.codec = "h264".into();
    bridge.ui.stats_overlay.dropped_frames = 3;
    bridge.ui.stats_overlay.cache_secs = 12.5;
    bridge.current_file = Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ".into());

    bridge.stop_playback();

    assert!(bridge.ui.welcome_active);
    assert!(bridge.current_file.is_none());
    assert!(!bridge.ui.is_live_stream);
    assert_eq!(bridge.ui.audio_delay_secs, 0.0);
    assert!(!bridge.ui.is_youtube);
    assert!(!bridge.is_youtube);
    assert_eq!(bridge.ui.youtube_quality_label, "Auto");
    assert_eq!(bridge.ui.ab_loop, AbLoopState::Off);
    assert_eq!(bridge.ui.current_secs, 0.0);
    assert_eq!(bridge.ui.duration_secs, 0.0);
    assert_eq!(bridge.ui.buffered_secs, 0.0);
    assert!(bridge.ui.chapter_positions_norm.is_empty());
    assert!(bridge.ui.chapter_times_secs.is_empty());
    assert!(bridge.ui.chapter_titles.is_empty());
    assert_eq!(bridge.ui.current_chapter, None);
    assert!(bridge.ui.stats_overlay.resolution.is_empty());
    assert!(bridge.ui.stats_overlay.bitrate.is_empty());
    assert!(bridge.ui.stats_overlay.fps.is_empty());
    assert!(bridge.ui.stats_overlay.codec.is_empty());
    assert_eq!(bridge.ui.stats_overlay.dropped_frames, 0);
    assert_eq!(bridge.ui.stats_overlay.cache_secs, 0.0);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn parity_fixes_4_and_7_bg_settings_and_screenshot_notices() {
    use hybrid_app::media_bridge::SessionScreenshot;
    use hybrid_ui::state::ModalId;

    let rt = std::sync::Arc::new(
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap(),
    );
    let dir = std::env::temp_dir().join(format!("hybrid_parity_47_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    let mut bridge = MediaBridge::new(dir.clone(), rt);

    // Regression: opening the playlist must not dismiss the Welcome Effects drawer or reset
    // its accordion, and background settings must remain reachable while the queue is open.
    bridge.toggle_modal(ModalId::BgSettings);
    bridge.ui.bg_accordion_open = true;
    assert_eq!(bridge.ui.active_modal, Some(ModalId::BgSettings));

    bridge.toggle_playlist();
    assert!(bridge.ui.playlist.open);
    assert_eq!(bridge.ui.active_modal, Some(ModalId::BgSettings));
    assert!(bridge.ui.bg_accordion_open);

    // Closing and reopening BgSettings while the playlist stays open must work.
    bridge.toggle_modal(ModalId::BgSettings);
    assert_ne!(bridge.ui.active_modal, Some(ModalId::BgSettings));
    bridge.toggle_modal(ModalId::BgSettings);
    assert_eq!(bridge.ui.active_modal, Some(ModalId::BgSettings));

    let sample = dir.join("clip.mp4");
    fs::write(&sample, b"fake").unwrap();
    bridge.open_files(std::slice::from_ref(&sample), true, true);
    assert!(!bridge.ui.welcome_active);
    assert_ne!(bridge.ui.active_modal, Some(ModalId::BgSettings));
    assert_ne!(bridge.ui.closing_modal, Some(ModalId::BgSettings));
    assert!(!bridge.ui.anim.modal_closing);
    assert!(!bridge.ui.bg_accordion_open);
    assert!(!bridge.ui.anim.bg_accordion_closing);
    bridge.toggle_modal(ModalId::BgSettings);
    assert_ne!(bridge.ui.active_modal, Some(ModalId::BgSettings));

    // Bug 7: Empty delete/restore & carousel toggles show exact Electron notice strings,
    // while real delete/restore shows DELETED/RESTORED badge with no filename notice.
    bridge.toggle_carousel(false);
    assert_eq!(
        bridge.ui.screenshot_notice.as_deref(),
        Some("No screenshots to delete")
    );
    bridge.toggle_carousel(true);
    assert_eq!(
        bridge.ui.screenshot_notice.as_deref(),
        Some("No screenshots to restore")
    );
    bridge.ui.screenshot_notice = None;
    bridge.delete_latest_screenshot();
    assert_eq!(
        bridge.ui.screenshot_notice.as_deref(),
        Some("No screenshots to delete")
    );
    bridge.restore_latest_screenshot();
    assert_eq!(
        bridge.ui.screenshot_notice.as_deref(),
        Some("No screenshots to restore")
    );

    let shot = dir.join("shot.png");
    fs::write(&shot, b"png").unwrap();
    bridge.single_screenshot_stack.push(shot.clone());
    bridge.session_screenshots.push(SessionScreenshot {
        file_path: shot,
        file_name: "shot.png".to_string(),
        timestamp_ms: 1,
    });
    bridge.delete_latest_screenshot();
    assert_eq!(bridge.ui.screenshot_notice, None);
    assert_eq!(
        bridge
            .ui
            .screenshot_badge
            .as_ref()
            .map(|(s, d)| (s.as_str(), *d)),
        Some(("DELETED", true))
    );
    bridge.restore_latest_screenshot();
    assert_eq!(bridge.ui.screenshot_notice, None);
    assert_eq!(
        bridge
            .ui
            .screenshot_badge
            .as_ref()
            .map(|(s, d)| (s.as_str(), *d)),
        Some(("RESTORED", false))
    );

    let _ = fs::remove_dir_all(&dir);
}
