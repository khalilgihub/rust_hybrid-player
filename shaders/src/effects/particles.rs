use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::types::{EffectInstanceConfig, EffectLifecycle, ParticlesConfig};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ParticlesUniforms {
    model_matrix: [[f32; 4]; 4],
    view_matrix: [[f32; 4]; 4],
    projection_matrix: [[f32; 4]; 4],
    // x = time, y = spread, z = base_size * pixel_ratio, w = size_randomness
    params: [f32; 4],
    // xy = viewport_size, z = alpha_particles, w = unused
    viewport_alpha: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ParticleInstanceGpu {
    position: [f32; 3],
    _pad0: f32,
    random: [f32; 4],
    color: [f32; 3],
    _pad1: f32,
}

struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn next_f32(&mut self) -> f32 {
        // Uniform in [0, 1)
        ((self.next_u64() >> 40) as f32) / ((1u64 << 24) as f32)
    }
}

const PARTICLES_BLIT_WGSL: &str = r#"
struct BlitVertexOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_particles_blit(@builtin(vertex_index) vid: u32) -> BlitVertexOut {
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
fn fs_particles_blit(in: BlitVertexOut) -> @location(0) vec4<f32> {
    return textureSampleLevel(t_src, s_linear, in.uv, 0.0);
}
"#;

struct ParticlesOffscreenTarget {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    blit_bind_group: wgpu::BindGroup,
    width: u32,
    height: u32,
}

/// GPU renderer for `particles.wgsl` using instanced camera-facing billboard quads (`6 x particleCount`).
pub struct ParticlesEffect {
    opaque_pipeline: wgpu::RenderPipeline,
    alpha_pipeline: wgpu::RenderPipeline,
    format: wgpu::TextureFormat,
    blit_pipeline: wgpu::RenderPipeline,
    blit_bgl: wgpu::BindGroupLayout,
    blit_sampler: wgpu::Sampler,
    offscreen: Option<ParticlesOffscreenTarget>,
    uniform_buf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    instance_buf: wgpu::Buffer,
    instance_count: u32,
    pub config: ParticlesConfig,
    pub instance: EffectInstanceConfig,
    pub lifecycle: EffectLifecycle,
    /// Optional quality-capped render DPR (`particles.js:125-130, 159`).
    pub effective_dpr: Option<f32>,
    elapsed_ms: f32,
    rot_z: f32,
}

impl ParticlesEffect {
    /// Creates the `particles` billboard pipelines and initial instance buffer.
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        config: ParticlesConfig,
        instance: EffectInstanceConfig,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("particles-wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/particles.wgsl").into()),
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particles-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("particles-pl"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });

        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<ParticleInstanceGpu>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 16,
                    shader_location: 1,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 32,
                    shader_location: 2,
                },
            ],
        };

        let make_pipeline = |blend: Option<wgpu::BlendState>, label: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_particles"),
                    buffers: &[Some(instance_layout.clone())],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_particles"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
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

        let opaque_pipeline = make_pipeline(None, "particles-opaque");
        let alpha_pipeline =
            make_pipeline(Some(wgpu::BlendState::ALPHA_BLENDING), "particles-alpha");

        let uniforms = ParticlesUniforms {
            model_matrix: mat4_identity(),
            view_matrix: mat4_identity(),
            projection_matrix: mat4_identity(),
            params: [
                0.0,
                config.particle_spread,
                config.particle_base_size,
                config.size_randomness,
            ],
            viewport_alpha: [
                1280.0,
                720.0,
                if config.alpha_particles { 1.0 } else { 0.0 },
                0.0,
            ],
        };
        let uniform_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("particles-uniforms"),
            contents: bytemuck::bytes_of(&uniforms),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("particles-bg"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });

        let instances = Self::build_instances(&config);
        let instance_count = instances.len() as u32;
        let instance_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("particles-instances"),
            contents: bytemuck::cast_slice(&instances),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });

        let blit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("particles-blit-wgsl"),
            source: wgpu::ShaderSource::Wgsl(PARTICLES_BLIT_WGSL.into()),
        });
        let blit_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particles-blit-bgl"),
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
            label: Some("particles-blit-pl"),
            bind_group_layouts: &[Some(&blit_bgl)],
            immediate_size: 0,
        });
        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("particles-blit-pipeline"),
            layout: Some(&blit_pl),
            vertex: wgpu::VertexState {
                module: &blit_shader,
                entry_point: Some("vs_particles_blit"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &blit_shader,
                entry_point: Some("fs_particles_blit"),
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
            label: Some("particles-blit-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let lifecycle = EffectLifecycle::from_instance(&instance);
        Self {
            opaque_pipeline,
            alpha_pipeline,
            format,
            blit_pipeline,
            blit_bgl,
            blit_sampler,
            offscreen: None,
            uniform_buf,
            bind_group,
            instance_buf,
            instance_count,
            config,
            instance,
            lifecycle,
            effective_dpr: None,
            elapsed_ms: 0.0,
            rot_z: 0.0,
        }
    }

    fn build_instances(config: &ParticlesConfig) -> Vec<ParticleInstanceGpu> {
        let count = config.particle_count.max(1) as usize;
        let palette: &[[f32; 3]] = if config.particle_colors.is_empty() {
            &[[1.0, 1.0, 1.0]]
        } else {
            &config.particle_colors
        };

        let mut rng = SplitMix64(config.seed);
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            let (mut x, mut y, mut z, mut len);
            loop {
                x = rng.next_f32() * 2.0 - 1.0;
                y = rng.next_f32() * 2.0 - 1.0;
                z = rng.next_f32() * 2.0 - 1.0;
                len = x * x + y * y + z * z;
                if len <= 1.0 && len > 0.0 {
                    break;
                }
            }
            let r = rng.next_f32().cbrt();
            let random = [
                rng.next_f32(),
                rng.next_f32(),
                rng.next_f32(),
                rng.next_f32(),
            ];
            let idx = ((rng.next_u64() as usize) % palette.len()).min(palette.len() - 1);
            let color = palette[idx];
            out.push(ParticleInstanceGpu {
                position: [x * r, y * r, z * r],
                _pad0: 0.0,
                random,
                color,
                _pad1: 0.0,
            });
        }
        out
    }

    /// Rebuilds the GPU particle instance buffer when count or color palette changes.
    pub fn rebuild_particles(&mut self, device: &wgpu::Device) {
        let instances = Self::build_instances(&self.config);
        self.instance_count = instances.len() as u32;
        self.instance_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("particles-instances"),
            contents: bytemuck::cast_slice(&instances),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
    }

    /// Updates particle configuration and rebuilds the GPU instance buffer only if count, palette, or seed changed.
    pub fn apply_config(&mut self, device: &wgpu::Device, next: ParticlesConfig) {
        let needs_rebuild = self.config.particle_count != next.particle_count
            || self.config.particle_colors != next.particle_colors
            || self.config.seed != next.seed;
        self.config = next;
        if needs_rebuild {
            self.rebuild_particles(device);
        }
    }

    /// Advances orbital drift and rotation (`particles.js:415,432-434`).
    pub fn update(&mut self, dt_secs: f32) {
        if !self.lifecycle.is_paused() {
            let dt_ms = dt_secs.max(0.0) * 1000.0;
            self.elapsed_ms += dt_ms * self.config.speed;
            if !self.config.disable_rotation {
                // JS: particles.rotation.z += 0.01 * options.speed per frame (~60fps)
                self.rot_z += 0.01 * self.config.speed * (dt_secs.max(0.0) * 60.0);
            }
        }
    }

    /// Sets deterministic time in seconds (for headless tests).
    pub fn set_time(&mut self, time_secs: f32) {
        self.elapsed_ms = time_secs * 1000.0 * self.config.speed;
        self.rot_z = if self.config.disable_rotation {
            0.0
        } else {
            0.6 * self.config.speed * time_secs
        };
    }

    /// Encodes the instanced billboard quad draw call (`6` vertices x `particle_count`),
    /// using an offscreen quality-capped render target + linear upsample when `effective_dpr < instance.dpr`.
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

        let (tx, ty) = if self.config.move_particles_on_hover {
            (
                -self.config.mouse_ndc[0] * self.config.particle_hover_factor,
                -self.config.mouse_ndc[1] * self.config.particle_hover_factor,
            )
        } else {
            (0.0, 0.0)
        };

        let (rx, ry, rz) = if self.config.disable_rotation {
            (0.0, 0.0, 0.0)
        } else {
            (
                (self.elapsed_ms * 0.0002).sin() * 0.1,
                (self.elapsed_ms * 0.0005).cos() * 0.15,
                self.rot_z,
            )
        };

        let model_matrix = mat4_trs([tx, ty, 0.0], rx, ry, rz);
        let view_matrix = mat4_translation(0.0, 0.0, -self.config.camera_distance);
        let projection_matrix =
            mat4_perspective_rh(self.config.fov_deg.to_radians(), aspect, 0.1, 100.0);

        let uniforms = ParticlesUniforms {
            model_matrix,
            view_matrix,
            projection_matrix,
            params: [
                self.elapsed_ms * 0.001,
                self.config.particle_spread,
                self.config.particle_base_size * eff_dpr,
                self.config.size_randomness,
            ],
            viewport_alpha: [
                w,
                h,
                if self.config.alpha_particles {
                    1.0
                } else {
                    0.0
                },
                0.0,
            ],
        };
        queue.write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&uniforms));

        if eff_w == tw && eff_h == th {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("particles-pass"),
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
            let pipeline = if self.config.alpha_particles {
                &self.alpha_pipeline
            } else {
                &self.opaque_pipeline
            };
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.instance_buf.slice(..));
            pass.draw(0..6, 0..self.instance_count);
            return;
        }

        let needs_rebuild = match &self.offscreen {
            Some(o) => o.width != eff_w || o.height != eff_h,
            None => true,
        };
        if needs_rebuild {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("particles-offscreen"),
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
                label: Some("particles-blit-bg"),
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
            self.offscreen = Some(ParticlesOffscreenTarget {
                _texture: tex,
                view,
                blit_bind_group,
                width: eff_w,
                height: eff_h,
            });
        }
        let off = self
            .offscreen
            .as_ref()
            .expect("particles offscreen target created");
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("particles-eff-pass"),
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
            let pipeline = if self.config.alpha_particles {
                &self.alpha_pipeline
            } else {
                &self.opaque_pipeline
            };
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.instance_buf.slice(..));
            pass.draw(0..6, 0..self.instance_count);
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("particles-blit-pass"),
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

    pub fn particle_count(&self) -> u32 {
        self.instance_count
    }
}

fn mat4_identity() -> [[f32; 4]; 4] {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn mat4_translation(tx: f32, ty: f32, tz: f32) -> [[f32; 4]; 4] {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [tx, ty, tz, 1.0],
    ]
}

fn mat4_trs(t: [f32; 3], rx: f32, ry: f32, rz: f32) -> [[f32; 4]; 4] {
    let (sx, cx) = rx.sin_cos();
    let (sy, cy) = ry.sin_cos();
    let (sz, cz) = rz.sin_cos();

    // Rz * Ry * Rx (column-major)
    let m00 = cy * cz;
    let m01 = cy * sz;
    let m02 = -sy;

    let m10 = sx * sy * cz - cx * sz;
    let m11 = sx * sy * sz + cx * cz;
    let m12 = sx * cy;

    let m20 = cx * sy * cz + sx * sz;
    let m21 = cx * sy * sz - sx * cz;
    let m22 = cx * cy;

    [
        [m00, m01, m02, 0.0],
        [m10, m11, m12, 0.0],
        [m20, m21, m22, 0.0],
        [t[0], t[1], t[2], 1.0],
    ]
}

fn mat4_perspective_rh(fov_y_rad: f32, aspect: f32, near: f32, far: f32) -> [[f32; 4]; 4] {
    let f = 1.0 / (fov_y_rad * 0.5).tan().max(1e-5);
    let nf = 1.0 / (near - far);
    [
        [f / aspect.max(1e-4), 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, far * nf, -1.0],
        [0.0, 0.0, (far * near) * nf, 0.0],
    ]
}
