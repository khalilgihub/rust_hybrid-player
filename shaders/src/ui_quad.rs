use std::collections::BTreeMap;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::types::WelcomeBackgroundKind;

/// Identifier for a texture bound in [`UiQuadPass`] or [`crate::FrameCompositor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct UiTextureId(pub u32);

impl UiTextureId {
    /// Built-in 1x1 opaque white `Rgba8Unorm` texture for solid quads.
    pub const WHITE: Self = Self(0);
    /// Reserved slot for the `400x240` Dither Waves accordion preview.
    pub const PREVIEW_DITHER: Self = Self(1);
    /// Reserved slot for the `400x240` Particles accordion preview.
    pub const PREVIEW_PARTICLES: Self = Self(2);
    /// Reserved slot for the `400x240` Faulty Terminal accordion preview.
    pub const PREVIEW_FAULTY: Self = Self(3);
    /// Reserved slot for the `400x240` GridMotion accordion preview.
    pub const PREVIEW_GRIDMOTION: Self = Self(4);

    const FIRST_DYNAMIC_ID: u32 = 16;

    /// Returns the reserved accordion preview texture ID for a welcome effect, if applicable.
    pub const fn for_welcome_preview(kind: WelcomeBackgroundKind) -> Option<Self> {
        match kind {
            WelcomeBackgroundKind::Dither => Some(Self::PREVIEW_DITHER),
            WelcomeBackgroundKind::Particles => Some(Self::PREVIEW_PARTICLES),
            WelcomeBackgroundKind::Faulty => Some(Self::PREVIEW_FAULTY),
            WelcomeBackgroundKind::Gridmotion => Some(Self::PREVIEW_GRIDMOTION),
            WelcomeBackgroundKind::None => None,
        }
    }
}

/// Texture sampling / tinting interpretation in `ui_quad.wgsl`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u32)]
pub enum UiSampleMode {
    /// Straight-alpha RGBA texture (`tex * tint`), e.g. decoded images & previews.
    #[default]
    Rgba = 0,
    /// Single-channel `R8Unorm` coverage mask (`vec4(tint.rgb, tint.a * tex.r)`), e.g. `cosmic-text` glyphs.
    MaskR8 = 1,
    /// Premultiplied RGBA pixmap (e.g. `resvg` / `tiny-skia` rasterized SVGs), un-premultiplied before alpha blending.
    PremultipliedRgba = 2,
    /// Ignores sampled texel and fills with `tint` directly (fast solid/rounded rects).
    SolidColor = 3,
    /// CSS linear gradient from `tint` (`0%`) to `grad_color` (`100%`) at `gradient_angle_deg`.
    LinearGradient = 4,
}

/// High-level UI textured, glyph, SVG, or solid quad primitive for Pass 2.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiTexturedQuad {
    /// Stacking order matching CSS z-index; lower values draw first.
    pub z_index: i32,
    /// Bounding rectangle `[x, y, width, height]` in target pixel coordinates (top-left origin).
    pub rect: [f32; 4],
    /// Normalized texture coordinates `[u0, v0, u1, v1]`.
    pub uv_rect: [f32; 4],
    /// RGBA multiplier in `0.0..=1.0` (or gradient start color when `sample_mode == LinearGradient`).
    pub tint: [f32; 4],
    /// Gradient end RGBA color in `0.0..=1.0` (used when `sample_mode == LinearGradient`).
    pub grad_color: [f32; 4],
    /// CSS linear-gradient angle in degrees (`0deg` = bottom->top, `180deg` = top->bottom).
    pub gradient_angle_deg: f32,
    /// Corner radii in CSS order `[top_left, top_right, bottom_right, bottom_left]` (px).
    pub corner_radii: [f32; 4],
    /// Optional SDF stroke width in pixels (`0.0` = no border).
    pub border_width: f32,
    /// Optional SDF border stroke color `[r, g, b, a]`.
    pub border_color: [f32; 4],
    /// Clockwise rotation around quad center in degrees.
    pub rotation_deg: f32,
    /// Texture slot bound in [`UiQuadPass`].
    pub texture: UiTextureId,
    /// How `texture` channels combine with `tint`.
    pub sample_mode: UiSampleMode,
    /// Optional clip rectangle `[x, y, width, height]` in target pixels (hardware scissor + fragment clip).
    pub clip_rect: Option<[f32; 4]>,
    /// Optional rounded clip corner radii `[top_left, top_right, bottom_right, bottom_left]` (px).
    pub clip_corner_radii: [f32; 4],
}

impl Default for UiTexturedQuad {
    fn default() -> Self {
        Self {
            z_index: 0,
            rect: [0.0, 0.0, 0.0, 0.0],
            uv_rect: [0.0, 0.0, 1.0, 1.0],
            tint: [1.0, 1.0, 1.0, 1.0],
            grad_color: [1.0, 1.0, 1.0, 1.0],
            gradient_angle_deg: 180.0,
            corner_radii: [0.0; 4],
            border_width: 0.0,
            border_color: [0.0; 4],
            rotation_deg: 0.0,
            texture: UiTextureId::WHITE,
            sample_mode: UiSampleMode::Rgba,
            clip_rect: None,
            clip_corner_radii: [0.0; 4],
        }
    }
}

impl UiTexturedQuad {
    /// Creates an untextured solid quad with `[x, y, width, height]` and RGBA color.
    pub fn solid(z_index: i32, rect: [f32; 4], color: [f32; 4]) -> Self {
        Self {
            z_index,
            rect,
            tint: color,
            texture: UiTextureId::WHITE,
            sample_mode: UiSampleMode::SolidColor,
            ..Default::default()
        }
    }

    /// Creates an untextured CSS linear-gradient quad from `start_color` (`0%`) to `end_color` (`100%`).
    pub fn gradient(
        z_index: i32,
        rect: [f32; 4],
        start_color: [f32; 4],
        end_color: [f32; 4],
        angle_deg: f32,
    ) -> Self {
        Self {
            z_index,
            rect,
            tint: start_color,
            grad_color: end_color,
            gradient_angle_deg: angle_deg,
            texture: UiTextureId::WHITE,
            sample_mode: UiSampleMode::LinearGradient,
            ..Default::default()
        }
    }

    /// Creates a straight-alpha textured quad sampling `texture` across full `[0..1]` UVs.
    pub fn textured(z_index: i32, rect: [f32; 4], texture: UiTextureId) -> Self {
        Self {
            z_index,
            rect,
            texture,
            sample_mode: UiSampleMode::Rgba,
            ..Default::default()
        }
    }

    /// Creates a tinted `R8Unorm` glyph mask quad (`cosmic-text` / `parley` atlas page).
    pub fn glyph_mask(
        z_index: i32,
        rect: [f32; 4],
        uv_rect: [f32; 4],
        texture: UiTextureId,
        color: [f32; 4],
    ) -> Self {
        Self {
            z_index,
            rect,
            uv_rect,
            tint: color,
            texture,
            sample_mode: UiSampleMode::MaskR8,
            ..Default::default()
        }
    }

    /// Creates a premultiplied RGBA quad (such as a `resvg` SVG rasterization) with tint.
    pub fn premultiplied_svg(
        z_index: i32,
        rect: [f32; 4],
        texture: UiTextureId,
        tint: [f32; 4],
    ) -> Self {
        Self {
            z_index,
            rect,
            tint,
            texture,
            sample_mode: UiSampleMode::PremultipliedRgba,
            ..Default::default()
        }
    }

    pub fn with_uv_rect(mut self, uv_rect: [f32; 4]) -> Self {
        self.uv_rect = uv_rect;
        self
    }

    pub fn with_tint(mut self, tint: [f32; 4]) -> Self {
        self.tint = tint;
        self
    }

    pub fn with_corner_radius(mut self, radius: f32) -> Self {
        self.corner_radii = [radius; 4];
        self
    }

    pub fn with_corner_radii(mut self, corner_radii: [f32; 4]) -> Self {
        self.corner_radii = corner_radii;
        self
    }

    pub fn with_border(mut self, width: f32, color: [f32; 4]) -> Self {
        self.border_width = width;
        self.border_color = color;
        self
    }

    pub fn with_rotation_deg(mut self, rotation_deg: f32) -> Self {
        self.rotation_deg = rotation_deg;
        self
    }

    pub fn with_sample_mode(mut self, mode: UiSampleMode) -> Self {
        self.sample_mode = mode;
        self
    }

    pub fn with_clip_rect(mut self, clip_rect: [f32; 4]) -> Self {
        self.clip_rect = Some(clip_rect);
        self.clip_corner_radii = [0.0; 4];
        self
    }

    pub fn with_rounded_clip(mut self, clip_rect: [f32; 4], corner_radii: [f32; 4]) -> Self {
        self.clip_rect = Some(clip_rect);
        self.clip_corner_radii = corner_radii;
        self
    }

    /// Scales pixel geometry (`rect`, `corner_radii`, `border_width`, `clip_rect`, `clip_corner_radii`) by DPI factor.
    pub fn scaled(mut self, factor: f32) -> Self {
        if (factor - 1.0).abs() <= 1e-5 {
            return self;
        }
        for v in &mut self.rect {
            *v *= factor;
        }
        for r in &mut self.corner_radii {
            *r *= factor;
        }
        self.border_width *= factor;
        if let Some(ref mut cr) = self.clip_rect {
            for v in cr {
                *v *= factor;
            }
        }
        for r in &mut self.clip_corner_radii {
            *r *= factor;
        }
        self
    }

    pub fn to_gpu(&self) -> UiQuadInstanceGpu {
        // Convert CSS [tl, tr, br, bl] -> IQ SDF [tr, br, tl, bl]
        let css_to_iq = |c: [f32; 4]| [c[1], c[2], c[0], c[3]];
        let clip_rect = self.clip_rect.unwrap_or([0.0, 0.0, -1.0, -1.0]);
        let uv_or_grad = if self.sample_mode == UiSampleMode::LinearGradient {
            self.grad_color
        } else {
            self.uv_rect
        };
        UiQuadInstanceGpu {
            rect: self.rect,
            uv_rect: uv_or_grad,
            tint: self.tint,
            radii: css_to_iq(self.corner_radii),
            border_color: self.border_color,
            params: [
                self.sample_mode as u32 as f32,
                self.border_width,
                self.rotation_deg.to_radians(),
                self.gradient_angle_deg.to_radians(),
            ],
            clip_rect,
            clip_radii: css_to_iq(self.clip_corner_radii),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct UiQuadGlobals {
    viewport: [f32; 4],
}

/// GPU per-instance layout matching `UiQuadInstanceInput` in `ui_quad.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct UiQuadInstanceGpu {
    pub rect: [f32; 4],
    pub uv_rect: [f32; 4],
    pub tint: [f32; 4],
    pub radii: [f32; 4],
    pub border_color: [f32; 4],
    pub params: [f32; 4],
    pub clip_rect: [f32; 4],
    pub clip_radii: [f32; 4],
}

struct TextureSlot {
    _owned: Option<OwnedTexture>,
    bind_group: wgpu::BindGroup,
}

struct OwnedTexture {
    texture: wgpu::Texture,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
}

/// Pass 2 alpha-blended UI textured-quad, glyph-mask, SVG, and solid rounded-rect renderer.
pub struct UiQuadPass {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    globals_buf: wgpu::Buffer,
    instance_buf: wgpu::Buffer,
    instance_capacity: usize,
    textures: BTreeMap<u32, TextureSlot>,
    next_id: u32,
}

impl UiQuadPass {
    const INITIAL_CAPACITY: usize = 512;

    /// Creates the UI textured-quad pipeline and built-in `UiTextureId::WHITE` 1x1 texture.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ui-quad-wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/ui_quad.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui-quad-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui-quad-pl"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<UiQuadInstanceGpu>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 16,
                    shader_location: 1,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 32,
                    shader_location: 2,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 48,
                    shader_location: 3,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 64,
                    shader_location: 4,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 80,
                    shader_location: 5,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 96,
                    shader_location: 6,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 112,
                    shader_location: 7,
                },
            ],
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui-quad-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_ui_quad"),
                buffers: &[Some(instance_layout)],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_ui_quad"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ui-quad-linear"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let globals = UiQuadGlobals {
            viewport: [1280.0, 720.0, 0.0, 0.0],
        };
        let globals_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ui-quad-globals"),
            contents: bytemuck::bytes_of(&globals),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let instance_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui-quad-instances"),
            size: (Self::INITIAL_CAPACITY * std::mem::size_of::<UiQuadInstanceGpu>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut pass = Self {
            pipeline,
            bind_group_layout,
            sampler,
            globals_buf,
            instance_buf,
            instance_capacity: Self::INITIAL_CAPACITY,
            textures: BTreeMap::new(),
            next_id: UiTextureId::FIRST_DYNAMIC_ID,
        };

        pass.create_or_update_owned(
            device,
            queue,
            UiTextureId::WHITE,
            "ui-quad-white",
            1,
            1,
            wgpu::TextureFormat::Rgba8Unorm,
            &[255, 255, 255, 255],
        );

        pass
    }

    fn make_bind_group(&self, device: &wgpu::Device, view: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui-quad-bg"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.globals_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn create_or_update_owned(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        id: UiTextureId,
        label: &str,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        bytes: &[u8],
    ) {
        let w = width.max(1);
        let h = height.max(1);
        let bpp = match format {
            wgpu::TextureFormat::R8Unorm => 1,
            _ => 4,
        };
        let row_bytes = w * bpp;
        let expected = (row_bytes * h) as usize;
        if bytes.len() < expected {
            return;
        }

        let can_reuse = self
            .textures
            .get(&id.0)
            .and_then(|slot| slot._owned.as_ref())
            .is_some_and(|owned| owned.width == w && owned.height == h && owned.format == format);

        if can_reuse {
            if let Some(owned) = self.textures.get(&id.0).and_then(|s| s._owned.as_ref()) {
                queue.write_texture(
                    owned.texture.as_image_copy(),
                    &bytes[..expected],
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(row_bytes),
                        rows_per_image: Some(h),
                    },
                    wgpu::Extent3d {
                        width: w,
                        height: h,
                        depth_or_array_layers: 1,
                    },
                );
                return;
            }
        }

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            &bytes[..expected],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row_bytes),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.make_bind_group(device, &view);
        self.textures.insert(
            id.0,
            TextureSlot {
                _owned: Some(OwnedTexture {
                    texture,
                    width: w,
                    height: h,
                    format,
                }),
                bind_group,
            },
        );
    }

    fn alloc_id(&mut self) -> UiTextureId {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .saturating_add(1)
            .max(UiTextureId::FIRST_DYNAMIC_ID);
        UiTextureId(id)
    }

    /// Registers or replaces an external `TextureView` under `id` (e.g. accordion previews or `mpv` thumbnails).
    pub fn register_view(
        &mut self,
        device: &wgpu::Device,
        id: UiTextureId,
        view: &wgpu::TextureView,
    ) {
        let bind_group = self.make_bind_group(device, view);
        self.textures.insert(
            id.0,
            TextureSlot {
                _owned: None,
                bind_group,
            },
        );
    }

    /// Registers a new external `TextureView` and returns a freshly allocated [`UiTextureId`].
    pub fn register_external_view(
        &mut self,
        device: &wgpu::Device,
        view: &wgpu::TextureView,
    ) -> UiTextureId {
        let id = self.alloc_id();
        self.register_view(device, id, view);
        id
    }

    /// Allocates and uploads a new `Rgba8Unorm` texture (for `resvg` SVGs, thumbnails, or images).
    pub fn upload_rgba8(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        label: &str,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> UiTextureId {
        let id = self.alloc_id();
        self.create_or_update_owned(
            device,
            queue,
            id,
            label,
            width,
            height,
            wgpu::TextureFormat::Rgba8Unorm,
            rgba,
        );
        id
    }

    /// Updates an existing owned `Rgba8Unorm` texture in-place when dimensions match, or reallocates.
    pub fn update_rgba8(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        id: UiTextureId,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) {
        self.create_or_update_owned(
            device,
            queue,
            id,
            "ui-quad-rgba8",
            width,
            height,
            wgpu::TextureFormat::Rgba8Unorm,
            rgba,
        );
    }

    /// Allocates and uploads a new `R8Unorm` glyph coverage atlas texture.
    pub fn upload_r8(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        label: &str,
        width: u32,
        height: u32,
        mask: &[u8],
    ) -> UiTextureId {
        let id = self.alloc_id();
        self.create_or_update_owned(
            device,
            queue,
            id,
            label,
            width,
            height,
            wgpu::TextureFormat::R8Unorm,
            mask,
        );
        id
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
        self.create_or_update_owned(
            device,
            queue,
            id,
            "ui-quad-r8",
            width,
            height,
            wgpu::TextureFormat::R8Unorm,
            mask,
        );
    }

    /// Releases a previously registered or uploaded texture slot (never removes `UiTextureId::WHITE`).
    pub fn remove_texture(&mut self, id: UiTextureId) -> bool {
        if id == UiTextureId::WHITE {
            return false;
        }
        self.textures.remove(&id.0).is_some()
    }

    pub(crate) fn pipeline(&self) -> &wgpu::RenderPipeline {
        &self.pipeline
    }

    pub(crate) fn instance_buffer(&self) -> &wgpu::Buffer {
        &self.instance_buf
    }

    pub(crate) fn bind_group_for(&self, id: UiTextureId) -> &wgpu::BindGroup {
        self.textures
            .get(&id.0)
            .or_else(|| self.textures.get(&UiTextureId::WHITE.0))
            .map(|s| &s.bind_group)
            .expect("UiTextureId::WHITE bind group must always exist")
    }

    pub(crate) fn upload_instances(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        viewport_w: u32,
        viewport_h: u32,
        instances: &[UiQuadInstanceGpu],
    ) {
        let globals = UiQuadGlobals {
            viewport: [viewport_w.max(1) as f32, viewport_h.max(1) as f32, 0.0, 0.0],
        };
        queue.write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&globals));

        if instances.is_empty() {
            return;
        }

        if instances.len() > self.instance_capacity {
            self.instance_capacity = instances.len().next_power_of_two();
            self.instance_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ui-quad-instances"),
                size: (self.instance_capacity * std::mem::size_of::<UiQuadInstanceGpu>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }

        queue.write_buffer(&self.instance_buf, 0, bytemuck::cast_slice(instances));
    }

    /// Encodes `quads` sorted stably by `z_index` onto `target_view` (`LoadOp::Load`), batching by texture and scissor rect.
    #[allow(clippy::too_many_arguments)]
    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target_view: &wgpu::TextureView,
        viewport_w: u32,
        viewport_h: u32,
        quads: &[UiTexturedQuad],
    ) {
        if quads.is_empty() {
            return;
        }

        let mut order: Vec<usize> = (0..quads.len()).collect();
        order.sort_by_key(|&i| quads[i].z_index);

        let mut instances = Vec::with_capacity(order.len());
        let mut keys: Vec<(UiTextureId, [u32; 4])> = Vec::with_capacity(order.len());
        for idx in order {
            let q = &quads[idx];
            if let Some(scissor) = compute_scissor_rect(q.clip_rect, viewport_w, viewport_h) {
                instances.push(q.to_gpu());
                keys.push((q.texture, scissor));
            }
        }

        if instances.is_empty() {
            return;
        }

        self.upload_instances(device, queue, viewport_w, viewport_h, &instances);

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ui-quad-pass"),
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

        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, self.instance_buf.slice(..));

        let mut start = 0usize;
        while start < keys.len() {
            let key = keys[start];
            let mut end = start + 1;
            while end < keys.len() && keys[end] == key {
                end += 1;
            }
            let [sx, sy, sw, sh] = key.1;
            pass.set_scissor_rect(sx, sy, sw, sh);
            pass.set_bind_group(0, self.bind_group_for(key.0), &[]);
            pass.draw(0..6, start as u32..end as u32);
            start = end;
        }
    }
}

/// Computes a clamped hardware scissor rectangle `[x, y, w, h]` within `(viewport_w, viewport_h)`.
/// Returns `None` if the clipped rectangle is empty or invalid.
pub(crate) fn compute_scissor_rect(
    clip: Option<[f32; 4]>,
    viewport_w: u32,
    viewport_h: u32,
) -> Option<[u32; 4]> {
    let vp_w = viewport_w.max(1);
    let vp_h = viewport_h.max(1);
    let Some([x, y, w, h]) = clip else {
        return Some([0, 0, vp_w, vp_h]);
    };
    if !x.is_finite() || !y.is_finite() || !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0
    {
        return None;
    }
    let x0 = x.floor().clamp(0.0, vp_w as f32) as u32;
    let y0 = y.floor().clamp(0.0, vp_h as f32) as u32;
    let x1 = (x + w).ceil().clamp(0.0, vp_w as f32) as u32;
    let y1 = (y + h).ceil().clamp(0.0, vp_h as f32) as u32;
    if x1 <= x0 || y1 <= y0 {
        None
    } else {
        Some([x0, y0, x1 - x0, y1 - y0])
    }
}
