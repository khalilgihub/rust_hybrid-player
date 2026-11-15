use std::collections::HashSet;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hybrid_mpv_engine::{
    extract_video_dimensions, format_ytdl_option_value, resolve_libmpv_path, MpvEngine, MpvEvent,
    MpvStartupConfig, ScreenshotFormat, ScreenshotMode, SeekThumbnailer, OBSERVED_PROPERTIES,
};
use serde_json::json;
use tokio::sync::Notify;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/mpv-engine should sit two levels under workspace root")
        .to_path_buf()
}

#[test]
fn startup_options_and_observed_properties_match_spec() {
    assert_eq!(OBSERVED_PROPERTIES.len(), 21);
    let unique: HashSet<&str> = OBSERVED_PROPERTIES.iter().copied().collect();
    assert_eq!(unique.len(), 21);

    let cfg = MpvStartupConfig {
        ytdl_path: Some(PathBuf::from("C:\\tools\\yt-dlp.exe")),
        enable_ytdl_raw_options: true,
        ytdl_extractor_args: "youtube:player_client=web,tv;formats=missing_pot".to_owned(),
        ..Default::default()
    };
    let opts = cfg.build_option_pairs();
    let find_opt = |k: &str| -> Option<String> {
        opts.iter()
            .find(|(key, _)| key == k)
            .map(|(_, val)| val.clone())
    };

    assert_eq!(find_opt("config").as_deref(), Some("no"));
    assert_eq!(find_opt("vo").as_deref(), Some("libmpv"));
    assert_eq!(find_opt("hwdec").as_deref(), Some("d3d11va-zero-copy"));
    assert_eq!(
        find_opt("hwdec-codecs").as_deref(),
        Some("h264,h265,vp9,av1")
    );
    assert_eq!(find_opt("keepaspect-window").as_deref(), Some("no"));
    assert_eq!(find_opt("auto-window-resize").as_deref(), Some("no"));
    assert_eq!(find_opt("osc").as_deref(), Some("no"));
    assert_eq!(find_opt("osd-bar").as_deref(), Some("no"));
    assert_eq!(find_opt("osd-level").as_deref(), Some("0"));
    assert_eq!(find_opt("idle").as_deref(), Some("yes"));
    assert_eq!(find_opt("keep-open").as_deref(), Some("yes"));
    assert_eq!(find_opt("msg-level").as_deref(), Some("all=warn"));
    assert_eq!(find_opt("ytdl").as_deref(), Some("yes"));
    assert_eq!(find_opt("sub-auto").as_deref(), Some("fuzzy"));
    assert_eq!(
        find_opt("sub-file-paths").as_deref(),
        Some("subs:subtitles")
    );
    assert_eq!(find_opt("sub-fix-timing").as_deref(), Some("no"));
    assert_eq!(
        find_opt("demuxer-mkv-subtitle-preroll").as_deref(),
        Some("yes")
    );
    assert_eq!(find_opt("sub-ass-use-video-data").as_deref(), Some("all"));
    assert_eq!(find_opt("sub-ass-override").as_deref(), Some("scale"));
    assert_eq!(find_opt("screenshot-format").as_deref(), Some("jpg"));
    assert_eq!(find_opt("screenshot-jpeg-quality").as_deref(), Some("92"));
    assert_eq!(find_opt("cache").as_deref(), Some("yes"));
    assert_eq!(find_opt("demuxer-max-bytes").as_deref(), Some("150MiB"));
    assert_eq!(find_opt("demuxer-max-back-bytes").as_deref(), Some("50MiB"));
    assert_eq!(find_opt("input-default-bindings").as_deref(), Some("no"));
    assert_eq!(find_opt("cursor-autohide").as_deref(), Some("no"));
    assert_eq!(find_opt("hr-seek").as_deref(), Some("yes"));
    assert_eq!(find_opt("volume-max").as_deref(), Some("200"));
    assert_eq!(find_opt("sub-codepage").as_deref(), Some("auto"));
    assert!(find_opt("user-agent").is_some());
    assert!(find_opt("script-opts").is_some());
    assert!(find_opt("ytdl-raw-options").is_some());
    assert!(find_opt("wid").is_none());
}

#[test]
fn ytdl_formatting_and_rotation_dimensions() {
    assert_eq!(format_ytdl_option_value("simple"), "simple");
    assert_eq!(format_ytdl_option_value("foo=bar,baz"), "%11%foo=bar,baz");

    let normal = json!({ "w": 1920, "h": 1080, "rotate": 0 });
    assert_eq!(extract_video_dimensions(&normal), Some((1920, 1080)));

    let rotated = json!({ "dw": 1920, "dh": 1080, "rotate": 90 });
    assert_eq!(extract_video_dimensions(&rotated), Some((1080, 1920)));
}

#[test]
fn thumbnailer_quantization_sha1_and_local_guard() {
    assert_eq!(SeekThumbnailer::quantize_time(-3.0), 0.0);
    assert_eq!(SeekThumbnailer::quantize_time(1.24), 1.0);
    assert_eq!(SeekThumbnailer::quantize_time(1.26), 1.5);
    assert_eq!(SeekThumbnailer::quantize_time(2.0), 2.0);

    let (key0, sha0) = SeekThumbnailer::cache_key_and_sha1("C:\\vid\\a.mp4", 2.0);
    assert_eq!(key0, "C:\\vid\\a.mp4|2");
    assert_eq!(sha0.len(), 40);

    let (key1, _) = SeekThumbnailer::cache_key_and_sha1("C:\\vid\\a.mp4", 2.5);
    assert_eq!(key1, "C:\\vid\\a.mp4|2.5");

    assert!(!SeekThumbnailer::is_valid_local_media(
        "https://example.com/v.mp4"
    ));
    assert!(!SeekThumbnailer::is_valid_local_media(""));
    let sample = workspace_root()
        .join("hybrid-player")
        .join("test-sample.mp4");
    assert!(
        SeekThumbnailer::is_valid_local_media(sample.to_string_lossy().as_ref()),
        "expected test-sample.mp4 to exist at {}",
        sample.display()
    );
}

async fn readback_rgba_pixels(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    bridge: &hybrid_mpv_engine::DxBridge,
    frame: &hybrid_mpv_engine::ImportedVideoFrame,
) -> Vec<u8> {
    let bytes_per_pixel = 4u32;
    let unpadded_row = frame.width * bytes_per_pixel;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded_row = unpadded_row.div_ceil(align) * align;
    let buf_size = (padded_row * frame.height) as u64;

    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("spike_readback_buf"),
        size: buf_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("spike_readback_enc"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &frame.texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_row),
                rows_per_image: Some(frame.height),
            },
        },
        wgpu::Extent3d {
            width: frame.width,
            height: frame.height,
            depth_or_array_layers: 1,
        },
    );

    bridge.stage_wgpu_done_signal(frame);
    queue.submit(Some(encoder.finish()));
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("wgpu device poll failed");

    let slice = staging.slice(..);
    let (tx, rx) = tokio::sync::oneshot::channel();
    slice.map_async(wgpu::MapMode::Read, move |res| {
        let _ = tx.send(res);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("wgpu map poll failed");
    rx.await
        .expect("map callback dropped")
        .expect("staging buffer map failed");

    let mapped = slice.get_mapped_range().expect("get_mapped_range failed");
    let mut pixels = Vec::with_capacity((unpadded_row * frame.height) as usize);
    for row in 0..frame.height as usize {
        let start = row * padded_row as usize;
        pixels.extend_from_slice(&mapped[start..start + unpadded_row as usize]);
    }
    drop(mapped);
    staging.unmap();
    pixels
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spike_go_no_go_dx12_wgl_interop_render_resize_and_thumb() {
    let root = workspace_root();
    let dll_path = root.join("hybrid-player").join("mpv").join("libmpv-2.dll");
    let sample_path = root.join("hybrid-player").join("test-sample.mp4");
    assert!(dll_path.is_file(), "missing {}", dll_path.display());
    assert!(sample_path.is_file(), "missing {}", sample_path.display());
    assert!(resolve_libmpv_path(Some(&dll_path)).is_some());

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let temp_dir =
        std::env::temp_dir().join(format!("hybrid_mpv_spike_{}_{}", std::process::id(), nonce));
    let screenshot_dir = temp_dir.join("screenshots");
    fs::create_dir_all(&screenshot_dir).expect("create temp screenshot dir");

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        })
        .await
        .expect("no DX12 wgpu adapter available");
    let info = adapter.get_info();
    assert_eq!(info.backend, wgpu::Backend::Dx12);

    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("spike_dx12_device"),
            ..Default::default()
        })
        .await
        .expect("request_device failed");
    let device = Arc::new(device);
    let queue = Arc::new(queue);

    let frame_notify = Arc::new(Notify::new());
    let frame_count = Arc::new(AtomicU64::new(0));
    let notify_cb = frame_notify.clone();
    let count_cb = frame_count.clone();
    let cb: hybrid_mpv_engine::FrameReadyCallback = Arc::new(move |_frame| {
        count_cb.fetch_add(1, Ordering::Relaxed);
        notify_cb.notify_one();
    });

    let cfg = MpvStartupConfig {
        libmpv_path: Some(dll_path),
        screenshot_dir: screenshot_dir.clone(),
        screenshot_format: ScreenshotFormat::Jpg,
        auto_organize_screenshots: false,
        mute_audio_output: true,
        ..Default::default()
    };

    let engine = MpvEngine::new(
        cfg,
        device.clone(),
        queue.clone(),
        &adapter,
        640,
        360,
        false,
        Some(cb),
    )
    .expect("MpvEngine::new failed");

    let mut ev_rx = engine.client.subscribe();
    let sample_str = sample_path.to_string_lossy().into_owned();
    engine
        .client
        .load_file(&sample_str)
        .await
        .expect("load_file failed");

    let mut saw_video_params = false;
    let mut saw_duration = false;
    let mut saw_time_pos = false;
    let mut submitted_frame = None;

    let deadline = Instant::now() + Duration::from_secs(12);
    while Instant::now() < deadline {
        while let Ok(ev) = ev_rx.try_recv() {
            if let MpvEvent::PropertyChange { name, value, .. } = ev {
                match name.as_str() {
                    "video-params" if value.is_object() => saw_video_params = true,
                    "duration" if value.as_f64().unwrap_or(0.0) > 0.0 => saw_duration = true,
                    "time-pos" if value.as_f64().is_some() => saw_time_pos = true,
                    _ => {}
                }
            }
        }

        if let Some((frame, is_new)) = engine.bridge.acquire_frame_for_submit() {
            if is_new && frame.width > 0 && frame.height > 0 {
                let pixels = readback_rgba_pixels(&device, &queue, &engine.bridge, &frame).await;
                let non_black_rgb = pixels
                    .chunks_exact(4)
                    .any(|px| px[0] > 4 || px[1] > 4 || px[2] > 4);
                let any_alpha = pixels.chunks_exact(4).any(|px| px[3] > 0);
                if non_black_rgb && any_alpha {
                    submitted_frame = Some(frame);
                    if saw_video_params && saw_duration && saw_time_pos {
                        break;
                    }
                }
            }
        }

        let _ = tokio::time::timeout(Duration::from_millis(100), frame_notify.notified()).await;
    }

    assert!(saw_video_params, "did not observe video-params");
    assert!(saw_duration, "did not observe duration > 0");
    assert!(saw_time_pos, "did not observe time-pos");
    let first_frame = submitted_frame.expect("did not receive non-black RGBA frame from DxBridge");
    println!(
        "[spike] initial render OK: {}x{} (frames callback count={})",
        first_frame.width,
        first_frame.height,
        frame_count.load(Ordering::Relaxed)
    );

    // Test pause + retained frame across paused state
    engine
        .client
        .set_property_bool("pause", true)
        .expect("pause=true failed");
    // Drain any already-queued new frame, then verify retained_frame returns is_new=false
    if let Some((f, _)) = engine.bridge.acquire_frame_for_submit() {
        engine.bridge.stage_wgpu_done_signal(&f);
    }
    let retained = engine
        .bridge
        .acquire_frame_for_submit()
        .expect("retained frame missing while paused");
    assert!(!retained.1, "expected retained fallback (is_new=false)");

    // Test mid-playback resize stability
    engine.bridge.resize(800, 450);
    engine
        .client
        .set_property_bool("pause", false)
        .expect("pause=false failed");
    let mut resized_ok = false;
    let resize_deadline = Instant::now() + Duration::from_secs(6);
    while Instant::now() < resize_deadline {
        if let Some((frame, is_new)) = engine.bridge.acquire_frame_for_submit() {
            if is_new && frame.width == 800 && frame.height == 450 {
                let pixels = readback_rgba_pixels(&device, &queue, &engine.bridge, &frame).await;
                let non_black_rgb = pixels
                    .chunks_exact(4)
                    .any(|px| px[0] > 4 || px[1] > 4 || px[2] > 4);
                if non_black_rgb {
                    resized_ok = true;
                    break;
                }
            }
        }
        let _ = tokio::time::timeout(Duration::from_millis(100), frame_notify.notified()).await;
    }
    assert!(
        resized_ok,
        "did not observe non-black 800x450 resized frame"
    );

    // Test headless SeekThumbnailer
    let thumb = engine
        .thumbnailer
        .capture_thumbnail(&sample_str, 0.76)
        .await
        .expect("SeekThumbnailer failed to extract JPEG thumbnail");
    assert_eq!(thumb.time, 1.0);
    assert!(
        thumb.jpeg_bytes.len() > 100 && thumb.jpeg_bytes[0] == 0xFF && thumb.jpeg_bytes[1] == 0xD8,
        "thumbnail is not a valid JPEG"
    );
    let thumb_cached = engine
        .thumbnailer
        .capture_thumbnail(&sample_str, 0.80)
        .await
        .expect("SeekThumbnailer cache hit failed");
    assert_eq!(thumb_cached.jpeg_bytes.len(), thumb.jpeg_bytes.len());

    // Test auto_organize_screenshots subfolder resolution (`hybrid_core::media_scanner::resolve_screenshot_subfolder`)
    assert_eq!(engine.client.effective_screenshot_dir(), screenshot_dir);
    engine.client.set_auto_organize_screenshots(true);
    let organized_dir = engine.client.effective_screenshot_dir();
    assert_ne!(
        organized_dir, screenshot_dir,
        "auto_organize_screenshots=true should resolve a subfolder under base screenshot_dir"
    );
    assert!(
        organized_dir.starts_with(&screenshot_dir) && organized_dir.is_dir(),
        "expected organized subfolder {} inside {} to exist",
        organized_dir.display(),
        screenshot_dir.display()
    );
    let expected_sub = hybrid_core::media_scanner::resolve_screenshot_subfolder(
        &sample_str,
        &engine
            .client
            .get_property_string("media-title")
            .ok()
            .flatten()
            .unwrap_or_default(),
    );
    assert_eq!(organized_dir, screenshot_dir.join(expected_sub));
    let single_shot = engine
        .client
        .screenshot(ScreenshotMode::Video, None)
        .await
        .expect("organized single screenshot failed");
    assert_eq!(single_shot.parent(), Some(organized_dir.as_path()));
    assert!(single_shot.is_file());
    engine.client.set_auto_organize_screenshots(false);

    // Test real burst screenshot capture + <20 flatten
    let session_id = "spiketest";
    for seq in 1..=2 {
        let info = engine
            .client
            .screenshot_burst_frame(session_id, seq, ScreenshotMode::Video)
            .await
            .expect("screenshot_burst_frame failed");
        assert_eq!(info.seq_num, seq);
    }
    let finalized = engine.client.finalize_burst_session(session_id, 2);
    assert!(!finalized.in_subfolder, "<20 burst frames must flatten");
    assert_eq!(finalized.count, 2);
    for entry in &finalized.files {
        assert!(entry.file_path.is_file());
        assert!(entry.file_name.starts_with("hybrid-burst-spiketest_"));
    }

    // Test >=20 burst keep-in-subfolder logic (synthetic files)
    let large_id = "large20";
    let large_dir = screenshot_dir.join(format!("burst_{large_id}"));
    fs::create_dir_all(&large_dir).unwrap();
    for i in 1..=20 {
        let mut f = File::create(large_dir.join(format!("frame-{i:05}.jpg"))).unwrap();
        f.write_all(&[0xFF, 0xD8, 0xFF, 0xD9]).unwrap();
    }
    let kept = engine.client.finalize_burst_session(large_id, 20);
    assert!(
        kept.in_subfolder,
        ">=20 burst frames must remain in subfolder"
    );
    assert_eq!(kept.count, 20);

    drop(engine);
    let _ = fs::remove_dir_all(&temp_dir);
}
