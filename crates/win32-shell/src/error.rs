use thiserror::Error;

/// Errors returned by `hybrid-win32-shell` Win32 integration primitives.
#[derive(Debug, Error)]
pub enum Win32ShellError {
    #[error("invalid or null HWND")]
    InvalidHwnd,

    #[error("failed to extract native window handle: {0}")]
    WindowHandle(String),

    #[error("Win32 API call `{api}` failed (code {code:#010x})")]
    Win32 { api: &'static str, code: u32 },

    #[error("HRESULT call `{api}` failed ({hr:#010x}): {message}")]
    HResult {
        api: &'static str,
        hr: i32,
        message: String,
    },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("SVG/render error: {0}")]
    Render(String),
}

pub type Result<T> = std::result::Result<T, Win32ShellError>;
