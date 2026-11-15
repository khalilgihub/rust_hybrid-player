use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::error::ShaderError;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct KawaseUniforms {
    texel_offset: [f32; 4],
}

struct PyramidLevel {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
}

struct KawasePassSlot {
    _uniform_buf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

/// Pass 1: Dual-Kawase downsample/upsample pyramid generating `Blur4`, `Blur8`, `Blur16`, and `Blur32`.
pub struct DualKawaseBlur {
    format: wgpu::TextureFormat,
    downsample_pipeline: wgpu::RenderPipeline,
    upsample_pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    // down[0]=1/2, down[1]=1/4, down[2]=1/8, down[3]=1/16 (Blur32)
    down: Vec<PyramidLevel>,
    // up[0]=1/2 (Blur4), up[1]=1/4 (Blur8), up[2]=1/8 (Blur16)
    up: Vec<PyramidLevel>,
    down_passes: Vec<KawasePassSlot>,
    up_passes: Vec<KawasePassSlot>,
    scene_size: (u32, u32),
    offset_scale: f32,
}

impl DualKawaseBlur {
    /// Creates the Dual-Kawase pipelines and initial pyramid for `scene_view` (`width x height`).
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        scene_view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) -> Result<Self, ShaderError> {
        if width == 0 || height == 0 {
            return Err(ShaderError::InvalidDimensions { width, height });
        }

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("dual-kawase-wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/dual_kawase.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("kawase-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
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
            label: Some("kawase-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let make_pipeline = |entry: &str, label: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_kawase"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };

        let downsample_pipeline = make_pipeline("fs_downsample", "kawase-downsample");
        let upsample_pipeline = make_pipeline("fs_upsample", "kawase-upsample");

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("kawase-linear-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let mut blur = Self {
            format,
            downsample_pipeline,
            upsample_pipeline,
            bind_group_layout,
            sampler,
            down: Vec::new(),
            up: Vec::new(),
            down_passes: Vec::new(),
            up_passes: Vec::new(),
            scene_size: (0, 0),
            offset_scale: 1.75,
        };
        blur.rebuild(device, scene_view, width, height)?;
        Ok(blur)
    }

    /// Rebuilds pyramid textures and bind groups if `scene_view` dimensions changed.
    pub fn rebuild(
        &mut self,
        device: &wgpu::Device,
        scene_view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) -> Result<(), ShaderError> {
        if width == 0 || height == 0 {
            return Err(ShaderError::InvalidDimensions { width, height });
        }
        self.scene_size = (width, height);
        self.down.clear();
        self.up.clear();
        self.down_passes.clear();
        self.up_passes.clear();

        let make_level = |w: u32, h: u32, label: &str| {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
            PyramidLevel {
                _texture: tex,
                view,
                width: w,
                height: h,
            }
        };

        for idx in 0..4u32 {
            let div = 1u32 << (idx + 1);
            let lw = (width / div).max(1);
            let lh = (height / div).max(1);
            self.down
                .push(make_level(lw, lh, &format!("kawase-down-{idx}")));
        }
        for idx in 0..3u32 {
            let div = 1u32 << (idx + 1);
            let lw = (width / div).max(1);
            let lh = (height / div).max(1);
            self.up
                .push(make_level(lw, lh, &format!("kawase-up-{idx}")));
        }

        let make_slot = |src_view: &wgpu::TextureView, src_w: u32, src_h: u32, label: &str| {
            let uniform = KawaseUniforms {
                texel_offset: [
                    1.0 / (src_w.max(1) as f32),
                    1.0 / (src_h.max(1) as f32),
                    self.offset_scale,
                    0.0,
                ],
            };
            let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::bytes_of(&uniform),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &self.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(src_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            KawasePassSlot {
                _uniform_buf: buf,
                bind_group: bg,
            }
        };

        // Downsample chain: scene -> down[0] -> down[1] -> down[2] -> down[3]
        self.down_passes
            .push(make_slot(scene_view, width, height, "kawase-down-slot-0"));
        for idx in 1..4 {
            let src = &self.down[idx - 1];
            self.down_passes.push(make_slot(
                &src.view,
                src.width,
                src.height,
                &format!("kawase-down-slot-{idx}"),
            ));
        }

        // Upsample chain: down[3] -> up[2] -> up[1] -> up[0]
        self.up_passes.push(make_slot(
            &self.down[3].view,
            self.down[3].width,
            self.down[3].height,
            "kawase-up-slot-2",
        ));
        self.up_passes.push(make_slot(
            &self.up[2].view,
            self.up[2].width,
            self.up[2].height,
            "kawase-up-slot-1",
        ));
        self.up_passes.push(make_slot(
            &self.up[1].view,
            self.up[1].width,
            self.up[1].height,
            "kawase-up-slot-0",
        ));

        Ok(())
    }

    /// Encodes all Dual-Kawase downsample and upsample passes into `encoder`.
    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder) {
        for (idx, level) in self.down.iter().enumerate() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("kawase-downsample-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &level.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.downsample_pipeline);
            pass.set_bind_group(0, &self.down_passes[idx].bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        // Upsample targets order: up[2], up[1], up[0]
        let up_targets = [&self.up[2], &self.up[1], &self.up[0]];
        for (slot_idx, target) in up_targets.iter().enumerate() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("kawase-upsample-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.upsample_pipeline);
            pass.set_bind_group(0, &self.up_passes[slot_idx].bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    /// Texture view covering `blur(4px)` (upsampled 1/2 resolution).
    pub fn blur4_view(&self) -> &wgpu::TextureView {
        &self.up[0].view
    }

    /// Texture view covering `blur(8px)` (upsampled 1/4 resolution).
    pub fn blur8_view(&self) -> &wgpu::TextureView {
        &self.up[1].view
    }

    /// Texture view covering `blur(16px)` (upsampled 1/8 resolution).
    pub fn blur16_view(&self) -> &wgpu::TextureView {
        &self.up[2].view
    }

    /// Texture view covering `blur(32px)` (downsampled 1/16 resolution).
    pub fn blur32_view(&self) -> &wgpu::TextureView {
        &self.down[3].view
    }

    /// Underlying `Blur4` texture (useful for GPU readback verification).
    pub fn blur4_texture(&self) -> &wgpu::Texture {
        &self.up[0]._texture
    }

    /// Underlying `Blur32` texture (useful for GPU readback verification).
    pub fn blur32_texture(&self) -> &wgpu::Texture {
        &self.down[3]._texture
    }

    /// Dimensions of the `Blur4` and `Blur32` pyramid targets.
    pub fn level_dimensions(&self) -> [(u32, u32); 4] {
        [
            (self.up[0].width, self.up[0].height),
            (self.up[1].width, self.up[1].height),
            (self.up[2].width, self.up[2].height),
            (self.down[3].width, self.down[3].height),
        ]
    }
}
