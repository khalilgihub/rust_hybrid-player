use std::io::Cursor;

use hybrid_shaders::{
    decode_and_resize_image, decode_image_file_async, default_gradients_rgb,
    AccordionPreviewGallery, BlurRequests, ChameleonSkin, ClearContext, CssBackdropRule,
    DitherWavesConfig, EffectInstanceConfig, EffectLifecycle, FaultyTerminalConfig,
    FrameCompositor, GlassInstanceGpu, GlassRect, GpuContext, GridAspectRatio, GridMotionConfig,
    ParticlesConfig, QualityTier, RenderGraph, SceneSource, SdfGlassRect, ThemeMode,
    UiQuadInstanceGpu, UiTextureId, UiTexturedQuad, VideoFitMode, VideoFrameTransform,
    WelcomeBackgroundKind, CARD_TEXTS, DEFAULT_GRADIENT_HEXES, MAX_IMAGE_DIM,
};
use image::{ImageFormat, Rgba, RgbaImage};

fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
        .block_on(fut)
}

fn create_target(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test-target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

fn count_non_uniform_stats(rgba: &[u8]) -> (usize, usize) {
    let mut non_black = 0usize;
    let first = [rgba[0], rgba[1], rgba[2]];
    let mut diff_from_first = 0usize;
    for px in rgba.chunks_exact(4) {
        if px[0] > 4 || px[1] > 4 || px[2] > 4 {
            non_black += 1;
        }
        if px[0] != first[0] || px[1] != first[1] || px[2] != first[2] {
            diff_from_first += 1;
        }
    }
    (non_black, diff_from_first)
}

fn pixel_at(rgba: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let idx = ((y * width + x) * 4) as usize;
    [rgba[idx], rgba[idx + 1], rgba[idx + 2], rgba[idx + 3]]
}

#[test]
fn uniform_alignment_and_css_backdrop_catalog() {
    assert_eq!(std::mem::size_of::<GlassInstanceGpu>() % 16, 0);
    assert_eq!(CssBackdropRule::ACTIVE_RULES.len(), 23);
    assert_eq!(CssBackdropRule::NONE_OVERRIDES.len(), 7);

    for rule in CssBackdropRule::ACTIVE_RULES {
        let spec = rule.spec(ThemeMode::Dark);
        assert!(!spec.is_none_override);
        assert!(
            (4.0..=32.0).contains(&spec.blur_px),
            "rule {rule:?} blur_px {} out of [4, 32]",
            spec.blur_px
        );
        assert!(
            (1.0..=1.9).contains(&spec.saturation),
            "rule {rule:?} saturation {} out of [1.0, 1.9]",
            spec.saturation
        );
    }

    for rule in CssBackdropRule::NONE_OVERRIDES {
        let spec = rule.spec(ThemeMode::Oled);
        assert!(spec.is_none_override);
        assert_eq!(spec.blur_px, 0.0);
    }

    assert_eq!(ThemeMode::Dark.glass_blur_px(), 20.0);
    assert_eq!(ThemeMode::Oled.glass_blur_px(), 25.0);
    assert_eq!(ThemeMode::Light.glass_blur_px(), 20.0);

    // DPR caps: low <= 0.8, medium <= 1.0, high <= 1.5
    assert!((QualityTier::Low.cap_dpr(2.0) - 0.8).abs() < 1e-6);
    assert!((QualityTier::Medium.cap_dpr(2.0) - 1.0).abs() < 1e-6);
    assert!((QualityTier::High.cap_dpr(2.0) - 1.5).abs() < 1e-6);

    assert_eq!(DEFAULT_GRADIENT_HEXES.len(), 28);
    assert_eq!(CARD_TEXTS.len(), 28);
    assert_eq!(default_gradients_rgb().len(), 28);
}

#[test]
fn lifecycle_auto_pause_and_30fps_pacing() {
    let preview_cfg = EffectInstanceConfig::preview_400x240();
    assert_eq!(preview_cfg.pixel_size(), (400, 240));
    let mut lc = EffectLifecycle::from_instance(&preview_cfg);

    assert!(lc.should_render(0.000));
    // 10ms later -> throttled by 30fps (33.33ms)
    assert!(!lc.should_render(0.010));
    // 34ms later -> allowed
    assert!(lc.should_render(0.034));

    // Unfocused -> paused
    lc.window_focused = false;
    assert!(lc.is_paused());
    assert!(!lc.should_render(1.0));
    lc.window_focused = true;

    // Hidden -> paused
    lc.window_visible = false;
    assert!(lc.is_paused());
    assert!(!lc.should_render(2.0));
    lc.window_visible = true;

    // Playing video -> paused
    lc.video_playing = true;
    assert!(lc.is_paused());
    assert!(!lc.should_render(3.0));
    lc.video_playing = false;

    // Motion paused -> paused
    lc.motion_paused = true;
    assert!(lc.is_paused());
    assert!(!lc.should_render(4.0));
    lc.motion_paused = false;

    assert!(lc.should_render(5.0));
}

#[test]
fn pass0_clear_colors_and_aspect_fit_video() {
    block_on(async {
        let ctx = GpuContext::new_headless().await.expect("headless wgpu");
        let (w, h) = (160, 120); // 4:3 viewport
        let (target_tex, target_view) = create_target(&ctx.device, w, h);
        let mut graph = RenderGraph::new(
            &ctx.device,
            &ctx.queue,
            wgpu::TextureFormat::Rgba8Unorm,
            w,
            h,
            EffectInstanceConfig::default(),
        )
        .expect("render graph");

        // Normal clear -> #0c0d12
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        graph.encode(
            &ctx.device,
            &ctx.queue,
            &mut enc,
            &target_view,
            ClearContext::default(),
            SceneSource::ClearOnly,
            &[],
            0.0,
        );
        ctx.queue.submit(Some(enc.finish()));
        let pixels = ctx.readback_texture(&target_tex, w, h).expect("readback");
        assert_eq!(pixel_at(&pixels, w, 80, 60), [12, 13, 18, 255]);

        // welcome_active / app_cold_start / max_transition -> #000000
        for clear in [
            ClearContext {
                welcome_active: true,
                ..Default::default()
            },
            ClearContext {
                app_cold_start: true,
                ..Default::default()
            },
            ClearContext {
                max_transition: true,
                ..Default::default()
            },
        ] {
            let mut enc = ctx.device.create_command_encoder(&Default::default());
            graph.encode(
                &ctx.device,
                &ctx.queue,
                &mut enc,
                &target_view,
                clear,
                SceneSource::ClearOnly,
                &[],
                0.0,
            );
            ctx.queue.submit(Some(enc.finish()));
            let pixels = ctx.readback_texture(&target_tex, w, h).expect("readback");
            assert_eq!(pixel_at(&pixels, w, 80, 60), [0, 0, 0, 255]);
        }

        // Aspect-fit 16:9 solid green video inside 4:3 viewport -> top/bottom bars are #0c0d12, center is green
        let video_tex = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mock-video"),
            size: wgpu::Extent3d {
                width: 16,
                height: 9,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let green = [20u8, 220, 60, 255].repeat(16 * 9);
        ctx.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &video_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &green,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(16 * 4),
                rows_per_image: Some(9),
            },
            wgpu::Extent3d {
                width: 16,
                height: 9,
                depth_or_array_layers: 1,
            },
        );
        let video_view = video_tex.create_view(&Default::default());

        let mut enc = ctx.device.create_command_encoder(&Default::default());
        graph.encode(
            &ctx.device,
            &ctx.queue,
            &mut enc,
            &target_view,
            ClearContext::default(),
            SceneSource::Video {
                texture_view: &video_view,
                transform: VideoFrameTransform {
                    video_width: 16,
                    video_height: 9,
                    fit_mode: VideoFitMode::Contain,
                    opacity: 1.0,
                },
            },
            &[],
            0.0,
        );
        ctx.queue.submit(Some(enc.finish()));
        let pixels = ctx.readback_texture(&target_tex, w, h).expect("readback");
        // Top bar (y=2) -> #0c0d12
        assert_eq!(pixel_at(&pixels, w, 80, 2), [12, 13, 18, 255]);
        // Center (y=60) -> video green
        assert_eq!(pixel_at(&pixels, w, 80, 60), [20, 220, 60, 255]);
    });
}

#[test]
fn pass1_kawase_blur_pyramid_and_pass2_sdf_glass_chameleon() {
    block_on(async {
        let ctx = GpuContext::new_headless().await.expect("headless wgpu");
        let (w, h) = (320, 240);
        let (target_tex, target_view) = create_target(&ctx.device, w, h);
        let mut graph = RenderGraph::new(
            &ctx.device,
            &ctx.queue,
            wgpu::TextureFormat::Rgba8Unorm,
            w,
            h,
            EffectInstanceConfig::default(),
        )
        .expect("render graph");
        graph.force_blur_pyramid = true;

        // Upload a split left-black / right-vivid-red/coral video texture to test Dual-Kawase blur + saturation
        let vid_tex = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("split-test"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut raw = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let idx = ((y * w + x) * 4) as usize;
                if x < w / 2 {
                    raw[idx..idx + 4].copy_from_slice(&[10, 10, 15, 255]);
                } else {
                    raw[idx..idx + 4].copy_from_slice(&[180, 90, 90, 255]);
                }
            }
        }
        ctx.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &vid_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &raw,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        let vid_view = vid_tex.create_view(&Default::default());

        // Glass rect 1 (left-center, blur 32px, saturate 190%, translucent fill so blur shows)
        let mut glass_blur32_sat190 =
            GlassRect::new(110.0, 20.0, 100.0, 80.0, 14.0, ThemeMode::Dark)
                .with_backdrop_rule(CssBackdropRule::RecordingIndicator, ThemeMode::Dark);
        glass_blur32_sat190.fill_color_0[3] = 0.05;
        glass_blur32_sat190.fill_color_1[3] = 0.05;

        // Glass rect 2 (right side over muted coral, saturate 190% vs 110%)
        let mut glass_sat110 = GlassRect::new(190.0, 130.0, 50.0, 60.0, 10.0, ThemeMode::Dark)
            .with_backdrop_rule(CssBackdropRule::WelcomeBgFaulty, ThemeMode::Dark);
        glass_sat110.fill_color_0[3] = 0.0;
        glass_sat110.fill_color_1[3] = 0.0;

        let mut glass_sat190 = GlassRect::new(250.0, 130.0, 50.0, 60.0, 10.0, ThemeMode::Dark)
            .with_backdrop_rule(CssBackdropRule::ToastUpdatePill, ThemeMode::Dark);
        glass_sat190.fill_color_0[3] = 0.0;
        glass_sat190.fill_color_1[3] = 0.0;

        let mut enc = ctx.device.create_command_encoder(&Default::default());
        graph.encode(
            &ctx.device,
            &ctx.queue,
            &mut enc,
            &target_view,
            ClearContext::default(),
            SceneSource::Video {
                texture_view: &vid_view,
                transform: VideoFrameTransform {
                    video_width: w,
                    video_height: h,
                    fit_mode: VideoFitMode::Stretch,
                    opacity: 1.0,
                },
            },
            &[glass_blur32_sat190, glass_sat110, glass_sat190],
            0.5,
        );
        ctx.queue.submit(Some(enc.finish()));

        let pixels = ctx.readback_texture(&target_tex, w, h).expect("readback");
        // Verify blur diffused red across x = 160 boundary inside glass rect 1 (check x = 148, y = 60)
        let diffused_px = pixel_at(&pixels, w, 148, 60);
        assert!(
            diffused_px[0] > 25,
            "expected Dual-Kawase Blur32 diffusion across seam, got {diffused_px:?}"
        );

        // Verify saturate(190%) boosts chroma (R - G) strictly more than saturate(110%)
        let px_110 = pixel_at(&pixels, w, 215, 160);
        let px_190 = pixel_at(&pixels, w, 275, 160);
        let chroma_110 = px_110[0].saturating_sub(px_110[1]);
        let chroma_190 = px_190[0].saturating_sub(px_190[1]);
        assert!(
            chroma_190 > chroma_110,
            "expected saturate(190%) chroma {chroma_190} > saturate(110%) chroma {chroma_110}"
        );

        // Render all 5 Chameleon skins and confirm distinct, non-blank output
        let mut skin_samples = Vec::new();
        for skin in [
            ChameleonSkin::None,
            ChameleonSkin::Dither,
            ChameleonSkin::Faulty,
            ChameleonSkin::Particles,
            ChameleonSkin::Gridmotion,
        ] {
            let rect = GlassRect::new(40.0, 30.0, 240.0, 180.0, 16.0, ThemeMode::Dark)
                .with_backdrop_rule(CssBackdropRule::BgSettingsPanel, ThemeMode::Dark)
                .with_chameleon_skin(skin, ThemeMode::Dark);
            let mut enc = ctx.device.create_command_encoder(&Default::default());
            graph.encode(
                &ctx.device,
                &ctx.queue,
                &mut enc,
                &target_view,
                ClearContext {
                    welcome_active: true,
                    ..Default::default()
                },
                SceneSource::ClearOnly,
                &[rect],
                0.7,
            );
            ctx.queue.submit(Some(enc.finish()));
            let out = ctx.readback_texture(&target_tex, w, h).expect("readback");
            skin_samples.push(pixel_at(&out, w, 160, 120));
        }
        assert_ne!(skin_samples[2], skin_samples[3]); // Faulty != Particles
        assert_ne!(skin_samples[3], skin_samples[4]); // Particles != Gridmotion
    });
}

#[test]
fn all_four_welcome_effects_compile_render_and_determinism() {
    block_on(async {
        let ctx = GpuContext::new_headless().await.expect("headless wgpu");
        let (w, h) = (320, 180);
        let (target_tex, target_view) = create_target(&ctx.device, w, h);

        let mut graph = RenderGraph::new(
            &ctx.device,
            &ctx.queue,
            wgpu::TextureFormat::Rgba8Unorm,
            w,
            h,
            EffectInstanceConfig {
                width: w,
                height: h,
                dpr: 1.0,
                fps_limit: None,
                is_preview: false,
                pause_when_unfocused: true,
            },
        )
        .expect("render graph");

        graph.effects.dither.set_time(1.5);
        graph.effects.particles.set_time(1.5);
        graph
            .effects
            .faulty
            .set_time_and_mouse(2.0, [0.5, 0.5], 1.0);
        graph.effects.grid_motion.set_scroll_state(120.0, 0.0);

        for kind in [
            WelcomeBackgroundKind::Dither,
            WelcomeBackgroundKind::Particles,
            WelcomeBackgroundKind::Faulty,
            WelcomeBackgroundKind::Gridmotion,
        ] {
            let mut enc1 = ctx.device.create_command_encoder(&Default::default());
            graph.encode(
                &ctx.device,
                &ctx.queue,
                &mut enc1,
                &target_view,
                ClearContext {
                    welcome_active: true,
                    ..Default::default()
                },
                SceneSource::Welcome(kind),
                &[],
                0.0,
            );
            ctx.queue.submit(Some(enc1.finish()));
            let frame1 = ctx.readback_texture(&target_tex, w, h).expect("readback 1");
            let (non_black, diff) = count_non_uniform_stats(&frame1);
            assert!(
                non_black > 50 && diff > 50,
                "effect {kind:?} rendered blank or uniform frame (non_black={non_black}, diff={diff})"
            );

            // Second identical encode must be byte-for-byte deterministic
            let mut enc2 = ctx.device.create_command_encoder(&Default::default());
            graph.encode(
                &ctx.device,
                &ctx.queue,
                &mut enc2,
                &target_view,
                ClearContext {
                    welcome_active: true,
                    ..Default::default()
                },
                SceneSource::Welcome(kind),
                &[],
                0.0,
            );
            ctx.queue.submit(Some(enc2.finish()));
            let frame2 = ctx.readback_texture(&target_tex, w, h).expect("readback 2");
            assert_eq!(frame1, frame2, "effect {kind:?} was non-deterministic");
        }

        // Verify 400x240 @ 30fps AccordionPreviewGallery renders all 4 previews
        let mut gallery = AccordionPreviewGallery::new(&ctx.device, &ctx.queue);
        gallery.bundle.dither.set_time(1.0);
        gallery.bundle.particles.set_time(1.0);
        gallery
            .bundle
            .faulty
            .set_time_and_mouse(1.0, [0.5, 0.5], 1.0);
        gallery.bundle.grid_motion.set_scroll_state(80.0, 0.0);

        let mut enc = ctx.device.create_command_encoder(&Default::default());
        let rendered = gallery.render_due_previews(&ctx.device, &ctx.queue, &mut enc, 0.0, 0.016);
        assert_eq!(rendered, [true, true, true, true]);
        ctx.queue.submit(Some(enc.finish()));

        // Throttled 10ms later -> none due at 30fps
        let mut enc_skip = ctx.device.create_command_encoder(&Default::default());
        let skipped =
            gallery.render_due_previews(&ctx.device, &ctx.queue, &mut enc_skip, 0.010, 0.010);
        assert_eq!(skipped, [false, false, false, false]);

        for kind in [
            WelcomeBackgroundKind::Dither,
            WelcomeBackgroundKind::Particles,
            WelcomeBackgroundKind::Faulty,
            WelcomeBackgroundKind::Gridmotion,
        ] {
            let tex = gallery.texture(kind).expect("preview texture");
            let px = ctx
                .readback_texture(tex, 400, 240)
                .expect("preview readback");
            let (non_black, diff) = count_non_uniform_stats(&px);
            let min_pixels = if kind == WelcomeBackgroundKind::Particles {
                // Preview preset has 45 sparse particles with base_size=32 / dist -> ~1-3px dots
                8
            } else {
                50
            };
            assert!(
                non_black > min_pixels && diff > min_pixels,
                "400x240 preview {kind:?} rendered blank or uniform (non_black={non_black}, diff={diff})"
            );
        }
    });
}

#[test]
fn gridmotion_async_image_decode_1440px_cap_and_atlas_render() {
    block_on(async {
        // Synthesize a 2000x1000 PNG to verify max 1440px downscale
        let mut big = RgbaImage::new(2000, 1000);
        for px in big.pixels_mut() {
            *px = Rgba([30, 180, 240, 255]);
        }
        let mut png_bytes = Vec::new();
        big.write_to(&mut Cursor::new(&mut png_bytes), ImageFormat::Png)
            .expect("encode png");

        let resized = decode_and_resize_image(&png_bytes, MAX_IMAGE_DIM).expect("resize");
        assert_eq!(resized.width(), 1440);
        assert_eq!(resized.height(), 720);

        let tmp = std::env::temp_dir().join("hybrid_shaders_grid_test.png");
        std::fs::write(&tmp, &png_bytes).expect("write temp png");
        let async_img = decode_image_file_async(tmp.clone(), MAX_IMAGE_DIM)
            .await
            .expect("async decode");
        let _ = std::fs::remove_file(&tmp);
        assert_eq!(async_img.width(), 1440);

        // Render GridMotion with uploaded custom picture across all aspect ratios
        let ctx = GpuContext::new_headless().await.expect("headless wgpu");
        let (w, h) = (320, 180);
        let (target_tex, target_view) = create_target(&ctx.device, w, h);
        let mut graph = RenderGraph::new(
            &ctx.device,
            &ctx.queue,
            wgpu::TextureFormat::Rgba8Unorm,
            w,
            h,
            EffectInstanceConfig::default(),
        )
        .expect("render graph");

        graph
            .effects
            .grid_motion
            .upload_image_to_slot(&ctx.queue, 0, &async_img);
        assert_eq!(graph.effects.grid_motion.loaded_photo_count(), 1);

        for ratio in ["16:9", "1:1", "3:4", "4:3", "auto"] {
            graph.effects.grid_motion.config.aspect_ratio = GridAspectRatio::parse_str(ratio);
            let mut enc = ctx.device.create_command_encoder(&Default::default());
            graph.encode(
                &ctx.device,
                &ctx.queue,
                &mut enc,
                &target_view,
                ClearContext {
                    welcome_active: true,
                    ..Default::default()
                },
                SceneSource::Welcome(WelcomeBackgroundKind::Gridmotion),
                &[],
                0.0,
            );
            ctx.queue.submit(Some(enc.finish()));
            let px = ctx.readback_texture(&target_tex, w, h).expect("readback");
            let (non_black, _) = count_non_uniform_stats(&px);
            assert!(non_black > 100, "gridmotion ratio {ratio} rendered blank");
        }
    });
}

#[test]
fn presets_smoke_test() {
    let _ = DitherWavesConfig::for_quality(QualityTier::Low);
    let _ = ParticlesConfig::for_quality(QualityTier::Medium);
    let _ = FaultyTerminalConfig::for_quality(QualityTier::High);
    let _ = GridMotionConfig::for_quality(QualityTier::High);
    assert_eq!(std::mem::size_of::<UiQuadInstanceGpu>(), 128);
}

#[test]
fn frame_compositor_sdf_glass_ui_quads_and_z_ordering() {
    block_on(async {
        let ctx = GpuContext::new_headless().await.expect("headless wgpu");
        let (w, h) = (320, 180);
        let (target_tex, target_view) = create_target(&ctx.device, w, h);

        let mut compositor = FrameCompositor::new(
            &ctx.device,
            &ctx.queue,
            wgpu::TextureFormat::Rgba8Unorm,
            w,
            h,
            EffectInstanceConfig::default(),
        )
        .expect("FrameCompositor");

        // Upload an R8 glyph coverage mask (8x8 solid block) & test in-place update_r8
        let glyph_id =
            compositor.upload_r8(&ctx.device, &ctx.queue, "glyph-atlas", 8, 8, &[255u8; 64]);
        compositor.update_r8(&ctx.device, &ctx.queue, glyph_id, 8, 8, &[255u8; 64]);

        // Upload a premultiplied RGBA SVG texture (50% green premultiplied -> [0, 128, 0, 128])
        let mut svg_bytes = Vec::with_capacity(16 * 16 * 4);
        for _ in 0..(16 * 16) {
            svg_bytes.extend_from_slice(&[0, 128, 0, 128]);
        }
        let svg_id =
            compositor.upload_rgba8(&ctx.device, &ctx.queue, "svg-icon", 16, 16, &svg_bytes);

        // Upload a straight-alpha RGBA texture (blue opaque)
        let mut img_bytes = Vec::with_capacity(16 * 16 * 4);
        for _ in 0..(16 * 16) {
            img_bytes.extend_from_slice(&[30, 110, 255, 255]);
        }
        let img_id = compositor.upload_rgba8(&ctx.device, &ctx.queue, "thumb", 16, 16, &img_bytes);

        // Interleaved z-order test:
        // z=10: solid bright red background card covering (40,40, 240,100)
        // z=20: SdfGlassRect covering (80,50, 160,80) -> must draw OVER z=10 red card
        // z=30: glyph mask (yellow), premultiplied SVG (green), rounded bordered solid + RGBA thumb -> OVER z=20 glass
        let glass_rects = [SdfGlassRect::new(
            20,
            GlassRect::new(80.0, 50.0, 160.0, 80.0, 12.0, ThemeMode::Dark)
                .with_backdrop_rule(CssBackdropRule::ModalPanel, ThemeMode::Dark)
                .with_chameleon_skin(ChameleonSkin::Particles, ThemeMode::Dark),
        )];
        let ui_quads = [
            UiTexturedQuad::solid(10, [40.0, 40.0, 240.0, 100.0], [1.0, 0.0, 0.0, 1.0]),
            UiTexturedQuad::glyph_mask(
                30,
                [100.0, 70.0, 24.0, 24.0],
                [0.0, 0.0, 1.0, 1.0],
                glyph_id,
                [1.0, 0.9, 0.1, 1.0],
            ),
            UiTexturedQuad::premultiplied_svg(
                31,
                [140.0, 70.0, 24.0, 24.0],
                svg_id,
                [1.0, 1.0, 1.0, 1.0],
            ),
            UiTexturedQuad::textured(32, [180.0, 70.0, 24.0, 24.0], img_id).with_corner_radius(4.0),
            UiTexturedQuad::solid(33, [215.0, 70.0, 20.0, 20.0], [0.2, 0.2, 0.2, 1.0])
                .with_corner_radius(6.0)
                .with_border(2.0, [1.0, 1.0, 1.0, 1.0]),
        ];

        let blur_req = BlurRequests::from_glass_rects(&glass_rects);
        let mut enc1 = ctx.device.create_command_encoder(&Default::default());
        compositor.compose(
            &ctx.device,
            &ctx.queue,
            &mut enc1,
            &target_view,
            ClearContext::default(),
            SceneSource::ClearOnly,
            blur_req,
            &glass_rects,
            &ui_quads,
            0.5,
        );
        ctx.queue.submit(Some(enc1.finish()));
        let frame1 = ctx.readback_texture(&target_tex, w, h).expect("readback 1");

        // Outside all rects -> clear #0c0d12
        assert_eq!(pixel_at(&frame1, w, 5, 5), [12, 13, 18, 255]);
        // At (42, 42) -> z=10 red card outside z=20 glass & its downward drop shadow
        let red_px = pixel_at(&frame1, w, 42, 42);
        assert!(
            red_px[0] > 230 && red_px[1] < 20,
            "z=10 solid expected red, got {red_px:?}"
        );
        // At (65, 80) -> z=20 glass downward/side drop shadow softens z=10 red underneath
        let shadow_px = pixel_at(&frame1, w, 65, 80);
        assert!(
            shadow_px[0] > 100 && shadow_px[0] < 235 && shadow_px[1] < 20,
            "expected z=20 drop shadow over z=10 red, got {shadow_px:?}"
        );
        // At (90, 80) -> z=20 glass over z=10 red (must be dark blue-tinted Chameleon, NOT pure red)
        let glass_px = pixel_at(&frame1, w, 90, 80);
        assert!(
            glass_px[0] < 80,
            "z=20 glass must occlude z=10 red, got {glass_px:?}"
        );
        // At (112, 82) -> z=30 yellow glyph mask on top of z=20 glass
        let glyph_px = pixel_at(&frame1, w, 112, 82);
        assert!(
            glyph_px[0] > 220 && glyph_px[1] > 200 && glyph_px[2] < 60,
            "z=30 glyph expected yellow, got {glyph_px:?}"
        );
        // At (152, 82) -> z=31 premultiplied SVG green blended cleanly (no black fringe)
        let svg_px = pixel_at(&frame1, w, 152, 82);
        assert!(
            svg_px[1] > 110 && svg_px[0] < 40,
            "z=31 SVG expected green, got {svg_px:?}"
        );
        // At (192, 82) -> z=32 straight RGBA blue thumb on top
        let img_px = pixel_at(&frame1, w, 192, 82);
        assert!(
            img_px[2] > 220 && img_px[0] < 50,
            "z=32 RGBA quad expected blue, got {img_px:?}"
        );

        // Determinism check
        let mut enc2 = ctx.device.create_command_encoder(&Default::default());
        compositor.compose(
            &ctx.device,
            &ctx.queue,
            &mut enc2,
            &target_view,
            ClearContext::default(),
            SceneSource::ClearOnly,
            blur_req,
            &glass_rects,
            &ui_quads,
            0.5,
        );
        ctx.queue.submit(Some(enc2.finish()));
        let frame2 = ctx.readback_texture(&target_tex, w, h).expect("readback 2");
        assert_eq!(frame1, frame2);

        assert!(compositor.remove_texture(glyph_id));
        assert!(!compositor.remove_texture(UiTextureId::WHITE));
    });
}

#[test]
fn frame_compositor_clipping_and_accordion_preview_resize() {
    block_on(async {
        let ctx = GpuContext::new_headless().await.expect("headless wgpu");
        let (mut w, mut h) = (320, 180);
        let (target_tex, target_view) = create_target(&ctx.device, w, h);

        let mut compositor = FrameCompositor::new(
            &ctx.device,
            &ctx.queue,
            wgpu::TextureFormat::Rgba8Unorm,
            w,
            h,
            EffectInstanceConfig::default(),
        )
        .expect("FrameCompositor");

        // Render 400x240 @ 30fps previews
        compositor.previews.bundle.dither.set_time(1.0);
        let mut enc_prev = ctx.device.create_command_encoder(&Default::default());
        let due = compositor.update_previews(&ctx.device, &ctx.queue, &mut enc_prev, 0.0, 0.016);
        assert_eq!(due, [true, true, true, true]);
        ctx.queue.submit(Some(enc_prev.finish()));

        // Rounded clip around (60, 40, 120, 80) with r=24px, clipping both a larger solid rotated quad and an SdfGlassRect
        let clip_rect = [60.0, 40.0, 120.0, 80.0];
        let clip_radii = [24.0; 4];

        let glass_rects = [SdfGlassRect::new(
            5,
            GlassRect::new(20.0, 10.0, 260.0, 160.0, 0.0, ThemeMode::Light),
        )
        .with_clip_rect(clip_rect)];

        let dither_preview_id =
            FrameCompositor::preview_texture_id(WelcomeBackgroundKind::Dither).expect("preview id");
        let ui_quads = [
            UiTexturedQuad::solid(10, [20.0, 10.0, 260.0, 160.0], [1.0, 0.2, 0.2, 1.0])
                .with_rotation_deg(15.0)
                .with_rounded_clip(clip_rect, clip_radii),
            UiTexturedQuad::textured(20, [200.0, 20.0, 100.0, 60.0], dither_preview_id)
                .with_corner_radius(8.0),
        ];

        let mut enc = ctx.device.create_command_encoder(&Default::default());
        compositor.compose(
            &ctx.device,
            &ctx.queue,
            &mut enc,
            &target_view,
            ClearContext::default(),
            SceneSource::ClearOnly,
            BlurRequests::none(),
            &glass_rects,
            &ui_quads,
            0.0,
        );
        ctx.queue.submit(Some(enc.finish()));
        let px = ctx
            .readback_texture(&target_tex, w, h)
            .expect("readback clip");

        // Outside clip rect (e.g. x=30, y=20 inside the unclipped 20..280 box) must remain #0c0d12
        assert_eq!(pixel_at(&px, w, 30, 20), [12, 13, 18, 255]);
        // Top-left corner INSIDE rect (61, 41) is outside r=24 rounded clip corner, whereas SdfGlassRect at z=5 has rect scissor so (61,41) has glass, NOT z=10 red!
        let corner_px = pixel_at(&px, w, 61, 41);
        assert!(
            corner_px[1] > 180,
            "rounded clip on z=10 red must reveal z=5 light glass at corner, got {corner_px:?}"
        );
        // Center of clip region (120, 80) has z=10 red quad
        let center_px = pixel_at(&px, w, 120, 80);
        assert!(
            center_px[0] > 230 && center_px[1] < 80,
            "center inside clip must show z=10 red, got {center_px:?}"
        );

        // Preview quad rendered at (200..300, 20..80) must be non-blank
        let mut preview_non_clear = 0usize;
        for py in 24..76 {
            for px_x in 204..296 {
                if pixel_at(&px, w, px_x, py) != [12, 13, 18, 255] {
                    preview_non_clear += 1;
                }
            }
        }
        assert!(
            preview_non_clear > 200,
            "accordion preview quad rendered blank"
        );

        // Surface resize
        w = 360;
        h = 200;
        assert!(compositor.resize(&ctx.device, w, h).expect("resize"));
        assert!(!compositor
            .resize(&ctx.device, w, h)
            .expect("idempotent resize"));
        let (resized_tex, resized_view) = create_target(&ctx.device, w, h);
        let mut enc_res = ctx.device.create_command_encoder(&Default::default());
        compositor.compose(
            &ctx.device,
            &ctx.queue,
            &mut enc_res,
            &resized_view,
            ClearContext {
                welcome_active: true,
                ..Default::default()
            },
            SceneSource::ClearOnly,
            BlurRequests::none(),
            &[],
            &[UiTexturedQuad::solid(
                0,
                [10.0, 10.0, 40.0, 40.0],
                [0.0, 1.0, 0.5, 1.0],
            )],
            0.0,
        );
        ctx.queue.submit(Some(enc_res.finish()));
        let px_res = ctx
            .readback_texture(&resized_tex, w, h)
            .expect("resized readback");
        assert_eq!(pixel_at(&px_res, w, 5, 5), [0, 0, 0, 255]);
        let green_px = pixel_at(&px_res, w, 25, 25);
        assert!(green_px[1] > 240 && green_px[2] > 110);
    });
}

#[test]
fn medium_defaults_faulty_orientation_and_live_updates() {
    block_on(async {
        let ctx = GpuContext::new_headless().await.expect("headless wgpu");
        let (w, h) = (320, 180);
        let (target_tex, target_view) = create_target(&ctx.device, w, h);

        let mut graph = RenderGraph::new(
            &ctx.device,
            &ctx.queue,
            wgpu::TextureFormat::Rgba8Unorm,
            w,
            h,
            EffectInstanceConfig {
                width: w,
                height: h,
                dpr: 1.0,
                fps_limit: None,
                is_preview: false,
                pause_when_unfocused: true,
            },
        )
        .expect("render graph");

        // 1. Default full-bleed presets must match Electron WELCOME_QUALITY_DEFAULT = 'medium'
        assert_eq!(graph.effects.dither.config.pixel_size, 3.0);
        assert!((graph.effects.dither.config.wave_frequency - 2.85).abs() < 1e-5);
        assert!((graph.effects.dither.config.wave_amplitude - 0.29).abs() < 1e-5);
        assert_eq!(graph.effects.particles.config.particle_count, 210);
        assert_eq!(graph.effects.particles.config.particle_base_size, 95.0);
        assert_eq!(graph.effects.faulty.config.scale, 1.2);
        assert!((graph.effects.faulty.config.glitch_amount - 0.85).abs() < 1e-5);
        assert!((graph.effects.faulty.config.scanline_intensity - 0.6).abs() < 1e-5);
        assert!((graph.effects.faulty.config.curvature - 0.18).abs() < 1e-5);
        assert!((graph.effects.faulty.config.brightness - 0.8).abs() < 1e-5);
        assert_eq!(graph.effects.grid_motion.config.speed, 1.0);
        assert_eq!(graph.effects.grid_motion.config.max_move, 300.0);

        // 2. FaultyTerminal bottom-up vUv parity (no double flip):
        // mouse_uv = [0.2, 0.85] (bottom-up Y -> upper-left) must illuminate top half > bottom half.
        graph.effects.faulty.config.curvature = 0.0;
        graph.effects.faulty.config.noise_amp = 0.0;
        graph.effects.faulty.config.mouse_react = true;
        graph.effects.faulty.config.mouse_strength = 0.8;
        graph
            .effects
            .faulty
            .set_time_and_mouse(1.0, [0.2, 0.85], 1.0);

        let mut enc = ctx.device.create_command_encoder(&Default::default());
        graph.encode(
            &ctx.device,
            &ctx.queue,
            &mut enc,
            &target_view,
            ClearContext {
                welcome_active: true,
                ..Default::default()
            },
            SceneSource::Welcome(WelcomeBackgroundKind::Faulty),
            &[],
            0.0,
        );
        ctx.queue.submit(Some(enc.finish()));
        let px = ctx
            .readback_texture(&target_tex, w, h)
            .expect("faulty readback");

        let mut top_left_sum: u64 = 0;
        let mut bottom_left_sum: u64 = 0;
        for y in 0..h {
            for x in 0..(w / 2) {
                let p = pixel_at(&px, w, x, y);
                let luma = p[0] as u64 + p[1] as u64 + p[2] as u64;
                if y < h / 2 {
                    top_left_sum += luma;
                } else {
                    bottom_left_sum += luma;
                }
            }
        }
        assert!(
            top_left_sum > bottom_left_sum * 3,
            "expected bottom-up mouse_uv [0.2, 0.85] to brighten top-left ({top_left_sum}) >> bottom-left ({bottom_left_sum})"
        );

        // 3. Live config update via apply_configs & particles rebuild
        let mut p_cfg = ParticlesConfig::for_quality(QualityTier::High);
        p_cfg.particle_colors = vec![[1.0, 0.2, 0.2]];
        graph.effects.apply_configs(
            &ctx.device,
            &ctx.queue,
            DitherWavesConfig::for_quality(QualityTier::Low),
            p_cfg,
            FaultyTerminalConfig::for_quality(QualityTier::Medium),
            GridMotionConfig::for_quality(QualityTier::High),
        );
        assert_eq!(graph.effects.particles.particle_count(), 300);
        assert_eq!(graph.effects.dither.config.pixel_size, 4.0);
        assert_eq!(graph.effects.grid_motion.config.max_move, 450.0);
    });
}

#[test]
fn workstream_d2_light_chameleon_particles_dpr_faulty_progress_and_grid_hit_test() {
    // 1. Light theme chameleon override applies across ALL chameleon skins
    for skin in [
        ChameleonSkin::Dither,
        ChameleonSkin::Faulty,
        ChameleonSkin::Particles,
        ChameleonSkin::Gridmotion,
    ] {
        let glass = GlassRect::new(0.0, 0.0, 200.0, 100.0, 16.0, ThemeMode::Light)
            .with_chameleon_skin(skin, ThemeMode::Light);
        assert!((glass.fill_color_0[0] - 248.0 / 255.0).abs() < 1e-4);
        assert!((glass.fill_color_0[1] - 249.0 / 255.0).abs() < 1e-4);
        assert!((glass.fill_color_0[2] - 252.0 / 255.0).abs() < 1e-4);
        assert!((glass.fill_color_0[3] - 0.94).abs() < 1e-4);
        assert!((glass.border_color_0[3] - 0.12).abs() < 1e-4);
    }

    // 2. Particles DPR cap (`particles.js:129` <= 0.8 / 1.0 / 1.4)
    assert!((QualityTier::Low.cap_particles_dpr(3.0) - 0.8).abs() < 1e-5);
    assert!((QualityTier::Medium.cap_particles_dpr(3.0) - 1.0).abs() < 1e-5);
    assert!((QualityTier::High.cap_particles_dpr(3.0) - 1.4).abs() < 1e-5);

    // 3. Faulty page_load_progress default, preview_preset, randomize, and GPU hit test
    let def = FaultyTerminalConfig::default();
    assert!((def.page_load_progress - 0.0).abs() < 1e-5);
    assert!(def.page_load_animation);
    assert!(
        (FaultyTerminalConfig::for_quality(QualityTier::Medium).page_load_progress - 0.0).abs()
            < 1e-5
    );
    assert!((FaultyTerminalConfig::preview_preset().page_load_progress - 1.0).abs() < 1e-5);

    block_on(async {
        let ctx = GpuContext::new_headless().await.expect("headless wgpu");
        let mut graph = RenderGraph::new(
            &ctx.device,
            &ctx.queue,
            wgpu::TextureFormat::Rgba8Unorm,
            1280,
            720,
            EffectInstanceConfig {
                width: 1280,
                height: 720,
                dpr: 1.0,
                fps_limit: None,
                is_preview: false,
                pause_when_unfocused: true,
            },
        )
        .expect("render graph");

        graph.effects.faulty.config.page_load_animation = true;
        graph.effects.faulty.randomize_time_offset();
        assert_eq!(graph.effects.faulty.config.page_load_progress, 0.0);

        // Even row count (4) places an inter-row gap at exact center `(640, 360)`,
        // while offsetting upward to row 1 hits a card directly.
        assert!(graph
            .effects
            .grid_motion
            .hit_test_card(640.0, 360.0, 1280.0, 720.0)
            .is_none());
        let hit = graph
            .effects
            .grid_motion
            .hit_test_card(640.0, 300.0, 1280.0, 720.0);
        assert!(
            hit.is_some(),
            "point inside row 1 of GridMotion should hit a card"
        );
    });
}
