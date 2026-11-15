use std::collections::HashSet;

use hybrid_core::db::{HistoryEntry, PlaylistItemEntry};
use hybrid_core::equalizer::{EqualizerPreset, EQ_BAND_COUNT};
use hybrid_core::shortcuts::{SHORTCUT_CATALOG, VALID_SCREENSHOT_MOUSE_BUTTONS};
use hybrid_shaders::{
    ChameleonSkin, CssBackdropRule, QualityTier, ThemeMode, WelcomeBackgroundKind,
};
use hybrid_ui::a11y::{A11Y_MODAL_ID, A11Y_ROOT_ID};
use hybrid_ui::fonts::{FontRole, TextEngine, BUNDLED_FONT_DATA};
use hybrid_ui::icons::{SvgRasterizer, SvgVariant};
use hybrid_ui::motion::{
    evaluate_keyframe, ElasticRegion, ElasticVolumeSpring, KeyframeId, MotionProfile,
};
use hybrid_ui::primitives::{BreakpointState, HitTarget};
use hybrid_ui::state::{
    AbLoopState, CarouselMode, FocusedTextField, ModalId, PlaylistUiState, RecordingIndicatorMode,
    ToastItem, ToastKind, TrackOption, UiState, BG_CONTROLS_CATALOG, HOVER_TOOLTIP_DELAY_SECS,
    PLAYLIST_ITEM_HEIGHT, PLAYLIST_OVERSCAN_ROWS, PLAYLIST_VIRTUALIZE_THRESHOLD,
};
use hybrid_ui::svg_catalog::SvgId;
use hybrid_ui::tokens::{ChameleonTokens, FontConfig, ThemeTokens, ZIndex};
use hybrid_ui::HybridUi;

#[test]
fn bundled_fonts_load_measure_rasterize_and_fallback() {
    assert_eq!(BUNDLED_FONT_DATA.len(), 6);
    let mut engine = TextEngine::new();
    assert!(
        engine.bundled_face_count() >= 6,
        "expected >= 6 bundled faces loaded, got {}",
        engine.bundled_face_count()
    );
    assert!(engine.has_family("Sora"));
    assert!(engine.has_family("IBM Plex Sans"));

    let branded = FontConfig {
        brand_font_enabled: true,
    };
    assert_eq!(branded.display_family(), "Sora Local");
    assert_eq!(branded.ui_family(), "IBM Plex Sans Local");
    assert_eq!(branded.mono_family(), "Consolas");

    let fallback = FontConfig {
        brand_font_enabled: false,
    };
    assert_eq!(fallback.display_family(), "Segoe UI");
    assert_eq!(fallback.ui_family(), "Segoe UI");
    assert_eq!(fallback.mono_family(), "Consolas");

    let m = engine.measure(
        "Hybrid Player",
        28.0,
        35.0,
        None,
        FontRole::Display,
        700,
        branded,
    );
    assert!(m.width > 40.0 && m.height >= 35.0 && m.line_count == 1);

    let glyphs = engine.rasterize(
        "Hybrid",
        18.0,
        22.0,
        None,
        FontRole::Ui,
        500,
        [255, 255, 255, 255],
        branded,
    );
    assert!(!glyphs.is_empty());
    assert!(glyphs.iter().any(|g| g.data.iter().any(|&b| b > 0)));
}

#[test]
fn all_67_svgs_parse_and_rasterize_and_use_brand_mark() {
    assert_eq!(SvgId::MAIN_UI_COUNT, 64);
    assert_eq!(SvgId::COMPANION_AUTH_COUNT, 3);
    assert_eq!(SvgId::ALL.len(), 67);

    // Titlebar uses dotted-triangle brand-mark.svg geometry, NOT brand-lockup.svg
    let titlebar_brand = SvgId::BrandMarkTitlebar.raw_svg();
    assert!(!titlebar_brand.contains("brand-lockup"));
    assert!(titlebar_brand.contains("<circle"));

    let mut rast = SvgRasterizer::new();
    for &svg in &SvgId::ALL {
        let tree = rast
            .parse_tree(svg, SvgVariant::Default, [255, 255, 255, 255])
            .unwrap_or_else(|e| panic!("failed parsing SVG {svg:?}: {e}"));
        assert!(tree.size().width() > 0.0 && tree.size().height() > 0.0);

        let img = rast
            .rasterize(svg, SvgVariant::Default, 24, 24, [255, 255, 255, 255])
            .unwrap_or_else(|e| panic!("failed rasterizing SVG {svg:?}: {e}"));
        assert_eq!(img.rgba.len(), 24 * 24 * 4);
        assert!(
            img.rgba.chunks_exact(4).any(|px| px[3] > 0),
            "SVG {svg:?} produced completely transparent raster"
        );
    }
    assert_eq!(rast.cache_len(), 67);

    // Cache hit does not duplicate entry
    let _ = rast.rasterize(
        SvgId::CtrlPlay,
        SvgVariant::Default,
        24,
        24,
        [255, 255, 255, 255],
    );
    assert_eq!(rast.cache_len(), 67);

    // Stateful variants render cleanly and differ from default
    let r_def = rast
        .rasterize(
            SvgId::CtrlRepeat,
            SvgVariant::Default,
            24,
            24,
            [255, 255, 255, 255],
        )
        .unwrap();
    let r_one = rast
        .rasterize(
            SvgId::CtrlRepeat,
            SvgVariant::RepeatOne,
            24,
            24,
            [255, 255, 255, 255],
        )
        .unwrap();
    assert_ne!(r_def.rgba, r_one.rgba);

    let ab_def = rast
        .rasterize(
            SvgId::CtrlAbLoop,
            SvgVariant::Default,
            24,
            24,
            [255, 255, 255, 255],
        )
        .unwrap();
    let ab_a = rast
        .rasterize(
            SvgId::CtrlAbLoop,
            SvgVariant::AbLoopA,
            24,
            24,
            [255, 255, 255, 255],
        )
        .unwrap();
    let ab_ab = rast
        .rasterize(
            SvgId::CtrlAbLoop,
            SvgVariant::AbLoopAb,
            24,
            24,
            [255, 255, 255, 255],
        )
        .unwrap();
    assert_ne!(ab_def.rgba, ab_a.rgba);
    assert_ne!(ab_a.rgba, ab_ab.rgba);
}

#[test]
fn theme_tokens_accent_derivation_light_context_flips_and_chameleon() {
    let dark = ThemeTokens::for_mode(ThemeMode::Dark);
    let oled = ThemeTokens::for_mode(ThemeMode::Oled);
    let light = ThemeTokens::for_mode(ThemeMode::Light);

    assert_eq!(dark.glass_blur_px, 20.0);
    assert_eq!(oled.glass_blur_px, 25.0);
    assert_eq!(light.glass_blur_px, 20.0);

    // Always-dark Obsidian Volume OSD in light mode
    assert!(light.volume_osd_bg[0] < 0.1 && light.volume_osd_bg[3] > 0.8);
    assert_eq!(light.volume_osd_fg, [1.0, 1.0, 1.0, 1.0]);

    // Light-mode titlebar context flip (video playback = white fg + shadow; welcome/modal = dark text)
    let tb_video = light.titlebar_colors(false, false);
    assert_eq!(tb_video.title_fg, [1.0, 1.0, 1.0, 1.0]);
    assert!(tb_video.has_text_shadow);

    let tb_welcome = light.titlebar_colors(true, false);
    assert_eq!(tb_welcome.title_fg, light.text_primary);
    assert!(!tb_welcome.has_text_shadow);

    let tb_modal = light.titlebar_colors(false, true);
    assert_eq!(tb_modal.title_fg, light.text_primary);
    assert!(!tb_modal.has_text_shadow);

    // Accent derivation matches `_lighten(hex, 15)` (+38 per channel) & `--accent-dim` `0x4D`
    let themed = dark.clone().with_accent_hex("#38bdf8");
    assert_eq!(themed.accent_rgb, [0x38, 0xbd, 0xf8]);
    assert!((themed.accent_hover[0] - (0x38 + 38) as f32 / 255.0).abs() < 1e-5);
    assert!((themed.accent_hover[1] - (0xbd + 38) as f32 / 255.0).abs() < 1e-5);
    assert!((themed.accent_hover[2] - 1.0).abs() < 1e-5);
    assert!((themed.accent_dim[3] - 0x4D as f32 / 255.0).abs() < 1e-5);

    // Chameleon skins (`settings.css:1388-1459`)
    let faulty = ChameleonTokens::resolve(WelcomeBackgroundKind::Faulty, &dark, false, false);
    assert_eq!(faulty.skin, ChameleonSkin::Faulty);
    assert!(faulty.uses_monospace_body);
    assert!(faulty.has_sweep_overlay);
    assert_eq!(faulty.panel_blur_px, Some(4.0));

    let low_dither = ChameleonTokens::resolve(WelcomeBackgroundKind::Dither, &dark, true, false);
    assert_eq!(low_dither.panel_blur_px, None);

    // Backdrop rule catalog check
    let spec = CssBackdropRule::RecordingIndicator.spec(ThemeMode::Dark);
    assert_eq!(spec.blur_px, 32.0);
    assert!((spec.saturation - 1.9).abs() < 1e-5);
    assert!(
        CssBackdropRule::TitlebarNone
            .spec(ThemeMode::Dark)
            .is_none_override
    );
}

#[test]
fn motion_profiles_all_40_keyframes_and_elastic_volume_spring() {
    assert_eq!(MotionProfile::Balanced.fast_ms(), 120.0);
    assert_eq!(MotionProfile::Reduced.fast_ms(), 90.0);
    assert_eq!(MotionProfile::Showcase.slow_ms(), 360.0);

    assert_eq!(KeyframeId::ALL.len(), 40);
    let mut companion_count = 0usize;
    for &kf in &KeyframeId::ALL {
        if kf.is_companion_auth() {
            companion_count += 1;
        }
        for t in [0.0_f32, 0.5, 1.0] {
            let s = evaluate_keyframe(kf, t, MotionProfile::Balanced);
            assert!(s.opacity.is_finite() && s.scale_x.is_finite() && s.rotate_deg.is_finite());
        }
    }
    assert_eq!(companion_count, 2);

    // Elastic volume spring drag left & right + damped snap-back settlement
    let mut spring = ElasticVolumeSpring::default();
    let vol_left = spring.handle_drag(50.0, 100.0, 180.0, false);
    assert_eq!(vol_left, 0);
    assert_eq!(spring.region, ElasticRegion::Left);
    assert!(spring.transforms(80.0).btn_volume_translate_x < 0.0);

    let vol_right = spring.handle_drag(260.0, 100.0, 180.0, false);
    assert_eq!(vol_right, 100);
    assert_eq!(spring.region, ElasticRegion::Right);
    assert!(spring.overflow > 0.0 && spring.overflow <= ElasticVolumeSpring::MAX_OVERFLOW);

    let tx = spring.transforms(80.0);
    assert!(
        tx.scale_x > 1.0 && tx.volume_value_translate_x > 0.0 && tx.time_display_translate_x > 0.0
    );

    spring.release();
    let mut steps = 0;
    while spring.step() && steps < 200 {
        steps += 1;
    }
    assert!(steps > 0 && steps < 200);
    assert_eq!(spring.overflow, 0.0);
    assert_eq!(spring.region, ElasticRegion::Middle);
}

#[test]
fn responsive_breakpoints_and_hidden_screenshot_btn() {
    let bp_wide = BreakpointState::evaluate(1280.0, 720.0, false);
    assert_eq!(bp_wide.titlebar_btn_width(), 46.0);
    assert_eq!(bp_wide.ctrl_btn_size(), 40.0);
    assert_eq!(bp_wide.volume_slider_width(), 80.0);
    assert!(bp_wide.show_secondary_toggles());
    assert!(bp_wide.show_youtube_quality(true));
    assert!(!bp_wide.show_screenshot_btn());

    let bp_1150_pl = BreakpointState::evaluate(1100.0, 720.0, true);
    assert!(bp_1150_pl.max_w_1150_playlist);
    assert_eq!(bp_1150_pl.ctrl_btn_size(), 24.0);
    assert_eq!(bp_1150_pl.volume_slider_width(), 34.0);
    assert!(!bp_1150_pl.show_eq_and_subs());
    assert!(!bp_1150_pl.show_speed_btn());
    // Playlist-open no longer suppresses repeat/shuffle/A-B loop/lock.
    assert!(bp_1150_pl.show_secondary_toggles());

    // 1151-1280px playlist-open keeps every playback button, using the compact sizing tier.
    let bp_1200_pl = BreakpointState::evaluate(1200.0, 720.0, true);
    assert!(bp_1200_pl.max_w_1280_playlist);
    assert_eq!(bp_1200_pl.ctrl_btn_size(), 30.0);
    assert_eq!(bp_1200_pl.ctrl_play_btn_size(), 32.0);
    assert!(bp_1200_pl.show_eq_and_subs());
    assert!(bp_1200_pl.show_speed_btn());
    assert!(bp_1200_pl.show_secondary_toggles());
    // The welcome screen keeps full-size controls: Electron hides `bgSettingsToggle` there,
    // so the remaining buttons already fit and must not be compacted.
    assert_eq!(bp_1200_pl.ctrl_btn_size_for(true), 40.0);
    assert_eq!(bp_1200_pl.ctrl_play_btn_size_for(true), 44.0);

    let bp_1281_pl = BreakpointState::evaluate(1281.0, 720.0, true);
    assert!(!bp_1281_pl.max_w_1280_playlist);
    assert_eq!(bp_1281_pl.ctrl_btn_size(), 40.0);

    let bp_860 = BreakpointState::evaluate(860.0, 600.0, false);
    assert!(bp_860.max_w_860 && bp_860.max_h_620);
    assert!(!bp_860.show_secondary_toggles());
    assert_eq!(bp_860.volume_slider_width(), 50.0);

    let bp_760 = BreakpointState::evaluate(760.0, 600.0, false);
    assert!(!bp_760.show_volume_value());
    assert!(!bp_760.show_audio_track());
    assert!(!bp_760.show_youtube_quality(true));

    let bp_640_pl = BreakpointState::evaluate(640.0, 360.0, true);
    assert_eq!(bp_640_pl.titlebar_btn_width(), 38.0);
    assert_eq!(bp_640_pl.playlist_width(), 320.0);
    assert!(!bp_640_pl.show_prev_next());
    assert!(!bp_640_pl.show_time_display());

    // Confirm #btnScreenshot is never emitted in playback controls frame
    let ui = HybridUi::new();
    let state = UiState {
        welcome_active: false,
        playing: true,
        ..Default::default()
    };
    let frame = ui.build_frame(&state);
    assert!(!frame.svg_quads.iter().any(|q| q.id == "btnScreenshot"));
}

#[test]
fn playlist_virtualization_threshold_and_layout_window() {
    let make_items = |n: usize| -> Vec<PlaylistItemEntry> {
        (0..n)
            .map(|i| PlaylistItemEntry {
                path: format!("C:/video/{i}.mp4"),
                name: format!("Track {i}"),
            })
            .collect()
    };

    let pl80 = PlaylistUiState {
        open: true,
        items: make_items(PLAYLIST_VIRTUALIZE_THRESHOLD),
        ..Default::default()
    };
    let s80 = pl80.virtual_window(400.0);
    assert!(!s80.is_virtualized);
    assert_eq!((s80.start_index, s80.end_index), (0, 80));

    let pl1000 = PlaylistUiState {
        open: true,
        items: make_items(1000),
        scroll_top_px: 40.0 * PLAYLIST_ITEM_HEIGHT,
        ..Default::default()
    };
    let s1000 = pl1000.virtual_window(400.0);
    assert!(s1000.is_virtualized);
    assert_eq!(s1000.total_height_px, 1000.0 * PLAYLIST_ITEM_HEIGHT);
    assert_eq!(s1000.start_index, 40 - PLAYLIST_OVERSCAN_ROWS);
    assert_eq!(s1000.end_index, 50 + PLAYLIST_OVERSCAN_ROWS);

    // Frame builder only emits visible virtualized rows
    let ui = HybridUi::new();
    let state = UiState {
        playlist: pl1000,
        ..Default::default()
    };
    let frame = ui.build_frame(&state);
    let emitted_rows = frame
        .hit_regions
        .iter()
        .filter(|h| matches!(h.target, HitTarget::PlaylistRow(_)))
        .count();
    assert!(emitted_rows < 40 && emitted_rows > 0);
}

#[test]
fn taffy_layouts_across_three_viewports_osds_bezel_carousel_and_7_modals() {
    let ui = HybridUi::new();

    // 1. Welcome & Playback across 1280x720, 900x600, 640x360
    for (vw, vh) in [(1280.0_f32, 720.0), (900.0, 600.0), (640.0, 360.0)] {
        let mut s = UiState {
            viewport_w: vw,
            viewport_h: vh,
            recent_files: vec![HistoryEntry {
                path: "C:/a.mp4".into(),
                name: "Demo.mp4".into(),
                duration: 90.0,
                timestamp: 10,
            }],
            ..Default::default()
        };
        let wf = ui.build_frame(&s);
        assert!(wf
            .hit_regions
            .iter()
            .any(|h| h.target == HitTarget::OpenFile));
        for h in &wf.hit_regions {
            assert!(h.rect.width > 0.0 && h.rect.height > 0.0);
            assert!(h.rect.x >= 0.0 && h.rect.x + h.rect.width <= vw + 1.0);
        }

        s.welcome_active = false;
        s.playing = true;
        let pf = ui.build_frame(&s);
        assert!(pf
            .hit_regions
            .iter()
            .any(|h| h.target == HitTarget::PlayPause));
        assert!(pf
            .hit_regions
            .iter()
            .any(|h| h.target == HitTarget::ProgressSeek));
    }

    // 2. Recording burst pill (top: 48, z: 54) vs Clip flush bezel tab (top: 0, h: 28, z: 2000, stop z: 2501)
    let mut rs = UiState {
        welcome_active: false,
        recording_indicator: RecordingIndicatorMode::BurstActive { count: 4 },
        ..Default::default()
    };
    let burst_f = ui.build_frame(&rs);
    let burst_glass = burst_f
        .glass_panels
        .iter()
        .find(|g| g.id == "recording-burst-pill")
        .unwrap();
    assert_eq!(burst_glass.z_index, ZIndex::RECORDING_BURST_PILL);
    assert_eq!(burst_glass.glass.y, 48.0);

    rs.recording_indicator = RecordingIndicatorMode::ClipActive { elapsed_ds: 35 };
    let clip_f = ui.build_frame(&rs);
    let clip_glass = clip_f
        .glass_panels
        .iter()
        .find(|g| g.id == "recording-clip-bezel-tab")
        .unwrap();
    assert_eq!(clip_glass.z_index, ZIndex::MODAL_OVERLAY);
    assert_eq!(clip_glass.glass.y, 0.0);
    assert_eq!(clip_glass.glass.height, 28.0);
    let stop_hit = clip_f
        .hit_regions
        .iter()
        .find(|h| h.target == HitTarget::RecordingStopClip)
        .unwrap();
    assert_eq!(stop_hit.z_index, ZIndex::RECORDING_STOP_BTN);
    assert_eq!(stop_hit.rect.width, 32.0); // 20x20 expanded by 6px each side

    // 3. Screenshot preview vs Delete/Restore Carousel + Notice
    rs.carousel_mode = CarouselMode::DeleteCarousel;
    rs.screenshot_notice = Some("Saved".into());
    rs.toasts.push(ToastItem {
        id: 1,
        kind: ToastKind::UpdateAvailable,
        message: "yt-dlp update".into(),
        version_tag: Some("2026.1".into()),
        downloading: false,
        ..Default::default()
    });
    let cf = ui.build_frame(&rs);
    assert!(cf
        .glass_panels
        .iter()
        .any(|g| g.id == "screenshot-carousel" && g.z_index == ZIndex::SCREENSHOT_CAROUSEL));
    assert!(cf
        .solid_rects
        .iter()
        .any(|r| r.id == "screenshot-notice" && r.z_index == ZIndex::SCREENSHOT_NOTICE));

    // 4. All 7 Modals + #bgAccordionPopover + 27 welcome controls + 52 shortcuts + AccessKit modal inertness
    assert_eq!(BG_CONTROLS_CATALOG.len(), 27);
    assert_eq!(SHORTCUT_CATALOG.len(), 52);
    assert_eq!(VALID_SCREENSHOT_MOUSE_BUTTONS, [-1, 1, 3, 4]);

    let mut ms = UiState {
        welcome_active: true,
        ..Default::default()
    };
    for modal in [
        ModalId::Equalizer,
        ModalId::Speed,
        ModalId::Subtitles,
        ModalId::Audio,
        ModalId::Settings,
        ModalId::NetworkStream,
        ModalId::BgSettings,
    ] {
        ms.open_modal(modal);
        if modal == ModalId::BgSettings {
            ms.bg_accordion_open = true;
        }
        let mf = ui.build_frame(&ms);
        assert!(mf
            .hit_regions
            .iter()
            .any(|h| h.target == HitTarget::ModalClose));

        // AccessKit modal inertness check: background controls excluded when modal is open
        assert_eq!(mf.a11y_tree.focus, A11Y_MODAL_ID);
        let root_node = mf
            .a11y_tree
            .nodes
            .iter()
            .find(|(id, _)| *id == A11Y_ROOT_ID)
            .unwrap();
        assert_eq!(root_node.1.children(), &[A11Y_MODAL_ID]);

        if modal == ModalId::Equalizer {
            let eq_sliders = mf
                .hit_regions
                .iter()
                .filter(|h| matches!(h.target, HitTarget::EqBandSlider(_)))
                .count();
            assert_eq!(eq_sliders, EQ_BAND_COUNT);
            ms.set_eq_preset(EqualizerPreset::Rock);
            assert_ne!(ms.equalizer.bands, [0.0; EQ_BAND_COUNT]);
        }

        if modal == ModalId::Settings {
            assert_eq!(ms.filtered_shortcuts().len(), 52);
            ms.shortcut_search_query = "fullscreen".into();
            assert!(!ms.filtered_shortcuts().is_empty());
            ms.shortcut_search_query.clear();
        }

        if modal == ModalId::BgSettings {
            // Welcome content hidden while BgSettings drawer is open (main.css:2983-2986)
            assert!(!mf
                .hit_regions
                .iter()
                .any(|h| h.target == HitTarget::OpenFile));
            // All 4 effect cards in #bgAccordionPopover emit visible previews before hover (settings.js:2569)
            let previews: Vec<_> = mf.shader_slots.iter().filter(|s| s.is_preview).collect();
            assert_eq!(previews.len(), 4);
            assert!(previews
                .iter()
                .all(|p| p.rect.width >= 400.0 && p.rect.height >= 240.0));
        }
    }

    // Check top-most z-index hit-test resolution
    let mut hz = UiState {
        welcome_active: true,
        ..Default::default()
    };
    hz.open_modal(ModalId::Speed);
    let hf = ui.build_frame(&hz);
    let corner_hit = ui.hit_test(&hf, 10.0, 10.0).unwrap();
    assert_eq!(corner_hit.z_index, ZIndex::MODAL_OVERLAY);
    let slider_hit = ui.hit_test(&hf, 640.0, 350.0).unwrap();
    assert_eq!(slider_hit.z_index, ZIndex::MODAL_OVERLAY + 1);

    // Ensure every BackdropRule / ZIndex layer is sane
    let unique_keys: HashSet<&'static str> = BG_CONTROLS_CATALOG.iter().map(|c| c.id).collect();
    assert_eq!(unique_keys.len(), 27);
}

#[test]
fn golden_parity_welcome_controls_modals_playlist_and_overlays() {
    let ui = HybridUi::new();

    // 1. Welcome screen shows bottom control bar (welcome button subset), bottom-aligned actions, in-flow #recentFiles container, transparent titlebar
    let ws = UiState::default();
    let wf = ui.build_frame(&ws);
    assert!(!wf.solid_rects.iter().any(|r| r.id == "titlebar-bg"));
    assert!(!wf.svg_quads.iter().any(|q| q.id == "welcome-logo"));
    assert!(!wf.text_runs.iter().any(|t| t.id == "welcome-title"));
    assert!(wf.glass_panels.iter().any(|g| g.id == "welcome-recent-bg"));
    assert!(!wf.text_runs.iter().any(|t| t.id == "recent-header"));
    let mut ws_recent = UiState::default();
    ws_recent.recent_files.push(HistoryEntry {
        path: "C:/clip.mp4".into(),
        name: "clip.mp4".into(),
        duration: 120.0,
        timestamp: 1,
    });
    let wf_recent = ui.build_frame(&ws_recent);
    assert!(wf_recent
        .glass_panels
        .iter()
        .any(|g| g.id == "welcome-recent-bg"));
    assert!(wf_recent.text_runs.iter().any(|t| t.id == "recent-header"));
    assert!(wf
        .hit_regions
        .iter()
        .any(|h| h.target == HitTarget::OpenFile));
    assert!(wf
        .hit_regions
        .iter()
        .any(|h| h.target == HitTarget::OpenLink));
    // Welcome control bar visible with welcome-only right buttons
    assert!(wf
        .hit_regions
        .iter()
        .any(|h| h.target == HitTarget::PlayPause));
    assert!(wf
        .hit_regions
        .iter()
        .any(|h| h.target == HitTarget::BgSettingsToggle));
    assert!(wf
        .hit_regions
        .iter()
        .any(|h| h.target == HitTarget::PlaylistToggle));
    assert!(wf
        .hit_regions
        .iter()
        .any(|h| h.target == HitTarget::SettingsButton));
    assert!(wf
        .hit_regions
        .iter()
        .any(|h| h.target == HitTarget::FullscreenToggle));
    assert!(!wf
        .hit_regions
        .iter()
        .any(|h| h.target == HitTarget::SpeedButton));
    assert!(!wf
        .hit_regions
        .iter()
        .any(|h| h.target == HitTarget::EqualizerButton));
    assert!(!wf
        .hit_regions
        .iter()
        .any(|h| h.target == HitTarget::LockToggle));

    // At 640x360 welcome, BgSettingsToggle is hidden per controls.css:714
    let ws_narrow = UiState {
        viewport_w: 640.0,
        viewport_h: 360.0,
        ..Default::default()
    };
    let wfn = ui.build_frame(&ws_narrow);
    assert!(!wfn
        .hit_regions
        .iter()
        .any(|h| h.target == HitTarget::BgSettingsToggle));
    assert!(wfn
        .hit_regions
        .iter()
        .any(|h| h.target == HitTarget::PlaylistToggle));

    // 2. Settings modal opens at scroll_y = 0 showing Appearance / Behavior / Screenshots (NOT shortcuts)
    let mut ss = UiState::default();
    ss.open_modal(ModalId::Settings);
    let sf = ui.build_frame(&ss);
    assert!(sf.text_runs.iter().any(|t| t.text == "APPEARANCE"));
    assert!(sf.text_runs.iter().any(|t| t.text == "BEHAVIOR"));
    assert!(sf.text_runs.iter().any(|t| t.text == "SCREENSHOTS"));
    assert!(!sf
        .text_runs
        .iter()
        .any(|t| t.id.starts_with("shortcut-label-")));
    assert!(sf
        .hit_regions
        .iter()
        .any(|h| h.target == HitTarget::SettingsThemeSelect));

    // Searching in Settings keeps all settings cards visible and filters shortcuts
    ss.shortcut_search_query = "fullscreen".into();
    ss.settings_scroll_y = 740.0;
    let sf_search = ui.build_frame(&ss);
    assert!(sf_search
        .text_runs
        .iter()
        .any(|t| t.id.starts_with("shortcut-label-")));

    // 3. Equalizer modal renders title, preset, 10 band labels + Reset
    let mut eqs = UiState::default();
    eqs.open_modal(ModalId::Equalizer);
    let eqf = ui.build_frame(&eqs);
    assert!(eqf.text_runs.iter().any(|t| t.text == "Audio Equalizer"));
    assert!(eqf.text_runs.iter().any(|t| t.text == "31Hz"));
    assert!(eqf.text_runs.iter().any(|t| t.text == "16K"));
    assert!(eqf.text_runs.iter().any(|t| t.text == "Reset"));

    // 4. Empty Playlist shows "Playlist" header & "0 items" footer
    let mut pls = UiState::default();
    pls.playlist.open = true;
    let plf = ui.build_frame(&pls);
    assert!(plf
        .text_runs
        .iter()
        .any(|t| t.id == "playlist-header" && t.text == "Playlist"));
    assert!(plf
        .text_runs
        .iter()
        .any(|t| t.id == "playlist-footer-count" && t.text == "0 items"));

    // 5. Overlays: Burst pill, Clip bezel tab bottom-only corners + timer, compact top-left carousel
    let mut ov = UiState {
        recording_indicator: RecordingIndicatorMode::BurstActive { count: 1 },
        carousel_mode: CarouselMode::DeleteCarousel,
        carousel_index: 1,
        carousel_total: 2,
        ..Default::default()
    };
    let ovf = ui.build_frame(&ov);
    assert!(ovf.text_runs.iter().any(|t| t.text == "Recording"));
    assert!(ovf
        .text_runs
        .iter()
        .any(|t| t.id == "recording-burst-count" && t.text == "1"));
    assert!(ovf
        .text_runs
        .iter()
        .any(|t| t.id == "recording-burst-unit" && t.text == "frame"));
    let car = ovf
        .glass_panels
        .iter()
        .find(|g| g.id == "screenshot-carousel")
        .unwrap();
    assert_eq!(
        (car.glass.x, car.glass.y, car.glass.width),
        (24.0, 86.0, 260.0)
    );
    assert!(ovf
        .text_runs
        .iter()
        .any(|t| t.id == "carousel-badge" && t.text == "2 / 2"));

    ov.recording_indicator = RecordingIndicatorMode::ClipActive { elapsed_ds: 35 };
    let clipf = ui.build_frame(&ov);
    let bezel_bg = clipf
        .solid_rects
        .iter()
        .find(|r| r.id == "recording-clip-bezel-bg")
        .unwrap();
    assert_eq!(bezel_bg.corner_radii, [0.0, 0.0, 10.0, 10.0]);
    assert!(clipf
        .text_runs
        .iter()
        .any(|t| t.id == "recording-clip-timer" && t.text == "00:03.5"));

    // 6. Polish parity (#80): white volume fill + circular thumb, "1x" speed button label, bottom-center toast, 0.85 modal backdrop
    let mut pb = UiState {
        welcome_active: false,
        playing: true,
        ..Default::default()
    };
    pb.toasts.push(ToastItem {
        id: 7,
        kind: ToastKind::Info,
        message: "Loaded 1 file(s)".into(),
        version_tag: None,
        downloading: false,
        ..Default::default()
    });
    let pbf = ui.build_frame(&pb);
    let vol_fill = pbf
        .solid_rects
        .iter()
        .find(|r| r.id == "volume-slider-fill")
        .unwrap();
    assert_eq!(vol_fill.color, [1.0, 1.0, 1.0, 1.0]);
    assert!(pbf
        .solid_rects
        .iter()
        .any(|r| r.id == "volume-slider-thumb"));
    assert!(pbf
        .text_runs
        .iter()
        .any(|t| t.id == "btnSpeed-label" && t.text == "1x"));
    let toast_r = pbf.solid_rects.iter().find(|r| r.id == "toast-7").unwrap();
    assert!(((toast_r.rect.x + toast_r.rect.width * 0.5) - 640.0).abs() < 1.0);
    assert!((toast_r.rect.y + toast_r.rect.height - (720.0 - 80.0)).abs() < 1.0);
    assert!(!pbf.svg_quads.iter().any(|q| q.id == "toast-close-7"));

    let eq_bd = eqf
        .solid_rects
        .iter()
        .find(|r| r.id == "modal-backdrop")
        .unwrap();
    assert!((eq_bd.color[3] - 0.85).abs() < 1e-4);
}

#[test]
fn visual_parity_tasks_1_to_8() {
    let ui = HybridUi::new();

    // Task 1: Titlebar 12px text, text-shadow in Dark/OLED, crisp 20x20 BrandMarkTitlebar
    let mut s = UiState::default();
    let f_dark = ui.build_frame(&s);
    let title = f_dark
        .text_runs
        .iter()
        .find(|t| t.id == "titlebar-title")
        .unwrap();
    assert_eq!(title.font_size_px, 12.0);
    assert!(f_dark
        .text_runs
        .iter()
        .any(|t| t.id == "titlebar-title-shadow"));
    let mark = f_dark
        .svg_quads
        .iter()
        .find(|q| q.id == "titlebar-brand-mark")
        .unwrap();
    assert_eq!((mark.rect.width, mark.rect.height), (20.0, 20.0));

    // Task 2: #btnOpenLink always navy gradient even in Light theme
    s.theme = ThemeMode::Light;
    let f_light = ui.build_frame(&s);
    let open_link = f_light
        .solid_rects
        .iter()
        .find(|r| r.id == "btnOpenLink-bg")
        .unwrap();
    assert!(open_link.gradient.is_some());
    assert!((open_link.color[2] - (53.0 / 255.0)).abs() < 0.02);
    assert!((open_link.border_color[0] - (144.0 / 255.0)).abs() < 0.02);

    // Task 3: Playback controls gradient backdrop and exact SVG sizes
    let mut pb = UiState {
        welcome_active: false,
        playing: false,
        subtitle_tracks: vec![TrackOption {
            id: 1,
            label: "English".into(),
            lang: "en".into(),
            selected: true,
            external: false,
        }],
        ..Default::default()
    };
    let pbf = ui.build_frame(&pb);
    let backdrop_top = pbf
        .solid_rects
        .iter()
        .find(|r| r.id == "controls-bar-backdrop-top" && r.gradient.is_some())
        .expect("controls-bar-backdrop-top gradient");
    let backdrop_bot = pbf
        .solid_rects
        .iter()
        .find(|r| r.id == "controls-bar-backdrop" && r.gradient.is_some())
        .expect("controls-bar-backdrop gradient");
    // Regression: self-clipping each gradient half feathers the shared edge and draws a 1px
    // bright seam over the video. The quad bounds already clip the gradient.
    assert!(backdrop_top.clip_rect.is_none());
    assert!(backdrop_bot.clip_rect.is_none());
    let play_svg = pbf.svg_quads.iter().find(|q| q.id == "btnPlay").unwrap();
    assert_eq!((play_svg.rect.width, play_svg.rect.height), (32.0, 32.0));
    let rep_svg = pbf.svg_quads.iter().find(|q| q.id == "btnRepeat").unwrap();
    assert_eq!((rep_svg.rect.width, rep_svg.rect.height), (24.0, 12.0));
    let pl_svg = pbf
        .svg_quads
        .iter()
        .find(|q| q.id == "btnPlaylist")
        .unwrap();
    assert_eq!((pl_svg.rect.width, pl_svg.rect.height), (20.0, 20.0));

    // Regression: playlist-open must keep the playback secondary toggles mounted.
    pb.playlist.open = true;
    let pbf_pl = ui.build_frame(&pb);
    for visible in [
        HitTarget::RepeatToggle,
        HitTarget::ShuffleToggle,
        HitTarget::AbLoopToggle,
        HitTarget::LockToggle,
    ] {
        assert!(pbf_pl.hit_regions.iter().any(|h| h.target == visible));
    }
    // BgSettingsToggle remains playback-only, so it is absent here regardless of the playlist.
    assert!(!pbf_pl
        .hit_regions
        .iter()
        .any(|h| h.target == HitTarget::BgSettingsToggle));

    // The secondary toggles still collapse at <=860px where the control bar cannot fit them.
    pb.viewport_w = 860.0;
    pb.viewport_h = 600.0;
    let pbf_narrow = ui.build_frame(&pb);
    for hidden in [
        HitTarget::RepeatToggle,
        HitTarget::ShuffleToggle,
        HitTarget::AbLoopToggle,
        HitTarget::LockToggle,
    ] {
        assert!(!pbf_narrow.hit_regions.iter().any(|h| h.target == hidden));
    }

    // 1200px playlist-open: all playback buttons stay mounted and the bar ends before the
    // 320px sidebar (no clipped gear/fullscreen under the queue).
    pb.viewport_w = 1200.0;
    pb.viewport_h = 720.0;
    let pbf_tight = ui.build_frame(&pb);
    for visible in [
        HitTarget::SpeedButton,
        HitTarget::EqualizerButton,
        HitTarget::AudioTrackButton,
        HitTarget::SubtitlesButton,
        HitTarget::RepeatToggle,
        HitTarget::ShuffleToggle,
        HitTarget::AbLoopToggle,
        HitTarget::LockToggle,
        HitTarget::SettingsButton,
        HitTarget::FullscreenToggle,
    ] {
        assert!(pbf_tight.hit_regions.iter().any(|h| h.target == visible));
    }
    let fullscreen = pbf_tight
        .hit_regions
        .iter()
        .find(|h| h.target == HitTarget::FullscreenToggle)
        .unwrap();
    assert!(
        fullscreen.rect.x + fullscreen.rect.width <= 1200.0 - 320.0 + 0.5,
        "fullscreen button clipped by playlist: {:?}",
        fullscreen.rect
    );

    // Task 4: Populated overlays
    let mut ov = UiState {
        welcome_active: false,
        loading_visible: true,
        ab_loop: AbLoopState::Active(12_000, 45_000),
        recording_indicator: RecordingIndicatorMode::BurstActive { count: 3 },
        ..Default::default()
    };
    ov.skip_indicator.visible = true;
    ov.skip_indicator.forward = true;
    ov.skip_indicator.accumulated_secs = 10;
    ov.stats_overlay.visible = true;
    ov.stats_overlay.resolution = "1920x1080".into();
    ov.duration_secs = 3600.0;
    ov.hover_thumb_norm = Some(0.5004);
    ov.hover_thumb_has_frame = true;
    let ovf = ui.build_frame(&ov);
    assert!(ovf
        .solid_rects
        .iter()
        .any(|r| r.id == "loading-spinner-dot-0"));
    assert!(ovf
        .text_runs
        .iter()
        .any(|t| t.id == "skip-indicator-label" && t.text == "+10s"));
    assert_eq!(
        ovf.text_runs
            .iter()
            .filter(|t| t.id.starts_with("stats-row-key-"))
            .count(),
        7
    );
    assert!(ovf
        .text_runs
        .iter()
        .any(|t| t.id == "ab-loop-indicator-label" && t.text == "A-B: 0:12 - 0:45"));
    assert!(ovf
        .image_slots
        .iter()
        .any(|s| s.key == "hover-thumb:0.500400"));
    assert!(ovf
        .text_runs
        .iter()
        .any(|t| t.id == "progress-hover-thumb-time" && t.text == "30:01"));
    let burst = ovf
        .glass_panels
        .iter()
        .find(|g| g.id == "recording-burst-pill")
        .unwrap();
    assert_eq!(burst.glass.height, 30.0);

    // Task 5: Opaque playlist sidebar + flat 39px active row with left accent bar
    pb.playlist.items.push(PlaylistItemEntry {
        name: "Episode 1.mkv".into(),
        path: "ep1.mkv".into(),
    });
    pb.playlist.current_index = Some(0);
    let plf = ui.build_frame(&pb);
    let sidebar = plf
        .solid_rects
        .iter()
        .find(|r| r.id == "playlist-sidebar")
        .unwrap();
    assert_eq!(sidebar.rect.width, 320.0);
    let row = plf
        .solid_rects
        .iter()
        .find(|r| r.id == "playlist-row-0")
        .unwrap();
    assert_eq!(row.rect.height, 39.0);
    assert_eq!(row.corner_radii, [0.0; 4]);
    assert!(plf
        .solid_rects
        .iter()
        .any(|r| r.id == "playlist-row-accent-0" && r.rect.width == 3.0));

    // Task 6: BgSettings has no .bg-quality-pills & Accordion uses 1/2.0/3.8 flex + close + 22px check circle
    let mut bgs = UiState {
        bg_accordion_open: true,
        ..Default::default()
    };
    bgs.open_modal(ModalId::BgSettings);
    bgs.bg_accordion_open = true;
    let bgf = ui.build_frame(&bgs);
    assert!(!bgf
        .hit_regions
        .iter()
        .any(|h| matches!(h.target, HitTarget::BgQualityRadio(_))));
    assert!(bgf.svg_quads.iter().any(|q| q.id == "bg-pop-close-svg"));
    let check_bg = bgf
        .solid_rects
        .iter()
        .find(|r| r.id == "bg-check-badge-Dither")
        .unwrap();
    assert_eq!((check_bg.rect.width, check_bg.rect.height), (22.0, 22.0));
    let dither_card = bgf
        .solid_rects
        .iter()
        .find(|r| r.id == "bg-card-frame-Dither")
        .unwrap();
    let particles_card = bgf
        .solid_rects
        .iter()
        .find(|r| r.id == "bg-card-frame-Particles")
        .unwrap();
    assert!(dither_card.rect.width > particles_card.rect.width * 3.0);

    // Task 7: Speed all 10 presets distinct, Subtitles 4 appearance rows, Audio 0ms translucent, Settings no dividers & Player Actions
    let mut sp = UiState::default();
    sp.open_modal(ModalId::Speed);
    let spf = ui.build_frame(&sp);
    for idx in 0..10 {
        assert!(
            spf.hit_regions
                .iter()
                .any(|h| h.target == HitTarget::SpeedPreset(idx)),
            "missing SpeedPreset({idx})"
        );
    }

    let mut sub = UiState::default();
    sub.open_modal(ModalId::Subtitles);
    let subf = ui.build_frame(&sub);
    assert!(subf
        .hit_regions
        .iter()
        .any(|h| h.target == HitTarget::SubtitleFontSizeSlider));
    assert!(subf
        .hit_regions
        .iter()
        .any(|h| h.target == HitTarget::SubtitleFontColorPicker));
    assert!(subf
        .hit_regions
        .iter()
        .any(|h| h.target == HitTarget::SubtitleBgColorPicker));
    assert!(subf
        .hit_regions
        .iter()
        .any(|h| h.target == HitTarget::SubtitleBgOpacitySlider));

    let mut aud = UiState::default();
    aud.open_modal(ModalId::Audio);
    let audf = ui.build_frame(&aud);
    let zero_ms = audf
        .solid_rects
        .iter()
        .find(|r| r.id == "audio-sync-btn-1")
        .unwrap();
    assert!((zero_ms.color[3] - 0.30).abs() < 0.02);
    assert!(zero_ms.color[0] > 0.8 && zero_ms.color[2] < 0.15);

    let mut set = UiState::default();
    set.open_modal(ModalId::Settings);
    let setf0 = ui.build_frame(&set);
    assert!(!setf0
        .solid_rects
        .iter()
        .any(|r| r.id.starts_with("settings-beh-div-") || r.id == "settings-accent-divider"));
    set.settings_scroll_y = 2200.0;
    let setf_bot = ui.build_frame(&set);
    assert!(setf_bot
        .text_runs
        .iter()
        .any(|t| t.text == "PLAYER ACTIONS"));
    assert!(setf_bot
        .hit_regions
        .iter()
        .any(|h| h.target == HitTarget::SettingsQuit));

    // Task 8: Medium preset defaults + animation stepper settles cleanly
    let mut anim_s = UiState::default();
    assert_eq!(anim_s.welcome_quality, QualityTier::Medium);
    assert_eq!(anim_s.dither_cfg.pixel_size, 3.0);
    assert_eq!(anim_s.particles_cfg.particle_count, 210);
    anim_s.anim.trigger_toast_in();
    assert!(anim_s.step_animations(0.05));
    for _ in 0..40 {
        anim_s.step_animations(0.05);
    }
    assert!(!anim_s.step_animations(0.05));
}

#[test]
fn workstream_a_hit_surfaces_focus_only_search_focus_rings_and_delayed_tooltips() {
    let ui = HybridUi::new();
    let mut s = UiState {
        viewport_w: 1280.0,
        viewport_h: 720.0,
        ..Default::default()
    };
    s.open_modal(ModalId::Settings);
    s.anim.modal_t = 1.0;

    let modal_frame = ui.build_frame(&s);
    let panel_region = modal_frame
        .hit_regions
        .iter()
        .find(|r| r.target == HitTarget::ModalPanelSurface)
        .expect("ModalPanelSurface missing");
    // Clicking interior padding of modal must hit ModalPanelSurface, not ModalBackdrop
    assert_eq!(
        modal_frame
            .hit_test(
                panel_region.rect.x + 12.0,
                panel_region.rect.y + panel_region.rect.height - 12.0
            )
            .map(|r| &r.target),
        Some(&HitTarget::ModalPanelSurface)
    );
    // Clicking outside panel hits ModalBackdrop
    assert_eq!(
        modal_frame.hit_test(10.0, 100.0).map(|r| &r.target),
        Some(&HitTarget::ModalBackdrop)
    );

    // BgSettings + #bgAccordionPopover surfaces
    s.open_modal(ModalId::BgSettings);
    s.bg_accordion_open = true;
    s.anim.modal_t = 1.0;
    let bg_frame = ui.build_frame(&s);
    assert!(bg_frame
        .hit_regions
        .iter()
        .any(|r| r.target == HitTarget::BgAccordionSurface));

    // Welcome + Playlist surfaces & PlaylistSearch focus-only border/caret
    s.close_modals();
    s.welcome_active = false;
    s.playlist.open = true;
    s.anim.playlist_t = 1.0;
    s.hovered_target = Some(HitTarget::PlaylistSearch);
    s.focused_text_field = None;
    let pl_hover_only = ui.build_frame(&s);
    assert!(pl_hover_only
        .hit_regions
        .iter()
        .any(|r| r.target == HitTarget::ControlsBarSurface));
    assert!(pl_hover_only
        .hit_regions
        .iter()
        .any(|r| r.target == HitTarget::PlaylistSurface));
    assert!(pl_hover_only
        .hit_regions
        .iter()
        .any(|r| r.target == HitTarget::VolumeContainerSurface));
    assert!(!pl_hover_only
        .solid_rects
        .iter()
        .any(|r| r.id == "playlist-search-caret"));
    let tokens = ThemeTokens::for_mode(s.theme);
    let box_hover = pl_hover_only
        .solid_rects
        .iter()
        .find(|r| r.id == "playlist-search-box")
        .unwrap();
    assert_ne!(box_hover.border_color, tokens.accent);

    s.focused_text_field = Some(FocusedTextField::PlaylistSearch);
    let pl_focused = ui.build_frame(&s);
    assert!(pl_focused
        .solid_rects
        .iter()
        .any(|r| r.id == "playlist-search-caret"));
    let box_focused = pl_focused
        .solid_rects
        .iter()
        .find(|r| r.id == "playlist-search-box")
        .unwrap();
    assert_eq!(box_focused.border_color, tokens.accent);

    // Focus-visible ring omits ModalClose, renders on PlayPause when focus_visible = true
    s.focused_target = Some(HitTarget::PlayPause);
    s.focus_visible = true;
    let focus_frame = ui.build_frame(&s);
    assert!(focus_frame
        .solid_rects
        .iter()
        .any(|r| r.id == "focus-visible-ring"));
    s.focus_visible = false;
    let no_focus_frame = ui.build_frame(&s);
    assert!(!no_focus_frame
        .solid_rects
        .iter()
        .any(|r| r.id == "focus-visible-ring"));

    // Delayed hover tooltip
    s.hovered_target = Some(HitTarget::PlayPause);
    s.hover_tooltip_elapsed_secs = HOVER_TOOLTIP_DELAY_SECS - 0.1;
    let early_tip = ui.build_frame(&s);
    assert!(!early_tip
        .solid_rects
        .iter()
        .any(|r| r.id == "hover-tooltip-bg"));
    s.hover_tooltip_elapsed_secs = HOVER_TOOLTIP_DELAY_SECS + 0.05;
    let shown_tip = ui.build_frame(&s);
    assert!(shown_tip
        .solid_rects
        .iter()
        .any(|r| r.id == "hover-tooltip-bg"));
}

#[test]
fn workstream_b_titlebar_live_time_chapters_thumb_osd_eq_and_tracks() {
    use hybrid_ui::motion::UiAnimationState;
    use hybrid_ui::state::{DropdownKind, WindowMaximizeState};

    let ui = HybridUi::new();

    // B.1: Titlebar hidden when locked; kept visible when playlist.open; slides up in fullscreen auto-hide
    let mut s = UiState {
        viewport_w: 1280.0,
        viewport_h: 720.0,
        welcome_active: false,
        window_title: "Stream Title — Creator — Hybrid Player".to_string(),
        locked: true,
        auto_hide_controls: true,
        anim: UiAnimationState {
            controls_t: 0.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let locked_frame = ui.build_frame(&s);
    assert!(!locked_frame
        .text_runs
        .iter()
        .any(|t| t.id == "titlebar-title"));
    assert!(!locked_frame
        .hit_regions
        .iter()
        .any(|r| r.target == HitTarget::TitlebarClose));

    s.locked = false;
    s.playlist.open = true;
    s.anim.playlist_t = 1.0;
    let pl_open_frame = ui.build_frame(&s);
    assert!(pl_open_frame
        .text_runs
        .iter()
        .any(|t| t.id == "titlebar-title"));

    s.playlist.open = false;
    s.anim.playlist_t = 0.0;
    s.window_maximize_state = WindowMaximizeState::Fullscreen;
    s.anim.controls_t = 0.0;
    let fs_hidden_frame = ui.build_frame(&s);
    assert!(!fs_hidden_frame
        .text_runs
        .iter()
        .any(|t| t.id == "titlebar-title"));

    // B.2: Inline LIVE badge right after title text + #btnYoutubeQuality text label + LIVE duration
    s.auto_hide_controls = false;
    s.anim.controls_t = 1.0;
    s.is_youtube = true;
    s.is_live_stream = true;
    s.youtube_quality_label = "1080p".to_string();
    s.youtube_quality_options = vec!["Auto".to_string(), "1080p".to_string()];
    let live_frame = ui.build_frame(&s);
    let title_txt = live_frame
        .text_runs
        .iter()
        .find(|t| t.id == "titlebar-title")
        .expect("titlebar-title");
    let live_badge = live_frame
        .solid_rects
        .iter()
        .find(|r| r.id == "titlebar-live-badge")
        .expect("titlebar-live-badge");
    assert!(
        live_badge.rect.x >= title_txt.rect.x + title_txt.rect.width - 2.0
            && live_badge.rect.x < title_txt.rect.x + title_txt.rect.width + 48.0,
        "live badge should sit inline right after title text"
    );
    let yt_btn_lbl = live_frame
        .text_runs
        .iter()
        .find(|t| t.id == "btnYoutubeQuality-label")
        .expect("btnYoutubeQuality-label");
    assert_eq!(yt_btn_lbl.text, "1080p");
    let right_time = live_frame
        .text_runs
        .iter()
        .find(|t| t.id == "time-total")
        .expect("time-total");
    assert_eq!(right_time.text, "LIVE");

    // B.3: time_remaining_mode keeps elapsed on left (#currentTime) and -remaining on right (#totalTime)
    s.is_live_stream = false;
    s.duration_secs = 120.0;
    s.current_secs = 30.0;
    s.time_remaining_mode = true;
    let rem_frame = ui.build_frame(&s);
    let left_t = rem_frame
        .text_runs
        .iter()
        .find(|t| t.id == "time-display")
        .expect("time-display");
    let right_t = rem_frame
        .text_runs
        .iter()
        .find(|t| t.id == "time-total")
        .expect("time-total");
    assert_eq!(left_t.text, "0:30");
    assert_eq!(right_t.text, "-1:30");
    assert!(rem_frame
        .hit_regions
        .iter()
        .any(|r| r.target == HitTarget::TimeRemainingToggle));

    // B.4: Interactive ChapterMarker + hover thumb gated on hover_thumb_has_frame & is-loading shimmer
    s.chapter_positions_norm = vec![0.25, 0.75];
    s.chapter_times_secs = vec![30.0, 90.0];
    s.chapter_titles = vec!["Intro".to_string(), "Climax".to_string()];
    s.hovered_target = Some(HitTarget::ChapterMarker(1));
    s.hover_tooltip_elapsed_secs = HOVER_TOOLTIP_DELAY_SECS + 0.1;
    s.hover_thumb_norm = Some(0.5);
    s.hover_thumb_has_frame = false;
    let no_frame_thumb = ui.build_frame(&s);
    assert!(no_frame_thumb
        .hit_regions
        .iter()
        .any(|r| r.target == HitTarget::ChapterMarker(0)));
    assert!(no_frame_thumb
        .text_runs
        .iter()
        .any(|t| t.id == "hover-tooltip-text" && t.text.contains("Climax")));
    assert!(!no_frame_thumb
        .solid_rects
        .iter()
        .any(|r| r.id == "progress-hover-thumb"));

    s.hover_thumb_has_frame = true;
    s.hover_thumb_loading = true;
    let loading_thumb = ui.build_frame(&s);
    assert!(loading_thumb
        .solid_rects
        .iter()
        .any(|r| r.id == "progress-hover-thumb-dim"));
    assert!(loading_thumb
        .solid_rects
        .iter()
        .any(|r| r.id == "progress-hover-thumb-shimmer"));

    // B.5: Volume OSD compact `{val}%` layout shifted below active AB loop indicator
    s.hover_thumb_norm = None;
    s.volume_osd.visible = true;
    s.volume_osd.percent = 85;
    s.anim.volume_osd_ttl_secs = 0.9;
    s.ab_loop = AbLoopState::Active(10, 40);
    let osd_frame = ui.build_frame(&s);
    let vol_osd = osd_frame
        .solid_rects
        .iter()
        .find(|r| r.id == "top-right-volume-osd")
        .expect("top-right-volume-osd");
    let ab_ind = osd_frame
        .solid_rects
        .iter()
        .find(|r| r.id == "ab-loop-indicator")
        .expect("ab-loop-indicator");
    assert!((vol_osd.corner_radii[0] - 20.0).abs() < 0.01);
    assert!(vol_osd.rect.y >= ab_ind.rect.y + ab_ind.rect.height);
    assert_eq!(
        osd_frame
            .text_runs
            .iter()
            .find(|t| t.id == "top-right-volume-osd-val")
            .map(|t| t.text.as_str()),
        Some("85")
    );
    assert_eq!(
        osd_frame
            .text_runs
            .iter()
            .find(|t| t.id == "top-right-volume-osd-pct")
            .map(|t| t.text.as_str()),
        Some("%")
    );

    // B.6: EQ 0.5 dB formatting & Custom option in EqPreset dropdown
    s.open_modal(ModalId::Equalizer);
    s.anim.modal_t = 1.0;
    s.equalizer.bands[0] = 2.5;
    s.equalizer.bands[1] = -3.0;
    s.active_dropdown = Some(DropdownKind::EqPreset);
    let eq_frame = ui.build_frame(&s);
    assert!(eq_frame.text_runs.iter().any(|t| t.text == "+2.5 dB"));
    assert!(eq_frame.text_runs.iter().any(|t| t.text == "-3 dB"));
    assert!(eq_frame
        .hit_regions
        .iter()
        .any(|r| r.target == HitTarget::DropdownOption(EqualizerPreset::BUILTIN_PRESETS.len())));

    // B.7: Subtitle/Audio wrap >4 tracks + Off/Subs/Dubs active highlights
    s.active_dropdown = None;
    s.open_modal(ModalId::Subtitles);
    s.anim.modal_t = 1.0;
    s.subtitle_tracks = (1..=6)
        .map(|i| TrackOption {
            id: i,
            label: format!("Track {i} Long Name"),
            lang: "en".to_string(),
            selected: false,
            external: i == 6,
        })
        .collect();
    let sub_frame = ui.build_frame(&s);
    assert!(sub_frame
        .hit_regions
        .iter()
        .any(|r| r.target == HitTarget::SubtitleTrackSelect(6)));
    let off_pill = sub_frame
        .solid_rects
        .iter()
        .find(|r| r.id == "sub-off-pill")
        .expect("sub-off-pill");
    assert!(off_pill.border_color[1] > 0.9 && off_pill.border_color[2] > 0.9);

    s.open_modal(ModalId::Audio);
    s.anim.modal_t = 1.0;
    s.audio_tracks = vec![
        TrackOption {
            id: 1,
            label: "jpn • Japanese (AAC)".to_string(),
            lang: "jpn".to_string(),
            selected: true,
            external: false,
        },
        TrackOption {
            id: 2,
            label: "eng • English (AAC)".to_string(),
            lang: "eng".to_string(),
            selected: false,
            external: false,
        },
        TrackOption {
            id: 3,
            label: "spa • Spanish (AAC)".to_string(),
            lang: "spa".to_string(),
            selected: false,
            external: false,
        },
        TrackOption {
            id: 4,
            label: "fra • French (AAC)".to_string(),
            lang: "fra".to_string(),
            selected: false,
            external: false,
        },
        TrackOption {
            id: 5,
            label: "deu • German (AAC)".to_string(),
            lang: "deu".to_string(),
            selected: false,
            external: false,
        },
    ];
    let audio_frame = ui.build_frame(&s);
    assert!(audio_frame
        .hit_regions
        .iter()
        .any(|r| r.target == HitTarget::AudioTrackSelect(5)));

    // B.8: Stats overlay uses "-" empty placeholders
    s.close_modals();
    s.stats_overlay.visible = true;
    let stats_frame = ui.build_frame(&s);
    assert!(stats_frame.text_runs.iter().any(|t| t.text == "-"));
}

#[test]
fn workstream_c_welcome_playlist_recording_preview_settings_and_toasts() {
    let ui = HybridUi::new();

    // C.1: Floating playlist on welcome keeps welcome buttons centered on full viewport width
    let mut s = UiState::default();
    let wf_closed = ui.build_frame(&s);
    let open_closed = wf_closed
        .solid_rects
        .iter()
        .find(|r| r.id == "btnOpenFile-bg")
        .unwrap()
        .rect;
    s.playlist.open = true;
    s.anim.playlist_t = 1.0;
    let wf_open = ui.build_frame(&s);
    let open_with_pl = wf_open
        .solid_rects
        .iter()
        .find(|r| r.id == "btnOpenFile-bg")
        .unwrap()
        .rect;
    assert!((open_closed.x - open_with_pl.x).abs() < 0.1);
    assert!(wf_open
        .text_runs
        .iter()
        .any(|t| t.id == "playlist-empty-title" && t.text == "Playlist is empty"));

    // C.2: Playlist search no matches, active ▶ row, "N of M items", scrollbar
    s.playlist.items = vec![
        PlaylistItemEntry {
            path: "a.mp4".into(),
            name: "Alpha".into(),
        },
        PlaylistItemEntry {
            path: "b.mp4".into(),
            name: "Beta".into(),
        },
    ];
    s.playlist.current_index = Some(0);
    s.playlist.search_query = "alpha".into();
    let pl_filtered = ui.build_frame(&s);
    assert!(pl_filtered
        .text_runs
        .iter()
        .any(|t| t.id == "playlist-row-index-0" && t.text == "▶"));
    assert!(pl_filtered
        .text_runs
        .iter()
        .any(|t| t.id == "playlist-footer-count" && t.text == "1 of 2 items"));
    s.playlist.search_query = "zzz".into();
    let pl_none = ui.build_frame(&s);
    assert!(pl_none
        .text_runs
        .iter()
        .any(|t| t.id == "playlist-no-matches-title" && t.text == "No matches"));
    s.playlist.open = false;

    // C.3 / C.4: Recording ClipSaving, Saved burst count, Saved clip morph, SinglePreview aspect & no close btn
    s.welcome_active = false;
    s.recording_indicator = RecordingIndicatorMode::ClipSaving;
    let saving_f = ui.build_frame(&s);
    assert!(saving_f
        .text_runs
        .iter()
        .any(|t| t.id == "recording-clip-saving-lbl" && t.text == "Saving clip..."));

    s.recording_indicator = RecordingIndicatorMode::Saved { label_frames: true };
    s.recording_saved_frame_count = 3;
    let burst_saved_f = ui.build_frame(&s);
    assert!(burst_saved_f
        .text_runs
        .iter()
        .any(|t| t.id == "recording-saved-text" && t.text == "Saved to Screenshots"));
    assert!(burst_saved_f
        .text_runs
        .iter()
        .any(|t| t.id == "recording-saved-count" && t.text == "3 Frames"));

    s.recording_indicator = RecordingIndicatorMode::Saved {
        label_frames: false,
    };
    let clip_saved_f = ui.build_frame(&s);
    assert!(clip_saved_f
        .solid_rects
        .iter()
        .any(|r| r.id == "recording-saved-folder-bg"));

    s.carousel_mode = CarouselMode::SinglePreview;
    s.video_aspect_ratio = 1.0; // square video -> taller preview than 16:9
    s.screenshot_badge = Some(("Deleted".into(), true));
    let prev_f = ui.build_frame(&s);
    let card = prev_f
        .glass_panels
        .iter()
        .find(|g| g.id == "screenshot-preview")
        .unwrap();
    assert!((card.glass.height - 260.0).abs() < 1.0);
    let badge_bg = prev_f
        .solid_rects
        .iter()
        .find(|r| r.id == "screenshot-preview-badge-bg")
        .unwrap();
    assert_eq!(badge_bg.corner_radii, [6.0; 4]);
    assert!(prev_f.text_runs.iter().any(|t| {
        t.id == "screenshot-preview-badge-text"
            && t.text == "DELETED"
            && (t.letter_spacing_px - 0.5).abs() < 1e-4
    }));
    assert!(!prev_f
        .hit_regions
        .iter()
        .any(|h| h.target == HitTarget::CarouselClose));

    // C.3: Drop overlay content card
    s.drop_overlay_active = true;
    let drop_f = ui.build_frame(&s);
    assert!(drop_f
        .solid_rects
        .iter()
        .any(|r| r.id == "drop-overlay-content"));
    assert!(drop_f
        .text_runs
        .iter()
        .any(|t| t.id == "drop-overlay-text" && t.text == "Drop media file to play"));
    s.drop_overlay_active = false;

    // C.5: Settings search keeps Appearance, default screenshot dir, yt-dlp conditional update, Gridmotion thumbs & popover previews
    s.open_modal(ModalId::Settings);
    s.shortcut_search_query = "play".into();
    s.default_screenshot_dir = "C:/Users/Test/Pictures/Hybrid Player".into();
    s.recording_shortcut_id = Some("toggle-play".into());
    s.recording_shortcut_preview = Some("Ctrl + ...".into());
    s.ytdlp_version = "2025.03.21".into();
    let set_top = ui.build_frame(&s);
    assert!(set_top.text_runs.iter().any(|t| t.text == "APPEARANCE"));
    assert!(set_top
        .text_runs
        .iter()
        .any(|t| t.id == "settings-ss-dir-val"
            && t.text == "C:/Users/Test/Pictures/Hybrid Player (Default)"));
    s.settings_scroll_y = 740.0;
    let set_mid = ui.build_frame(&s);
    assert!(set_mid
        .text_runs
        .iter()
        .any(|t| t.id == "shortcut-key-lbl-toggle-play" && t.text == "Ctrl + ..."));
    s.settings_scroll_y = 1200.0;
    let set_bot_no_upd = ui.build_frame(&s);
    assert!(set_bot_no_upd
        .text_runs
        .iter()
        .any(|t| t.id == "ytdlp-ver-val" && t.text == "2025.03.21"));
    assert!(!set_bot_no_upd
        .hit_regions
        .iter()
        .any(|h| h.target == HitTarget::YtdlpPerformUpdate));
    s.ytdlp_update_available = true;
    s.ytdlp_latest_version = Some("2025.04.01".into());
    let set_bot_upd = ui.build_frame(&s);
    assert!(set_bot_upd
        .hit_regions
        .iter()
        .any(|h| h.target == HitTarget::YtdlpPerformUpdate));
    assert!(set_bot_upd
        .text_runs
        .iter()
        .any(|t| t.id == "ytdlp-update-btn-lbl" && t.text == "Update to v2025.04.01"));

    s.open_modal(ModalId::BgSettings);
    s.welcome_background = WelcomeBackgroundKind::Gridmotion;
    s.bg_accordion_open = true;
    s.anim.bg_accordion_t = 1.0;
    let bg_empty = ui.build_frame(&s);
    assert!(!bg_empty
        .hit_regions
        .iter()
        .any(|h| h.target == HitTarget::BgGridmotionClearImages));
    assert!(bg_empty
        .solid_rects
        .iter()
        .any(|r| r.id == "bg-accordion-preview-Dither" && r.gradient.is_some()));
    s.gridmotion_custom_pictures = vec!["C:/img1.png".into(), "C:/img2.png".into()];
    s.bg_settings_scroll_y = 260.0;
    let bg_pics = ui.build_frame(&s);
    assert!(bg_pics
        .hit_regions
        .iter()
        .any(|h| h.target == HitTarget::BgGridmotionClearImages));
    assert!(bg_pics
        .hit_regions
        .iter()
        .any(|h| h.target == HitTarget::BgGridmotionRemoveImage(0)));
    assert!(bg_pics
        .image_slots
        .iter()
        .any(|slot| slot.key == "bg-thumb:1"));

    // C.1: Light theme toast uses light background, UpdateAvailable pill exposes Update Now button
    s.close_modals();
    s.theme = ThemeMode::Light;
    s.toasts.push(ToastItem {
        id: 1,
        kind: ToastKind::Info,
        message: "Light hello".into(),
        version_tag: None,
        downloading: false,
        ..Default::default()
    });
    s.toasts.push(ToastItem {
        id: 2,
        kind: ToastKind::UpdateAvailable,
        message: "Update ready".into(),
        version_tag: Some("2025.04.01".into()),
        downloading: false,
        ..Default::default()
    });
    let tf = ui.build_frame(&s);
    let info_toast = tf.solid_rects.iter().find(|r| r.id == "toast-1").unwrap();
    assert!(info_toast.color[0] > 0.8);
    assert!(tf
        .hit_regions
        .iter()
        .any(|h| h.target == HitTarget::ToastUpdateNow));
    assert!(tf
        .text_runs
        .iter()
        .any(|t| t.id == "toast-ver-txt-2" && t.text == "v2025.04.01"));
}

#[test]
fn chapter_hover_keeps_progress_handle_and_tooltip_helper() {
    use hybrid_ui::layout::target_has_tooltip;

    let ui = HybridUi::new();
    let mut s = UiState {
        viewport_w: 1280.0,
        viewport_h: 720.0,
        welcome_active: false,
        duration_secs: 100.0,
        current_secs: 40.0,
        chapter_positions_norm: vec![0.25],
        chapter_times_secs: vec![25.0],
        chapter_titles: vec!["Chapter 1".into()],
        hovered_target: Some(HitTarget::ChapterMarker(0)),
        hover_tooltip_elapsed_secs: 0.5,
        ..UiState::default()
    };

    let f = ui.build_frame(&s);
    assert!(
        f.solid_rects.iter().any(|r| r.id == "progress-handle"),
        "hovering a chapter marker must keep the progress handle visible"
    );
    assert!(
        f.text_runs
            .iter()
            .any(|t| t.id == "hover-tooltip-text" && t.text == "Chapter 1 - 0:25"),
        "chapter marker tooltip should appear after delay"
    );
    assert!(target_has_tooltip(&s, &HitTarget::ChapterMarker(0)));
    assert!(target_has_tooltip(&s, &HitTarget::PlayPause));
    assert!(!target_has_tooltip(&s, &HitTarget::ControlsBarSurface));
    assert!(!target_has_tooltip(&s, &HitTarget::ModalBackdrop));

    s.hovered_target = None;
    let f2 = ui.build_frame(&s);
    assert!(!f2.solid_rects.iter().any(|r| r.id == "progress-handle"));
}

#[test]
fn parity_fixes_1_to_7_ui_animations_previews_toasts_notices() {
    let ui = HybridUi::new();

    // Bug 1 & 2: #bgAccordionGallery pre-hover previews visible (no opaque #050508 cover),
    // smooth per-card flex/reveal interpolation, and closing popover fades UiShaderSlot opacity.
    let mut s = UiState {
        viewport_w: 1280.0,
        viewport_h: 720.0,
        welcome_active: true,
        welcome_background: WelcomeBackgroundKind::Dither,
        ..Default::default()
    };
    s.open_modal_animated(ModalId::BgSettings);
    s.bg_accordion_open = true;
    s.anim.trigger_bg_accordion_in();
    for _ in 0..25 {
        s.step_animations(1.0 / 60.0);
    }
    let f_open = ui.build_frame(&s);
    assert!(!f_open
        .solid_rects
        .iter()
        .any(|r| r.id.starts_with("bg-accordion-live-mount-")));
    let open_previews: Vec<_> = f_open
        .shader_slots
        .iter()
        .filter(|slot| slot.is_preview)
        .collect();
    assert_eq!(open_previews.len(), 4);
    assert!(open_previews
        .iter()
        .any(|p| p.kind == WelcomeBackgroundKind::Particles && (p.opacity - 0.85).abs() < 1e-3));
    assert!(open_previews
        .iter()
        .any(|p| p.kind == WelcomeBackgroundKind::Dither && (p.opacity - 1.0).abs() < 1e-3));

    // Hover Particles card and step mid-transition (~100ms of 350ms).
    s.hovered_target = Some(HitTarget::BgAccordionCard(WelcomeBackgroundKind::Particles));
    for _ in 0..6 {
        s.step_animations(1.0 / 60.0);
    }
    let w_particles = s.anim.accordion_card_weights[1];
    let rev_particles = s.anim.accordion_card_reveal[1];
    assert!(
        w_particles > 1.01 && w_particles < 1.99,
        "hovered card flex weight should smoothly interpolate, got {w_particles}"
    );
    assert!(
        rev_particles > 0.01 && rev_particles < 0.99,
        "hovered card reveal should smoothly interpolate, got {rev_particles}"
    );

    // Close popover: shader preview opacity must attenuate smoothly (< 0.85) instead of staying 1.0.
    s.bg_accordion_open = false;
    s.anim.trigger_bg_accordion_out();
    assert!(s.anim.bg_accordion_closing);
    s.step_animations(0.12);
    let f_closing = ui.build_frame(&s);
    let closing_previews: Vec<_> = f_closing
        .shader_slots
        .iter()
        .filter(|slot| slot.is_preview)
        .collect();
    assert_eq!(closing_previews.len(), 4);
    assert!(closing_previews
        .iter()
        .all(|p| p.opacity < 0.85 && p.opacity > 0.0));

    // Bug 3 (UI): Over-video titlebar is fully transparent like Electron's `.titlebar`
    // (`main.css:155-163`): no full-width dark scrim and no per-button pill backing. Legibility
    // comes only from the icon drop shadows.
    let mut vid = UiState {
        viewport_w: 1280.0,
        viewport_h: 720.0,
        welcome_active: false,
        playing: true,
        ..Default::default()
    };
    let vf = ui.build_frame(&vid);
    assert!(
        !vf.solid_rects
            .iter()
            .any(|r| r.id == "titlebar-video-scrim"),
        "titlebar must stay transparent over video"
    );
    assert!(vf.svg_quads.iter().any(|q| q.id == "btnMinimize-shadow"));
    assert!(vf.svg_quads.iter().any(|q| q.id == "btnMaximize-shadow"));
    for id in ["btnMinimize", "btnMaximize", "btnClose"] {
        assert!(
            !vf.solid_rects
                .iter()
                .any(|r| r.id == format!("{id}-contrast-pill")),
            "{id} must not render a contrast pill"
        );
    }

    // Bug 4 (UI): #bgSettingsToggle hidden <=860px and during playback, but it must stay
    // available while the playlist drawer is open (regression: opening Playlist removed it).
    let mut wt = UiState {
        viewport_w: 900.0,
        viewport_h: 720.0,
        welcome_active: true,
        ..Default::default()
    };
    assert!(ui
        .build_frame(&wt)
        .hit_regions
        .iter()
        .any(|h| h.target == HitTarget::BgSettingsToggle));
    wt.viewport_w = 860.0;
    assert!(!ui
        .build_frame(&wt)
        .hit_regions
        .iter()
        .any(|h| h.target == HitTarget::BgSettingsToggle));
    wt.viewport_w = 1280.0;
    wt.playlist.open = true;
    assert!(ui
        .build_frame(&wt)
        .hit_regions
        .iter()
        .any(|h| h.target == HitTarget::BgSettingsToggle));
    wt.viewport_w = 860.0;
    assert!(!ui
        .build_frame(&wt)
        .hit_regions
        .iter()
        .any(|h| h.target == HitTarget::BgSettingsToggle));
    wt.viewport_w = 1280.0;
    wt.playlist.open = false;
    wt.welcome_active = false;
    assert!(!ui
        .build_frame(&wt)
        .hit_regions
        .iter()
        .any(|h| h.target == HitTarget::BgSettingsToggle));

    // Bug 6: Toasts auto-dismiss after TTL (2s info, 12s error, paused when downloading) with exit fade.
    vid.push_toast(ToastItem::new(1, ToastKind::Info, "Audio: Track 1"));
    let mut err_toast = ToastItem::new(2, ToastKind::Error, "Stream failed");
    err_toast.downloading = true;
    vid.push_toast(err_toast);
    assert_eq!(vid.toasts.len(), 2);
    for _ in 0..125 {
        vid.step_animations(1.0 / 60.0);
    }
    assert!(
        vid.toasts[0].closing,
        "info toast should begin closing after 2s"
    );
    assert!(!vid.toasts[1].closing, "downloading toast should pause TTL");
    for _ in 0..20 {
        vid.step_animations(1.0 / 60.0);
    }
    assert_eq!(vid.toasts.len(), 1);
    assert_eq!(vid.toasts[0].id, 2);
    vid.toasts[0].downloading = false;
    for _ in 0..750 {
        vid.step_animations(1.0 / 60.0);
    }
    assert!(
        vid.toasts.is_empty(),
        "error toast should expire after 12s + 250ms fade"
    );

    // Bug 7 (UI): #screenshotNotice is top-left (`left: 24px`, `max-width: 260px`, radius 9px).
    vid.screenshot_notice = Some("No screenshots to delete".into());
    let nf = ui.build_frame(&vid);
    let notice_rect = nf
        .solid_rects
        .iter()
        .find(|r| r.id == "screenshot-notice")
        .unwrap();
    assert!((notice_rect.rect.x - 24.0).abs() < 1e-4);
    assert!((notice_rect.rect.y - 86.0).abs() < 1e-4);
    assert!(notice_rect.rect.width <= 260.0);
    assert_eq!(notice_rect.corner_radii, [9.0; 4]);
    vid.stats_overlay.visible = true;
    let nf_stats = ui.build_frame(&vid);
    let notice_stats = nf_stats
        .solid_rects
        .iter()
        .find(|r| r.id == "screenshot-notice")
        .unwrap();
    assert!((notice_stats.rect.y - 296.0).abs() < 1e-4);
}

#[test]
fn regression_hover_thumb_default_modal_accordion_and_titlebar_contrast() {
    let ui = HybridUi::new();

    // 1. Default hover_thumb_has_frame must be false; hovering before frame arrives must NOT emit image slot.
    let mut s = UiState {
        viewport_w: 1280.0,
        viewport_h: 720.0,
        welcome_active: false,
        duration_secs: 120.0,
        hover_thumb_norm: Some(0.4),
        ..Default::default()
    };
    assert!(!s.hover_thumb_has_frame);
    let f_no_thumb = ui.build_frame(&s);
    assert!(!f_no_thumb
        .image_slots
        .iter()
        .any(|img| img.key.starts_with("hover-thumb:")));

    // 2. open_modal clears bg_accordion_open AND bg_accordion_closing.
    s.bg_accordion_open = true;
    s.anim.bg_accordion_closing = true;
    s.open_modal(ModalId::Settings);
    assert!(!s.bg_accordion_open);
    assert!(!s.anim.bg_accordion_closing);

    // 3. Over-video hovered btnMinimize still gets a legible hover surface, but the titlebar
    // itself remains transparent (no full-width scrim).
    let vid_hov = UiState {
        viewport_w: 1280.0,
        viewport_h: 720.0,
        welcome_active: false,
        playing: true,
        hovered_target: Some(HitTarget::TitlebarMinimize),
        ..Default::default()
    };
    let hf = ui.build_frame(&vid_hov);
    assert!(!hf
        .solid_rects
        .iter()
        .any(|r| r.id == "titlebar-video-scrim"));
    let min_hov = hf
        .solid_rects
        .iter()
        .find(|r| r.id == "btnMinimize-hover")
        .unwrap();
    assert!(min_hov.color[0] < 0.05 && min_hov.color[3] >= 0.45);

    // 4. Narrow viewport #bgAccordionPopover centers preview vertically inside card_rect.
    let narrow = UiState {
        viewport_w: 880.0,
        viewport_h: 720.0,
        welcome_active: true,
        bg_accordion_open: true,
        active_modal: Some(ModalId::BgSettings),
        welcome_background: WelcomeBackgroundKind::Dither,
        ..Default::default()
    };
    let nf = ui.build_frame(&narrow);
    let card = nf
        .solid_rects
        .iter()
        .find(|r| r.id == "bg-card-frame-Dither")
        .unwrap();
    let slot = nf
        .shader_slots
        .iter()
        .find(|sl| sl.id == "bg-preview-Dither")
        .unwrap();
    let card_center_y = card.rect.y + card.rect.height * 0.5;
    let slot_center_y = slot.rect.y + slot.rect.height * 0.5;
    assert!((card_center_y - slot_center_y).abs() <= 1.5);
}

#[test]
fn ab_loop_badge_pulses_opacity_without_scaling_or_shifting_text() {
    let ui = HybridUi::new();
    let base = UiState {
        viewport_w: 1280.0,
        viewport_h: 720.0,
        welcome_active: false,
        playing: true,
        ..Default::default()
    };

    // Electron `.ab-loop-indicator` (`main.css:2636-2656`) runs `animation: pulse`, whose
    // keyframes (`animations.css:123-126`) animate opacity only. Scaling the subtree (the old
    // `PulseGlow`) re-rasterized the label at a new fractional font size every frame, making the
    // text shimmer and shake.
    for ab_loop in [
        AbLoopState::PointASet(2_000),
        AbLoopState::Active(2_000, 12_000),
    ] {
        let mut rest = base.clone();
        rest.ab_loop = ab_loop;
        rest.anim.ab_pulse_phase = 0.0;
        let rest_frame = ui.build_frame(&rest);
        let rest_pill = rest_frame
            .solid_rects
            .iter()
            .find(|r| r.id == "ab-loop-indicator")
            .expect("ab-loop-indicator");
        let rest_label = rest_frame
            .text_runs
            .iter()
            .find(|t| t.id == "ab-loop-indicator-label")
            .expect("ab-loop-indicator-label");
        assert_eq!(rest_label.font_role, FontRole::Ui);
        assert_eq!(rest_label.weight, 600);
        assert!((rest_label.font_size_px - 12.0).abs() < f32::EPSILON);
        assert!(
            (rest_label.rect.width - (rest_pill.rect.width - 28.0)).abs() < 0.01,
            "Electron uses 14px horizontal padding ({ab_loop:?})"
        );
        if matches!(ab_loop, AbLoopState::PointASet(_)) {
            assert!(
                rest_pill.rect.width < 100.0,
                "the A-only pill must hug its label instead of using the old 108px min-width"
            );
        }

        let mut peak = rest.clone();
        peak.anim.ab_pulse_phase = 0.5;
        let peak_frame = ui.build_frame(&peak);
        let peak_pill = peak_frame
            .solid_rects
            .iter()
            .find(|r| r.id == "ab-loop-indicator")
            .expect("ab-loop-indicator");
        let peak_label = peak_frame
            .text_runs
            .iter()
            .find(|t| t.id == "ab-loop-indicator-label")
            .expect("ab-loop-indicator-label");

        assert_eq!(
            peak_pill.rect, rest_pill.rect,
            "pulse must not move or scale the pill ({ab_loop:?})"
        );
        assert_eq!(
            peak_label.rect, rest_label.rect,
            "pulse must not move or scale the label ({ab_loop:?})"
        );
        assert!(
            (peak_label.font_size_px - 12.0).abs() < f32::EPSILON,
            "pulse must not change the rasterized font size ({ab_loop:?})"
        );
        assert!(
            (peak_pill.color[3] - 0.6).abs() < 0.01,
            "pulse peak should fade the pill to 0.6 ({ab_loop:?})"
        );
        assert!(
            (peak_label.color[3] - 0.6).abs() < 0.01,
            "the label should fade with the pill ({ab_loop:?})"
        );
    }
}
