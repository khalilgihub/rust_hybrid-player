//! Phase 6 QA parity suite: pure-Rust golden pixel-diff engine and Windows UIA +
//! `SendInput` + DWM/HWND verification harness.

pub mod pixel_diff;
pub mod uia_runner;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum QaError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("image error: {0}")]
    Image(#[from] image::ImageError),

    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("dimension mismatch: expected {expected_w}x{expected_h}, got {actual_w}x{actual_h}")]
    DimensionMismatch {
        expected_w: u32,
        expected_h: u32,
        actual_w: u32,
        actual_h: u32,
    },

    #[error("invalid RGBA buffer length: expected {expected} bytes for {width}x{height}, got {actual}")]
    InvalidBufferLength {
        width: u32,
        height: u32,
        expected: usize,
        actual: usize,
    },

    #[error("Windows / UIA error: {0}")]
    Win32(String),

    #[error("process launch / timeout error: {0}")]
    Process(String),
}

pub type Result<T> = std::result::Result<T, QaError>;
