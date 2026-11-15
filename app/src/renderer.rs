//! `winit` desktop window + `wgpu`/`FrameCompositor` UI/video/shader compositing,
//! AccessKit tree bridging, Win32 subclass hit-test sync, and `--headless-snapshot` PNG export.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use accesskit::{Action, ActionData, ActionRequest};
use accesskit_winit::Adapter as AccessKitAdapter;
use hybrid_core::binary_resolver::BinarySearchPaths;
use hybrid_shaders::{
    BlurRequests, EffectInstanceConfig, FrameCompositor, GpuContext, SceneSource, SdfGlassRect,
    ThemeMode, UiSampleMode, UiTextureId, UiTexturedQuad, VideoFitMode, VideoFrameTransform,
    WelcomeBackgroundKind,
};
use hybrid_ui::fonts::{FontRole, TextEngine};
use hybrid_ui::icons::{SvgRasterizer, SvgVariant};
use hybrid_ui::primitives::{CursorHint, HitTarget, RectPx, TextAlign, UiFrameOutput, UiTextRun};
use hybrid_ui::state::{
    BgControlInputType, CarouselMode, ModalId, RecordingIndicatorMode, ToastItem, ToastKind,
    UiState, WindowMaximizeState, BG_CONTROLS_CATALOG, PLAYLIST_ITEM_HEIGHT,
};
use hybrid_ui::svg_catalog::SvgId;
use hybrid_ui::tokens::ZIndex;
use hybrid_ui::HybridUi;
use hybrid_win32_shell::{
    apply_null_brush, extract_hwnd_from_winit, DipRect, ShellWindowEvent, SingleInstanceServer,
    ThemeMode as ShellThemeMode, WindowSubclass,
};
use tokio::runtime::Runtime;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::window::{CursorIcon, Window, WindowAttributes, WindowId};

use crate::cli::{AppCliArgs, HeadlessSnapshotOptions, SnapshotOverlayKind};
use crate::input::{
    build_combo_string, classify_dropped_files, DroppedFilesPlan, InputAction, InputContext,
    InputController, KeyModifiers,
};
use crate::media_bridge::{FocusedTextField, MediaBridge};

#[inline]
fn rect_arr(r: RectPx) -> [f32; 4] {
    [r.x, r.y, r.width, r.height]
}

fn to_shell_theme(theme: ThemeMode) -> ShellThemeMode {
    match theme {
        ThemeMode::Dark => ShellThemeMode::Dark,
        ThemeMode::Oled => ShellThemeMode::Oled,
        ThemeMode::Light => ShellThemeMode::Light,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SvgCacheKey {
    id: SvgId,
    variant: SvgVariant,
    w_px: u32,
    h_px: u32,
    tint: [u8; 4],
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TextCacheKey {
    text: String,
    role_id: u8,
    font_size_bits: u32,
    line_height_bits: u32,
    weight: u16,
    letter_spacing_bits: u32,
    color_u8: [u8; 4],
    box_w_px: u32,
    box_h_px: u32,
    align_id: u8,
    allow_wrap: bool,
    brand_font: bool,
}

/// Bridges `HybridUi` primitives (`TextEngine`, `SvgRasterizer`, `UiFrameOutput`) onto
/// `hybrid_shaders::FrameCompositor` (`SdfGlassRect` + `UiTexturedQuad`).
pub struct UiGpuBridge {
    pub compositor: FrameCompositor,
    pub text_engine: TextEngine,
    pub svg_rasterizer: SvgRasterizer,
    svg_textures: HashMap<SvgCacheKey, UiTextureId>,
    text_textures: HashMap<TextCacheKey, Option<(UiTextureId, u32, u32)>>,
    image_textures: HashMap<String, (UiTextureId, u32, u32)>,
    image_key_order: VecDeque<String>,
    last_welcome_bg: Option<WelcomeBackgroundKind>,
}

impl UiGpuBridge {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        present_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        instance: EffectInstanceConfig,
    ) -> Result<Self, String> {
        let compositor =
            FrameCompositor::new(device, queue, present_format, width, height, instance)
                .map_err(|e| format!("FrameCompositor::new: {e}"))?;
        Ok(Self {
            compositor,
            text_engine: TextEngine::new(),
            svg_rasterizer: SvgRasterizer::new(),
            svg_textures: HashMap::new(),
            text_textures: HashMap::new(),
            image_textures: HashMap::new(),
            image_key_order: VecDeque::new(),
            last_welcome_bg: None,
        })
    }

    pub fn has_image(&self, key: &str) -> bool {
        self.image_textures.contains_key(key)
    }

    pub fn remove_image(&mut self, key: &str) {
        if let Some((old_id, _, _)) = self.image_textures.remove(key) {
            self.compositor.remove_texture(old_id);
        }
        self.image_key_order.retain(|k| k != key);
    }

    /// Uploads or replaces an RGBA8 image texture for a [`hybrid_ui::primitives::UiImageSlot`] key.
    pub fn set_image_rgba(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: &str,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) {
        if width == 0 || height == 0 || rgba.len() < (width * height * 4) as usize {
            return;
        }
        if let Some((old_id, _, _)) = self.image_textures.remove(key) {
            self.compositor.remove_texture(old_id);
        }
        self.image_key_order.retain(|k| k != key);
        let tex_id = self
            .compositor
            .upload_rgba8(device, queue, key, width, height, rgba);
        self.image_textures
            .insert(key.to_string(), (tex_id, width, height));
        self.image_key_order.push_back(key.to_string());
        while self.image_textures.len() > 64 {
            if let Some(evicted_key) = self.image_key_order.pop_front() {
                if let Some((old_id, _, _)) = self.image_textures.remove(&evicted_key) {
                    self.compositor.remove_texture(old_id);
                }
            }
        }
    }

    /// Syncs live `UiState` shader configs and optional GridMotion custom pictures into `FrameCompositor`.
    pub fn sync_welcome_effects(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        ui_state: &UiState,
        custom_pictures: &[String],
        custom_pics_dirty: bool,
    ) {
        if self
            .last_welcome_bg
            .is_some_and(|prev| prev != ui_state.welcome_background)
            && ui_state.welcome_background == WelcomeBackgroundKind::Faulty
        {
            self.compositor
                .graph
                .effects
                .faulty
                .restart_load_animation();
        }
        self.last_welcome_bg = Some(ui_state.welcome_background);

        let effects = &mut self.compositor.graph.effects;
        let mut faulty = ui_state.faulty_cfg.clone();
        faulty.page_load_progress = effects.faulty.config.page_load_progress;
        let mut gridmotion = ui_state.gridmotion_cfg.clone();
        gridmotion.brand_font_enabled = ui_state.font_config.brand_font_enabled;

        effects.apply_configs(
            device,
            queue,
            ui_state.dither_cfg.clone(),
            ui_state.particles_cfg.clone(),
            faulty,
            gridmotion,
        );

        let preview_bundle = &mut self.compositor.previews.bundle;
        preview_bundle.faulty.config.curvature = if ui_state.faulty_cfg.curvature > 0.0 {
            ui_state.faulty_cfg.curvature
        } else {
            0.14
        };
        preview_bundle.faulty.config.glitch_amount = ui_state.faulty_cfg.glitch_amount;
        preview_bundle.faulty.config.scanline_intensity = ui_state.faulty_cfg.scanline_intensity;
        preview_bundle.faulty.config.flicker_amount = ui_state.faulty_cfg.flicker_amount;
        preview_bundle.faulty.config.chromatic_aberration =
            ui_state.faulty_cfg.chromatic_aberration;
        preview_bundle.faulty.config.brightness = ui_state.faulty_cfg.brightness;
        preview_bundle.faulty.config.tint = ui_state.faulty_cfg.tint;
        preview_bundle.grid_motion.config.brand_font_enabled =
            ui_state.font_config.brand_font_enabled;

        if custom_pics_dirty {
            if custom_pictures.is_empty() {
                effects.grid_motion.clear_custom_images();
                preview_bundle.grid_motion.clear_custom_images();
            } else {
                effects
                    .grid_motion
                    .load_image_sources_async(custom_pictures.to_vec());
                preview_bundle
                    .grid_motion
                    .load_image_sources_async(custom_pictures.to_vec());
            }
        }
    }

    pub fn update_instance_dpr(&mut self, width: u32, height: u32, dpr: f32) {
        let effects = &mut self.compositor.graph.effects;
        effects.set_dpr(dpr);
        for inst in [
            &mut effects.dither.instance,
            &mut effects.particles.instance,
            &mut effects.faulty.instance,
            &mut effects.grid_motion.instance,
        ] {
            inst.width = width.max(1);
            inst.height = height.max(1);
        }
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let _ = self.compositor.resize(device, width.max(1), height.max(1));
    }

    /// Converts a logical `UiFrameOutput` at DPI factor `dpr` and encodes Pass 0..2 into `target_view`.
    #[allow(clippy::too_many_arguments)]
    pub fn encode_frame(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target_view: &wgpu::TextureView,
        ui_state: &UiState,
        frame: &UiFrameOutput,
        scene_source: SceneSource<'_>,
        dpr: f32,
        now_secs: f64,
        dt_secs: f32,
    ) {
        if ui_state.welcome_active && dt_secs > 0.0 {
            self.compositor
                .graph
                .effects
                .update(ui_state.welcome_background, dt_secs);
        }
        if ui_state.bg_accordion_open
            || (ui_state.anim.bg_accordion_closing && ui_state.anim.bg_accordion_t < 1.0)
        {
            self.compositor.update_previews(
                device,
                queue,
                encoder,
                now_secs,
                dt_secs.max(1.0 / 60.0),
            );
        }

        let (glass_rects, ui_quads) =
            self.build_gpu_primitives(device, queue, ui_state, frame, dpr);
        let blur_req = BlurRequests::from_glass_rects(&glass_rects);
        self.compositor.compose(
            device,
            queue,
            encoder,
            target_view,
            frame.clear_context,
            scene_source,
            blur_req,
            &glass_rects,
            &ui_quads,
            now_secs as f32,
        );
    }

    fn build_gpu_primitives(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        ui_state: &UiState,
        frame: &UiFrameOutput,
        dpr: f32,
    ) -> (Vec<SdfGlassRect>, Vec<UiTexturedQuad>) {
        // Bound GPU texture cache growth over very long sessions.
        if self.text_textures.len() > 2048 {
            for (_, val) in self.text_textures.drain() {
                if let Some((tex_id, _, _)) = val {
                    self.compositor.remove_texture(tex_id);
                }
            }
        }

        let glass_rects: Vec<SdfGlassRect> = frame
            .glass_panels
            .iter()
            .map(|p| {
                SdfGlassRect {
                    z_index: p.z_index,
                    glass: p.glass.clone(),
                    clip_rect: p.clip_rect.map(rect_arr),
                }
                .scaled(dpr)
            })
            .collect();

        let mut quads = Vec::with_capacity(
            frame.shader_slots.len()
                + frame.solid_rects.len()
                + frame.image_slots.len()
                + frame.svg_quads.len()
                + frame.text_runs.len(),
        );

        for slot in &frame.shader_slots {
            if slot.is_preview {
                let opacity = slot.opacity.clamp(0.0, 1.0);
                if opacity <= 0.001 {
                    continue;
                }
                if let Some(tex_id) = FrameCompositor::preview_texture_id(slot.kind) {
                    let rx = (slot.rect.x * dpr).round() / dpr;
                    let ry = (slot.rect.y * dpr).round() / dpr;
                    let border_1px = (1.0 * dpr).floor().max(1.0) / dpr;
                    let inner_r = (12.0 - border_1px).max(0.0);
                    let snapped = [rx, ry, slot.rect.width, slot.rect.height];
                    let mut q = UiTexturedQuad::textured(slot.z_index, snapped, tex_id)
                        .with_corner_radius(inner_r)
                        .with_tint([1.0, 1.0, 1.0, opacity]);
                    if let Some(clip) = slot.clip_rect {
                        q = q.with_rounded_clip(rect_arr(clip), [inner_r; 4]);
                    }
                    quads.push(q.scaled(dpr));
                }
            }
        }

        for r in &frame.solid_rects {
            let mut q = if let Some((end_color, angle_deg)) = r.gradient {
                UiTexturedQuad::gradient(r.z_index, rect_arr(r.rect), r.color, end_color, angle_deg)
            } else {
                UiTexturedQuad::solid(r.z_index, rect_arr(r.rect), r.color)
            }
            .with_corner_radii(r.corner_radii)
            .with_border(r.border_width, r.border_color);
            if let Some(clip) = r.clip_rect {
                q = q.with_clip_rect(rect_arr(clip));
            }
            quads.push(q.scaled(dpr));
        }

        for slot in &frame.image_slots {
            let tex_entry = self.image_textures.get(&slot.key).or_else(|| {
                slot.key
                    .starts_with("hover-thumb:")
                    .then(|| self.image_textures.get("hover-thumb:latest"))
                    .flatten()
            });
            if let Some(&(tex_id, _, _)) = tex_entry {
                let mut q = UiTexturedQuad::textured(slot.z_index, rect_arr(slot.rect), tex_id)
                    .with_tint([1.0, 1.0, 1.0, slot.opacity])
                    .with_corner_radius(slot.corner_radius);
                if let Some(clip) = slot.clip_rect {
                    q = q.with_clip_rect(rect_arr(clip));
                }
                quads.push(q.scaled(dpr));
            }
        }

        for svg in &frame.svg_quads {
            let w_px = (svg.rect.width * dpr).round().max(1.0) as u32;
            let h_px = (svg.rect.height * dpr).round().max(1.0) as u32;
            let key = SvgCacheKey {
                id: svg.svg,
                variant: svg.variant,
                w_px,
                h_px,
                tint: svg.tint_rgba,
            };
            let tex_id = if let Some(&cached) = self.svg_textures.get(&key) {
                cached
            } else if let Ok(rasterized) =
                self.svg_rasterizer
                    .rasterize(svg.svg, svg.variant, w_px, h_px, svg.tint_rgba)
            {
                let id = self.compositor.upload_rgba8(
                    device,
                    queue,
                    &svg.id,
                    rasterized.width,
                    rasterized.height,
                    &rasterized.rgba,
                );
                self.svg_textures.insert(key, id);
                id
            } else {
                continue;
            };

            let mut q = UiTexturedQuad::premultiplied_svg(
                svg.z_index,
                rect_arr(svg.rect),
                tex_id,
                [1.0, 1.0, 1.0, svg.opacity],
            )
            .with_sample_mode(UiSampleMode::PremultipliedRgba)
            .with_rotation_deg(svg.rotation_deg);
            if let Some(clip) = svg.clip_rect {
                q = q.with_clip_rect(rect_arr(clip));
            }
            quads.push(q.scaled(dpr));
        }

        for run in &frame.text_runs {
            if let Some(mut q) = self.rasterize_text_run_quad(device, queue, ui_state, run, dpr) {
                if let Some(clip) = run.clip_rect {
                    let scaled_clip = [
                        clip.x * dpr,
                        clip.y * dpr,
                        clip.width * dpr,
                        clip.height * dpr,
                    ];
                    q = q.with_clip_rect(scaled_clip);
                }
                quads.push(q);
            }
        }

        (glass_rects, quads)
    }

    fn rasterize_text_run_quad(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        ui_state: &UiState,
        run: &UiTextRun,
        dpr: f32,
    ) -> Option<UiTexturedQuad> {
        if run.text.is_empty() || run.rect.width <= 0.5 || run.rect.height <= 0.5 {
            return None;
        }
        let box_w_px = (run.rect.width * dpr).ceil().max(1.0) as u32;
        let box_h_px = (run.rect.height * dpr).ceil().max(1.0) as u32;
        let color_u8 = [
            (run.color[0].clamp(0.0, 1.0) * 255.0).round() as u8,
            (run.color[1].clamp(0.0, 1.0) * 255.0).round() as u8,
            (run.color[2].clamp(0.0, 1.0) * 255.0).round() as u8,
            (run.color[3].clamp(0.0, 1.0) * 255.0).round() as u8,
        ];
        if color_u8[3] == 0 {
            return None;
        }

        let role_id = match run.font_role {
            FontRole::Display => 0,
            FontRole::Ui => 1,
            FontRole::Mono => 2,
        };
        let align_id = match run.align {
            TextAlign::Left => 0,
            TextAlign::Center => 1,
            TextAlign::Right => 2,
        };
        let font_px = (run.font_size_px * dpr).max(1.0);
        let line_px = (run.line_height_px * dpr).max(1.0);
        let letter_spacing_scaled = run.letter_spacing_px * dpr;

        let key = TextCacheKey {
            text: run.text.clone(),
            role_id,
            font_size_bits: font_px.to_bits(),
            line_height_bits: line_px.to_bits(),
            weight: run.weight,
            letter_spacing_bits: letter_spacing_scaled.to_bits(),
            color_u8,
            box_w_px,
            box_h_px,
            align_id,
            allow_wrap: run.allow_wrap,
            brand_font: ui_state.font_config.brand_font_enabled,
        };

        let snapped_x = (run.rect.x * dpr).round();
        let snapped_y = (run.rect.y * dpr).round();

        if let Some(cached) = self.text_textures.get(&key) {
            let (tex_id, tw, th) = (*cached)?;
            return Some(UiTexturedQuad::textured(
                run.z_index,
                [snapped_x, snapped_y, tw as f32, th as f32],
                tex_id,
            ));
        }

        let max_w = if run.rect.width > 1.0 {
            Some(box_w_px as f32)
        } else {
            None
        };
        let metrics = self.text_engine.measure_with_options(
            &run.text,
            font_px,
            line_px,
            max_w,
            run.font_role,
            run.weight,
            ui_state.font_config,
            letter_spacing_scaled,
            run.allow_wrap,
        );
        let glyphs = self.text_engine.rasterize_with_options(
            &run.text,
            font_px,
            line_px,
            max_w,
            run.font_role,
            run.weight,
            color_u8,
            ui_state.font_config,
            letter_spacing_scaled,
            run.allow_wrap,
        );

        if glyphs.is_empty() {
            self.text_textures.insert(key, None);
            return None;
        }

        let x_offset = match run.align {
            TextAlign::Left => 0,
            TextAlign::Center => {
                (((box_w_px as f32 - metrics.width).max(0.0)) * 0.5).round() as i32
            }
            TextAlign::Right => (box_w_px as f32 - metrics.width).max(0.0).round() as i32,
        };
        let y_offset = (((box_h_px as f32 - metrics.height).max(0.0)) * 0.5).round() as i32;

        let mut bitmap = vec![0u8; (box_w_px as usize) * (box_h_px as usize) * 4];
        for g in glyphs {
            let dst_x0 = g.x + x_offset;
            let dst_y0 = g.y + y_offset;
            for gy in 0..g.height as i32 {
                let py = dst_y0 + gy;
                if py < 0 || py >= box_h_px as i32 {
                    continue;
                }
                for gx in 0..g.width as i32 {
                    let px = dst_x0 + gx;
                    if px < 0 || px >= box_w_px as i32 {
                        continue;
                    }
                    let src_idx = ((gy as u32 * g.width + gx as u32) * 4) as usize;
                    let dst_idx = ((py as u32 * box_w_px + px as u32) * 4) as usize;
                    let sa = g.data[src_idx + 3];
                    if sa == 0 {
                        continue;
                    }
                    let da = bitmap[dst_idx + 3];
                    if g.is_color {
                        if da == 0 {
                            bitmap[dst_idx..dst_idx + 4]
                                .copy_from_slice(&g.data[src_idx..src_idx + 4]);
                        } else {
                            let sa_f = sa as f32 / 255.0;
                            let da_f = da as f32 / 255.0;
                            let out_a = sa_f + da_f * (1.0 - sa_f);
                            if out_a > 1e-4 {
                                for c in 0..3 {
                                    let sc = g.data[src_idx + c] as f32 / 255.0;
                                    let dc = bitmap[dst_idx + c] as f32 / 255.0;
                                    let out_c = (sc * sa_f + dc * da_f * (1.0 - sa_f)) / out_a;
                                    bitmap[dst_idx + c] =
                                        (out_c.clamp(0.0, 1.0) * 255.0).round() as u8;
                                }
                            }
                            bitmap[dst_idx + 3] = (out_a.clamp(0.0, 1.0) * 255.0).round() as u8;
                        }
                    } else {
                        // Monochrome tinted glyphs share RGB = color_u8; composite coverage cleanly
                        let out_a = (sa as u16 + da as u16 - ((sa as u16 * da as u16) / 255))
                            .min(255) as u8;
                        bitmap[dst_idx] = g.data[src_idx];
                        bitmap[dst_idx + 1] = g.data[src_idx + 1];
                        bitmap[dst_idx + 2] = g.data[src_idx + 2];
                        bitmap[dst_idx + 3] = out_a;
                    }
                }
            }
        }

        let tex_id = self
            .compositor
            .upload_rgba8(device, queue, &run.id, box_w_px, box_h_px, &bitmap);
        self.text_textures
            .insert(key, Some((tex_id, box_w_px, box_h_px)));
        Some(UiTexturedQuad::textured(
            run.z_index,
            [snapped_x, snapped_y, box_w_px as f32, box_h_px as f32],
            tex_id,
        ))
    }
}

fn upload_rgba_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> wgpu::Texture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    texture
}

fn create_smpte_sample_rgba(width: u32, height: u32) -> Vec<u8> {
    const TOP_BARS: [[u8; 4]; 7] = [
        [191, 191, 191, 255],
        [191, 191, 0, 255],
        [0, 191, 191, 255],
        [0, 191, 0, 255],
        [191, 0, 191, 255],
        [191, 0, 0, 255],
        [0, 0, 191, 255],
    ];
    const MID_BARS: [[u8; 4]; 7] = [
        [0, 0, 191, 255],
        [19, 19, 19, 255],
        [191, 0, 191, 255],
        [19, 19, 19, 255],
        [0, 191, 191, 255],
        [19, 19, 19, 255],
        [191, 191, 191, 255],
    ];

    let mut rgba = vec![0u8; (width * height * 4) as usize];
    let top_h = (height * 2) / 3;
    let mid_h = (height * 3) / 4;

    for y in 0..height {
        for x in 0..width {
            let col = ((x * 7) / width.max(1)).min(6) as usize;
            let px = if y < top_h {
                TOP_BARS[col]
            } else if y < mid_h {
                MID_BARS[col]
            } else {
                let ramp = ((x * 255) / width.max(1)) as u8;
                match (x * 4) / width.max(1) {
                    0 => [0, 33, 76, 255],
                    1 => [255, 255, 255, 255],
                    2 => [50, 0, 106, 255],
                    _ => [ramp, ramp, ramp, 255],
                }
            };
            let idx = ((y * width + x) * 4) as usize;
            rgba[idx..idx + 4].copy_from_slice(&px);
        }
    }
    rgba
}

fn find_sample_media_path(snapshot_media: Option<&std::path::Path>) -> Option<PathBuf> {
    if let Some(p) = snapshot_media {
        if p.exists() {
            return Some(p.to_path_buf());
        }
    }
    let mut roots = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut cur = exe.parent();
        for _ in 0..5 {
            if let Some(dir) = cur {
                roots.push(dir.to_path_buf());
                cur = dir.parent();
            }
        }
    }
    for root in roots {
        let candidate = root.join("hybrid-player").join("test-sample.mp4");
        if candidate.exists() {
            return Some(candidate);
        }
        let direct = root.join("test-sample.mp4");
        if direct.exists() {
            return Some(direct);
        }
    }
    None
}

fn find_ffmpeg_for_snapshot() -> Option<PathBuf> {
    let search = BinarySearchPaths::from_env();
    if let Some(bin) = hybrid_core::binary_resolver::resolve_ffmpeg_binary(
        search.resources_dir.as_deref(),
        search.exec_path.as_deref(),
        search.app_dir.as_deref(),
        hybrid_core::binary_resolver::ResolveOptions::for_runtime(),
    ) {
        return Some(bin);
    }

    let exe = std::env::current_exe().ok();
    let cwd = std::env::current_dir().ok();
    let mut roots = Vec::new();
    if let Some(c) = cwd {
        roots.push(c);
    }
    if let Some(mut dir) = exe.as_deref().and_then(|p| p.parent()) {
        for _ in 0..5 {
            roots.push(dir.to_path_buf());
            match dir.parent() {
                Some(parent) => dir = parent,
                None => break,
            }
        }
    }
    for root in roots {
        for rel in [
            "hybrid-player/dist/win-unpacked/resources/ffmpeg/ffmpeg.exe",
            "hybrid-player/node_modules/ffmpeg-static/ffmpeg.exe",
        ] {
            let p = root.join(rel);
            if p.exists() {
                return Some(p);
            }
        }
    }
    Some(PathBuf::from("ffmpeg"))
}

fn decode_sample_video_frame_rgba(
    media_path: &std::path::Path,
    time_secs: f32,
) -> Option<(u32, u32, Vec<u8>)> {
    let ffmpeg = find_ffmpeg_for_snapshot()?;
    let ts = format!("{:.3}", time_secs.max(0.0));
    let output = std::process::Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-ss", &ts, "-i"])
        .arg(media_path)
        .args([
            "-vf",
            "scale=in_color_matrix=bt709:in_range=tv:flags=accurate_rnd+full_chroma_int",
            "-frames:v",
            "1",
            "-f",
            "image2pipe",
            "-vcodec",
            "png",
            "-",
        ])
        .output()
        .ok()?;
    if !output.status.success() || output.stdout.is_empty() {
        return None;
    }
    let img = image::load_from_memory(&output.stdout).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return None;
    }
    Some((w, h, img.into_raw()))
}

fn create_carousel_sample_rgba(text_engine: &mut TextEngine, width: u32, height: u32) -> Vec<u8> {
    const STRIPES: [[u8; 3]; 5] = [
        [200, 30, 70],
        [224, 148, 24],
        [14, 156, 100],
        [11, 136, 198],
        [110, 68, 204],
    ];
    let w = width.max(1);
    let h = height.max(1);
    let mut rgba = vec![0u8; (w * h * 4) as usize];

    let band_y0 = (h * 34) / 100;
    let band_y1 = (h * 66) / 100;
    let band_x0 = (w * 12) / 100;
    let band_x1 = (w * 88) / 100;

    for y in 0..h {
        for x in 0..w {
            let col = ((x * 5) / w).min(4) as usize;
            let [mut r, mut g, mut b] = STRIPES[col];
            if y >= band_y0 && y <= band_y1 && x >= band_x0 && x <= band_x1 {
                r = ((r as u16 * 45) / 100) as u8;
                g = ((g as u16 * 45) / 100) as u8;
                b = ((b as u16 * 45) / 100) as u8;
            }
            let idx = ((y * w + x) * 4) as usize;
            rgba[idx] = r;
            rgba[idx + 1] = g;
            rgba[idx + 2] = b;
            rgba[idx + 3] = 255;
        }
    }

    let font_cfg = hybrid_ui::tokens::FontConfig::default();
    let label = "PHASE 0 SHOT";
    let font_px = 18.0;
    let line_px = 22.0;
    let metrics = text_engine.measure(
        label,
        font_px,
        line_px,
        Some(w as f32),
        FontRole::Ui,
        700,
        font_cfg,
    );
    let glyphs = text_engine.rasterize(
        label,
        font_px,
        line_px,
        Some(w as f32),
        FontRole::Ui,
        700,
        [255, 255, 255, 255],
        font_cfg,
    );
    let x_offset = (((w as f32 - metrics.width).max(0.0)) * 0.5).round() as i32;
    let y_offset = (((h as f32 - metrics.height).max(0.0)) * 0.5).round() as i32;
    for g in glyphs {
        let dst_x0 = g.x + x_offset;
        let dst_y0 = g.y + y_offset;
        for gy in 0..g.height as i32 {
            let py = dst_y0 + gy;
            if py < 0 || py >= h as i32 {
                continue;
            }
            for gx in 0..g.width as i32 {
                let px = dst_x0 + gx;
                if px < 0 || px >= w as i32 {
                    continue;
                }
                let src_idx = ((gy as u32 * g.width + gx as u32) * 4) as usize;
                let sa = g.data[src_idx + 3] as u16;
                if sa == 0 {
                    continue;
                }
                let dst_idx = ((py as u32 * w + px as u32) * 4) as usize;
                let inv = 255 - sa;
                rgba[dst_idx] =
                    ((g.data[src_idx] as u16 * sa + rgba[dst_idx] as u16 * inv) / 255) as u8;
                rgba[dst_idx + 1] = ((g.data[src_idx + 1] as u16 * sa
                    + rgba[dst_idx + 1] as u16 * inv)
                    / 255) as u8;
                rgba[dst_idx + 2] = ((g.data[src_idx + 2] as u16 * sa
                    + rgba[dst_idx + 2] as u16 * inv)
                    / 255) as u8;
            }
        }
    }

    rgba
}

/// Renders an offscreen `UiState` snapshot (`--headless-snapshot`) and writes a PNG to `opts.output_path`.
pub fn render_headless_snapshot(opts: &HeadlessSnapshotOptions) -> Result<(), String> {
    let rt = Runtime::new().map_err(|e| format!("tokio runtime failed: {e}"))?;
    let gpu = rt
        .block_on(GpuContext::new_headless())
        .map_err(|e| format!("GpuContext::new_headless: {e}"))?;

    let lw = opts.width.clamp(320, 7680);
    let lh = opts.height.clamp(240, 4320);
    let dpr = if opts.dpr.is_finite() {
        opts.dpr.clamp(0.5, 4.0)
    } else {
        1.0
    };
    let phys_w = ((lw as f32) * dpr).round().max(1.0) as u32;
    let phys_h = ((lh as f32) * dpr).round().max(1.0) as u32;

    let mut ui_state = UiState {
        viewport_w: lw as f32,
        viewport_h: lh as f32,
        theme: opts.theme,
        welcome_background: opts.welcome_bg,
        active_modal: opts.modal,
        bg_accordion_open: opts.bg_accordion_open,
        ..Default::default()
    };
    ui_state.playlist.open = opts.playlist_open;

    match opts.overlay {
        SnapshotOverlayKind::None => {}
        SnapshotOverlayKind::BurstPill => {
            ui_state.recording_indicator = RecordingIndicatorMode::BurstActive {
                count: opts.burst_count,
            };
        }
        SnapshotOverlayKind::ClipBezel => {
            ui_state.recording_indicator = RecordingIndicatorMode::ClipActive {
                elapsed_ds: (opts.clip_time_secs * 10.0).round() as u64,
            };
        }
        SnapshotOverlayKind::Carousel => {
            ui_state.carousel_mode = CarouselMode::DeleteCarousel;
            ui_state.carousel_total = opts.carousel_index.saturating_add(1).max(2);
            ui_state.carousel_index = opts.carousel_index.min(ui_state.carousel_total - 1);
        }
    }

    if opts.playback_sample {
        ui_state.welcome_active = false;
        ui_state.playing = true;
        ui_state.center_play_visible = false;
        ui_state.duration_secs = 15.0;
        ui_state.current_secs = opts.time_secs as f64;
        ui_state.window_title = "test-sample.mp4 — Hybrid Player".to_string();
        ui_state.push_toast(ToastItem::new(1, ToastKind::Info, "Loaded 1 file(s)"));
    }

    let format = wgpu::TextureFormat::Rgba8Unorm;
    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("headless-snapshot-target"),
        size: wgpu::Extent3d {
            width: phys_w,
            height: phys_h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());

    let mut sample_video_dims = (phys_w, phys_h);
    let sample_video_tex = if opts.playback_sample {
        let decoded = find_sample_media_path(opts.snapshot_media.as_deref())
            .and_then(|p| decode_sample_video_frame_rgba(&p, opts.time_secs));
        let (vw, vh, rgba) = match decoded {
            Some(frame_data) => frame_data,
            None => (phys_w, phys_h, create_smpte_sample_rgba(phys_w, phys_h)),
        };
        sample_video_dims = (vw, vh);
        Some(upload_rgba_texture(
            &gpu.device,
            &gpu.queue,
            "sample-video-frame",
            vw,
            vh,
            &rgba,
        ))
    } else {
        None
    };
    let sample_video_view = sample_video_tex
        .as_ref()
        .map(|t| t.create_view(&wgpu::TextureViewDescriptor::default()));

    let effect_cfg = EffectInstanceConfig {
        width: lw,
        height: lh,
        dpr,
        fps_limit: Some(60.0),
        ..EffectInstanceConfig::default()
    };

    ui_state.dpr = dpr;
    ui_state.gridmotion_cfg.pointer_x = (lw as f32) * 0.5;
    let mut gpu_bridge =
        UiGpuBridge::new(&gpu.device, &gpu.queue, format, phys_w, phys_h, effect_cfg)?;
    gpu_bridge.sync_welcome_effects(&gpu.device, &gpu.queue, &ui_state, &[], false);
    gpu_bridge.update_instance_dpr(lw, lh, dpr);
    {
        let effects = &mut gpu_bridge.compositor.graph.effects;
        effects.dither.set_time(opts.time_secs);
        effects.particles.set_time(opts.time_secs);
        effects
            .faulty
            .set_time_and_mouse(opts.time_secs, ui_state.faulty_cfg.mouse_uv, 1.0);
        effects.grid_motion.set_scroll_state(0.0, 0.0);
        if opts.bg_accordion_open {
            let ptime = opts.preview_time_secs.unwrap_or(opts.time_secs);
            let pbundle = &mut gpu_bridge.compositor.previews.bundle;
            pbundle.dither.set_time(ptime);
            pbundle.particles.set_time(ptime);
            pbundle
                .faulty
                .set_time_and_mouse(ptime, pbundle.faulty.config.mouse_uv, 1.0);
        }
    }
    let hybrid_ui = HybridUi::new();
    let frame = hybrid_ui.build_frame(&ui_state);

    for slot in &frame.image_slots {
        let sw = (slot.rect.width * dpr).round().max(1.0) as u32;
        let sh = (slot.rect.height * dpr).round().max(1.0) as u32;
        let sample_rgba = create_carousel_sample_rgba(&mut gpu_bridge.text_engine, sw, sh);
        gpu_bridge.set_image_rgba(&gpu.device, &gpu.queue, &slot.key, sw, sh, &sample_rgba);
    }

    let scene_source = if let Some(ref view) = sample_video_view {
        SceneSource::Video {
            texture_view: view,
            transform: VideoFrameTransform {
                video_width: sample_video_dims.0,
                video_height: sample_video_dims.1,
                fit_mode: VideoFitMode::Contain,
                opacity: 1.0,
            },
        }
    } else if ui_state.welcome_active {
        SceneSource::Welcome(ui_state.welcome_background)
    } else {
        SceneSource::ClearOnly
    };

    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("headless-snapshot-encoder"),
        });

    gpu_bridge.encode_frame(
        &gpu.device,
        &gpu.queue,
        &mut encoder,
        &target_view,
        &ui_state,
        &frame,
        scene_source,
        dpr,
        opts.time_secs as f64,
        0.0,
    );
    gpu.queue.submit(Some(encoder.finish()));

    let pixels = gpu
        .readback_texture(&target, phys_w, phys_h)
        .map_err(|e| format!("readback_texture: {e}"))?;
    let img = image::RgbaImage::from_raw(phys_w, phys_h, pixels)
        .ok_or_else(|| "invalid RGBA buffer dimensions".to_string())?;

    if let Some(parent) = opts
        .output_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
    {
        let _ = std::fs::create_dir_all(parent);
    }
    img.save(&opts.output_path)
        .map_err(|e| format!("failed to write snapshot PNG: {e}"))?;
    Ok(())
}

/// Custom event loop messages routed through `winit::event_loop::EventLoopProxy`.
#[derive(Debug)]
pub enum AppUserEvent {
    Wake,
    SingleInstanceArgs(Vec<String>),
    Shell(ShellWindowEvent),
    AccessKit(accesskit_winit::Event),
}

impl From<accesskit_winit::Event> for AppUserEvent {
    fn from(ev: accesskit_winit::Event) -> Self {
        Self::AccessKit(ev)
    }
}

struct DesktopSurfaceState {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    surface_config: wgpu::SurfaceConfiguration,
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    gpu_bridge: UiGpuBridge,
    subclass: WindowSubclass,
    accesskit: AccessKitAdapter,
    dx_bridge: Option<hybrid_mpv_engine::DxBridge>,
    last_frame_output: Option<UiFrameOutput>,
    uploaded_image_sources: HashMap<String, String>,
}

struct DesktopApp {
    rt: Arc<Runtime>,
    cli: AppCliArgs,
    bridge: MediaBridge,
    input: InputController,
    ui_engine: HybridUi,
    proxy: EventLoopProxy<AppUserEvent>,
    single_instance: Option<SingleInstanceServer>,
    state: Option<DesktopSurfaceState>,
    mods: ModifiersState,
    cursor_dip: (f32, f32),
    mouse_down: bool,
    active_slider_drag: Option<(HitTarget, RectPx)>,
    last_left_release: Option<(Instant, (f32, f32))>,
    pending_video_click: Option<Instant>,
    started_at: Instant,
    last_tick: Instant,
    last_shell_theme: ThemeMode,
    last_window_title: String,
    last_gridmotion_gen: u64,
    last_auto_hide: bool,
    ui_dirty: bool,
}

impl DesktopApp {
    fn current_input_context(&self) -> InputContext {
        let (x, y) = self.cursor_dip;
        let hover_target = self
            .state
            .as_ref()
            .and_then(|s| s.last_frame_output.as_ref())
            .and_then(|f| self.ui_engine.hit_test(f, x, y))
            .map(|r| r.target.clone());
        let hovering_controls = hover_target
            .as_ref()
            .is_some_and(|t| !matches!(t, HitTarget::ModalBackdrop | HitTarget::DropdownBackdrop));
        let in_wheel_exclusion_zone = hover_target.is_some();

        let is_fs = self
            .state
            .as_ref()
            .is_some_and(|s| s.subclass.is_fullscreen());

        InputContext {
            has_active_media: !self.bridge.ui.welcome_active || self.bridge.current_file.is_some(),
            is_playing: self.bridge.ui.playing,
            is_fullscreen: is_fs,
            is_locked: self.bridge.ui.locked,
            modal_open: self.bridge.ui.active_modal.is_some(),
            bg_accordion_open: self.bridge.ui.bg_accordion_open,
            dropdown_open: self.bridge.ui.active_dropdown.is_some()
                || self.bridge.ui.active_color_picker.is_some(),
            playlist_open: self.bridge.ui.playlist.open,
            text_input_focused: self.bridge.focused_input.is_some(),
            recording_shortcut: self.bridge.ui.recording_shortcut_id.is_some(),
            carousel_open: matches!(
                self.bridge.ui.carousel_mode,
                CarouselMode::DeleteCarousel | CarouselMode::RestoreCarousel
            ),
            carousel_restore_mode: self.bridge.ui.carousel_mode == CarouselMode::RestoreCarousel,
            hovering_controls,
            dragging_slider: self.active_slider_drag.is_some(),
            in_wheel_exclusion_zone,
            mouse_button_down: self.mouse_down,
            interactive_control_focused: self
                .bridge
                .ui
                .focused_target
                .as_ref()
                .is_some_and(is_interactive_control_for_space),
        }
    }

    fn dispatch_input_actions(&mut self, actions: Vec<InputAction>, event_loop: &ActiveEventLoop) {
        for action in actions {
            match action {
                InputAction::ShortcutAction(id) => match id.as_str() {
                    "toggle-fullscreen" => {
                        if let Some(st) = &self.state {
                            let next = !st.subclass.is_fullscreen();
                            let _ = st.subclass.set_fullscreen(next);
                        }
                        self.sync_window_maximize_state();
                        self.ui_dirty = true;
                    }
                    "exit-fullscreen" => {
                        if let Some(st) = &self.state {
                            let _ = st.subclass.set_fullscreen(false);
                        }
                        self.sync_window_maximize_state();
                        self.ui_dirty = true;
                    }
                    "quit" => {
                        self.bridge.shutdown_for_exit();
                        event_loop.exit();
                    }
                    other => self.bridge.execute_shortcut_action(other),
                },
                InputAction::TakeSingleScreenshot => self.bridge.take_single_screenshot(None),
                InputAction::TakePngScreenshot => {
                    self.bridge
                        .take_single_screenshot(Some(hybrid_mpv_engine::ScreenshotFormat::Png));
                }
                InputAction::StartBurstCapture => self.bridge.start_burst_capture(),
                InputAction::StopBurstCapture => self.bridge.stop_burst_capture(),
                InputAction::BurstTickCapture => self.bridge.burst_tick_frame(),
                InputAction::CarouselNavigate(dir) => self.bridge.navigate_carousel(dir),
                InputAction::CarouselClose => {
                    self.bridge.ui.carousel_mode = CarouselMode::Hidden;
                }
                InputAction::CarouselConfirmAction => self.bridge.confirm_carousel_action(),
                InputAction::DismissModalOrPopover => {
                    if self.bridge.ui.active_dropdown.is_some() {
                        self.bridge.ui.active_dropdown = None;
                    } else if self.bridge.ui.active_color_picker.is_some() {
                        self.bridge.ui.active_color_picker = None;
                    } else if self.bridge.ui.bg_accordion_open {
                        self.bridge.ui.bg_accordion_open = false;
                        self.bridge.ui.anim.trigger_bg_accordion_out();
                    } else if self.bridge.ui.active_modal.is_some() {
                        self.bridge.ui.close_modals_animated();
                    }
                    self.bridge.focused_input = None;
                }
                InputAction::TogglePlaylistDrawer => self.bridge.toggle_playlist(),
                InputAction::ExitFullscreen => {
                    if let Some(st) = &self.state {
                        let _ = st.subclass.set_fullscreen(false);
                    }
                    self.sync_window_maximize_state();
                    self.ui_dirty = true;
                }
                InputAction::AdjustVolumeDelta(step) => self.bridge.adjust_volume(step),
            }
        }
    }

    fn handle_hit_action(
        &mut self,
        target: &HitTarget,
        rect: Option<RectPx>,
        pointer_dip: Option<(f32, f32)>,
        event_loop: &ActiveEventLoop,
    ) {
        match target {
            HitTarget::TitlebarMinimize => {
                if let Some(st) = &self.state {
                    st.subclass.minimize();
                }
            }
            HitTarget::TitlebarMaximize => {
                if let Some(st) = &self.state {
                    if st.subclass.is_fullscreen() {
                        let _ = st.subclass.exit_fullscreen_to_windowed();
                    } else {
                        st.subclass.toggle_maximize();
                    }
                }
                self.sync_window_maximize_state();
                self.ui_dirty = true;
            }
            HitTarget::TitlebarClose => {
                self.bridge.shutdown_for_exit();
                event_loop.exit();
            }
            HitTarget::FullscreenToggle => {
                if let Some(st) = &self.state {
                    let next = !st.subclass.is_fullscreen();
                    let _ = st.subclass.set_fullscreen(next);
                }
                self.sync_window_maximize_state();
                self.ui_dirty = true;
            }
            other => {
                if let (Some(r), Some((px, py))) = (rect, pointer_dip) {
                    let rx = if r.width > 0.0 {
                        ((px - r.x) / r.width).clamp(0.0, 1.0)
                    } else {
                        0.5
                    };
                    let ry = if r.height > 0.0 {
                        ((py - r.y) / r.height).clamp(0.0, 1.0)
                    } else {
                        0.5
                    };
                    self.bridge.apply_hit_with_ratio(other, Some(rx), Some(ry));
                } else {
                    self.bridge.handle_hit_target(other);
                }
                if matches!(
                    other,
                    HitTarget::ShortcutResetOne(_) | HitTarget::ShortcutResetAll
                ) {
                    self.input
                        .shortcuts
                        .apply_custom_shortcuts(&self.bridge.ui.custom_shortcuts);
                } else if matches!(
                    other,
                    HitTarget::SettingsMouseScreenshotSelect | HitTarget::DropdownOption(_)
                ) {
                    self.input
                        .shortcuts
                        .set_screenshot_mouse_button(self.bridge.ui.screenshot_mouse_button);
                }
                if self.bridge.quit_requested {
                    event_loop.exit();
                }
            }
        }
    }

    fn sync_window_maximize_state(&mut self) {
        if let Some(st) = &self.state {
            self.bridge.ui.window_maximize_state = if st.subclass.is_fullscreen() {
                WindowMaximizeState::Fullscreen
            } else if st.subclass.is_maximized() {
                WindowMaximizeState::Maximized
            } else {
                WindowMaximizeState::Restored
            };
        }
    }

    fn render_frame(&mut self) {
        self.sync_window_maximize_state();
        let Some(st) = &mut self.state else {
            return;
        };

        let dpr = st.window.scale_factor() as f32;
        let size = st.window.inner_size();
        self.bridge.ui.viewport_w = (size.width as f32 / dpr).max(320.0);
        self.bridge.ui.viewport_h = (size.height as f32 / dpr).max(240.0);
        self.bridge.ui.dpr = dpr;

        if self.bridge.ui.theme != self.last_shell_theme {
            let _ = st.subclass.set_theme(to_shell_theme(self.bridge.ui.theme));
            self.last_shell_theme = self.bridge.ui.theme;
        }
        if self.bridge.ui.window_title != self.last_window_title {
            st.window.set_title(&self.bridge.ui.window_title);
            self.last_window_title = self.bridge.ui.window_title.clone();
        }

        let now = Instant::now();
        let dt = now
            .duration_since(self.last_tick)
            .as_secs_f32()
            .clamp(0.001, 0.1);
        self.last_tick = now;
        let elapsed = now.duration_since(self.started_at).as_secs_f64();

        let _ = self.bridge.ui.step_animations(dt);

        let custom_pics_dirty = self.last_gridmotion_gen != self.bridge.gridmotion_pictures_gen;
        if custom_pics_dirty {
            self.last_gridmotion_gen = self.bridge.gridmotion_pictures_gen;
        }
        st.gpu_bridge.sync_welcome_effects(
            &st.device,
            &st.queue,
            &self.bridge.ui,
            &self.bridge.gridmotion_custom_pictures,
            custom_pics_dirty,
        );
        st.gpu_bridge
            .compositor
            .graph
            .effects
            .set_quality(self.bridge.ui.welcome_quality);

        self.bridge.ui.focused_text_field = self.bridge.focused_input;
        let frame = self.ui_engine.build_frame(&self.bridge.ui);
        if self.bridge.ui.active_modal.is_some() && self.bridge.ui.focused_target.is_none() {
            if let Some(first) = collect_modal_focusables(&frame).into_iter().next() {
                self.bridge.ui.focused_target = Some(first);
            }
        }
        sync_hit_test_regions(&st.subclass, &frame, self.bridge.ui.viewport_w);

        let a11y_dpr = dpr as f64;
        st.accesskit
            .update_if_active(|| scale_a11y_tree_for_window(&frame.a11y_tree, a11y_dpr));

        let cur_file_prefix = format!("{}|", self.bridge.current_file.as_deref().unwrap_or(""));
        let keep_latest_hover = self.bridge.ui.hover_thumb_has_frame
            && self.bridge.ui.hover_thumb_norm.is_some()
            && st
                .uploaded_image_sources
                .get("hover-thumb:latest")
                .is_some_and(|prev| prev.starts_with(&cur_file_prefix));
        if !keep_latest_hover
            && st
                .uploaded_image_sources
                .remove("hover-thumb:latest")
                .is_some()
        {
            st.gpu_bridge.remove_image("hover-thumb:latest");
        }

        for slot in &frame.image_slots {
            if let Some(norm_str) = slot.key.strip_prefix("hover-thumb:") {
                if let Ok(norm) = norm_str.parse::<f32>() {
                    let src_id = format!(
                        "{}|{}",
                        self.bridge.current_file.as_deref().unwrap_or(""),
                        norm_str
                    );
                    let needs_upload = st
                        .uploaded_image_sources
                        .get(&slot.key)
                        .is_none_or(|prev| prev != &src_id);
                    let needs_latest = st
                        .uploaded_image_sources
                        .get("hover-thumb:latest")
                        .is_none_or(|prev| prev != &src_id);
                    if needs_upload || needs_latest {
                        let thumb_norm = self.bridge.ui.hover_thumb_norm.unwrap_or(norm);
                        if let Some((w, h, rgba)) = self.bridge.get_seek_thumbnail_rgba(thumb_norm)
                        {
                            if needs_upload {
                                st.gpu_bridge
                                    .set_image_rgba(&st.device, &st.queue, &slot.key, *w, *h, rgba);
                                st.uploaded_image_sources
                                    .insert(slot.key.clone(), src_id.clone());
                            }
                            if needs_latest {
                                st.gpu_bridge.set_image_rgba(
                                    &st.device,
                                    &st.queue,
                                    "hover-thumb:latest",
                                    *w,
                                    *h,
                                    rgba,
                                );
                                st.uploaded_image_sources
                                    .insert("hover-thumb:latest".to_string(), src_id);
                            }
                        }
                    }
                }
            } else if let Some(idx_str) = slot.key.strip_prefix("screenshot-deleted:") {
                if let Ok(idx) = idx_str.parse::<usize>() {
                    let path_id = self
                        .bridge
                        .deleted_screenshot_stack
                        .get(idx)
                        .map(|r| r.staged_path.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let needs_upload = !path_id.is_empty()
                        && st
                            .uploaded_image_sources
                            .get(&slot.key)
                            .is_none_or(|prev| prev != &path_id);
                    if needs_upload {
                        if let Some((w, h, rgba)) = self
                            .bridge
                            .resolve_or_request_screenshot_rgba(&slot.key, idx, true)
                        {
                            st.gpu_bridge
                                .set_image_rgba(&st.device, &st.queue, &slot.key, w, h, &rgba);
                            st.uploaded_image_sources.insert(slot.key.clone(), path_id);
                        }
                    }
                }
            } else if let Some(idx_str) = slot.key.strip_prefix("screenshot:") {
                if let Ok(idx) = idx_str.parse::<usize>() {
                    let path_id = self
                        .bridge
                        .session_screenshots
                        .get(idx)
                        .map(|s| s.file_path.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let needs_upload = !path_id.is_empty()
                        && st
                            .uploaded_image_sources
                            .get(&slot.key)
                            .is_none_or(|prev| prev != &path_id);
                    if needs_upload {
                        if let Some((w, h, rgba)) = self
                            .bridge
                            .resolve_or_request_screenshot_rgba(&slot.key, idx, false)
                        {
                            st.gpu_bridge
                                .set_image_rgba(&st.device, &st.queue, &slot.key, w, h, &rgba);
                            st.uploaded_image_sources.insert(slot.key.clone(), path_id);
                        }
                    }
                }
            } else if let Some(rest) = slot.key.strip_prefix("carousel:") {
                if let Some((idx_str, _)) = rest.split_once(':') {
                    if let Ok(idx) = idx_str.parse::<usize>() {
                        let restore_mode =
                            self.bridge.ui.carousel_mode == CarouselMode::RestoreCarousel;
                        let path_id = if restore_mode {
                            self.bridge
                                .deleted_screenshot_stack
                                .get(idx)
                                .map(|r| r.staged_path.to_string_lossy().into_owned())
                                .unwrap_or_default()
                        } else {
                            self.bridge
                                .session_screenshots
                                .get(idx)
                                .map(|s| s.file_path.to_string_lossy().into_owned())
                                .unwrap_or_default()
                        };
                        let needs_upload = !path_id.is_empty()
                            && st
                                .uploaded_image_sources
                                .get(&slot.key)
                                .is_none_or(|prev| prev != &path_id);
                        if needs_upload {
                            if let Some((w, h, rgba)) = self
                                .bridge
                                .resolve_or_request_screenshot_rgba(&slot.key, idx, restore_mode)
                            {
                                st.gpu_bridge
                                    .set_image_rgba(&st.device, &st.queue, &slot.key, w, h, &rgba);
                                st.uploaded_image_sources.insert(slot.key.clone(), path_id);
                            }
                        }
                    }
                }
            } else if let Some(idx_str) = slot.key.strip_prefix("bg-thumb:") {
                if let Ok(idx) = idx_str.parse::<usize>() {
                    let path_id = self
                        .bridge
                        .gridmotion_custom_pictures
                        .get(idx)
                        .cloned()
                        .unwrap_or_default();
                    let needs_upload = !path_id.is_empty()
                        && st
                            .uploaded_image_sources
                            .get(&slot.key)
                            .is_none_or(|prev| prev != &path_id);
                    if needs_upload {
                        if let Some((w, h, rgba)) = self
                            .bridge
                            .resolve_or_request_gridmotion_thumb_rgba(&slot.key, idx)
                        {
                            st.gpu_bridge
                                .set_image_rgba(&st.device, &st.queue, &slot.key, w, h, &rgba);
                            st.uploaded_image_sources.insert(slot.key.clone(), path_id);
                        }
                    }
                }
            }
        }

        let surface_tex = match st.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(tex)
            | wgpu::CurrentSurfaceTexture::Suboptimal(tex) => tex,
            _ => return,
        };
        let target_view = surface_tex
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let acquired_video = if !self.bridge.ui.welcome_active {
            st.dx_bridge
                .as_ref()
                .and_then(|dx| dx.acquire_frame_for_submit())
        } else {
            None
        };

        let scene_source = if let Some((ref video_frame, _)) = acquired_video {
            SceneSource::Video {
                texture_view: &video_frame.view,
                transform: VideoFrameTransform {
                    video_width: video_frame.width.max(1),
                    video_height: video_frame.height.max(1),
                    fit_mode: VideoFitMode::Contain,
                    opacity: 1.0,
                },
            }
        } else if self.bridge.ui.welcome_active {
            SceneSource::Welcome(self.bridge.ui.welcome_background)
        } else {
            SceneSource::ClearOnly
        };

        let mut encoder = st
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("hybrid-main-encoder"),
            });

        st.gpu_bridge.encode_frame(
            &st.device,
            &st.queue,
            &mut encoder,
            &target_view,
            &self.bridge.ui,
            &frame,
            scene_source,
            dpr,
            elapsed,
            dt,
        );

        if let (Some(dx), Some((ref video_frame, _))) =
            (st.dx_bridge.as_ref(), acquired_video.as_ref())
        {
            dx.stage_wgpu_done_signal(video_frame);
        }

        st.queue.submit(Some(encoder.finish()));
        st.queue.present(surface_tex);
        st.last_frame_output = Some(frame);
        self.ui_dirty = false;
    }
}

fn load_app_window_icon() -> Option<winit::window::Icon> {
    let bytes = include_bytes!("../../../hybrid-player/assets/icons/icon.png");
    let img = image::load_from_memory(bytes).ok()?.into_rgba8();
    let (w, h) = img.dimensions();
    winit::window::Icon::from_rgba(img.into_raw(), w, h).ok()
}

fn is_network_media_arg(raw: &str) -> bool {
    let lower = raw.trim().to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("rtsp://")
        || lower.starts_with("rtmp://")
        || lower.starts_with("rtmps://")
        || lower.starts_with("srt://")
        || lower.starts_with("ytdl://")
}

impl ApplicationHandler<AppUserEvent> for DesktopApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }

        let attrs = WindowAttributes::default()
            .with_title("Hybrid Player")
            .with_decorations(false)
            .with_transparent(false)
            .with_window_icon(load_app_window_icon())
            .with_inner_size(LogicalSize::new(1280.0, 720.0))
            .with_min_inner_size(LogicalSize::new(640.0, 360.0))
            .with_visible(false);

        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .expect("create main winit window"),
        );
        let accesskit =
            AccessKitAdapter::with_event_loop_proxy(event_loop, &window, self.proxy.clone());

        let hwnd = extract_hwnd_from_winit(&window).expect("valid Win32 HWND");
        let _ = apply_null_brush(hwnd);
        let subclass = WindowSubclass::install(hwnd, to_shell_theme(self.bridge.ui.theme))
            .expect("install Win32 WindowSubclass");
        let proxy_clone = self.proxy.clone();
        subclass.set_event_callback(move |ev| {
            let _ = proxy_clone.send_event(AppUserEvent::Shell(ev));
        });

        self.bridge.attach_smtc(hwnd);
        if self.bridge.mpv.is_none() {
            let _ = self.bridge.init_mpv_engine_with_wid(Some(hwnd.0 as isize));
        }

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::DX12,
            flags: wgpu::InstanceFlags::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            backend_options: wgpu::BackendOptions::default(),
            display: None,
        });
        let surface = instance
            .create_surface(Arc::clone(&window))
            .expect("create wgpu surface");
        let adapter = self
            .rt
            .block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                apply_limit_buckets: true,
            }))
            .expect("DX12 wgpu adapter");

        let (device, queue) = self
            .rt
            .block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("hybrid-app-device"),
                required_features: wgpu::Features::empty(),
                required_limits:
                    wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
                ..Default::default()
            }))
            .expect("DX12 wgpu device");
        let device = Arc::new(device);
        let queue = Arc::new(queue);

        let size = window.inner_size();
        let dpr = window.scale_factor() as f32;
        let logical_w = ((size.width as f32) / dpr).round().max(320.0) as u32;
        let logical_h = ((size.height as f32) / dpr).round().max(240.0) as u32;

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(wgpu::TextureFormat::Bgra8Unorm);
        let alpha_mode = caps
            .alpha_modes
            .iter()
            .copied()
            .find(|m| *m == wgpu::CompositeAlphaMode::Opaque)
            .unwrap_or(caps.alpha_modes[0]);

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

        let mut gpu_bridge = UiGpuBridge::new(
            &device,
            &queue,
            format,
            surface_config.width,
            surface_config.height,
            EffectInstanceConfig {
                width: logical_w,
                height: logical_h,
                dpr,
                ..EffectInstanceConfig::default()
            },
        )
        .expect("UiGpuBridge");
        gpu_bridge
            .compositor
            .graph
            .effects
            .faulty
            .randomize_time_offset();

        let dx_bridge = if let Some(mpv) = &self.bridge.mpv {
            let win_clone = Arc::clone(&window);
            match hybrid_mpv_engine::DxBridge::new(
                Arc::clone(mpv),
                Arc::clone(&device),
                Arc::clone(&queue),
                &adapter,
                surface_config.width,
                surface_config.height,
                true,
                Some(Arc::new(move |_| win_clone.request_redraw())),
            ) {
                Ok(dx) => Some(dx),
                Err(err) => {
                    if !mpv.is_subprocess() {
                        eprintln!(
                            "[hybrid-app] DxBridge unavailable ({err}); trying mpv --wid subprocess fallback"
                        );
                        if let Err(sub_err) =
                            self.bridge.fallback_to_mpv_subprocess(hwnd.0 as isize)
                        {
                            eprintln!("[hybrid-app] mpv --wid fallback unavailable: {sub_err}");
                        }
                    }
                    None
                }
            }
        } else {
            None
        };

        if let Some(server) = &self.single_instance {
            let proxy = self.proxy.clone();
            server.start_listener(Some(hwnd), move |args| {
                let _ = proxy.send_event(AppUserEvent::SingleInstanceArgs(args));
            });
        }

        if !self.cli.media_inputs.is_empty() {
            let first_url = self
                .cli
                .media_inputs
                .iter()
                .find(|s| is_network_media_arg(s))
                .cloned();
            let paths: Vec<PathBuf> = self
                .cli
                .media_inputs
                .iter()
                .filter(|s| !is_network_media_arg(s))
                .map(PathBuf::from)
                .collect();
            if !paths.is_empty() {
                self.bridge.open_files(&paths, true, true);
            } else if let Some(url) = first_url {
                self.bridge.open_url_or_path(&url);
            }
        }

        self.last_shell_theme = self.bridge.ui.theme;
        self.last_window_title = self.bridge.ui.window_title.clone();
        window.set_title(&self.bridge.ui.window_title);

        self.state = Some(DesktopSurfaceState {
            window: Arc::clone(&window),
            surface,
            surface_config,
            device,
            queue,
            gpu_bridge,
            subclass,
            accesskit,
            dx_bridge,
            last_frame_output: None,
            uploaded_image_sources: HashMap::new(),
        });

        // Render the styled first frame into the swapchain before revealing the window.
        self.render_frame();
        window.set_visible(true);
        if let Some(st) = &self.state {
            st.subclass.maximize();
            st.window.request_redraw();
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AppUserEvent) {
        match event {
            AppUserEvent::Wake => {}
            AppUserEvent::SingleInstanceArgs(args) => {
                let mut paths = Vec::new();
                let mut first_url = None;
                for raw in args {
                    let trimmed = raw.trim();
                    if trimmed.is_empty()
                        || trimmed == "--focus"
                        || trimmed == "--focus-only"
                        || (trimmed.starts_with("--") && !std::path::Path::new(trimmed).exists())
                    {
                        continue;
                    }
                    if is_network_media_arg(trimmed) {
                        if first_url.is_none() {
                            first_url = Some(trimmed.to_string());
                        }
                    } else {
                        paths.push(PathBuf::from(trimmed));
                    }
                }
                if !paths.is_empty() {
                    self.bridge.open_files(&paths, true, true);
                } else if let Some(url) = first_url {
                    self.bridge.open_url_or_path(&url);
                }
                self.ui_dirty = true;
                if let Some(st) = &self.state {
                    if st.window.is_minimized() == Some(true) {
                        st.window.set_minimized(false);
                    }
                    st.window.focus_window();
                    st.window.request_redraw();
                }
            }
            AppUserEvent::Shell(ev) => match ev {
                ShellWindowEvent::MaxButtonHovered(hovered) => {
                    let mut changed = false;
                    if hovered {
                        if self.bridge.ui.hovered_target != Some(HitTarget::TitlebarMaximize) {
                            self.bridge.ui.hovered_target = Some(HitTarget::TitlebarMaximize);
                            changed = true;
                        }
                    } else if self.bridge.ui.hovered_target == Some(HitTarget::TitlebarMaximize) {
                        self.bridge.ui.hovered_target = None;
                        changed = true;
                    }
                    if let Some(st) = &self.state {
                        if hovered {
                            st.window.set_cursor(CursorIcon::Pointer);
                        }
                        if changed {
                            self.ui_dirty = true;
                            st.window.request_redraw();
                        }
                    }
                }
                ShellWindowEvent::MaxButtonClicked => {
                    self.handle_hit_action(&HitTarget::TitlebarMaximize, None, None, event_loop);
                    self.ui_dirty = true;
                    if let Some(st) = &self.state {
                        st.window.request_redraw();
                    }
                }
                ShellWindowEvent::DpiChanged { scale_factor, .. } => {
                    if let Some(st) = &mut self.state {
                        let dpr = scale_factor as f32;
                        let size = st.window.inner_size();
                        let lw = ((size.width as f32) / dpr).round().max(320.0) as u32;
                        let lh = ((size.height as f32) / dpr).round().max(240.0) as u32;
                        st.gpu_bridge.update_instance_dpr(lw, lh, dpr);
                        self.ui_dirty = true;
                        st.window.request_redraw();
                    }
                }
                ShellWindowEvent::MaximizedChanged(_)
                | ShellWindowEvent::FullscreenChanged(_)
                | ShellWindowEvent::BoundsAnimating(_) => {
                    self.sync_window_maximize_state();
                    self.ui_dirty = true;
                    if let Some(st) = &self.state {
                        st.window.request_redraw();
                    }
                }
            },
            AppUserEvent::AccessKit(ak_ev) => match ak_ev.window_event {
                accesskit_winit::WindowEvent::InitialTreeRequested => {
                    let frame = self.ui_engine.build_frame(&self.bridge.ui);
                    if let Some(st) = &mut self.state {
                        let a11y_dpr = st.window.scale_factor();
                        st.accesskit.update_if_active(|| {
                            scale_a11y_tree_for_window(&frame.a11y_tree, a11y_dpr)
                        });
                        st.last_frame_output = Some(frame);
                        st.window.request_redraw();
                    }
                }
                accesskit_winit::WindowEvent::ActionRequested(req) => {
                    let frame = self
                        .state
                        .as_ref()
                        .and_then(|s| s.last_frame_output.clone())
                        .unwrap_or_else(|| self.ui_engine.build_frame(&self.bridge.ui));

                    if req.action == Action::Click {
                        if let Some(target) =
                            find_hit_target_for_accesskit_node(&frame, req.target_node)
                        {
                            if matches!(
                                target,
                                HitTarget::TitlebarMinimize
                                    | HitTarget::TitlebarMaximize
                                    | HitTarget::TitlebarClose
                                    | HitTarget::FullscreenToggle
                            ) {
                                self.handle_hit_action(&target, None, None, event_loop);
                                self.ui_dirty = true;
                                if let Some(st) = &self.state {
                                    st.window.request_redraw();
                                }
                                return;
                            }
                        }
                    }

                    if dispatch_accesskit_request(&mut self.bridge, &mut self.input, &frame, &req) {
                        self.ui_dirty = true;
                        if self.bridge.quit_requested {
                            event_loop.exit();
                        }
                        if let Some(st) = &self.state {
                            st.window.request_redraw();
                        }
                    }
                }
                accesskit_winit::WindowEvent::AccessibilityDeactivated => {}
            },
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        if let Some(st) = &mut self.state {
            st.accesskit.process_event(&st.window, &event);
        }

        match event {
            WindowEvent::CloseRequested => {
                self.bridge.shutdown_for_exit();
                event_loop.exit();
            }
            WindowEvent::Resized(new_size) => {
                if let Some(st) = &mut self.state {
                    if new_size.width > 0 && new_size.height > 0 {
                        let dpr = st.window.scale_factor() as f32;
                        let lw = ((new_size.width as f32) / dpr).round().max(320.0) as u32;
                        let lh = ((new_size.height as f32) / dpr).round().max(240.0) as u32;
                        st.surface_config.width = new_size.width;
                        st.surface_config.height = new_size.height;
                        st.surface.configure(&st.device, &st.surface_config);
                        st.gpu_bridge
                            .resize(&st.device, new_size.width, new_size.height);
                        st.gpu_bridge.update_instance_dpr(lw, lh, dpr);
                        if let Some(dx) = &st.dx_bridge {
                            dx.resize(new_size.width, new_size.height);
                        }
                        self.ui_dirty = true;
                        st.window.request_redraw();
                    }
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if let Some(st) = &mut self.state {
                    let dpr = scale_factor as f32;
                    let size = st.window.inner_size();
                    let lw = ((size.width as f32) / dpr).round().max(320.0) as u32;
                    let lh = ((size.height as f32) / dpr).round().max(240.0) as u32;
                    st.gpu_bridge.update_instance_dpr(lw, lh, dpr);
                    self.ui_dirty = true;
                    st.window.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(m) => {
                self.mods = m.state();
            }
            WindowEvent::CursorMoved { position, .. } => {
                let dpr = self
                    .state
                    .as_ref()
                    .map(|s| s.window.scale_factor())
                    .unwrap_or(1.0);
                self.cursor_dip = ((position.x / dpr) as f32, (position.y / dpr) as f32);
                self.input.cursor_tracker.record_activity(Instant::now());
                self.input.chrome_hide.record_activity(Instant::now());

                let mut needs_redraw = false;

                if let Some((ref slider_target, rect)) = self.active_slider_drag.clone() {
                    if *slider_target == HitTarget::VolumeSlider {
                        let boost = self.bridge.ui.volume_boost_enabled;
                        let pct = self.bridge.ui.volume_spring.handle_drag(
                            self.cursor_dip.0,
                            rect.x,
                            rect.x + rect.width,
                            boost,
                        );
                        self.bridge.set_volume_pct(pct);
                    } else {
                        let rx = if rect.width > 0.0 {
                            ((self.cursor_dip.0 - rect.x) / rect.width).clamp(0.0, 1.0)
                        } else {
                            0.5
                        };
                        let ry = if rect.height > 0.0 {
                            ((self.cursor_dip.1 - rect.y) / rect.height).clamp(0.0, 1.0)
                        } else {
                            0.5
                        };
                        if *slider_target == HitTarget::ProgressSeek {
                            self.bridge.preview_seek_normalized(rx as f64);
                        } else {
                            self.bridge
                                .apply_hit_with_ratio(slider_target, Some(rx), Some(ry));
                        }
                    }
                    needs_redraw = true;
                }

                // Forward pointer coordinates to interactive welcome shaders
                if self.bridge.ui.welcome_active {
                    let vw = self.bridge.ui.viewport_w.max(1.0);
                    let vh = self.bridge.ui.viewport_h.max(1.0);
                    self.bridge.ui.dither_cfg.mouse_pos_px = [self.cursor_dip.0, self.cursor_dip.1];
                    self.bridge.ui.particles_cfg.mouse_ndc = [
                        (self.cursor_dip.0 / vw) * 2.0 - 1.0,
                        -((self.cursor_dip.1 / vh) * 2.0 - 1.0),
                    ];
                    self.bridge.ui.faulty_cfg.mouse_uv = [
                        (self.cursor_dip.0 / vw).clamp(0.0, 1.0),
                        (1.0 - (self.cursor_dip.1 / vh)).clamp(0.0, 1.0),
                    ];
                    self.bridge.ui.gridmotion_cfg.pointer_x = self.cursor_dip.0;
                    needs_redraw = true;
                }

                let hit_region = self
                    .state
                    .as_ref()
                    .and_then(|s| s.last_frame_output.as_ref())
                    .and_then(|f| {
                        self.ui_engine
                            .hit_test(f, self.cursor_dip.0, self.cursor_dip.1)
                            .cloned()
                    });

                let next_hover = hit_region.as_ref().map(|r| r.target.clone());
                if self.bridge.ui.hovered_target != next_hover {
                    self.bridge.ui.hovered_target = next_hover.clone();
                    self.bridge.ui.hover_tooltip_elapsed_secs = 0.0;
                    needs_redraw = true;
                }

                let next_grid_card = if self.bridge.ui.welcome_active
                    && self.bridge.ui.welcome_background == WelcomeBackgroundKind::Gridmotion
                    && self.bridge.ui.active_modal.is_none()
                    && next_hover.is_none()
                {
                    self.state.as_ref().and_then(|st| {
                        st.gpu_bridge
                            .compositor
                            .graph
                            .effects
                            .grid_motion
                            .hit_test_card(
                                self.cursor_dip.0,
                                self.cursor_dip.1,
                                self.bridge.ui.viewport_w,
                                self.bridge.ui.viewport_h,
                            )
                    })
                } else {
                    None
                };
                if self.bridge.ui.gridmotion_cfg.hovered_card != next_grid_card {
                    self.bridge.ui.gridmotion_cfg.hovered_card = next_grid_card;
                    needs_redraw = true;
                }

                // Update seekbar hover thumbnail norm & async thumbnail extraction (suppressed while dragging)
                let next_thumb = if self.active_slider_drag.is_none() {
                    hit_region.as_ref().and_then(|region| {
                        if (region.target == HitTarget::ProgressSeek
                            || matches!(region.target, HitTarget::ChapterMarker(_)))
                            && !self.bridge.ui.welcome_active
                            && !self.bridge.ui.is_live_stream
                            && self.bridge.ui.duration_secs > 0.0
                            && self.bridge.has_local_seek_thumbnails()
                            && region.rect.width > 0.0
                        {
                            if let Some(prog_reg) = self
                                .state
                                .as_ref()
                                .and_then(|s| s.last_frame_output.as_ref())
                                .and_then(|f| {
                                    f.hit_regions
                                        .iter()
                                        .find(|r| r.target == HitTarget::ProgressSeek)
                                })
                            {
                                if prog_reg.rect.width > 0.0 {
                                    return Some(
                                        ((self.cursor_dip.0 - prog_reg.rect.x)
                                            / prog_reg.rect.width)
                                            .clamp(0.0, 1.0),
                                    );
                                }
                            }
                            Some(
                                ((self.cursor_dip.0 - region.rect.x) / region.rect.width)
                                    .clamp(0.0, 1.0),
                            )
                        } else {
                            None
                        }
                    })
                } else {
                    None
                };
                if self.bridge.ui.hover_thumb_norm != next_thumb {
                    let was_has_frame = self.bridge.ui.hover_thumb_has_frame;
                    let entered_seekbar = self.bridge.ui.hover_thumb_norm.is_none();
                    self.bridge.ui.hover_thumb_norm = next_thumb;
                    if let Some(norm) = next_thumb {
                        self.bridge.request_seek_thumbnail(norm);
                        let has_cur = self.bridge.get_seek_thumbnail_rgba(norm).is_some();
                        if has_cur {
                            self.bridge.ui.hover_thumb_has_frame = true;
                            if !was_has_frame {
                                self.bridge.ui.anim.trigger_hover_thumb_in();
                            }
                        } else if entered_seekbar {
                            self.bridge.ui.hover_thumb_has_frame = false;
                        }
                        self.bridge.ui.hover_thumb_loading =
                            !has_cur && self.bridge.is_seek_thumb_pending(norm);
                    } else {
                        self.bridge.ui.hover_thumb_has_frame = false;
                        self.bridge.ui.hover_thumb_loading = false;
                    }
                    needs_redraw = true;
                }

                if self.bridge.ui.auto_hide_controls {
                    self.bridge.ui.auto_hide_controls = false;
                    needs_redraw = true;
                }

                if let Some(st) = &self.state {
                    st.window.set_cursor_visible(true);
                    let icon = if self.active_slider_drag.is_some() {
                        CursorIcon::Grabbing
                    } else {
                        match hit_region.as_ref().map(|r| r.cursor) {
                            Some(CursorHint::Pointer) => CursorIcon::Pointer,
                            Some(CursorHint::Text) => CursorIcon::Text,
                            Some(CursorHint::Grab) => {
                                if self.mouse_down {
                                    CursorIcon::Grabbing
                                } else {
                                    CursorIcon::Grab
                                }
                            }
                            Some(CursorHint::Default) | None => CursorIcon::Default,
                        }
                    };
                    st.window.set_cursor(icon);
                    if needs_redraw {
                        self.ui_dirty = true;
                        st.window.request_redraw();
                    }
                }
            }
            WindowEvent::CursorLeft { .. } => {
                if self.bridge.ui.hovered_target.is_some()
                    || self.bridge.ui.hover_thumb_norm.is_some()
                {
                    self.bridge.ui.hovered_target = None;
                    self.bridge.ui.hover_tooltip_elapsed_secs = 0.0;
                    self.bridge.ui.hover_thumb_norm = None;
                    self.bridge.ui.hover_thumb_has_frame = false;
                    self.bridge.ui.hover_thumb_loading = false;
                    self.ui_dirty = true;
                    if let Some(st) = &self.state {
                        st.window.request_redraw();
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.mouse_down = state == ElementState::Pressed;
                if state == ElementState::Pressed {
                    self.bridge.ui.focus_visible = false;
                    self.bridge.ui.hover_tooltip_elapsed_secs = 0.0;
                }
                let ctx = self.current_input_context();
                if let Some(btn_code) = map_mouse_button(button) {
                    let actions = match state {
                        ElementState::Pressed => {
                            self.input
                                .on_mouse_button_down(Instant::now(), btn_code, &ctx)
                        }
                        ElementState::Released => self.input.on_mouse_button_up(btn_code),
                    };
                    self.dispatch_input_actions(actions, event_loop);
                }

                if button == MouseButton::Left && state == ElementState::Pressed {
                    let hit = self
                        .state
                        .as_ref()
                        .and_then(|s| s.last_frame_output.as_ref())
                        .and_then(|f| {
                            self.ui_engine
                                .hit_test(f, self.cursor_dip.0, self.cursor_dip.1)
                                .cloned()
                        });
                    if let Some(region) = hit {
                        self.pending_video_click = None;
                        if is_slider_target(&region.target) {
                            self.active_slider_drag = Some((region.target.clone(), region.rect));
                            if region.target == HitTarget::ProgressSeek {
                                self.bridge.is_dragging_progress = true;
                                self.bridge.ui.is_dragging_progress = true;
                                self.bridge.ui.hover_thumb_norm = None;
                                self.bridge.ui.hover_thumb_has_frame = false;
                                self.bridge.ui.hover_thumb_loading = false;
                            }
                            if region.target == HitTarget::VolumeSlider {
                                self.bridge.ui.volume_spring.dragging = true;
                                let boost = self.bridge.ui.volume_boost_enabled;
                                let pct = self.bridge.ui.volume_spring.handle_drag(
                                    self.cursor_dip.0,
                                    region.rect.x,
                                    region.rect.x + region.rect.width,
                                    boost,
                                );
                                self.bridge.set_volume_pct(pct);
                            } else {
                                self.handle_hit_action(
                                    &region.target,
                                    Some(region.rect),
                                    Some(self.cursor_dip),
                                    event_loop,
                                );
                            }
                        }
                    }
                } else if button == MouseButton::Left && state == ElementState::Released {
                    if let Some((slider_target, rect)) = self.active_slider_drag.take() {
                        if slider_target == HitTarget::ProgressSeek {
                            self.bridge.is_dragging_progress = false;
                            self.bridge.ui.is_dragging_progress = false;
                        }
                        if slider_target == HitTarget::VolumeSlider {
                            self.bridge.ui.volume_spring.release();
                        } else if matches!(slider_target, HitTarget::EqBandSlider(_)) {
                            self.handle_hit_action(
                                &slider_target,
                                Some(rect),
                                Some(self.cursor_dip),
                                event_loop,
                            );
                            self.bridge.flush_eq_debounce();
                        } else {
                            self.handle_hit_action(
                                &slider_target,
                                Some(rect),
                                Some(self.cursor_dip),
                                event_loop,
                            );
                        }
                        self.ui_dirty = true;
                        if let Some(st) = &self.state {
                            st.window.request_redraw();
                        }
                        return;
                    }

                    let hit = self
                        .state
                        .as_ref()
                        .and_then(|s| s.last_frame_output.as_ref())
                        .and_then(|f| {
                            self.ui_engine
                                .hit_test(f, self.cursor_dip.0, self.cursor_dip.1)
                                .cloned()
                        });
                    if let Some(region) = hit {
                        self.pending_video_click = None;
                        if !matches!(
                            region.target,
                            HitTarget::PlaylistSearch
                                | HitTarget::NetworkStreamInput
                                | HitTarget::ShortcutSearch
                        ) {
                            self.bridge.set_focused_input(None);
                        }
                        if !matches!(
                            region.target,
                            HitTarget::ModalBackdrop
                                | HitTarget::ModalPanelSurface
                                | HitTarget::DropdownBackdrop
                                | HitTarget::PopoverSurface
                                | HitTarget::BgAccordionSurface
                                | HitTarget::ControlsBarSurface
                                | HitTarget::PlaylistSurface
                                | HitTarget::WelcomeContentSurface
                                | HitTarget::VolumeContainerSurface
                                | HitTarget::ShortcutScrollArea
                        ) {
                            self.bridge.ui.focused_target = Some(region.target.clone());
                        }
                        self.last_left_release = None;
                        if region.target == HitTarget::WelcomeContentSurface {
                            if self.bridge.ui.playlist.open && !self.bridge.ui.locked {
                                self.bridge.toggle_playlist();
                            }
                        } else {
                            self.handle_hit_action(
                                &region.target,
                                Some(region.rect),
                                Some(self.cursor_dip),
                                event_loop,
                            );
                        }
                    } else {
                        self.bridge.set_focused_input(None);
                        self.bridge.ui.focused_target = None;
                        let now = Instant::now();
                        let is_double_click =
                            self.last_left_release
                                .take()
                                .is_some_and(|(prev_at, (px, py))| {
                                    now.saturating_duration_since(prev_at)
                                        <= Duration::from_millis(450)
                                        && (self.cursor_dip.0 - px).abs() <= 6.0
                                        && (self.cursor_dip.1 - py).abs() <= 6.0
                                });
                        if is_double_click {
                            self.pending_video_click = None;
                            if self.bridge.ui.double_click_fullscreen
                                && !self.bridge.ui.locked
                                && self.bridge.ui.active_modal.is_none()
                            {
                                if let Some(st) = &self.state {
                                    let next = !st.subclass.is_fullscreen();
                                    let _ = st.subclass.set_fullscreen(next);
                                }
                                self.sync_window_maximize_state();
                                self.ui_dirty = true;
                            }
                        } else {
                            self.last_left_release = Some((now, self.cursor_dip));
                            if !self.bridge.ui.locked && self.bridge.ui.active_modal.is_none() {
                                if self.bridge.ui.playlist.open {
                                    self.bridge.toggle_playlist();
                                } else if !self.bridge.ui.welcome_active
                                    && self.bridge.current_file.is_some()
                                {
                                    if !self.bridge.ui.double_click_fullscreen {
                                        self.bridge.toggle_play();
                                    } else {
                                        self.pending_video_click =
                                            Some(now + Duration::from_millis(250));
                                    }
                                }
                            }
                        }
                    }
                }

                self.ui_dirty = true;
                if let Some(st) = &self.state {
                    st.window.request_redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let dpr = self
                    .state
                    .as_ref()
                    .map(|s| s.window.scale_factor() as f32)
                    .unwrap_or(1.0)
                    .max(0.5);
                let scroll_dy = match delta {
                    MouseScrollDelta::LineDelta(_x, y) => -y * 42.0,
                    MouseScrollDelta::PixelDelta(PhysicalPosition { x: _, y }) => -(y as f32) / dpr,
                };

                let hovered_target = self
                    .state
                    .as_ref()
                    .and_then(|s| s.last_frame_output.as_ref())
                    .and_then(|f| {
                        self.ui_engine
                            .hit_test(f, self.cursor_dip.0, self.cursor_dip.1)
                    })
                    .map(|r| r.target.clone());
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (x, y),
                    MouseScrollDelta::PixelDelta(PhysicalPosition { x, y }) => (x as f32, y as f32),
                };

                if self.bridge.ui.active_modal == Some(ModalId::Settings) {
                    let over_shortcuts = self
                        .state
                        .as_ref()
                        .and_then(|s| s.last_frame_output.as_ref())
                        .is_some_and(|f| {
                            f.hit_regions.iter().any(|r| {
                                matches!(
                                    r.target,
                                    HitTarget::ShortcutScrollArea
                                        | HitTarget::ShortcutRebind(_)
                                        | HitTarget::ShortcutResetOne(_)
                                ) && r.rect.contains(self.cursor_dip.0, self.cursor_dip.1)
                            })
                        });
                    if over_shortcuts {
                        let max_scroll = self.bridge.ui.shortcut_scroll_max();
                        self.bridge.ui.shortcut_scroll_y =
                            (self.bridge.ui.shortcut_scroll_y + scroll_dy).clamp(0.0, max_scroll);
                    } else {
                        self.bridge.scroll_settings_modal(scroll_dy);
                    }
                } else if self.bridge.ui.active_modal == Some(ModalId::BgSettings) {
                    self.bridge.scroll_bg_settings_modal(scroll_dy);
                } else if self.bridge.ui.active_modal.is_some() {
                    // Do not trigger background volume scroll while a modal is open.
                } else if self.bridge.ui.playlist.open
                    && self.cursor_dip.0 >= (self.bridge.ui.viewport_w - 320.0).max(0.0)
                {
                    let total = self.bridge.ui.playlist.filtered_items().len();
                    let client_h = (self.bridge.ui.viewport_h - 190.0).max(120.0);
                    let max_scroll = ((total as f32) * PLAYLIST_ITEM_HEIGHT - client_h).max(0.0);
                    self.bridge.ui.playlist.scroll_top_px =
                        (self.bridge.ui.playlist.scroll_top_px + scroll_dy).clamp(0.0, max_scroll);
                } else if !self.bridge.ui.locked
                    && matches!(
                        hovered_target,
                        Some(
                            HitTarget::VolumeSlider
                                | HitTarget::MuteToggle
                                | HitTarget::VolumeContainerSurface
                        )
                    )
                {
                    if dy.abs() >= 1.0 && dx.abs() <= dy.abs() {
                        let step = if dy > 0.0 { 5 } else { -5 };
                        self.bridge.adjust_volume(step);
                    }
                } else if self.bridge.ui.welcome_active
                    && self.bridge.ui.welcome_background == WelcomeBackgroundKind::Gridmotion
                    && !self.bridge.ui.bg_accordion_open
                    && hovered_target.is_none()
                {
                    if self.mods.control_key() {
                        let step = if scroll_dy < 0.0 { 0.05 } else { -0.05 };
                        let next_scale = self.bridge.ui.gridmotion_cfg.scale + step;
                        self.bridge.update_gridmotion_scale_from_wheel(next_scale);
                    } else if let Some(st) = &mut self.state {
                        st.gpu_bridge
                            .compositor
                            .graph
                            .effects
                            .grid_motion
                            .apply_wheel_scroll(scroll_dy);
                    }
                } else {
                    let ctx = self.current_input_context();
                    if let Some(act) = self.input.on_mouse_wheel(Instant::now(), dx, dy, &ctx) {
                        self.dispatch_input_actions(vec![act], event_loop);
                    }
                }

                self.ui_dirty = true;
                if let Some(st) = &self.state {
                    st.window.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    let web_code = map_key_code(code);
                    let mods = KeyModifiers {
                        ctrl: self.mods.control_key(),
                        alt: self.mods.alt_key(),
                        shift: self.mods.shift_key(),
                        meta: self.mods.super_key(),
                    };

                    // 1. Recording a shortcut rebind in Settings (`settings.js:1920-2184`)
                    if let Some(action_id) = self.bridge.ui.recording_shortcut_id.clone() {
                        let is_pure_mod = matches!(
                            code,
                            KeyCode::ControlLeft
                                | KeyCode::ControlRight
                                | KeyCode::AltLeft
                                | KeyCode::AltRight
                                | KeyCode::ShiftLeft
                                | KeyCode::ShiftRight
                                | KeyCode::SuperLeft
                                | KeyCode::SuperRight
                        );
                        if is_pure_mod {
                            let mut parts = Vec::new();
                            if mods.ctrl {
                                parts.push("Ctrl");
                            }
                            if mods.alt {
                                parts.push("Alt");
                            }
                            if mods.shift {
                                parts.push("Shift");
                            }
                            if mods.meta {
                                parts.push("Meta");
                            }
                            self.bridge.ui.recording_shortcut_preview = if parts.is_empty() {
                                None
                            } else {
                                Some(format!("{} + ...", parts.join(" + ")))
                            };
                            if let Some(st) = &self.state {
                                st.window.request_redraw();
                            }
                            return;
                        }
                        if event.state == ElementState::Pressed && !event.repeat {
                            if code == KeyCode::Escape && action_id != "exit-fullscreen" {
                                self.bridge.ui.recording_shortcut_id = None;
                                self.bridge.ui.recording_shortcut_preview = None;
                            } else if !web_code.is_empty() {
                                let combo = build_combo_string(web_code, mods);
                                self.bridge.commit_shortcut_rebind(&action_id, &combo);
                                self.input
                                    .shortcuts
                                    .apply_custom_shortcuts(&self.bridge.ui.custom_shortcuts);
                            }
                            if let Some(st) = &self.state {
                                st.window.request_redraw();
                            }
                        }
                        return;
                    }

                    // 2. Modal Tab / Shift+Tab focus trap (`settings.js:382-516`)
                    if code == KeyCode::Tab
                        && event.state == ElementState::Pressed
                        && self.bridge.ui.active_modal.is_some()
                    {
                        let focusables = self
                            .state
                            .as_ref()
                            .and_then(|s| s.last_frame_output.as_ref())
                            .map(collect_modal_focusables)
                            .unwrap_or_default();
                        if !focusables.is_empty() {
                            let cur_idx = self
                                .bridge
                                .ui
                                .focused_target
                                .as_ref()
                                .and_then(|cur| focusables.iter().position(|t| t == cur));
                            let next_idx = if mods.shift {
                                match cur_idx {
                                    Some(0) | None => focusables.len() - 1,
                                    Some(i) => i - 1,
                                }
                            } else {
                                match cur_idx {
                                    Some(i) => (i + 1) % focusables.len(),
                                    None => 0,
                                }
                            };
                            let next_target = focusables[next_idx].clone();
                            let next_input = match next_target {
                                HitTarget::PlaylistSearch => Some(FocusedTextField::PlaylistSearch),
                                HitTarget::NetworkStreamInput => {
                                    Some(FocusedTextField::NetworkStreamUrl)
                                }
                                HitTarget::ShortcutSearch => Some(FocusedTextField::ShortcutSearch),
                                _ => None,
                            };
                            self.bridge.set_focused_input(next_input);
                            self.bridge.ui.focused_target = Some(next_target);
                            self.bridge.ui.focus_visible = true;
                            self.ui_dirty = true;
                            if let Some(st) = &self.state {
                                st.window.request_redraw();
                            }
                        }
                        return;
                    }

                    // 3. Typing into focused UI text fields (Playlist search, Network URL, Shortcut search)
                    if let Some(focused) = self.bridge.focused_input {
                        if event.state == ElementState::Pressed {
                            match code {
                                KeyCode::Escape => {
                                    self.bridge.set_focused_input(None);
                                    if self.bridge.ui.active_modal.is_some()
                                        || self.bridge.ui.active_dropdown.is_some()
                                        || self.bridge.ui.active_color_picker.is_some()
                                        || self.bridge.ui.bg_accordion_open
                                    {
                                        self.dispatch_input_actions(
                                            vec![InputAction::DismissModalOrPopover],
                                            event_loop,
                                        );
                                    }
                                }
                                KeyCode::Backspace => {
                                    self.bridge.backspace_focused_input();
                                }
                                KeyCode::Delete => {
                                    self.bridge.delete_focused_input();
                                }
                                KeyCode::ArrowLeft => {
                                    self.bridge.move_focused_input_left(mods.shift);
                                }
                                KeyCode::ArrowRight => {
                                    self.bridge.move_focused_input_right(mods.shift);
                                }
                                KeyCode::Home => {
                                    self.bridge.home_focused_input(mods.shift);
                                }
                                KeyCode::End => {
                                    self.bridge.end_focused_input(mods.shift);
                                }
                                KeyCode::KeyA if mods.ctrl && !mods.alt && !mods.meta => {
                                    self.bridge.select_all_focused_input();
                                }
                                KeyCode::KeyC if mods.ctrl && !mods.alt && !mods.meta => {
                                    self.bridge.copy_focused_input();
                                }
                                KeyCode::KeyX if mods.ctrl && !mods.alt && !mods.meta => {
                                    self.bridge.cut_focused_input();
                                }
                                KeyCode::KeyV if mods.ctrl && !mods.alt && !mods.meta => {
                                    self.bridge.paste_focused_input();
                                }
                                KeyCode::Enter | KeyCode::NumpadEnter
                                    if focused == FocusedTextField::NetworkStreamUrl =>
                                {
                                    self.bridge.handle_hit_target(&HitTarget::NetworkStreamOpen);
                                }
                                _ if !mods.ctrl && !mods.alt && !mods.meta => {
                                    if let Some(text) = event.text.as_deref() {
                                        self.bridge.append_text_to_focused_input(text);
                                    }
                                }
                                _ => {}
                            }
                            self.ui_dirty = true;
                            if let Some(st) = &self.state {
                                st.window.request_redraw();
                            }
                        }
                        return;
                    }

                    // 4. Keyboard activation (Enter / Space) on focused interactive modal controls
                    if self.bridge.ui.active_modal.is_some()
                        && event.state == ElementState::Pressed
                        && !event.repeat
                        && !mods.ctrl
                        && !mods.alt
                        && !mods.meta
                        && matches!(code, KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space)
                    {
                        if let Some(target) = self.bridge.ui.focused_target.clone() {
                            if is_interactive_control_for_space(&target) {
                                self.handle_hit_action(&target, None, None, event_loop);
                                self.ui_dirty = true;
                                if let Some(st) = &self.state {
                                    st.window.request_redraw();
                                }
                                return;
                            }
                        }
                    }

                    let ctx = self.current_input_context();
                    let actions = match event.state {
                        ElementState::Pressed => self.input.on_key_down(
                            Instant::now(),
                            web_code,
                            code as u32,
                            mods,
                            event.repeat,
                            &ctx,
                        ),
                        ElementState::Released => {
                            self.input.on_key_up(web_code, code as u32, mods, &ctx)
                        }
                    };
                    self.dispatch_input_actions(actions, event_loop);
                    self.ui_dirty = true;
                    if let Some(st) = &self.state {
                        st.window.request_redraw();
                    }
                }
            }
            WindowEvent::HoveredFile(_) => {
                if !self.bridge.ui.drop_overlay_active {
                    self.bridge.ui.drop_overlay_active = true;
                    self.ui_dirty = true;
                    if let Some(st) = &self.state {
                        st.window.request_redraw();
                    }
                }
            }
            WindowEvent::HoveredFileCancelled => {
                if self.bridge.ui.drop_overlay_active {
                    self.bridge.ui.drop_overlay_active = false;
                    self.ui_dirty = true;
                    if let Some(st) = &self.state {
                        st.window.request_redraw();
                    }
                }
            }
            WindowEvent::DroppedFile(path) => {
                self.bridge.ui.drop_overlay_active = false;
                let has_media =
                    !self.bridge.ui.welcome_active || self.bridge.current_file.is_some();
                match classify_dropped_files(&[path], has_media) {
                    DroppedFilesPlan::LoadSubtitlesOnly(subs) => {
                        self.bridge.add_companion_subtitles(&subs);
                    }
                    DroppedFilesPlan::SubtitlesWithoutMedia => {
                        self.bridge.push_toast(
                            ToastKind::Info,
                            "Open a video first before loading subtitles",
                        );
                    }
                    DroppedFilesPlan::OpenMedia {
                        media_files,
                        companion_subtitles,
                        replace_playlist,
                    } => {
                        self.bridge.open_files(&media_files, replace_playlist, true);
                        if !companion_subtitles.is_empty() {
                            self.bridge.add_companion_subtitles(&companion_subtitles);
                        }
                    }
                    DroppedFilesPlan::Empty => {}
                }
                self.ui_dirty = true;
                if let Some(st) = &self.state {
                    st.window.request_redraw();
                }
            }
            WindowEvent::Focused(false) => {
                self.active_slider_drag = None;
                self.bridge.is_dragging_progress = false;
                self.bridge.ui.is_dragging_progress = false;
                self.bridge.flush_eq_debounce();
                self.bridge.ui.volume_spring.release();
                self.bridge.ui.hovered_target = None;
                self.bridge.ui.hover_tooltip_elapsed_secs = 0.0;
                self.bridge.ui.hover_thumb_norm = None;
                self.bridge.ui.hover_thumb_has_frame = false;
                self.bridge.ui.hover_thumb_loading = false;
                self.pending_video_click = None;
                let actions = self.input.on_focus_lost();
                self.dispatch_input_actions(actions, event_loop);
                self.ui_dirty = true;
                if let Some(st) = &self.state {
                    st.window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                self.render_frame();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let dirty_bridge = self.bridge.poll_events();
        if self.bridge.quit_requested {
            event_loop.exit();
            return;
        }

        if let Some(deadline) = self.pending_video_click {
            if now >= deadline {
                self.pending_video_click = None;
                if !self.bridge.ui.locked
                    && self.bridge.ui.active_modal.is_none()
                    && !self.bridge.ui.bg_accordion_open
                    && !self.bridge.ui.welcome_active
                {
                    self.bridge.toggle_play();
                    self.ui_dirty = true;
                }
            }
        }

        self.input.burst_in_flight = self.bridge.burst_in_flight;
        if let Some(fps) = self.bridge.video_fps {
            self.input.set_burst_fps(fps);
        }
        let ctx = self.current_input_context();
        let (actions, hide_cursor, hide_chrome) = self.input.tick(now, &ctx);
        let had_actions = !actions.is_empty();
        self.dispatch_input_actions(actions, event_loop);

        let auto_hide_changed = self.last_auto_hide != hide_chrome;
        self.last_auto_hide = hide_chrome;
        self.bridge.ui.auto_hide_controls = hide_chrome;
        if auto_hide_changed && hide_chrome {
            self.bridge.ui.hovered_target = None;
            self.bridge.ui.hover_tooltip_elapsed_secs = 0.0;
            self.bridge.ui.hover_thumb_norm = None;
            self.bridge.ui.hover_thumb_has_frame = false;
            self.bridge.ui.hover_thumb_loading = false;
        }

        let anim_active = has_active_ui_animations(&self.bridge.ui);

        if let Some(st) = &self.state {
            st.window.set_cursor_visible(!hide_cursor);
            if dirty_bridge
                || had_actions
                || self.ui_dirty
                || auto_hide_changed
                || anim_active
                || self.bridge.ui.welcome_active
                || self.bridge.ui.playing
                || self.bridge.clip_started_at.is_some()
            {
                st.window.request_redraw();
            }
        }

        let mut next_wake = self.input.next_wake_deadline(now, &ctx);
        if let Some(click_deadline) = self.pending_video_click {
            next_wake = Some(match next_wake {
                Some(d) => d.min(click_deadline),
                None => click_deadline,
            });
        }
        if anim_active {
            let anim_deadline = now + Duration::from_millis(16);
            next_wake = Some(match next_wake {
                Some(d) => d.min(anim_deadline),
                None => anim_deadline,
            });
        }

        if let Some(deadline) = next_wake {
            event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}

fn has_active_ui_animations(ui: &UiState) -> bool {
    let controls_target = if ui.auto_hide_controls || ui.locked {
        0.0
    } else {
        1.0
    };
    ui.volume_spring.overflow.abs() > 0.05
        || ui.volume_spring.velocity.abs() > 0.05
        || ui.anim.modal_t < 1.0
        || ui.anim.modal_closing
        || ui.anim.playlist_t < 1.0
        || ui.anim.playlist_closing
        || ui.anim.bg_accordion_t < 1.0
        || ui.anim.bg_accordion_closing
        || ui.anim.accordion_card_animating()
        || (ui.anim.controls_t - controls_target).abs() > 0.002
        || ui.anim.hover_thumb_t < 1.0
        || ui.anim.toast_t < 1.0
        || ui
            .toasts
            .iter()
            .any(|t| !t.downloading || t.enter_t < 1.0 || t.closing)
        || ui.anim.skip_t < 1.0
        || ui.anim.screenshot_t < 1.0
        || (ui.carousel_mode == CarouselMode::SinglePreview && ui.anim.screenshot_ttl_secs > 0.0)
        || (ui.screenshot_notice.is_some() && ui.anim.screenshot_notice_ttl_secs > 0.0)
        || ui.skip_indicator.visible
        || ui.volume_osd.visible
        || ui.loading_visible
        || matches!(
            ui.recording_indicator,
            RecordingIndicatorMode::BurstActive { .. }
                | RecordingIndicatorMode::ClipActive { .. }
                | RecordingIndicatorMode::Saved { .. }
        )
        || ui.ab_loop != hybrid_ui::state::AbLoopState::Off
        || (ui.hover_tooltip_elapsed_secs < hybrid_ui::state::HOVER_TOOLTIP_DELAY_SECS
            && ui
                .hovered_target
                .as_ref()
                .is_some_and(|t| hybrid_ui::layout::target_has_tooltip(ui, t)))
}

fn is_slider_target(target: &HitTarget) -> bool {
    match target {
        HitTarget::ProgressSeek
        | HitTarget::VolumeSlider
        | HitTarget::EqBandSlider(_)
        | HitTarget::SpeedSlider
        | HitTarget::SubDelaySlider
        | HitTarget::SubtitleFontSizeSlider
        | HitTarget::SubtitleBgOpacitySlider
        | HitTarget::ColorPickerSpectrum
        | HitTarget::ColorPickerChannel(_) => true,
        HitTarget::BgControlInput(id) => BG_CONTROLS_CATALOG
            .iter()
            .any(|c| c.id == id && matches!(c.input_type, BgControlInputType::Range { .. })),
        _ => false,
    }
}

fn scale_a11y_tree_for_window(tree: &accesskit::TreeUpdate, dpr: f64) -> accesskit::TreeUpdate {
    let mut scaled = tree.clone();
    if dpr.is_finite() && (dpr - 1.0).abs() > 1e-3 && dpr > 0.0 {
        for (_, node) in &mut scaled.nodes {
            if let Some(b) = node.bounds() {
                node.set_bounds(accesskit::Rect {
                    x0: b.x0 * dpr,
                    y0: b.y0 * dpr,
                    x1: b.x1 * dpr,
                    y1: b.y1 * dpr,
                });
            }
        }
    }
    scaled
}

/// Resolves an AccessKit `NodeId` back to its backing UI [`HitTarget`] via matching `bounds`.
pub fn find_hit_target_for_accesskit_node(
    frame: &UiFrameOutput,
    node_id: accesskit::NodeId,
) -> Option<HitTarget> {
    let (_, node) = frame
        .a11y_tree
        .nodes
        .iter()
        .find(|(id, _)| *id == node_id)?;
    let b = node.bounds()?;
    let cx = ((b.x0 + b.x1) * 0.5) as f32;
    let cy = ((b.y0 + b.y1) * 0.5) as f32;

    if let Some(exact) = frame.hit_regions.iter().find(|r| {
        (r.rect.x as f64 - b.x0).abs() < 0.5
            && (r.rect.y as f64 - b.y0).abs() < 0.5
            && ((r.rect.x + r.rect.width) as f64 - b.x1).abs() < 0.5
            && ((r.rect.y + r.rect.height) as f64 - b.y1).abs() < 0.5
    }) {
        return Some(exact.target.clone());
    }

    frame
        .hit_regions
        .iter()
        .filter(|r| r.rect.contains(cx, cy))
        .max_by_key(|r| r.z_index)
        .map(|r| r.target.clone())
}

/// Dispatches an AccessKit `ActionRequest` against `MediaBridge` and `InputController`.
pub fn dispatch_accesskit_request(
    bridge: &mut MediaBridge,
    input: &mut InputController,
    frame: &UiFrameOutput,
    req: &ActionRequest,
) -> bool {
    let Some(target) = find_hit_target_for_accesskit_node(frame, req.target_node) else {
        return false;
    };

    match req.action {
        Action::Focus => {
            match target {
                HitTarget::PlaylistSearch => {
                    bridge.set_focused_input(Some(FocusedTextField::PlaylistSearch));
                }
                HitTarget::NetworkStreamInput => {
                    bridge.set_focused_input(Some(FocusedTextField::NetworkStreamUrl));
                }
                HitTarget::ShortcutSearch => {
                    bridge.set_focused_input(Some(FocusedTextField::ShortcutSearch));
                }
                _ => {
                    bridge.set_focused_input(None);
                }
            }
            bridge.ui.focused_target = Some(target);
            bridge.ui.focus_visible = true;
            true
        }
        Action::Click => {
            bridge.handle_hit_target(&target);
            if matches!(
                target,
                HitTarget::ShortcutResetOne(_) | HitTarget::ShortcutResetAll
            ) {
                input
                    .shortcuts
                    .apply_custom_shortcuts(&bridge.ui.custom_shortcuts);
            } else if matches!(
                target,
                HitTarget::SettingsMouseScreenshotSelect | HitTarget::DropdownOption(_)
            ) {
                input
                    .shortcuts
                    .set_screenshot_mouse_button(bridge.ui.screenshot_mouse_button);
            }
            true
        }
        Action::SetValue => match req.data.as_ref() {
            Some(ActionData::NumericValue(v)) => match target {
                HitTarget::ProgressSeek => {
                    bridge.ui.current_secs = v.clamp(0.0, bridge.ui.duration_secs.max(1.0));
                    if let Some(mpv) = &bridge.mpv {
                        let _ = mpv.seek(bridge.ui.current_secs, "absolute");
                    }
                    true
                }
                HitTarget::VolumeSlider => {
                    bridge.set_volume_pct(v.round().clamp(0.0, 200.0) as u32);
                    true
                }
                HitTarget::EqBandSlider(idx) => {
                    bridge.set_eq_band(idx, *v);
                    true
                }
                HitTarget::SpeedSlider => {
                    bridge.set_speed(*v);
                    true
                }
                HitTarget::SubDelaySlider => {
                    bridge.set_subtitle_delay(*v);
                    true
                }
                HitTarget::SubtitleFontSizeSlider => {
                    bridge.set_subtitle_font_size(v.round().clamp(12.0, 64.0) as u32);
                    true
                }
                HitTarget::SubtitleBgOpacitySlider => {
                    bridge.set_subtitle_bg_opacity(*v);
                    true
                }
                _ => false,
            },
            Some(ActionData::Value(text)) => match target {
                HitTarget::PlaylistSearch => {
                    bridge
                        .ui
                        .text_field_set(FocusedTextField::PlaylistSearch, text);
                    true
                }
                HitTarget::NetworkStreamInput => {
                    bridge
                        .ui
                        .text_field_set(FocusedTextField::NetworkStreamUrl, text);
                    true
                }
                HitTarget::ShortcutSearch => {
                    bridge
                        .ui
                        .text_field_set(FocusedTextField::ShortcutSearch, text);
                    true
                }
                _ => false,
            },
            _ => false,
        },
        Action::Increment | Action::Decrement => {
            let dir = if req.action == Action::Increment {
                1.0
            } else {
                -1.0
            };
            match target {
                HitTarget::ProgressSeek => {
                    bridge.seek_relative(dir * (bridge.ui.seek_step_secs as f64));
                    true
                }
                HitTarget::VolumeSlider => {
                    bridge.adjust_volume((dir as i32) * 5);
                    true
                }
                HitTarget::EqBandSlider(idx) => {
                    let cur = bridge.equalizer.bands.get(idx).copied().unwrap_or(0.0);
                    bridge.set_eq_band(idx, cur + dir);
                    true
                }
                HitTarget::SpeedSlider => {
                    bridge.set_speed(bridge.ui.speed + dir * 0.25);
                    true
                }
                HitTarget::SubDelaySlider => {
                    bridge.adjust_subtitle_delay(dir * 0.1);
                    true
                }
                HitTarget::SubtitleFontSizeSlider => {
                    let next =
                        (bridge.ui.subtitle_font_size as i32 + dir as i32).clamp(12, 64) as u32;
                    bridge.set_subtitle_font_size(next);
                    true
                }
                HitTarget::SubtitleBgOpacitySlider => {
                    let next = (bridge.subtitle_style.bg_opacity + dir * 0.05).clamp(0.0, 1.0);
                    bridge.set_subtitle_bg_opacity(next);
                    true
                }
                _ => false,
            }
        }
        _ => false,
    }
}

fn sync_hit_test_regions(subclass: &WindowSubclass, frame: &UiFrameOutput, _viewport_w: f32) {
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
            HitTarget::TitlebarMinimize | HitTarget::TitlebarClose => {
                exclusions.push(dr);
            }
            _ => {
                if region.rect.y < 40.0 {
                    exclusions.push(dr);
                }
            }
        }
    }

    subclass.set_maximize_button_rect(max_rect);
    subclass.set_titlebar_drag_rect(drag_rect);
    subclass.set_hit_test_exclusions(exclusions);
}

fn map_mouse_button(btn: MouseButton) -> Option<i32> {
    match btn {
        MouseButton::Middle => Some(1),
        MouseButton::Back => Some(3),
        MouseButton::Forward => Some(4),
        _ => None,
    }
}

/// Maps `winit::keyboard::KeyCode` to Web `KeyboardEvent.code` strings used by `ShortcutManager`.
pub fn map_key_code(code: KeyCode) -> &'static str {
    match code {
        KeyCode::Space => "Space",
        KeyCode::Enter => "Enter",
        KeyCode::Escape => "Escape",
        KeyCode::Tab => "Tab",
        KeyCode::Backspace => "Backspace",
        KeyCode::Delete => "Delete",
        KeyCode::Insert => "Insert",
        KeyCode::Home => "Home",
        KeyCode::End => "End",
        KeyCode::PageUp => "PageUp",
        KeyCode::PageDown => "PageDown",
        KeyCode::ArrowLeft => "ArrowLeft",
        KeyCode::ArrowRight => "ArrowRight",
        KeyCode::ArrowUp => "ArrowUp",
        KeyCode::ArrowDown => "ArrowDown",
        KeyCode::KeyA => "KeyA",
        KeyCode::KeyB => "KeyB",
        KeyCode::KeyC => "KeyC",
        KeyCode::KeyD => "KeyD",
        KeyCode::KeyE => "KeyE",
        KeyCode::KeyF => "KeyF",
        KeyCode::KeyG => "KeyG",
        KeyCode::KeyH => "KeyH",
        KeyCode::KeyI => "KeyI",
        KeyCode::KeyJ => "KeyJ",
        KeyCode::KeyK => "KeyK",
        KeyCode::KeyL => "KeyL",
        KeyCode::KeyM => "KeyM",
        KeyCode::KeyN => "KeyN",
        KeyCode::KeyO => "KeyO",
        KeyCode::KeyP => "KeyP",
        KeyCode::KeyQ => "KeyQ",
        KeyCode::KeyR => "KeyR",
        KeyCode::KeyS => "KeyS",
        KeyCode::KeyT => "KeyT",
        KeyCode::KeyU => "KeyU",
        KeyCode::KeyV => "KeyV",
        KeyCode::KeyW => "KeyW",
        KeyCode::KeyX => "KeyX",
        KeyCode::KeyY => "KeyY",
        KeyCode::KeyZ => "KeyZ",
        KeyCode::Digit0 => "Digit0",
        KeyCode::Digit1 => "Digit1",
        KeyCode::Digit2 => "Digit2",
        KeyCode::Digit3 => "Digit3",
        KeyCode::Digit4 => "Digit4",
        KeyCode::Digit5 => "Digit5",
        KeyCode::Digit6 => "Digit6",
        KeyCode::Digit7 => "Digit7",
        KeyCode::Digit8 => "Digit8",
        KeyCode::Digit9 => "Digit9",
        KeyCode::Numpad0 => "Numpad0",
        KeyCode::Numpad1 => "Numpad1",
        KeyCode::Numpad2 => "Numpad2",
        KeyCode::Numpad3 => "Numpad3",
        KeyCode::Numpad4 => "Numpad4",
        KeyCode::Numpad5 => "Numpad5",
        KeyCode::Numpad6 => "Numpad6",
        KeyCode::Numpad7 => "Numpad7",
        KeyCode::Numpad8 => "Numpad8",
        KeyCode::Numpad9 => "Numpad9",
        KeyCode::NumpadEnter => "NumpadEnter",
        KeyCode::NumpadAdd => "NumpadAdd",
        KeyCode::NumpadSubtract => "NumpadSubtract",
        KeyCode::NumpadMultiply => "NumpadMultiply",
        KeyCode::NumpadDivide => "NumpadDivide",
        KeyCode::NumpadDecimal => "NumpadDecimal",
        KeyCode::NumpadEqual => "NumpadEqual",
        KeyCode::CapsLock => "CapsLock",
        KeyCode::NumLock => "NumLock",
        KeyCode::ScrollLock => "ScrollLock",
        KeyCode::PrintScreen => "PrintScreen",
        KeyCode::Pause => "Pause",
        KeyCode::ContextMenu => "ContextMenu",
        KeyCode::F1 => "F1",
        KeyCode::F2 => "F2",
        KeyCode::F3 => "F3",
        KeyCode::F4 => "F4",
        KeyCode::F5 => "F5",
        KeyCode::F6 => "F6",
        KeyCode::F7 => "F7",
        KeyCode::F8 => "F8",
        KeyCode::F9 => "F9",
        KeyCode::F10 => "F10",
        KeyCode::F11 => "F11",
        KeyCode::F12 => "F12",
        KeyCode::Period => "Period",
        KeyCode::Comma => "Comma",
        KeyCode::Slash => "Slash",
        KeyCode::Backslash => "Backslash",
        KeyCode::Semicolon => "Semicolon",
        KeyCode::Quote => "Quote",
        KeyCode::Backquote => "Backquote",
        KeyCode::BracketLeft => "BracketLeft",
        KeyCode::BracketRight => "BracketRight",
        KeyCode::Equal => "Equal",
        KeyCode::Minus => "Minus",
        _ => "",
    }
}

/// Launches the live `winit` desktop player event loop.
pub fn run_desktop_app(
    cli: AppCliArgs,
    single_instance: SingleInstanceServer,
    user_data_dir: PathBuf,
) -> Result<(), String> {
    let rt = Arc::new(Runtime::new().map_err(|e| format!("tokio runtime failed: {e}"))?);
    let mut bridge = MediaBridge::new(user_data_dir, Arc::clone(&rt));
    if let Err(err) = bridge.init_mpv_engine() {
        eprintln!("[hybrid-app] mpv init deferred/unavailable: {err}");
    }

    let input = InputController::new(
        bridge.db.preferences.custom_shortcuts.clone(),
        bridge.db.preferences.screenshot_mouse_button,
    );

    let event_loop = EventLoop::<AppUserEvent>::with_user_event()
        .build()
        .map_err(|e| format!("winit EventLoop::build: {e}"))?;
    let proxy = event_loop.create_proxy();
    let wake_proxy = proxy.clone();
    bridge.set_wake_callback(Arc::new(move || {
        let _ = wake_proxy.send_event(AppUserEvent::Wake);
    }));

    let initial_theme = bridge.ui.theme;
    let initial_title = bridge.ui.window_title.clone();
    let mut app = DesktopApp {
        rt,
        cli,
        bridge,
        input,
        ui_engine: HybridUi::new(),
        proxy,
        single_instance: Some(single_instance),
        state: None,
        mods: ModifiersState::default(),
        cursor_dip: (0.0, 0.0),
        mouse_down: false,
        active_slider_drag: None,
        last_left_release: None,
        pending_video_click: None,
        started_at: Instant::now(),
        last_tick: Instant::now(),
        last_shell_theme: initial_theme,
        last_window_title: initial_title,
        last_gridmotion_gen: 0,
        last_auto_hide: false,
        ui_dirty: true,
    };

    event_loop
        .run_app(&mut app)
        .map_err(|e| format!("winit run_app: {e}"))
}

fn is_focusable_modal_target(target: &HitTarget) -> bool {
    !matches!(
        target,
        HitTarget::ModalBackdrop
            | HitTarget::ModalPanelSurface
            | HitTarget::DropdownBackdrop
            | HitTarget::PopoverSurface
            | HitTarget::BgAccordionSurface
            | HitTarget::ControlsBarSurface
            | HitTarget::PlaylistSurface
            | HitTarget::WelcomeContentSurface
            | HitTarget::VolumeContainerSurface
            | HitTarget::ShortcutScrollArea
            | HitTarget::ToastDismiss(_)
            | HitTarget::ToastUpdateNow
            | HitTarget::SinglePreviewOpenFolder
            | HitTarget::CarouselPrev
            | HitTarget::CarouselNext
            | HitTarget::CarouselConfirm
            | HitTarget::CarouselClose
            | HitTarget::RecordingStopClip
            | HitTarget::RecordingOpenFolder
    )
}

fn collect_modal_focusables(frame: &UiFrameOutput) -> Vec<HitTarget> {
    let mut focusables = Vec::new();
    for region in &frame.hit_regions {
        if region.z_index >= ZIndex::MODAL_OVERLAY
            && region.z_index < ZIndex::TOAST_CONTAINER
            && is_focusable_modal_target(&region.target)
            && !focusables.contains(&region.target)
        {
            focusables.push(region.target.clone());
        }
    }
    focusables
}

fn is_interactive_control_for_space(target: &HitTarget) -> bool {
    match target {
        HitTarget::ModalBackdrop
        | HitTarget::ModalPanelSurface
        | HitTarget::DropdownBackdrop
        | HitTarget::PopoverSurface
        | HitTarget::BgAccordionSurface
        | HitTarget::ControlsBarSurface
        | HitTarget::PlaylistSurface
        | HitTarget::WelcomeContentSurface
        | HitTarget::VolumeContainerSurface
        | HitTarget::ShortcutScrollArea
        | HitTarget::ProgressSeek
        | HitTarget::VolumeSlider
        | HitTarget::EqBandSlider(_)
        | HitTarget::SpeedSlider
        | HitTarget::SubDelaySlider
        | HitTarget::SubtitleFontSizeSlider
        | HitTarget::SubtitleBgOpacitySlider
        | HitTarget::PlaylistSearch
        | HitTarget::NetworkStreamInput
        | HitTarget::ShortcutSearch
        | HitTarget::ColorPickerSpectrum
        | HitTarget::ColorPickerChannel(_) => false,
        HitTarget::BgControlInput(id) => !BG_CONTROLS_CATALOG
            .iter()
            .any(|c| c.id == id && matches!(c.input_type, BgControlInputType::Range { .. })),
        _ => true,
    }
}
