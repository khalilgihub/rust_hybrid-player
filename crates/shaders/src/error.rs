use thiserror::Error;

/// Errors returned by the `hybrid-shaders` GPU pipeline and effect loaders.
#[derive(Debug, Error)]
pub enum ShaderError {
    /// No compatible `wgpu` adapter was found on this system.
    #[error("failed to find a compatible wgpu GPU adapter")]
    NoAdapter,

    /// Creating the logical `wgpu::Device` or `wgpu::Queue` failed.
    #[error("failed to request wgpu device: {0}")]
    DeviceRequest(#[from] wgpu::RequestDeviceError),

    /// Surface or offscreen render target dimensions were invalid (zero width or height).
    #[error("invalid render target dimensions: {width}x{height}")]
    InvalidDimensions { width: u32, height: u32 },

    /// An image asset failed to decode or resize for `GridMotion`.
    #[error("failed to decode grid motion image: {0}")]
    ImageDecode(String),

    /// Mapping a GPU staging buffer for readback failed.
    #[error("failed to map GPU readback buffer: {0}")]
    BufferMap(String),
}
