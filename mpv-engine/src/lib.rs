//! `libmpv` dynamic loader, `WGL_NV_DX_interop2` + D3D11/DX12 zero-copy bridge,
//! burst screenshot manager, and headless seekbar thumbnailer.

pub mod client;
pub mod config;
pub mod dx_bridge;
pub mod error;
pub mod ffi;
pub mod thumbnailer;

use std::sync::Arc;

use serde_json::Value;

pub use client::{
    parse_subprocess_ipc_line, BurstFileEntry, BurstFinalizeResult, BurstFrameInfo, EndFileReason,
    MpvClient, MpvEvent, ParsedIpcLine,
};
pub use config::{
    build_ytdl_raw_options, format_ytdl_option_value, resolve_libmpv_path, MpvStartupConfig,
    ScreenshotFormat, ScreenshotMode, DEFAULT_USER_AGENT, OBSERVED_PROPERTIES,
};
pub use dx_bridge::{DxBridge, FrameReadyCallback, ImportedVideoFrame, NUM_BUFFERS};
pub use error::{MpvError, Result};
pub use ffi::MpvApi;
pub use thumbnailer::{SeekThumbnailer, ThumbnailPayload, THUMB_CACHE_MAX_ENTRIES};

/// Extracts display width and height from a `video-params` JSON object, accounting
/// for 90/270-degree rotation (`mpv-process.js:344-353`).
pub fn extract_video_dimensions(val: &Value) -> Option<(u32, u32)> {
    let obj = val.as_object()?;
    let rot = obj.get("rotate").and_then(|v| v.as_i64()).unwrap_or(0);
    let is_rotated = (rot % 180) != 0;

    let raw_w = obj
        .get("dw")
        .or_else(|| obj.get("w"))
        .or_else(|| obj.get("width"))
        .and_then(|v| v.as_u64())? as u32;
    let raw_h = obj
        .get("dh")
        .or_else(|| obj.get("h"))
        .or_else(|| obj.get("height"))
        .and_then(|v| v.as_u64())? as u32;

    if raw_w == 0 || raw_h == 0 {
        return None;
    }
    if is_rotated {
        Some((raw_h, raw_w))
    } else {
        Some((raw_w, raw_h))
    }
}

/// High-level facade bundling `MpvClient`, `DxBridge`, and `SeekThumbnailer`.
pub struct MpvEngine {
    pub api: Arc<MpvApi>,
    pub client: Arc<MpvClient>,
    pub bridge: Arc<DxBridge>,
    pub thumbnailer: Arc<SeekThumbnailer>,
    _resize_task: tokio::task::JoinHandle<()>,
}

impl MpvEngine {
    /// Loads `libmpv-2.dll`, initializes the primary `MpvClient` (`vo=libmpv`),
    /// starts the `DxBridge` on the exact LUID-matched DX12 adapter, and prepares
    /// the secondary headless `SeekThumbnailer`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: MpvStartupConfig,
        wgpu_device: Arc<wgpu::Device>,
        wgpu_queue: Arc<wgpu::Queue>,
        adapter: &wgpu::Adapter,
        initial_width: u32,
        initial_height: u32,
        flip_y: bool,
        on_frame: Option<FrameReadyCallback>,
    ) -> Result<Self> {
        let dll_path = resolve_libmpv_path(config.libmpv_path.as_deref()).ok_or_else(|| {
            MpvError::LibraryLoad("libmpv-2.dll not found in app dir, mpv/, or PATH".to_owned())
        })?;
        let api = MpvApi::load(&dll_path)?;
        let client = MpvClient::new(api.clone(), &config)?;
        let bridge = Arc::new(DxBridge::new(
            client.clone(),
            wgpu_device,
            wgpu_queue,
            adapter,
            initial_width,
            initial_height,
            flip_y,
            on_frame,
        )?);
        let thumbnailer = Arc::new(SeekThumbnailer::new(api.clone())?);

        let mut rx = client.subscribe();
        let bridge_for_resize = bridge.clone();
        let resize_task = tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(MpvEvent::PropertyChange { name, value, .. }) if name == "video-params" => {
                        if let Some((w, h)) = extract_video_dimensions(&value) {
                            bridge_for_resize.resize(w, h);
                        }
                    }
                    Ok(MpvEvent::Shutdown)
                    | Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    _ => {}
                }
            }
        });

        Ok(Self {
            api,
            client,
            bridge,
            thumbnailer,
            _resize_task: resize_task,
        })
    }
}

impl Drop for MpvEngine {
    fn drop(&mut self) {
        self._resize_task.abort();
    }
}
