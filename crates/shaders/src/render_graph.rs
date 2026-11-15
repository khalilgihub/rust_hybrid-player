use crate::blur::DualKawaseBlur;
use crate::effects::WelcomeEffectsBundle;
use crate::error::ShaderError;
use crate::glass::{GlassInstanceGpu, GlassRect, SdfGlassPass, SdfGlassRect};
use crate::scene::ScenePass;
use crate::types::{
    ClearContext, EffectInstanceConfig, VideoFrameTransform, WelcomeBackgroundKind,
};
use crate::ui_quad::{
    compute_scissor_rect, UiQuadInstanceGpu, UiQuadPass, UiTextureId, UiTexturedQuad,
};

/// Source rendered in Pass 0 (`SceneTex`).
pub enum SceneSource<'a> {
    /// Solid clear color (`#0c0d12` or `#000000`).
    ClearOnly,
    /// Aspect-fitted video texture from `mpv`.
    Video {
        texture_view: &'a wgpu::TextureView,
        transform: VideoFrameTransform,
    },
    /// Active Welcome Screen background shader (`none`, `dither`, `particles`, `faulty`, `gridmotion`).
    Welcome(WelcomeBackgroundKind),
}

/// Complete 3-Pass Render Graph:
/// - Pass 0: `SceneTex` (aspect-fitted video or Welcome Screen shader, `#0c0d12`/`#000000` clear)
/// - Pass 1: `Blur4..Blur32` Dual-Kawase downsample/upsample pyramid
/// - Pass 2: SDF Glassmorphism & Chameleon skin overlays
pub struct RenderGraph {
    pub scene: ScenePass,
    pub blur: DualKawaseBlur,
    pub glass: SdfGlassPass,
    pub effects: WelcomeEffectsBundle,
    pub force_blur_pyramid: bool,
}

impl RenderGraph {
    /// Creates a new 3-pass render graph targeting `present_format` at `(width, height)`.
    ///
    /// Offscreen `SceneTex` and `Blur4..Blur32` use `wgpu::TextureFormat::Rgba8Unorm` so CSS
    /// hex colors and shader math are never double-gamma-shifted.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        present_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        instance: EffectInstanceConfig,
    ) -> Result<Self, ShaderError> {
        let scene_format = wgpu::TextureFormat::Rgba8Unorm;
        let scene = ScenePass::new(device, scene_format, present_format, width, height)?;
        let blur = DualKawaseBlur::new(device, scene_format, scene.view(), width, height)?;
        let glass = SdfGlassPass::new(device, present_format, scene.view(), &blur);
        let effects = WelcomeEffectsBundle::new_full_bleed(device, queue, scene_format, instance);

        Ok(Self {
            scene,
            blur,
            glass,
            effects,
            force_blur_pyramid: false,
        })
    }

    /// Resizes `SceneTex` and rebuilds the Dual-Kawase blur pyramid if dimensions changed.
    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> Result<bool, ShaderError> {
        if !self.scene.resize(device, width, height)? {
            return Ok(false);
        }
        self.blur
            .rebuild(device, self.scene.view(), width, height)?;
        self.glass
            .rebind_textures(device, self.scene.view(), &self.blur);
        Ok(true)
    }

    /// Encodes all 3 passes into `encoder`, writing final composited output to `target_view`.
    #[allow(clippy::too_many_arguments)]
    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target_view: &wgpu::TextureView,
        clear: ClearContext,
        source: SceneSource<'_>,
        glass_rects: &[GlassRect],
        time_secs: f32,
    ) {
        let (w, h) = self.scene.dimensions();

        // Pass 0: SceneTex
        match source {
            SceneSource::ClearOnly => self.scene.encode_clear(encoder, clear),
            SceneSource::Video {
                texture_view,
                transform,
            } => {
                self.scene
                    .encode_video(device, queue, encoder, clear, texture_view, transform);
            }
            SceneSource::Welcome(kind) => {
                if kind == WelcomeBackgroundKind::None {
                    self.scene.encode_clear(encoder, clear);
                } else {
                    self.effects
                        .encode(device, queue, encoder, kind, self.scene.view(), w, h);
                }
            }
        }

        // Pass 1: Dual-Kawase blur pyramid (Blur4..Blur32)
        let needs_blur = self.force_blur_pyramid || glass_rects.iter().any(|r| r.blur_px > 0.001);
        if needs_blur {
            self.blur.encode(encoder);
        }

        // Present copy + Pass 2: SDF Glass & Chameleon
        self.scene.encode_copy_to(encoder, target_view);
        self.glass.encode(
            device,
            queue,
            encoder,
            target_view,
            w,
            h,
            time_secs,
            glass_rects,
        );
    }
}

/// Offscreen 400x240 @ 30fps preview targets for the 4 Welcome Effects in `#bgAccordionPopover`.
pub struct AccordionPreviewGallery {
    pub bundle: WelcomeEffectsBundle,
    textures: [wgpu::Texture; 4],
    views: [wgpu::TextureView; 4],
}

impl AccordionPreviewGallery {
    pub const PREVIEW_WIDTH: u32 = 400;
    pub const PREVIEW_HEIGHT: u32 = 240;

    /// Allocates four `400x240` `Rgba8Unorm` render targets and `30fps` preview effect instances.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let bundle = WelcomeEffectsBundle::new_accordion_previews(device, queue, format);

        let make_target = |label: &str| {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: Self::PREVIEW_WIDTH,
                    height: Self::PREVIEW_HEIGHT,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
            (tex, view)
        };

        let (t0, v0) = make_target("preview-dither-400x240");
        let (t1, v1) = make_target("preview-particles-400x240");
        let (t2, v2) = make_target("preview-faulty-400x240");
        let (t3, v3) = make_target("preview-gridmotion-400x240");

        Self {
            bundle,
            textures: [t0, t1, t2, t3],
            views: [v0, v1, v2, v3],
        }
    }

    /// Updates and renders any preview whose `30fps` frame pacing gate allows rendering at `now_secs`.
    pub fn render_due_previews(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        now_secs: f64,
        dt_secs: f32,
    ) -> [bool; 4] {
        let mut rendered = [false; 4];
        if self.bundle.dither.lifecycle.should_render(now_secs) {
            self.bundle.dither.update(dt_secs);
            self.bundle.dither.encode(
                queue,
                encoder,
                &self.views[0],
                Self::PREVIEW_WIDTH,
                Self::PREVIEW_HEIGHT,
            );
            rendered[0] = true;
        }
        if self.bundle.particles.lifecycle.should_render(now_secs) {
            self.bundle.particles.update(dt_secs);
            self.bundle.particles.encode(
                device,
                queue,
                encoder,
                &self.views[1],
                Self::PREVIEW_WIDTH,
                Self::PREVIEW_HEIGHT,
            );
            rendered[1] = true;
        }
        if self.bundle.faulty.lifecycle.should_render(now_secs) {
            self.bundle.faulty.update(dt_secs);
            self.bundle.faulty.encode(
                device,
                queue,
                encoder,
                &self.views[2],
                Self::PREVIEW_WIDTH,
                Self::PREVIEW_HEIGHT,
            );
            rendered[2] = true;
        }
        if self.bundle.grid_motion.lifecycle.should_render(now_secs) {
            self.bundle.grid_motion.update(dt_secs);
            self.bundle.grid_motion.encode(
                device,
                queue,
                encoder,
                &self.views[3],
                Self::PREVIEW_WIDTH,
                Self::PREVIEW_HEIGHT,
            );
            rendered[3] = true;
        }
        rendered
    }

    pub fn texture(&self, kind: WelcomeBackgroundKind) -> Option<&wgpu::Texture> {
        match kind {
            WelcomeBackgroundKind::Dither => Some(&self.textures[0]),
            WelcomeBackgroundKind::Particles => Some(&self.textures[1]),
            WelcomeBackgroundKind::Faulty => Some(&self.textures[2]),
            WelcomeBackgroundKind::Gridmotion => Some(&self.textures[3]),
            WelcomeBackgroundKind::None => None,
        }
    }

    pub fn view(&self, kind: WelcomeBackgroundKind) -> Option<&wgpu::TextureView> {
        match kind {
            WelcomeBackgroundKind::Dither => Some(&self.views[0]),
            WelcomeBackgroundKind::Particles => Some(&self.views[1]),
            WelcomeBackgroundKind::Faulty => Some(&self.views[2]),
            WelcomeBackgroundKind::Gridmotion => Some(&self.views[3]),
            WelcomeBackgroundKind::None => None,
        }
    }
}

/// Blur pyramid requirements for a frame passed to [`FrameCompositor::compose`].
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BlurRequests {
    /// Forces building `Blur4..Blur32` even when no `SdfGlassRect` has `blur_px > 0`.
    pub force_pyramid: bool,
    /// Maximum requested CSS blur radius in pixels (`0.0..=32.0`).
    pub max_blur_px: f32,
}

impl BlurRequests {
    /// No explicit blur override (Pass 1 runs automatically if any `SdfGlassRect` has `blur_px > 0`).
    pub const fn none() -> Self {
        Self {
            force_pyramid: false,
            max_blur_px: 0.0,
        }
    }

    /// Forces the full Dual-Kawase `Blur4..Blur32` pyramid to be built for the frame.
    pub const fn full() -> Self {
        Self {
            force_pyramid: true,
            max_blur_px: 32.0,
        }
    }

    /// Summarizes blur requirements across a slice of [`SdfGlassRect`] panels.
    pub fn from_glass_rects(rects: &[SdfGlassRect]) -> Self {
        let mut max_blur_px = 0.0f32;
        for r in rects {
            max_blur_px = max_blur_px.max(r.glass.blur_px);
        }
        Self {
            force_pyramid: false,
            max_blur_px,
        }
    }

    pub(crate) fn needs_pyramid(self, rects: &[SdfGlassRect]) -> bool {
        self.force_pyramid
            || self.max_blur_px > 0.001
            || rects.iter().any(|r| r.glass.blur_px > 0.001)
    }
}

#[derive(Clone, Copy)]
enum CompositorOp {
    Glass {
        z_index: i32,
        seq: usize,
        instance_idx: u32,
        scissor: [u32; 4],
    },
    Quad {
        z_index: i32,
        seq: usize,
        instance_idx: u32,
        texture: UiTextureId,
        scissor: [u32; 4],
    },
}

impl CompositorOp {
    fn sort_key(&self) -> (i32, u8, usize) {
        match *self {
            Self::Glass { z_index, seq, .. } => (z_index, 0, seq),
            Self::Quad { z_index, seq, .. } => (z_index, 1, seq),
        }
    }
}

/// Complete frame compositor uniting Pass 0 (`ScenePass`), Pass 1 (`DualKawaseBlur`),
/// Pass 2 (`SdfGlassPass` + Chameleon skins + `UiQuadPass` z-ordered & scissor-clipped),
/// surface resize, and `400x240 @ 30fps` accordion preview textures.
pub struct FrameCompositor {
    pub graph: RenderGraph,
    pub ui_quads: UiQuadPass,
    pub previews: AccordionPreviewGallery,
}

impl FrameCompositor {
    /// Creates a new compositor targeting `present_format` at `(width, height)`.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        present_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        instance: EffectInstanceConfig,
    ) -> Result<Self, ShaderError> {
        let graph = RenderGraph::new(device, queue, present_format, width, height, instance)?;
        let mut ui_quads = UiQuadPass::new(device, queue, present_format);
        let previews = AccordionPreviewGallery::new(device, queue);

        for (kind, id) in [
            (WelcomeBackgroundKind::Dither, UiTextureId::PREVIEW_DITHER),
            (
                WelcomeBackgroundKind::Particles,
                UiTextureId::PREVIEW_PARTICLES,
            ),
            (WelcomeBackgroundKind::Faulty, UiTextureId::PREVIEW_FAULTY),
            (
                WelcomeBackgroundKind::Gridmotion,
                UiTextureId::PREVIEW_GRIDMOTION,
            ),
        ] {
            if let Some(view) = previews.view(kind) {
                ui_quads.register_view(device, id, view);
            }
        }

        Ok(Self {
            graph,
            ui_quads,
            previews,
        })
    }

    /// Resizes `SceneTex` and rebuilds the Dual-Kawase blur pyramid if surface dimensions changed.
    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> Result<bool, ShaderError> {
        self.graph.resize(device, width, height)
    }

    /// Uploads a new `Rgba8Unorm` texture (SVGs, thumbnails, images) and returns its [`UiTextureId`].
    pub fn upload_rgba8(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        label: &str,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> UiTextureId {
        self.ui_quads
            .upload_rgba8(device, queue, label, width, height, rgba)
    }

    /// Updates an existing owned `Rgba8Unorm` texture in-place when dimensions match.
    pub fn update_rgba8(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        id: UiTextureId,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) {
        self.ui_quads
            .update_rgba8(device, queue, id, width, height, rgba);
    }

    /// Uploads a new `R8Unorm` glyph coverage atlas texture and returns its [`UiTextureId`].
    pub fn upload_r8(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        label: &str,
        width: u32,
        height: u32,
        mask: &[u8],
    ) -> UiTextureId {
        self.ui_quads
            .upload_r8(device, queue, label, width, height, mask)
    }

    /// Updates an existing owned `R8Unorm` glyph atlas texture in-place when dimensions match.
    pub fn update_r8(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        id: UiTextureId,
        width: u32,
        height: u32,
        mask: &[u8],
    ) {
        self.ui_quads
            .update_r8(device, queue, id, width, height, mask);
    }

    /// Registers an external `TextureView` (e.g. `mpv` seek thumbnail) for quad rendering.
    pub fn register_external_view(
        &mut self,
        device: &wgpu::Device,
        view: &wgpu::TextureView,
    ) -> UiTextureId {
        self.ui_quads.register_external_view(device, view)
    }

    /// Removes a dynamically allocated texture slot.
    pub fn remove_texture(&mut self, id: UiTextureId) -> bool {
        self.ui_quads.remove_texture(id)
    }

    /// Updates and renders any `400x240 @ 30fps` accordion preview due at `now_secs`.
    pub fn update_previews(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        now_secs: f64,
        dt_secs: f32,
    ) -> [bool; 4] {
        self.previews
            .render_due_previews(device, queue, encoder, now_secs, dt_secs)
    }

    /// Returns the `400x240` offscreen texture for `kind` (for readback/export).
    pub fn preview_texture(&self, kind: WelcomeBackgroundKind) -> Option<&wgpu::Texture> {
        self.previews.texture(kind)
    }

    /// Returns the `400x240` offscreen texture view for `kind`.
    pub fn preview_texture_view(&self, kind: WelcomeBackgroundKind) -> Option<&wgpu::TextureView> {
        self.previews.view(kind)
    }

    /// Returns the bound [`UiTextureId`] for sampling `kind`'s `400x240` preview inside a [`UiTexturedQuad`].
    pub const fn preview_texture_id(kind: WelcomeBackgroundKind) -> Option<UiTextureId> {
        UiTextureId::for_welcome_preview(kind)
    }

    /// Encodes Pass 0 (`SceneTex`), Pass 1 (`DualKawaseBlur`), Pass 2 (`SdfGlassRect` + `UiTexturedQuad`
    /// interleaved in `z_index` order with hardware + SDF rounded clipping) into `target_view`.
    #[allow(clippy::too_many_arguments)]
    pub fn compose(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target_view: &wgpu::TextureView,
        clear: ClearContext,
        source: SceneSource<'_>,
        blur_requests: BlurRequests,
        glass_rects: &[SdfGlassRect],
        ui_quads: &[UiTexturedQuad],
        time_secs: f32,
    ) {
        let (w, h) = self.graph.scene.dimensions();

        // Pass 0: SceneTex
        match source {
            SceneSource::ClearOnly => self.graph.scene.encode_clear(encoder, clear),
            SceneSource::Video {
                texture_view,
                transform,
            } => {
                self.graph.scene.encode_video(
                    device,
                    queue,
                    encoder,
                    clear,
                    texture_view,
                    transform,
                );
            }
            SceneSource::Welcome(kind) => {
                if kind == WelcomeBackgroundKind::None {
                    self.graph.scene.encode_clear(encoder, clear);
                } else {
                    self.graph.effects.encode(
                        device,
                        queue,
                        encoder,
                        kind,
                        self.graph.scene.view(),
                        w,
                        h,
                    );
                }
            }
        }

        // Pass 1: Dual-Kawase blur pyramid (Blur4..Blur32)
        if self.graph.force_blur_pyramid || blur_requests.needs_pyramid(glass_rects) {
            self.graph.blur.encode(encoder);
        }

        // Copy Pass 0 SceneTex to target
        self.graph.scene.encode_copy_to(encoder, target_view);

        // Pass 2: Z-ordered SDF Glass + Textured UI Quads
        let mut ops: Vec<CompositorOp> = Vec::with_capacity(glass_rects.len() + ui_quads.len());
        let mut glass_inputs: Vec<(i32, usize, usize, [u32; 4])> =
            Vec::with_capacity(glass_rects.len());
        let mut quad_inputs: Vec<(i32, usize, usize, [u32; 4])> =
            Vec::with_capacity(ui_quads.len());

        for (i, g) in glass_rects.iter().enumerate() {
            let eff_clip = g.clip_rect.or(g.glass.clip_rect);
            if let Some(scissor) = compute_scissor_rect(eff_clip, w, h) {
                glass_inputs.push((g.z_index, i, i, scissor));
            }
        }
        for (i, q) in ui_quads.iter().enumerate() {
            if let Some(scissor) = compute_scissor_rect(q.clip_rect, w, h) {
                quad_inputs.push((q.z_index, i, i, scissor));
            }
        }

        glass_inputs.sort_by_key(|&(z, seq, _, _)| (z, seq));
        quad_inputs.sort_by_key(|&(z, seq, _, _)| (z, seq));

        let glass_gpu: Vec<GlassInstanceGpu> = glass_inputs
            .iter()
            .map(|&(_, _, idx, _)| glass_rects[idx].glass.to_gpu())
            .collect();
        let quad_gpu: Vec<UiQuadInstanceGpu> = quad_inputs
            .iter()
            .map(|&(_, _, idx, _)| ui_quads[idx].to_gpu())
            .collect();

        for (inst_idx, &(z, seq, _, scissor)) in glass_inputs.iter().enumerate() {
            ops.push(CompositorOp::Glass {
                z_index: z,
                seq,
                instance_idx: inst_idx as u32,
                scissor,
            });
        }
        for (inst_idx, &(z, seq, orig_idx, scissor)) in quad_inputs.iter().enumerate() {
            ops.push(CompositorOp::Quad {
                z_index: z,
                seq,
                instance_idx: inst_idx as u32,
                texture: ui_quads[orig_idx].texture,
                scissor,
            });
        }

        if ops.is_empty() {
            return;
        }

        ops.sort_by_key(CompositorOp::sort_key);

        if !glass_gpu.is_empty() {
            self.graph
                .glass
                .upload_instances(device, queue, w, h, time_secs, &glass_gpu);
        }
        if !quad_gpu.is_empty() {
            self.ui_quads
                .upload_instances(device, queue, w, h, &quad_gpu);
        }

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("compositor-pass2"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        #[derive(PartialEq, Eq)]
        enum ActivePipe {
            None,
            Glass,
            Quad,
        }

        let mut active_pipe = ActivePipe::None;
        let mut active_scissor: Option<[u32; 4]> = None;
        let mut active_quad_tex: Option<UiTextureId> = None;

        let mut idx = 0usize;
        while idx < ops.len() {
            match ops[idx] {
                CompositorOp::Glass {
                    instance_idx,
                    scissor,
                    ..
                } => {
                    let start = instance_idx;
                    let mut end = start + 1;
                    let mut next = idx + 1;
                    while next < ops.len() {
                        if let CompositorOp::Glass {
                            instance_idx: n_inst,
                            scissor: n_sc,
                            ..
                        } = ops[next]
                        {
                            if n_sc == scissor && n_inst == end {
                                end += 1;
                                next += 1;
                                continue;
                            }
                        }
                        break;
                    }

                    if active_pipe != ActivePipe::Glass {
                        pass.set_pipeline(self.graph.glass.pipeline());
                        pass.set_bind_group(0, self.graph.glass.bind_group(), &[]);
                        pass.set_vertex_buffer(0, self.graph.glass.instance_buffer().slice(..));
                        active_pipe = ActivePipe::Glass;
                        active_quad_tex = None;
                    }
                    if active_scissor != Some(scissor) {
                        pass.set_scissor_rect(scissor[0], scissor[1], scissor[2], scissor[3]);
                        active_scissor = Some(scissor);
                    }
                    pass.draw(0..6, start..end);
                    idx = next;
                }
                CompositorOp::Quad {
                    instance_idx,
                    texture,
                    scissor,
                    ..
                } => {
                    let start = instance_idx;
                    let mut end = start + 1;
                    let mut next = idx + 1;
                    while next < ops.len() {
                        if let CompositorOp::Quad {
                            instance_idx: n_inst,
                            texture: n_tex,
                            scissor: n_sc,
                            ..
                        } = ops[next]
                        {
                            if n_tex == texture && n_sc == scissor && n_inst == end {
                                end += 1;
                                next += 1;
                                continue;
                            }
                        }
                        break;
                    }

                    if active_pipe != ActivePipe::Quad {
                        pass.set_pipeline(self.ui_quads.pipeline());
                        pass.set_vertex_buffer(0, self.ui_quads.instance_buffer().slice(..));
                        active_pipe = ActivePipe::Quad;
                    }
                    if active_quad_tex != Some(texture) {
                        pass.set_bind_group(0, self.ui_quads.bind_group_for(texture), &[]);
                        active_quad_tex = Some(texture);
                    }
                    if active_scissor != Some(scissor) {
                        pass.set_scissor_rect(scissor[0], scissor[1], scissor[2], scissor[3]);
                        active_scissor = Some(scissor);
                    }
                    pass.draw(0..6, start..end);
                    idx = next;
                }
            }
        }
    }
}
