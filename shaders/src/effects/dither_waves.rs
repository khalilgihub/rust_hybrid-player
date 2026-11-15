use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::types::{DitherWavesConfig, EffectInstanceConfig, EffectLifecycle};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DitherWavesUniforms {
    res_aspect_time: [f32; 4],
    wave_params: [f32; 4],
    wave_color_num: [f32; 4],
    bg_color_mouse: [f32; 4],
    mouse_params: [f32; 4],
    raster_params: [f32; 4],
}

/// GPU renderer for `dither_waves.wgsl` (supports full-bleed & `400x240 @ 30fps` preview).
pub struct DitherWavesEffect {
    pipeline: wgpu::RenderPipeline,
    uniform_buf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    pub config: DitherWavesConfig,
    pub instance: EffectInstanceConfig,
    pub lifecycle: EffectLifecycle,
    pub effective_dpr: Option<f32>,
    elapsed_secs: f32,
}

impl DitherWavesEffect {
    /// Creates the `dither_waves` pipeline targeting `format`.
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        config: DitherWavesConfig,
        instance: EffectInstanceConfig,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("dither-waves-wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/dither_waves.wgsl").into()),
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("dither-waves-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("dither-waves-pl"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("dither-waves-pipeline"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_dither_waves"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_dither_waves"),
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
        });

        let initial = DitherWavesUniforms {
            res_aspect_time: [1.0, 1.0, 1.0, 0.0],
            wave_params: [
                config.wave_speed,
                config.wave_frequency,
                config.wave_amplitude,
                config.pixel_size,
            ],
            wave_color_num: [
                config.wave_color[0],
                config.wave_color[1],
                config.wave_color[2],
                config.color_num,
            ],
            bg_color_mouse: [
                config.bg_color[0],
                config.bg_color[1],
                config.bg_color[2],
                if config.enable_mouse_interaction {
                    1.0
                } else {
                    0.0
                },
            ],
            mouse_params: [
                config.mouse_pos_px[0],
                config.mouse_pos_px[1],
                config.mouse_radius,
                config.mouse_intensity,
            ],
            raster_params: [1.0, 1.0, 1.0, 1.0],
        };

        let uniform_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("dither-waves-uniforms"),
            contents: bytemuck::bytes_of(&initial),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("dither-waves-bg"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });

        let lifecycle = EffectLifecycle::from_instance(&instance);
        Self {
            pipeline,
            uniform_buf,
            bind_group,
            config,
            instance,
            lifecycle,
            effective_dpr: None,
            elapsed_secs: 0.0,
        }
    }

    /// Advances simulation time by `dt_secs` when not paused.
    pub fn update(&mut self, dt_secs: f32) {
        if !self.lifecycle.is_paused() {
            self.elapsed_secs += dt_secs.max(0.0);
        }
    }

    /// Sets explicit deterministic elapsed time (useful for tests).
    pub fn set_time(&mut self, time_secs: f32) {
        self.elapsed_secs = time_secs;
    }

    /// Writes uniforms and encodes a render pass onto `target_view`.
    pub fn encode(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target_view: &wgpu::TextureView,
        target_w: u32,
        target_h: u32,
    ) {
        let w = target_w.max(1) as f32;
        let h = target_h.max(1) as f32;
        let inst_dpr = if self.instance.dpr.is_finite() && self.instance.dpr > 0.0 {
            self.instance.dpr
        } else {
            1.0
        };
        let eff_dpr = match self.effective_dpr {
            Some(d) if d.is_finite() && d > 0.0 => d,
            _ => inst_dpr,
        };
        let eff_w = if (eff_dpr - inst_dpr).abs() < 1e-4 {
            w
        } else {
            (w * (eff_dpr / inst_dpr)).floor().max(1.0)
        };
        let eff_h = if (eff_dpr - inst_dpr).abs() < 1e-4 {
            h
        } else {
            (h * (eff_dpr / inst_dpr)).floor().max(1.0)
        };
        let aspect = eff_w / eff_h;
        let uniforms = DitherWavesUniforms {
            res_aspect_time: [eff_w, eff_h, aspect, self.elapsed_secs],
            wave_params: [
                self.config.wave_speed,
                self.config.wave_frequency,
                self.config.wave_amplitude,
                self.config.pixel_size.max(1.0),
            ],
            wave_color_num: [
                self.config.wave_color[0],
                self.config.wave_color[1],
                self.config.wave_color[2],
                self.config.color_num.max(2.0),
            ],
            bg_color_mouse: [
                self.config.bg_color[0],
                self.config.bg_color[1],
                self.config.bg_color[2],
                if self.config.enable_mouse_interaction {
                    1.0
                } else {
                    0.0
                },
            ],
            mouse_params: [
                self.config.mouse_pos_px[0] * eff_dpr,
                self.config.mouse_pos_px[1] * eff_dpr,
                self.config.mouse_radius,
                self.config.mouse_intensity,
            ],
            raster_params: [w, h, eff_dpr, inst_dpr],
        };
        queue.write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&uniforms));

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("dither-waves-pass"),
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
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}
