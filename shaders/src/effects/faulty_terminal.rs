use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::types::{EffectInstanceConfig, EffectLifecycle, FaultyTerminalConfig};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FaultyTerminalUniforms {
    res_time: [f32; 4],
    scale_grid_digit: [f32; 4],
    glitch_params: [f32; 4],
    post_params: [f32; 4],
    tint_use_mouse: [f32; 4],
    mouse_load: [f32; 4],
    load_progress: [f32; 4],
}

const FAULTY_BLIT_WGSL: &str = r#"
struct BlitVertexOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_faulty_blit(@builtin(vertex_index) vid: u32) -> BlitVertexOut {
    let x = f32((vid << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(vid & 2u) * 2.0 - 1.0;
    var out: BlitVertexOut;
    out.pos = vec4<f32>(x, -y, 0.0, 1.0);
    out.uv = vec2<f32>(x * 0.5 + 0.5, y * 0.5 + 0.5);
    return out;
}

@group(0) @binding(0)
var t_src: texture_2d<f32>;
@group(0) @binding(1)
var s_linear: sampler;

@fragment
fn fs_faulty_blit(in: BlitVertexOut) -> @location(0) vec4<f32> {
    return textureSampleLevel(t_src, s_linear, in.uv, 0.0);
}
"#;

struct FaultyOffscreenTarget {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    blit_bind_group: wgpu::BindGroup,
    width: u32,
    height: u32,
}

/// GPU renderer for `faulty_terminal.wgsl` (full-bleed & `400x240 @ 30fps` preview).
pub struct FaultyTerminalEffect {
    pipeline: wgpu::RenderPipeline,
    format: wgpu::TextureFormat,
    blit_pipeline: wgpu::RenderPipeline,
    blit_bgl: wgpu::BindGroupLayout,
    blit_sampler: wgpu::Sampler,
    offscreen: Option<FaultyOffscreenTarget>,
    uniform_buf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    pub config: FaultyTerminalConfig,
    pub instance: EffectInstanceConfig,
    pub lifecycle: EffectLifecycle,
    /// Optional quality-capped render DPR (`faultyTerminal.js:368-373`); when lower than `instance.dpr`,
    /// renders to a smaller canvas texture and bilinearly upsamples to the full surface target.
    pub effective_dpr: Option<f32>,
    elapsed_secs: f32,
    load_elapsed_secs: f32,
    smooth_mouse: [f32; 2],
}

impl FaultyTerminalEffect {
    /// Creates the `faulty_terminal` pipeline targeting `format`.
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        config: FaultyTerminalConfig,
        instance: EffectInstanceConfig,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("faulty-terminal-wgsl"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shaders/faulty_terminal.wgsl").into(),
            ),
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("faulty-terminal-bgl"),
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
            label: Some("faulty-terminal-pl"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("faulty-terminal-pipeline"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_faulty_terminal"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_faulty_terminal"),
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

        let initial = FaultyTerminalUniforms {
            res_time: [1280.0, 720.0, 1280.0 / 720.0, 0.0],
            scale_grid_digit: [
                config.grid_mul[0],
                config.grid_mul[1],
                config.scale,
                config.digit_size,
            ],
            glitch_params: [
                config.scanline_intensity,
                config.glitch_amount,
                config.flicker_amount,
                config.noise_amp,
            ],
            post_params: [
                config.chromatic_aberration,
                config.dither,
                config.curvature,
                config.brightness,
            ],
            tint_use_mouse: [
                config.tint[0],
                config.tint[1],
                config.tint[2],
                if config.mouse_react { 1.0 } else { 0.0 },
            ],
            mouse_load: [
                config.mouse_uv[0],
                config.mouse_uv[1],
                config.mouse_strength,
                if config.page_load_animation { 1.0 } else { 0.0 },
            ],
            load_progress: [config.page_load_progress, 0.0, 0.0, 0.0],
        };

        let uniform_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("faulty-terminal-uniforms"),
            contents: bytemuck::bytes_of(&initial),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("faulty-terminal-bg"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });

        let smooth_mouse = config.mouse_uv;
        let load_elapsed_secs = if config.page_load_animation {
            config.page_load_progress * 2.0
        } else {
            2.0
        };

        let blit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("faulty-terminal-blit-wgsl"),
            source: wgpu::ShaderSource::Wgsl(FAULTY_BLIT_WGSL.into()),
        });
        let blit_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("faulty-terminal-blit-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let blit_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("faulty-terminal-blit-pl"),
            bind_group_layouts: &[Some(&blit_bgl)],
            immediate_size: 0,
        });
        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("faulty-terminal-blit-pipeline"),
            layout: Some(&blit_pl),
            vertex: wgpu::VertexState {
                module: &blit_shader,
                entry_point: Some("vs_faulty_blit"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &blit_shader,
                entry_point: Some("fs_faulty_blit"),
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
        let blit_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("faulty-terminal-blit-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let lifecycle = EffectLifecycle::from_instance(&instance);
        Self {
            pipeline,
            format,
            blit_pipeline,
            blit_bgl,
            blit_sampler,
            offscreen: None,
            uniform_buf,
            bind_group,
            config,
            instance,
            lifecycle,
            effective_dpr: None,
            elapsed_secs: 0.0,
            load_elapsed_secs,
            smooth_mouse,
        }
    }

    /// Advances shader time, 2000ms page-load animation, and damped mouse interpolation.
    pub fn update(&mut self, dt_secs: f32) {
        if self.lifecycle.is_paused() {
            return;
        }
        let dt = dt_secs.max(0.0);
        if !self.config.pause {
            self.elapsed_secs += dt;
        }
        if self.config.page_load_animation {
            self.load_elapsed_secs = (self.load_elapsed_secs + dt).min(2.0);
            self.config.page_load_progress = (self.load_elapsed_secs / 2.0).clamp(0.0, 1.0);
        }
        if self.config.mouse_react {
            let alpha = (dt * 60.0 * 0.08).clamp(0.0, 1.0);
            self.smooth_mouse[0] += (self.config.mouse_uv[0] - self.smooth_mouse[0]) * alpha;
            self.smooth_mouse[1] += (self.config.mouse_uv[1] - self.smooth_mouse[1]) * alpha;
        }
    }

    /// Sets explicit deterministic time & mouse state (for headless tests).
    pub fn set_time_and_mouse(&mut self, time_secs: f32, mouse_uv: [f32; 2], load_progress: f32) {
        self.elapsed_secs = time_secs;
        self.smooth_mouse = mouse_uv;
        self.config.mouse_uv = mouse_uv;
        self.config.page_load_progress = load_progress.clamp(0.0, 1.0);
        self.load_elapsed_secs = self.config.page_load_progress * 2.0;
    }

    /// Randomizes initial `time_offset` in `[0, 100)` (`faultyTerminal.js:409`).
    pub fn randomize_time_offset(&mut self) {
        self.config.time_offset = rand::random::<f32>() * 100.0;
    }

    /// Restarts the 2-second CRT boot-up `pageLoadAnimation` (`faultyTerminal.js:435`).
    pub fn restart_load_animation(&mut self) {
        self.load_elapsed_secs = 0.0;
        self.config.page_load_progress = if self.config.page_load_animation {
            0.0
        } else {
            1.0
        };
    }

    /// Writes uniforms and encodes the render pass onto `target_view` (with optional DPR downscale + linear upsample).
    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target_view: &wgpu::TextureView,
        target_w: u32,
        target_h: u32,
    ) {
        let tw = target_w.max(1);
        let th = target_h.max(1);
        let inst_dpr = if self.instance.dpr.is_finite() && self.instance.dpr > 0.0 {
            self.instance.dpr
        } else {
            1.0
        };
        let eff_dpr = match self.effective_dpr {
            Some(d) if d.is_finite() && d > 0.0 => d,
            _ => inst_dpr,
        };
        let (eff_w, eff_h) = if (eff_dpr - inst_dpr).abs() < 1e-4 {
            (tw, th)
        } else {
            let ratio = eff_dpr / inst_dpr;
            (
                ((tw as f32 * ratio).floor() as u32).max(1),
                ((th as f32 * ratio).floor() as u32).max(1),
            )
        };
        let w = eff_w as f32;
        let h = eff_h as f32;
        let aspect = w / h;
        let i_time = (self.elapsed_secs + self.config.time_offset) * self.config.time_scale;
        let grid_y = self.config.grid_mul[1];
        let grid_x = grid_y * aspect;

        let uniforms = FaultyTerminalUniforms {
            res_time: [w, h, aspect, i_time],
            scale_grid_digit: [grid_x, grid_y, self.config.scale, self.config.digit_size],
            glitch_params: [
                self.config.scanline_intensity,
                self.config.glitch_amount,
                self.config.flicker_amount,
                self.config.noise_amp,
            ],
            post_params: [
                self.config.chromatic_aberration,
                self.config.dither,
                self.config.curvature,
                self.config.brightness,
            ],
            tint_use_mouse: [
                self.config.tint[0],
                self.config.tint[1],
                self.config.tint[2],
                if self.config.mouse_react { 1.0 } else { 0.0 },
            ],
            mouse_load: [
                self.smooth_mouse[0],
                self.smooth_mouse[1],
                self.config.mouse_strength,
                if self.config.page_load_animation {
                    1.0
                } else {
                    0.0
                },
            ],
            load_progress: [self.config.page_load_progress, 0.0, 0.0, 0.0],
        };
        queue.write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&uniforms));

        if eff_w == tw && eff_h == th {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("faulty-terminal-pass"),
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
            return;
        }

        let needs_rebuild = match &self.offscreen {
            Some(o) => o.width != eff_w || o.height != eff_h,
            None => true,
        };
        if needs_rebuild {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("faulty-terminal-offscreen"),
                size: wgpu::Extent3d {
                    width: eff_w,
                    height: eff_h,
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
            let blit_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("faulty-terminal-blit-bg"),
                layout: &self.blit_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.blit_sampler),
                    },
                ],
            });
            self.offscreen = Some(FaultyOffscreenTarget {
                _texture: tex,
                view,
                blit_bind_group,
                width: eff_w,
                height: eff_h,
            });
        }
        let off = self.offscreen.as_ref().expect("offscreen target created");
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("faulty-terminal-eff-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &off.view,
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
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("faulty-terminal-blit-pass"),
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
            pass.set_pipeline(&self.blit_pipeline);
            pass.set_bind_group(0, &off.blit_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}
