use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::error::ShaderError;
use crate::types::{ClearContext, VideoFrameTransform};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SceneBlitUniforms {
    uv_rect: [f32; 4],
    params: [f32; 4],
}

/// Pass 0 (`SceneTex`): offscreen scene target holding aspect-fitted video or active Welcome shader.
pub struct ScenePass {
    format: wgpu::TextureFormat,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
    blit_pipeline: wgpu::RenderPipeline,
    copy_pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform_buf: wgpu::Buffer,
    _fallback_tex: wgpu::Texture,
    fallback_view: wgpu::TextureView,
    blit_bind_group: wgpu::BindGroup,
    copy_bind_group: wgpu::BindGroup,
}

impl ScenePass {
    /// Creates Pass 0 (`SceneTex`) and the aspect-fit video blit + present copy pipelines.
    pub fn new(
        device: &wgpu::Device,
        scene_format: wgpu::TextureFormat,
        present_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Result<Self, ShaderError> {
        if width == 0 || height == 0 {
            return Err(ShaderError::InvalidDimensions { width, height });
        }

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fullscreen-scene-wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/fullscreen.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene-blit-bgl"),
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
            label: Some("scene-blit-pl"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("scene-video-blit"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_fullscreen"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_scene_blit"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: scene_format,
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

        let copy_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("scene-copy-present"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_fullscreen"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_copy_present"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: present_format,
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
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("scene-linear-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let uniform = SceneBlitUniforms {
            uv_rect: [1.0, 1.0, 0.0, 0.0],
            params: [1.0, 0.0, 0.0, 0.0],
        };
        let uniform_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("scene-blit-uniform"),
            contents: bytemuck::bytes_of(&uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let fallback_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("scene-fallback-1x1"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: scene_format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let fallback_view = fallback_tex.create_view(&wgpu::TextureViewDescriptor::default());

        let (texture, view) = Self::create_scene_target(device, scene_format, width, height);

        let blit_bind_group = Self::make_bind_group(
            device,
            &bind_group_layout,
            &uniform_buf,
            &fallback_view,
            &sampler,
        );
        let copy_bind_group =
            Self::make_bind_group(device, &bind_group_layout, &uniform_buf, &view, &sampler);

        Ok(Self {
            format: scene_format,
            texture,
            view,
            width,
            height,
            blit_pipeline,
            copy_pipeline,
            bind_group_layout,
            sampler,
            uniform_buf,
            _fallback_tex: fallback_tex,
            fallback_view,
            blit_bind_group,
            copy_bind_group,
        })
    }

    fn create_scene_target(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("SceneTex"),
            size: wgpu::Extent3d {
                width,
                height,
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
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    fn make_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        uniform_buf: &wgpu::Buffer,
        tex_view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene-blit-bg"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(tex_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        })
    }

    /// Resizes `SceneTex` if needed.
    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> Result<bool, ShaderError> {
        if width == 0 || height == 0 {
            return Err(ShaderError::InvalidDimensions { width, height });
        }
        if self.width == width && self.height == height {
            return Ok(false);
        }
        let (texture, view) = Self::create_scene_target(device, self.format, width, height);
        self.texture = texture;
        self.view = view;
        self.width = width;
        self.height = height;
        self.copy_bind_group = Self::make_bind_group(
            device,
            &self.bind_group_layout,
            &self.uniform_buf,
            &self.view,
            &self.sampler,
        );
        Ok(true)
    }

    /// Clears `SceneTex` to `#0c0d12` or `#000000` depending on `ClearContext`.
    pub fn encode_clear(&self, encoder: &mut wgpu::CommandEncoder, clear: ClearContext) {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scene-clear-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear.clear_color()),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }

    /// Clears `SceneTex` and blits an aspect-fitted video texture into it.
    pub fn encode_video(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        clear: ClearContext,
        video_view: &wgpu::TextureView,
        transform: VideoFrameTransform,
    ) {
        let uniform = SceneBlitUniforms {
            uv_rect: transform.uv_rect(self.width, self.height),
            params: [transform.opacity.clamp(0.0, 1.0), 1.0, 0.0, 0.0],
        };
        queue.write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&uniform));
        self.blit_bind_group = Self::make_bind_group(
            device,
            &self.bind_group_layout,
            &self.uniform_buf,
            video_view,
            &self.sampler,
        );

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scene-video-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear.clear_color()),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.blit_pipeline);
        pass.set_bind_group(0, &self.blit_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Copies `SceneTex` to the final target view before Pass 2 SDF glass overlays.
    pub fn encode_copy_to(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target_view: &wgpu::TextureView,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scene-copy-to-present"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target_view,
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
        pass.set_pipeline(&self.copy_pipeline);
        pass.set_bind_group(0, &self.copy_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn fallback_view(&self) -> &wgpu::TextureView {
        &self.fallback_view
    }
}
