use std::path::PathBuf;
use std::sync::OnceLock;

use bytemuck::{Pod, Zeroable};
use cosmic_text::{
    Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, SwashContent, Weight, Wrap,
};
use image::imageops::FilterType;
use image::RgbaImage;
use tokio::sync::mpsc;
use wgpu::util::DeviceExt;

use crate::error::ShaderError;
use crate::types::{parse_hex_rgb, EffectInstanceConfig, EffectLifecycle, GridMotionConfig};

const SORA_REGULAR_TTF: &[u8] =
    include_bytes!("../../../../hybrid-player/assets/fonts/Sora-Regular.ttf");
const SORA_SEMIBOLD_TTF: &[u8] =
    include_bytes!("../../../../hybrid-player/assets/fonts/Sora-SemiBold.ttf");
const SORA_BOLD_TTF: &[u8] = include_bytes!("../../../../hybrid-player/assets/fonts/Sora-Bold.ttf");
const IBM_PLEX_SANS_REGULAR_TTF: &[u8] =
    include_bytes!("../../../../hybrid-player/assets/fonts/IBMPlexSans-Regular.ttf");
const IBM_PLEX_SANS_MEDIUM_TTF: &[u8] =
    include_bytes!("../../../../hybrid-player/assets/fonts/IBMPlexSans-Medium.ttf");
const IBM_PLEX_SANS_SEMIBOLD_TTF: &[u8] =
    include_bytes!("../../../../hybrid-player/assets/fonts/IBMPlexSans-SemiBold.ttf");

/// Maximum longest edge when decoding custom GridMotion images (`settings.js:90`).
pub const MAX_IMAGE_DIM: u32 = 1440;

/// All 28 card labels from `gridMotion.js:61-66`.
pub const CARD_TEXTS: [&str; 28] = [
    "HYBRID",
    "PLAYER",
    "ULTRA",
    "4K",
    "HEVC",
    "GSAP",
    "MPV",
    "SMOOTH",
    "EQUALIZER",
    "AUDIO",
    "FAST",
    "CSS3",
    "ELECTRON",
    "SHADERS",
    "CHAMELEON",
    "PLAYLIST",
    "SUBTITLES",
    "AESTHETIC",
    "MOTION",
    "CANVAS",
    "DESKTOP",
    "STREAM",
    "RENDER",
    "DIAGNOSTICS",
    "PRESETS",
    "VOLUME",
    "THEMES",
    "CONTROLS",
];

/// All 28 default 135-deg linear gradients `(start_hex, end_hex)` from `gridMotion.js:30-59`.
pub const DEFAULT_GRADIENT_HEXES: [(&str, &str); 28] = [
    ("#1f005c", "#ac005c"),
    ("#0f0c1b", "#37063b"),
    ("#5f2c82", "#49a09d"),
    ("#EC008C", "#FC6767"),
    ("#24C6DC", "#514A9D"),
    ("#00c6ff", "#0072ff"),
    ("#f857a6", "#ff5858"),
    ("#11998e", "#38ef7d"),
    ("#FFAFBD", "#ffc3a0"),
    ("#2193b0", "#6dd5ed"),
    ("#ee9ca7", "#ffdde1"),
    ("#de6262", "#ffb88c"),
    ("#4568dc", "#b06ab3"),
    ("#ff5e62", "#ff9966"),
    ("#3a7bd5", "#3a6073"),
    ("#4ca1af", "#c4e0e5"),
    ("#f4c4f3", "#fc67fa"),
    ("#00c3ff", "#ffff1c"),
    ("#ff0844", "#ffb199"),
    ("#f12711", "#f5af19"),
    ("#a8c0ff", "#3f2b96"),
    ("#396afc", "#2948ff"),
    ("#8a2387", "#f27121"),
    ("#1e3c72", "#2a5298"),
    ("#0575e6", "#00f2fe"),
    ("#4b6cb7", "#182848"),
    ("#d3cbb8", "#6d6027"),
    ("#2c3e50", "#bdc3c7"),
];

/// Returns all 28 default gradient `(start_rgb, end_rgb)` pairs.
pub fn default_gradients_rgb() -> Vec<([f32; 3], [f32; 3])> {
    DEFAULT_GRADIENT_HEXES
        .iter()
        .map(|(a, b)| {
            (
                parse_hex_rgb(a).unwrap_or([0.1, 0.1, 0.1]),
                parse_hex_rgb(b).unwrap_or([0.2, 0.2, 0.2]),
            )
        })
        .collect()
}

/// Decodes raw image bytes and downscales so `max(width, height) <= max_dim` (`1440px`).
pub fn decode_and_resize_image(bytes: &[u8], max_dim: u32) -> Result<RgbaImage, ShaderError> {
    let img =
        image::load_from_memory(bytes).map_err(|e| ShaderError::ImageDecode(e.to_string()))?;
    let w = img.width();
    let h = img.height();
    let cap = max_dim.max(1);
    let resized = if w > cap || h > cap {
        img.resize(cap, cap, FilterType::Triangle)
    } else {
        img
    };
    Ok(resized.to_rgba8())
}

/// Decodes and resizes an image file on a blocking Tokio worker thread.
pub async fn decode_image_file_async(
    path: PathBuf,
    max_dim: u32,
) -> Result<RgbaImage, ShaderError> {
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|e| ShaderError::ImageDecode(format!("{}: {e}", path.display())))?;
    tokio::task::spawn_blocking(move || decode_and_resize_image(&bytes, max_dim))
        .await
        .map_err(|e| ShaderError::ImageDecode(e.to_string()))?
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GridBgUniforms {
    viewport: [f32; 4],
    center_color_scale: [f32; 4],
    tilt_cos_sin: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GridCardInstanceGpu {
    rect: [f32; 4],
    color_0: [f32; 4],
    color_1: [f32; 4],
    color_2: [f32; 4],
    color_3: [f32; 4],
    atlas_uv: [f32; 4],
    style: [f32; 4],
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn gradient_4_stops(idx: usize, fallback: ([f32; 3], [f32; 3])) -> [[f32; 4]; 4] {
    let to4 = |c: [f32; 3]| [c[0], c[1], c[2], 1.0];
    match idx {
        // gridMotion.js:31: #1f005c, #5b0060, #870060, #ac005c
        0 => [
            to4(parse_hex_rgb("#1f005c").unwrap_or(fallback.0)),
            to4(parse_hex_rgb("#5b0060").unwrap_or(fallback.0)),
            to4(parse_hex_rgb("#870060").unwrap_or(fallback.1)),
            to4(parse_hex_rgb("#ac005c").unwrap_or(fallback.1)),
        ],
        // gridMotion.js:32: #0f0c1b, #170d2c, #240a36, #37063b
        1 => [
            to4(parse_hex_rgb("#0f0c1b").unwrap_or(fallback.0)),
            to4(parse_hex_rgb("#170d2c").unwrap_or(fallback.0)),
            to4(parse_hex_rgb("#240a36").unwrap_or(fallback.1)),
            to4(parse_hex_rgb("#37063b").unwrap_or(fallback.1)),
        ],
        // gridMotion.js:53: #8a2387, #e94057, #f27121
        22 => {
            let a = parse_hex_rgb("#8a2387").unwrap_or(fallback.0);
            let m = parse_hex_rgb("#e94057").unwrap_or(fallback.0);
            let b = parse_hex_rgb("#f27121").unwrap_or(fallback.1);
            [
                to4(a),
                to4(lerp3(a, m, 2.0 / 3.0)),
                to4(lerp3(m, b, 1.0 / 3.0)),
                to4(b),
            ]
        }
        _ => [
            to4(fallback.0),
            to4(lerp3(fallback.0, fallback.1, 1.0 / 3.0)),
            to4(lerp3(fallback.0, fallback.1, 2.0 / 3.0)),
            to4(fallback.1),
        ],
    }
}

/// Decoded custom picture waiting to be uploaded into the texture atlas.
pub struct DecodedAtlasImage {
    pub slot_index: usize,
    pub aspect: f32,
    pub image: RgbaImage,
}

const ATLAS_SIZE: u32 = 2048;
// Top 256px holds 28 pre-rasterized CARD_TEXTS cells (8 cols x 4 rows of 256x64px)
const LABEL_CELL_W: u32 = 256;
const LABEL_CELL_H: u32 = 64;
const LABEL_COLS: u32 = 8;
const PHOTO_Y_OFFSET: u32 = 256;
// Bottom 1792px holds up to 16 custom image tiles (4x4 grid of 512x448px tiles)
const PHOTO_COLS: u32 = 4;
const PHOTO_ROWS: u32 = 4;
const PHOTO_TILE_W: u32 = ATLAS_SIZE / PHOTO_COLS;
const PHOTO_TILE_H: u32 = (ATLAS_SIZE - PHOTO_Y_OFFSET) / PHOTO_ROWS;
const MAX_CUSTOM_PICTURES: usize = (PHOTO_COLS * PHOTO_ROWS) as usize;

/// GPU renderer for `GridMotion` (instanced 2D rounded-quad pass + texture atlas).
pub struct GridMotionEffect {
    bg_pipeline: wgpu::RenderPipeline,
    card_pipeline: wgpu::RenderPipeline,
    uniform_buf: wgpu::Buffer,
    uniform_bg: wgpu::BindGroup,
    atlas_tex: wgpu::Texture,
    atlas_bg: wgpu::BindGroup,
    instance_buf: wgpu::Buffer,
    instance_capacity: usize,
    label_uvs: Vec<[f32; 4]>,
    photo_uvs: Vec<Option<[f32; 4]>>,
    photo_aspects: Vec<Option<f32>>,
    brand_font_loaded: bool,
    upload_tx: mpsc::UnboundedSender<DecodedAtlasImage>,
    upload_rx: mpsc::UnboundedReceiver<DecodedAtlasImage>,
    gradients: Vec<([f32; 3], [f32; 3])>,
    pub config: GridMotionConfig,
    pub instance: EffectInstanceConfig,
    pub lifecycle: EffectLifecycle,
    target_scroll_offset: f32,
    row_scroll_offsets: Vec<f32>,
    auto_offset: f32,
}

impl GridMotionEffect {
    /// Creates the `GridMotion` pipelines and pre-populates the 28 `CARD_TEXTS` label atlas cells.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        config: GridMotionConfig,
        instance: EffectInstanceConfig,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("grid-motion-wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/grid_motion.wgsl").into()),
        });

        let uniform_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("grid-uniform-bgl"),
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

        let atlas_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("grid-atlas-bgl"),
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

        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("grid-motion-pl"),
            bind_group_layouts: &[Some(&uniform_bgl), Some(&atlas_bgl)],
            immediate_size: 0,
        });

        let bg_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("grid-motion-bg-pipeline"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_grid_bg"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_grid_bg"),
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

        let card_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GridCardInstanceGpu>() as wgpu::BufferAddress,
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
            ],
        };

        let card_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("grid-motion-card-pipeline"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_grid_card"),
                buffers: &[Some(card_layout)],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_grid_card"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
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

        let uniform_init = GridBgUniforms {
            viewport: [1280.0, 720.0, 0.0, 0.0],
            center_color_scale: [
                config.gradient_color[0],
                config.gradient_color[1],
                config.gradient_color[2],
                config.scale,
            ],
            tilt_cos_sin: [1.0, 0.0, 0.0, 0.0],
        };
        let uniform_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("grid-motion-uniforms"),
            contents: bytemuck::bytes_of(&uniform_init),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let uniform_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("grid-motion-uniform-bg"),
            layout: &uniform_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });

        let atlas_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("grid-motion-atlas"),
            size: wgpu::Extent3d {
                width: ATLAS_SIZE,
                height: ATLAS_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let atlas_view = atlas_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let atlas_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("grid-atlas-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let atlas_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("grid-motion-atlas-bg"),
            layout: &atlas_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&atlas_sampler),
                },
            ],
        });

        let brand_font_loaded = config.brand_font_enabled;
        let label_uvs = Self::populate_label_atlas(queue, &atlas_tex, brand_font_loaded);
        let rows = config.rows.max(1) as usize;
        let cols = config.cols.max(2) as usize;
        let instance_capacity = (rows * cols).max(64);
        let instance_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("grid-motion-instances"),
            size: (instance_capacity * std::mem::size_of::<GridCardInstanceGpu>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let (upload_tx, upload_rx) = mpsc::unbounded_channel();
        let lifecycle = EffectLifecycle::from_instance(&instance);

        Self {
            bg_pipeline,
            card_pipeline,
            uniform_buf,
            uniform_bg,
            atlas_tex,
            atlas_bg,
            instance_buf,
            instance_capacity,
            label_uvs,
            photo_uvs: Vec::new(),
            photo_aspects: Vec::new(),
            brand_font_loaded,
            upload_tx,
            upload_rx,
            gradients: default_gradients_rgb(),
            config,
            instance,
            lifecycle,
            target_scroll_offset: 0.0,
            row_scroll_offsets: vec![0.0; rows],
            auto_offset: 0.0,
        }
    }

    fn populate_label_atlas(
        queue: &wgpu::Queue,
        atlas_tex: &wgpu::Texture,
        brand_font_enabled: bool,
    ) -> Vec<[f32; 4]> {
        let mut uvs = Vec::with_capacity(CARD_TEXTS.len());
        let cached_labels = cached_label_bitmaps(brand_font_enabled);

        for (idx, pixels) in cached_labels.iter().enumerate() {
            let col = (idx as u32) % LABEL_COLS;
            let row = (idx as u32) / LABEL_COLS;
            let x = col * LABEL_CELL_W;
            let y = row * LABEL_CELL_H;

            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: atlas_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x, y, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(LABEL_CELL_W * 4),
                    rows_per_image: Some(LABEL_CELL_H),
                },
                wgpu::Extent3d {
                    width: LABEL_CELL_W,
                    height: LABEL_CELL_H,
                    depth_or_array_layers: 1,
                },
            );

            let f_atlas = ATLAS_SIZE as f32;
            uvs.push([
                (x as f32 + 1.0) / f_atlas,
                (y as f32 + 1.0) / f_atlas,
                ((x + LABEL_CELL_W) as f32 - 1.0) / f_atlas,
                ((y + LABEL_CELL_H) as f32 - 1.0) / f_atlas,
            ]);
        }
        uvs
    }

    /// Updates `GridMotionConfig` and refreshes the label font atlas if `brand_font_enabled` changed.
    pub fn apply_config(&mut self, queue: &wgpu::Queue, next: GridMotionConfig) {
        let brand_changed = next.brand_font_enabled != self.brand_font_loaded;
        self.config = next;
        if brand_changed {
            self.brand_font_loaded = self.config.brand_font_enabled;
            self.label_uvs =
                Self::populate_label_atlas(queue, &self.atlas_tex, self.brand_font_loaded);
        }
    }

    /// Uploads a decoded `RgbaImage` directly into atlas slot `slot_index` (`0..16`).
    pub fn upload_image_to_slot(
        &mut self,
        queue: &wgpu::Queue,
        slot_index: usize,
        img: &RgbaImage,
    ) {
        if slot_index >= MAX_CUSTOM_PICTURES {
            return;
        }
        let orig_aspect = (img.width().max(1) as f32) / (img.height().max(1) as f32);
        self.upload_image_with_aspect(queue, slot_index, orig_aspect, img);
    }

    fn upload_image_with_aspect(
        &mut self,
        queue: &wgpu::Queue,
        slot_index: usize,
        aspect: f32,
        img: &RgbaImage,
    ) {
        if slot_index >= MAX_CUSTOM_PICTURES {
            return;
        }
        let tile = image::imageops::resize(img, PHOTO_TILE_W, PHOTO_TILE_H, FilterType::Triangle);
        let col = (slot_index as u32) % PHOTO_COLS;
        let row = (slot_index as u32) / PHOTO_COLS;
        let x = col * PHOTO_TILE_W;
        let y = PHOTO_Y_OFFSET + row * PHOTO_TILE_H;

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.atlas_tex,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            tile.as_raw(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(PHOTO_TILE_W * 4),
                rows_per_image: Some(PHOTO_TILE_H),
            },
            wgpu::Extent3d {
                width: PHOTO_TILE_W,
                height: PHOTO_TILE_H,
                depth_or_array_layers: 1,
            },
        );

        if self.photo_uvs.len() <= slot_index {
            self.photo_uvs.resize(slot_index + 1, None);
        }
        if self.photo_aspects.len() <= slot_index {
            self.photo_aspects.resize(slot_index + 1, None);
        }
        let f_atlas = ATLAS_SIZE as f32;
        self.photo_uvs[slot_index] = Some([
            (x as f32 + 1.0) / f_atlas,
            (y as f32 + 1.0) / f_atlas,
            ((x + PHOTO_TILE_W) as f32 - 1.0) / f_atlas,
            ((y + PHOTO_TILE_H) as f32 - 1.0) / f_atlas,
        ]);
        self.photo_aspects[slot_index] = Some(aspect);
    }

    /// Clears all custom images (reverting cards to the 28 gradients + `CARD_TEXTS`).
    pub fn clear_custom_images(&mut self) {
        self.photo_uvs.clear();
        self.photo_aspects.clear();
    }

    /// Spawns async Tokio tasks to decode & resize up to 16 image files (`max_dim = 1440px`) into the atlas.
    pub fn load_images_async(&mut self, paths: Vec<PathBuf>) {
        self.photo_uvs.clear();
        self.photo_aspects.clear();
        let count = paths.len().min(MAX_CUSTOM_PICTURES);
        self.photo_uvs.resize(count, None);
        self.photo_aspects.resize(count, None);

        for (slot_index, path) in paths.into_iter().take(MAX_CUSTOM_PICTURES).enumerate() {
            let tx = self.upload_tx.clone();
            let fut = async move {
                if let Ok(image) = decode_image_file_async(path, MAX_IMAGE_DIM).await {
                    let aspect = (image.width().max(1) as f32) / (image.height().max(1) as f32);
                    let _ = tx.send(DecodedAtlasImage {
                        slot_index,
                        aspect,
                        image,
                    });
                }
            };
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(fut);
            } else {
                std::thread::spawn(move || {
                    if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        rt.block_on(fut);
                    }
                });
            }
        }
    }

    /// Spawns async Tokio tasks to decode & resize up to 16 image sources (file paths or `data:image/...;base64,...` URLs).
    pub fn load_image_sources_async(&mut self, sources: Vec<String>) {
        self.photo_uvs.clear();
        self.photo_aspects.clear();
        let count = sources.len().min(MAX_CUSTOM_PICTURES);
        self.photo_uvs.resize(count, None);
        self.photo_aspects.resize(count, None);

        for (slot_index, src) in sources.into_iter().take(MAX_CUSTOM_PICTURES).enumerate() {
            let tx = self.upload_tx.clone();
            let fut = async move {
                let decoded = if src.starts_with("data:") {
                    if let Some(bytes) = decode_data_url_bytes(&src) {
                        tokio::task::spawn_blocking(move || {
                            decode_and_resize_image(&bytes, MAX_IMAGE_DIM)
                        })
                        .await
                        .ok()
                        .and_then(Result::ok)
                    } else {
                        None
                    }
                } else {
                    decode_image_file_async(PathBuf::from(src), MAX_IMAGE_DIM)
                        .await
                        .ok()
                };
                if let Some(image) = decoded {
                    let aspect = (image.width().max(1) as f32) / (image.height().max(1) as f32);
                    let _ = tx.send(DecodedAtlasImage {
                        slot_index,
                        aspect,
                        image,
                    });
                }
            };
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(fut);
            } else {
                std::thread::spawn(move || {
                    if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        rt.block_on(fut);
                    }
                });
            }
        }
    }

    /// Drains any finished async decoded images and uploads them to the GPU texture atlas.
    pub fn poll_async_uploads(&mut self, queue: &wgpu::Queue) -> usize {
        let mut uploaded = 0;
        while let Ok(item) = self.upload_rx.try_recv() {
            self.upload_image_with_aspect(queue, item.slot_index, item.aspect, &item.image);
            uploaded += 1;
        }
        uploaded
    }

    /// Applies mouse wheel scroll delta (`gridMotion.js:405-409`).
    pub fn apply_wheel_scroll(&mut self, delta_y: f32) {
        let speed = self.config.speed.max(0.1);
        self.target_scroll_offset += (-delta_y) * (0.85 * speed);
    }

    /// Resets scroll offsets (`gridMotion.js:570-576`).
    pub fn reset_scroll(&mut self) {
        self.target_scroll_offset = 0.0;
        self.auto_offset = 0.0;
        self.row_scroll_offsets.fill(0.0);
    }

    /// Advances row inertial scroll & auto-glide (`gridMotion.js:530-563`).
    pub fn update(&mut self, dt_secs: f32) {
        if self.lifecycle.is_paused() {
            return;
        }
        let dt = dt_secs.clamp(0.0, 0.1);
        let speed = self.config.speed.max(0.1);
        if self.config.auto_move {
            self.auto_offset += 42.0 * speed * dt;
        }

        let rows = self.config.rows.max(1) as usize;
        if self.row_scroll_offsets.len() != rows {
            self.row_scroll_offsets.resize(rows, 0.0);
        }
        let inertia = [1.2_f32, 0.95, 0.8, 0.7, 1.1, 0.85];
        for (r, offset) in self.row_scroll_offsets.iter_mut().enumerate() {
            let factor = (dt * 8.0 * inertia[r % inertia.len()]).min(1.0);
            *offset += (self.target_scroll_offset - *offset) * factor;
        }
    }

    /// Sets deterministic scroll/auto offsets (for headless tests).
    pub fn set_scroll_state(&mut self, auto_offset: f32, target_scroll: f32) {
        self.auto_offset = auto_offset;
        self.target_scroll_offset = target_scroll;
        self.row_scroll_offsets.fill(target_scroll);
    }

    /// Returns number of active custom picture slots loaded.
    pub fn loaded_photo_count(&self) -> usize {
        self.photo_uvs.iter().filter(|u| u.is_some()).count()
    }

    /// Hit-tests a logical DIP cursor coordinate against the `-12deg` tilted card grid.
    pub fn hit_test_card(
        &self,
        cursor_x_dip: f32,
        cursor_y_dip: f32,
        viewport_w_dip: f32,
        viewport_h_dip: f32,
    ) -> Option<(u32, u32)> {
        let vw_dip = viewport_w_dip.max(1.0);
        let vh_dip = viewport_h_dip.max(1.0);
        let rows = self.config.rows.max(1);
        let cols = self.config.cols.max(2);
        let scale = if self.config.miniature {
            1.0
        } else {
            self.config.scale.clamp(0.35, 1.6)
        };
        let angle_rad = (-12.0_f32).to_radians();
        let (sn, cs) = angle_rad.sin_cos();

        let dx = (cursor_x_dip - vw_dip * 0.5) / scale;
        let dy = (cursor_y_dip - vh_dip * 0.5) / scale;
        let gx = dx * cs + dy * sn;
        let gy = -dx * sn + dy * cs;

        let container_w = if self.config.miniature {
            vw_dip * 3.2
        } else {
            vw_dip * ((cols as f32) / 3.0)
        };
        let gap = if self.config.miniature { 5.6 } else { 8.0 };
        let card_w = ((container_w - gap * ((cols - 1) as f32)) / (cols as f32)).max(4.0);
        let card_h = (card_w / self.config.aspect_ratio.as_f32()).max(4.0);
        let card_pitch = card_w + gap;
        let row_pitch = card_h + gap;

        let active_count = self.photo_uvs.iter().filter(|u| u.is_some()).count();
        let num_pics = if active_count > 0 {
            active_count
        } else {
            (cols as usize).min(7)
        };
        let loop_w = (num_pics as f32 * card_pitch).max(1.0);
        let mouse_delta = (self.config.pointer_x / vw_dip).clamp(0.0, 1.0) * self.config.max_move
            - self.config.max_move * 0.5;
        let half_rows_span = ((rows - 1) as f32) * row_pitch * 0.5;
        let half_cols_span = ((cols - 1) as f32) * card_pitch * 0.5;

        for r in 0..rows {
            let cy = (r as f32) * row_pitch - half_rows_span;
            if (gy - cy).abs() > card_h * 0.5 {
                continue;
            }
            let dir = if r % 2 == 0 { 1.0 } else { -1.0 };
            let scroll = self
                .row_scroll_offsets
                .get(r as usize)
                .copied()
                .unwrap_or(0.0);
            let raw_x = (scroll + self.auto_offset + mouse_delta) * dir;
            let wrapped_x = ((raw_x + loop_w * 0.5).rem_euclid(loop_w)) - loop_w * 0.5;
            for c in 0..cols {
                let cx = (c as f32) * card_pitch - half_cols_span + wrapped_x;
                if (gx - cx).abs() <= card_w * 0.5 {
                    return Some((r, c));
                }
            }
        }
        None
    }

    /// Encodes background radial gradient + instanced `-12deg` tilted rounded card grid.
    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target_view: &wgpu::TextureView,
        target_w: u32,
        target_h: u32,
    ) {
        self.poll_async_uploads(queue);

        let vw = target_w.max(1) as f32;
        let vh = target_h.max(1) as f32;
        let rows = self.config.rows.max(1);
        let cols = self.config.cols.max(2);

        let scale = if self.config.miniature {
            1.0
        } else {
            self.config.scale.clamp(0.35, 1.6)
        };
        let angle_rad = (-12.0_f32).to_radians();
        let (sn, cs) = angle_rad.sin_cos();

        let uniforms = GridBgUniforms {
            viewport: [vw, vh, 0.0, 0.0],
            center_color_scale: [
                self.config.gradient_color[0],
                self.config.gradient_color[1],
                self.config.gradient_color[2],
                scale,
            ],
            tilt_cos_sin: [cs * scale, sn * scale, 0.0, 0.0],
        };
        queue.write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&uniforms));

        let dpr = if self.instance.dpr.is_finite() && self.instance.dpr > 0.0 {
            self.instance.dpr
        } else {
            1.0
        };

        // Grid geometry matching gridMotion.js:98, 128-163, 259-261
        let container_w = if self.config.miniature {
            vw * 3.2
        } else {
            vw * ((cols as f32) / 3.0)
        };
        let gap = if self.config.miniature { 5.6 } else { 8.0 } * dpr;
        let card_w = ((container_w - gap * ((cols - 1) as f32)) / (cols as f32)).max(4.0);
        let card_h = (card_w / self.config.aspect_ratio.as_f32()).max(4.0);
        let card_pitch = card_w + gap;
        let row_pitch = card_h + gap;

        let active_photos: Vec<([f32; 4], f32)> = self
            .photo_uvs
            .iter()
            .enumerate()
            .filter_map(|(i, uv)| {
                uv.map(|u| {
                    let aspect = self.photo_aspects.get(i).and_then(|a| *a).unwrap_or(0.0);
                    (u, aspect)
                })
            })
            .collect();
        let has_pics = !active_photos.is_empty();
        let num_pics = if has_pics {
            active_photos.len()
        } else {
            (cols as usize).min(7)
        };
        let loop_w = (num_pics as f32 * card_pitch).max(1.0);
        let mouse_delta = ((self.config.pointer_x * dpr / vw).clamp(0.0, 1.0)
            * self.config.max_move
            - self.config.max_move * 0.5)
            * dpr;
        let corner_radius = if self.config.miniature { 6.0 } else { 12.0 } * dpr;

        let total_cards = (rows as usize) * (cols as usize);
        if total_cards > self.instance_capacity {
            self.instance_capacity = total_cards.next_power_of_two();
            self.instance_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("grid-motion-instances"),
                size: (self.instance_capacity * std::mem::size_of::<GridCardInstanceGpu>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }

        let mut instances = Vec::with_capacity(total_cards);
        let half_rows_span = ((rows - 1) as f32) * row_pitch * 0.5;
        let half_cols_span = ((cols - 1) as f32) * card_pitch * 0.5;

        for r in 0..rows {
            let dir = if r % 2 == 0 { 1.0 } else { -1.0 };
            let scroll = self
                .row_scroll_offsets
                .get(r as usize)
                .copied()
                .unwrap_or(0.0);
            let raw_x = (scroll * dpr + self.auto_offset * dpr + mouse_delta) * dir;
            let wrapped_x = ((raw_x + loop_w * 0.5).rem_euclid(loop_w)) - loop_w * 0.5;
            let cy = (r as f32) * row_pitch - half_rows_span;

            for c in 0..cols {
                let cx = (c as f32) * card_pitch - half_cols_span + wrapped_x;
                let is_hovered = self.config.hovered_card == Some((r, c));
                let hover_zoom = if is_hovered { 1.06 } else { 1.0 };
                let brightness = if is_hovered { 1.08 } else { 1.0 };

                if has_pics {
                    let pic_idx = ((c as usize) + (r as usize) * 2) % active_photos.len();
                    let (atlas_uv, img_aspect) = active_photos[pic_idx];
                    let solid = [0.067, 0.067, 0.067, 1.0];
                    instances.push(GridCardInstanceGpu {
                        rect: [cx, cy, card_w, card_h],
                        color_0: [0.067, 0.067, 0.067, img_aspect],
                        color_1: solid,
                        color_2: solid,
                        color_3: solid,
                        atlas_uv,
                        style: [corner_radius, hover_zoom, brightness, 1.0],
                    });
                } else {
                    // Matches gridMotion.js:208,236: initialNumPics = min(cols, 7), textIdx = (c + r * 3) % initialNumPics
                    let text_idx = ((c as usize) + (r as usize) * 3) % num_pics.max(1);
                    let grad_idx = text_idx % self.gradients.len();
                    let label_idx = text_idx % self.label_uvs.len();
                    let stops = gradient_4_stops(grad_idx, self.gradients[grad_idx]);
                    instances.push(GridCardInstanceGpu {
                        rect: [cx, cy, card_w, card_h],
                        color_0: stops[0],
                        color_1: stops[1],
                        color_2: stops[2],
                        color_3: stops[3],
                        atlas_uv: self.label_uvs[label_idx],
                        style: [corner_radius, hover_zoom, brightness, 0.0],
                    });
                }
            }
        }

        queue.write_buffer(&self.instance_buf, 0, bytemuck::cast_slice(&instances));

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("grid-motion-pass"),
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

        pass.set_bind_group(0, &self.uniform_bg, &[]);
        pass.set_bind_group(1, &self.atlas_bg, &[]);

        // Radial gradient backdrop
        pass.set_pipeline(&self.bg_pipeline);
        pass.draw(0..3, 0..1);

        // Instanced tilted rounded card grid
        pass.set_pipeline(&self.card_pipeline);
        pass.set_vertex_buffer(0, self.instance_buf.slice(..));
        pass.draw(0..6, 0..instances.len() as u32);
    }
}

fn cached_label_bitmaps(brand_font_enabled: bool) -> &'static Vec<Vec<u8>> {
    static BRAND_CACHE: OnceLock<Vec<Vec<u8>>> = OnceLock::new();
    static SYSTEM_CACHE: OnceLock<Vec<Vec<u8>>> = OnceLock::new();
    let cell = if brand_font_enabled {
        &BRAND_CACHE
    } else {
        &SYSTEM_CACHE
    };
    cell.get_or_init(|| rasterize_all_labels_with_cosmic_text(brand_font_enabled))
}

/// Renders all 28 `CARD_TEXTS` cells using `cosmic-text` (`IBM Plex Sans` / `Segoe UI`, `16.64px`,
/// `font-weight: 700`, `letter-spacing: 0.05em`, `rgba(255,255,255,0.85)`) with a 2D separable
/// Gaussian `text-shadow: 0 2px 10px rgba(0, 0, 0, 0.8)` into premultiplied RGBA8 buffers.
fn rasterize_all_labels_with_cosmic_text(brand_font_enabled: bool) -> Vec<Vec<u8>> {
    let mut font_system = FontSystem::new();
    for bytes in [
        IBM_PLEX_SANS_REGULAR_TTF,
        IBM_PLEX_SANS_MEDIUM_TTF,
        IBM_PLEX_SANS_SEMIBOLD_TTF,
        SORA_REGULAR_TTF,
        SORA_SEMIBOLD_TTF,
        SORA_BOLD_TTF,
    ] {
        font_system.db_mut().load_font_data(bytes.to_vec());
    }
    let mut swash_cache = SwashCache::new();

    let family = if brand_font_enabled {
        Family::Name("IBM Plex Sans")
    } else {
        Family::Name("Segoe UI")
    };
    let attrs = Attrs::new()
        .family(family)
        .weight(Weight::BOLD)
        .letter_spacing(0.05);
    let metrics = Metrics::new(16.64, 22.0);
    let kernel = gaussian_kernel_1d(4.5, 14);

    let w = LABEL_CELL_W as usize;
    let h = LABEL_CELL_H as usize;
    let mut out = Vec::with_capacity(CARD_TEXTS.len());

    for text in CARD_TEXTS {
        let mut buffer = Buffer::new(&mut font_system, metrics);
        {
            let mut borrowed = buffer.borrow_with(&mut font_system);
            borrowed.set_wrap(Wrap::None);
            borrowed.set_size(None, None);
            borrowed.set_text(text, &attrs, Shaping::Advanced, None);
            borrowed.shape_until_scroll(true);
        }

        struct GlyphPatch {
            x: i32,
            y: i32,
            w: u32,
            h: u32,
            alpha: Vec<f32>,
        }
        let mut patches = Vec::new();
        let mut min_x = i32::MAX;
        let mut min_y = i32::MAX;
        let mut max_x = i32::MIN;
        let mut max_y = i32::MIN;

        for run in buffer.layout_runs() {
            for glyph in run.glyphs.iter() {
                let physical = glyph.physical((0.0, 0.0), 1.0);
                let Some(img) =
                    swash_cache.get_image_uncached(&mut font_system, physical.cache_key)
                else {
                    continue;
                };
                if img.placement.width == 0 || img.placement.height == 0 {
                    continue;
                }
                let gx = physical.x + img.placement.left;
                let gy = (run.line_y as i32) + physical.y - img.placement.top;
                let gw = img.placement.width;
                let gh = img.placement.height;

                min_x = min_x.min(gx);
                min_y = min_y.min(gy);
                max_x = max_x.max(gx + gw as i32);
                max_y = max_y.max(gy + gh as i32);

                let alpha = match img.content {
                    SwashContent::Mask => img.data.iter().map(|&a| a as f32 / 255.0).collect(),
                    SwashContent::SubpixelMask => img
                        .data
                        .chunks_exact(3)
                        .map(|c| (c[0] as f32 + c[1] as f32 + c[2] as f32) / (3.0 * 255.0))
                        .collect(),
                    SwashContent::Color => img
                        .data
                        .chunks_exact(4)
                        .map(|c| c[3] as f32 / 255.0)
                        .collect(),
                };
                patches.push(GlyphPatch {
                    x: gx,
                    y: gy,
                    w: gw,
                    h: gh,
                    alpha,
                });
            }
        }

        let mut pixels = vec![0u8; w * h * 4];
        if patches.is_empty() || min_x >= max_x || min_y >= max_y {
            out.push(pixels);
            continue;
        }

        let offset_x = ((w as i32) - (max_x - min_x)) / 2 - min_x;
        let offset_y = ((h as i32) - (max_y - min_y)) / 2 - min_y;

        let mut fg_mask = vec![0.0_f32; w * h];
        for patch in &patches {
            let bx = patch.x + offset_x;
            let by = patch.y + offset_y;
            for py in 0..patch.h {
                let dst_y = by + py as i32;
                if dst_y < 0 || dst_y >= h as i32 {
                    continue;
                }
                for px in 0..patch.w {
                    let dst_x = bx + px as i32;
                    if dst_x < 0 || dst_x >= w as i32 {
                        continue;
                    }
                    let src = patch.alpha[(py * patch.w + px) as usize];
                    let dst = &mut fg_mask[(dst_y as usize) * w + (dst_x as usize)];
                    *dst = 1.0 - (1.0 - *dst) * (1.0 - src);
                }
            }
        }

        // Separable 2D Gaussian blur for CSS `text-shadow: 0 2px 10px rgba(0, 0, 0, 0.8)`
        let shadow = blur_mask_2d_offset(&fg_mask, w, h, &kernel, 2);

        for i in 0..(w * h) {
            let fg_a = (fg_mask[i] * 0.85).clamp(0.0, 1.0);
            let sh_a = (shadow[i] * 0.80).clamp(0.0, 1.0);
            let out_a = fg_a + sh_a * (1.0 - fg_a);
            if out_a <= 0.002 {
                continue;
            }
            // Premultiplied RGBA: white foreground (0.85) over black drop shadow
            let c = (fg_a * 255.0).round() as u8;
            let a = (out_a * 255.0).round() as u8;
            let idx = i * 4;
            pixels[idx] = c;
            pixels[idx + 1] = c;
            pixels[idx + 2] = c;
            pixels[idx + 3] = a;
        }
        out.push(pixels);
    }

    out
}

fn gaussian_kernel_1d(sigma: f32, radius: usize) -> Vec<f32> {
    let mut weights = Vec::with_capacity(radius * 2 + 1);
    let denom = 2.0 * sigma * sigma;
    let mut sum = 0.0_f32;
    for i in -(radius as isize)..=(radius as isize) {
        let x = i as f32;
        let w = (-x * x / denom).exp();
        weights.push(w);
        sum += w;
    }
    if sum > 0.0 {
        for w in &mut weights {
            *w /= sum;
        }
    }
    weights
}

fn blur_mask_2d_offset(
    mask: &[f32],
    w: usize,
    h: usize,
    kernel: &[f32],
    y_offset: isize,
) -> Vec<f32> {
    let radius = (kernel.len() / 2) as isize;
    let mut tmp = vec![0.0_f32; w * h];
    for y in 0..h {
        let row = y * w;
        for x in 0..w {
            let mut acc = 0.0_f32;
            for (k_idx, &kw) in kernel.iter().enumerate() {
                let sx = (x as isize) + (k_idx as isize) - radius;
                if sx >= 0 && sx < w as isize {
                    acc += mask[row + sx as usize] * kw;
                }
            }
            tmp[row + x] = acc;
        }
    }

    let mut out = vec![0.0_f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0_f32;
            for (k_idx, &kw) in kernel.iter().enumerate() {
                let sy = (y as isize) - y_offset + (k_idx as isize) - radius;
                if sy >= 0 && sy < h as isize {
                    acc += tmp[(sy as usize) * w + x] * kw;
                }
            }
            out[y * w + x] = acc;
        }
    }
    out
}

fn decode_data_url_bytes(url: &str) -> Option<Vec<u8>> {
    let rest = url.strip_prefix("data:")?;
    let (meta, payload) = rest.split_once(',')?;
    if !meta
        .split(';')
        .any(|part| part.eq_ignore_ascii_case("base64"))
    {
        return None;
    }
    decode_base64(payload)
}

fn decode_base64(input: &str) -> Option<Vec<u8>> {
    fn val(b: u8) -> Option<u8> {
        match b {
            b'A'..=b'Z' => Some(b - b'A'),
            b'a'..=b'z' => Some(b - b'a' + 26),
            b'0'..=b'9' => Some(b - b'0' + 52),
            b'+' | b'-' => Some(62),
            b'/' | b'_' => Some(63),
            _ => None,
        }
    }

    let mut out = Vec::with_capacity((input.len() * 3) / 4);
    let mut buf = [0u8; 4];
    let mut len = 0usize;
    let mut pad = 0usize;

    for &b in input.as_bytes() {
        if b.is_ascii_whitespace() {
            continue;
        }
        if b == b'=' {
            pad += 1;
            buf[len] = 0;
            len += 1;
        } else {
            if pad > 0 {
                return None;
            }
            buf[len] = val(b)?;
            len += 1;
        }
        if len == 4 {
            let n = ((buf[0] as u32) << 18)
                | ((buf[1] as u32) << 12)
                | ((buf[2] as u32) << 6)
                | (buf[3] as u32);
            out.push((n >> 16) as u8);
            if pad < 2 {
                out.push((n >> 8) as u8);
            }
            if pad == 0 {
                out.push(n as u8);
            }
            len = 0;
            pad = 0;
        }
    }

    if len != 0 {
        return None;
    }
    Some(out)
}
