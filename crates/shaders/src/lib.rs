//! WGSL welcome effects, Dual-Kawase blur pyramid, SDF glassmorphism, and Chameleon compositing.

pub mod blur;
pub mod context;
pub mod effects;
pub mod error;
pub mod glass;
pub mod render_graph;
pub mod scene;
pub mod types;
pub mod ui_quad;

pub use blur::DualKawaseBlur;
pub use context::GpuContext;
pub use effects::{
    decode_and_resize_image, decode_image_file_async, default_gradients_rgb, DitherWavesEffect,
    FaultyTerminalEffect, GridMotionEffect, ParticlesEffect, WelcomeEffectsBundle, CARD_TEXTS,
    DEFAULT_GRADIENT_HEXES, MAX_IMAGE_DIM,
};
pub use error::ShaderError;
pub use glass::{
    BackdropFilterSpec, ChameleonSkin, CssBackdropRule, GlassInstanceGpu, GlassRect, SdfGlassPass,
    SdfGlassRect,
};
pub use render_graph::{
    AccordionPreviewGallery, BlurRequests, FrameCompositor, RenderGraph, SceneSource,
};
pub use scene::ScenePass;
pub use types::{
    parse_hex_rgb, parse_hex_rgba, ClearContext, DitherWavesConfig, EffectInstanceConfig,
    EffectLifecycle, FaultyTerminalConfig, GridAspectRatio, GridMotionConfig, ParticlesConfig,
    QualityTier, ThemeMode, VideoFitMode, VideoFrameTransform, WelcomeBackgroundKind,
};
pub use ui_quad::{UiQuadInstanceGpu, UiQuadPass, UiSampleMode, UiTextureId, UiTexturedQuad};
