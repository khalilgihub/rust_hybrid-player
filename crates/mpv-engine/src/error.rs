use thiserror::Error;

/// Errors returned by `hybrid-mpv-engine`.
#[derive(Debug, Error)]
pub enum MpvError {
    #[error("failed to locate or load libmpv-2.dll: {0}")]
    LibraryLoad(String),

    #[error("missing symbol `{0}` in libmpv-2.dll")]
    MissingSymbol(&'static str),

    #[error("mpv API error ({code}): {message}")]
    Api { code: i32, message: String },

    #[error("mpv_create() returned null")]
    CreateFailed,

    #[error("invalid string passed to libmpv: {0}")]
    InvalidString(String),

    #[error("Win32 / WGL initialization failed: {0}")]
    Wgl(String),

    #[error("D3D11 / DXGI / DX12 interop error: {0}")]
    D3dInterop(String),

    #[error("operation timed out: {0}")]
    Timeout(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, MpvError>;
