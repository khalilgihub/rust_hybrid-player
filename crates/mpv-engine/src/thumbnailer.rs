//! Headless secondary `libmpv` seekbar thumbnailer.
//!
//! Mirrors `hybrid-player/src/main/mpv-ipc-bridge.js:78-257,1333-1413` and
//! `thumbnails.js:64`:
//! - Headless secondary `libmpv` (`force-window=no, mute=yes, pause=yes, audio=no, vo=null, hwdec=no`)
//! - `0.5s` quantization (`round(t * 2.0) / 2.0`)
//! - SHA-1 `mediaPath|rounded` cache (max 180 entries, FIFO eviction)
//! - 1 in-flight + 1 pending (latest request supersedes previous pending)
//! - `5000ms` timeout, local-files-only guard.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use sha1::{Digest, Sha1};
use tokio::sync::{oneshot, Mutex};

use crate::client::{MpvClient, MpvEvent};
use crate::error::Result;
use crate::ffi::MpvApi;

pub const THUMB_CACHE_MAX_ENTRIES: usize = 180;
pub const THUMB_CAPTURE_TIMEOUT: Duration = Duration::from_millis(5000);

#[derive(Debug, Clone)]
pub struct ThumbnailPayload {
    pub time: f64,
    pub jpeg_bytes: Vec<u8>,
    pub file_path: PathBuf,
}

struct PendingRequest {
    media_path: String,
    rounded_time: f64,
    cache_key: String,
    tx: oneshot::Sender<Option<ThumbnailPayload>>,
}

struct ThumbnailCache {
    entries: HashMap<String, (ThumbnailPayload, PathBuf)>,
    order: VecDeque<String>,
}

impl ThumbnailCache {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn get(&self, key: &str) -> Option<ThumbnailPayload> {
        self.entries.get(key).map(|(p, _)| p.clone())
    }

    fn insert(&mut self, key: String, payload: ThumbnailPayload, file_path: PathBuf) {
        if !self.entries.contains_key(&key) {
            self.order.push_back(key.clone());
        }
        self.entries.insert(key, (payload, file_path));
        while self.entries.len() > THUMB_CACHE_MAX_ENTRIES {
            if let Some(oldest) = self.order.pop_front() {
                if let Some((_, stale_path)) = self.entries.remove(&oldest) {
                    let _ = fs::remove_file(stale_path);
                }
            }
        }
    }

    fn clear_and_unlink(&mut self) {
        for (_, (_, path)) in self.entries.drain() {
            let _ = fs::remove_file(path);
        }
        self.order.clear();
    }
}

struct QueueState {
    in_flight: bool,
    next_pending: Option<PendingRequest>,
}

struct WorkerInner {
    api: Arc<MpvApi>,
    preview_dir: PathBuf,
    client: Mutex<Option<Arc<MpvClient>>>,
    loaded_path: Mutex<Option<String>>,
    cache: Mutex<ThumbnailCache>,
    queue: Mutex<QueueState>,
}

/// Secondary headless `libmpv` seekbar thumbnail extractor.
pub struct SeekThumbnailer {
    inner: Arc<WorkerInner>,
}

impl SeekThumbnailer {
    pub fn new(api: Arc<MpvApi>) -> Result<Self> {
        let preview_dir =
            std::env::temp_dir().join(format!("hybrid-player-thumbs-{}", std::process::id()));
        fs::create_dir_all(&preview_dir)?;

        Ok(Self {
            inner: Arc::new(WorkerInner {
                api,
                preview_dir,
                client: Mutex::new(None),
                loaded_path: Mutex::new(None),
                cache: Mutex::new(ThumbnailCache::new()),
                queue: Mutex::new(QueueState {
                    in_flight: false,
                    next_pending: None,
                }),
            }),
        })
    }

    pub fn preview_dir(&self) -> &Path {
        &self.inner.preview_dir
    }

    pub async fn cache_len(&self) -> usize {
        self.inner.cache.lock().await.entries.len()
    }

    /// Quantizes `time` to the nearest `0.5s` step (`round(t * 2.0) / 2.0`).
    pub fn quantize_time(time: f64) -> f64 {
        let safe = if time.is_finite() && time > 0.0 {
            time
        } else {
            0.0
        };
        let r = (safe * 2.0).round() / 2.0;
        if r == 0.0 {
            0.0
        } else {
            r
        }
    }

    /// Formats `mediaPath|rounded` identically to JS `${mediaPath}|${rounded}` and returns SHA-1 hex.
    pub fn cache_key_and_sha1(media_path: &str, rounded: f64) -> (String, String) {
        let rounded_str = if (rounded - rounded.trunc()).abs() < 1e-9 {
            format!("{rounded:.0}")
        } else {
            format!("{rounded:.1}")
        };
        let cache_key = format!("{media_path}|{rounded_str}");
        let mut hasher = Sha1::new();
        hasher.update(cache_key.as_bytes());
        let digest = hasher.finalize();
        let mut hex = String::with_capacity(40);
        for b in digest {
            use std::fmt::Write as _;
            let _ = write!(&mut hex, "{b:02x}");
        }
        (cache_key, hex)
    }

    /// Local-files-only guard (`mpv-ipc-bridge.js:351-355,1342`).
    pub fn is_valid_local_media(media_path: &str) -> bool {
        if media_path.is_empty() || media_path.contains('\0') {
            return false;
        }
        let p = Path::new(media_path);
        let is_drive_or_unc = {
            let bytes = media_path.as_bytes();
            (bytes.len() >= 3
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && (bytes[2] == b'\\' || bytes[2] == b'/'))
                || media_path.starts_with("\\\\")
                || p.is_absolute()
        };
        is_drive_or_unc && p.is_file()
    }

    /// Clears the loaded media on the preview instance (called on main `stop` / `end-file`).
    pub async fn stop(&self) {
        *self.inner.loaded_path.lock().await = None;
        if let Some(client) = self.inner.client.lock().await.as_ref() {
            let _ = client.stop();
        }
    }

    /// Captures or retrieves a cached `0.5s`-quantized JPEG thumbnail for a local media file.
    ///
    /// Enforces 1 in-flight + 1 pending (latest supersedes prior pending) and `5000ms` timeout.
    pub async fn capture_thumbnail(&self, media_path: &str, time: f64) -> Option<ThumbnailPayload> {
        if !Self::is_valid_local_media(media_path) {
            return None;
        }

        let rounded = Self::quantize_time(time);
        let (cache_key, _sha1) = Self::cache_key_and_sha1(media_path, rounded);

        if let Some(hit) = self.inner.cache.lock().await.get(&cache_key) {
            return Some(hit);
        }

        let (tx, rx) = oneshot::channel();
        let req = PendingRequest {
            media_path: media_path.to_owned(),
            rounded_time: rounded,
            cache_key,
            tx,
        };

        let to_start = {
            let mut q = self.inner.queue.lock().await;
            if !q.in_flight {
                q.in_flight = true;
                Some(req)
            } else {
                if let Some(superseded) = q.next_pending.take() {
                    let _ = superseded.tx.send(None);
                }
                q.next_pending = Some(req);
                None
            }
        };

        if let Some(req) = to_start {
            let inner = self.inner.clone();
            tokio::spawn(async move {
                Self::process_loop(inner, req).await;
            });
        }

        match tokio::time::timeout(THUMB_CAPTURE_TIMEOUT, rx).await {
            Ok(Ok(payload)) => payload,
            _ => None,
        }
    }

    async fn process_loop(inner: Arc<WorkerInner>, first: PendingRequest) {
        let mut current = Some(first);
        while let Some(req) = current {
            let res = tokio::time::timeout(
                THUMB_CAPTURE_TIMEOUT,
                Self::execute_capture(&inner, &req.media_path, req.rounded_time, &req.cache_key),
            )
            .await
            .unwrap_or(None);

            let _ = req.tx.send(res);

            let mut q = inner.queue.lock().await;
            if let Some(next) = q.next_pending.take() {
                current = Some(next);
            } else {
                q.in_flight = false;
                current = None;
            }
        }
    }

    async fn ensure_client(inner: &WorkerInner) -> Option<Arc<MpvClient>> {
        let mut guard = inner.client.lock().await;
        if let Some(existing) = guard.as_ref() {
            return Some(existing.clone());
        }
        let client = MpvClient::new_headless(inner.api.clone(), &inner.preview_dir).ok()?;
        *guard = Some(client.clone());
        Some(client)
    }

    async fn execute_capture(
        inner: &WorkerInner,
        media_path: &str,
        rounded: f64,
        cache_key: &str,
    ) -> Option<ThumbnailPayload> {
        if let Some(hit) = inner.cache.lock().await.get(cache_key) {
            return Some(hit);
        }

        let client = Self::ensure_client(inner).await?;

        let needs_load = {
            let loaded = inner.loaded_path.lock().await;
            loaded.as_deref() != Some(media_path)
        };

        if needs_load {
            *inner.loaded_path.lock().await = None;
            let mut rx = client.subscribe();
            if client.load_file(media_path).await.is_err() {
                return None;
            }
            let _ = wait_for_event(&mut rx, Duration::from_millis(6000), |ev| {
                matches!(ev, MpvEvent::FileLoaded)
            })
            .await;
            let _ = client.pause();
            *inner.loaded_path.lock().await = Some(media_path.to_owned());
        }

        // Seek with `absolute` and wait up to 400ms for playback-restart (fallback 80ms)
        {
            let mut rx = client.subscribe();
            if client.seek_async(rounded, "absolute").await.is_err() {
                return None;
            }
            let restarted = wait_for_event(&mut rx, Duration::from_millis(400), |ev| {
                matches!(ev, MpvEvent::PlaybackRestart)
            })
            .await;
            if !restarted {
                tokio::time::sleep(Duration::from_millis(80)).await;
            }
        }

        let (_, sha1_hex) = Self::cache_key_and_sha1(media_path, rounded);
        let thumb_path = inner.preview_dir.join(format!("{sha1_hex}.jpg"));

        if thumb_path.exists()
            && fs::metadata(&thumb_path)
                .map(|m| m.len() == 0)
                .unwrap_or(true)
        {
            let _ = fs::remove_file(&thumb_path);
        }

        if !thumb_path.exists() {
            let path_str = thumb_path.to_string_lossy().into_owned();
            let _ = client
                .command_async(&["screenshot-to-file", &path_str, "video"])
                .await;
            for _ in 0..15 {
                if fs::metadata(&thumb_path)
                    .map(|m| m.len() > 0)
                    .unwrap_or(false)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }

        let len = fs::metadata(&thumb_path).map(|m| m.len()).unwrap_or(0);
        if len == 0 {
            return None;
        }

        let jpeg_bytes = fs::read(&thumb_path).ok()?;
        if jpeg_bytes.is_empty() {
            return None;
        }

        let payload = ThumbnailPayload {
            time: rounded,
            jpeg_bytes,
            file_path: thumb_path.clone(),
        };
        inner
            .cache
            .lock()
            .await
            .insert(cache_key.to_owned(), payload.clone(), thumb_path);

        Some(payload)
    }
}

impl Drop for SeekThumbnailer {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.inner.client.try_lock() {
            if let Some(c) = guard.take() {
                c.shutdown();
            }
        }
        if let Ok(mut cache) = self.inner.cache.try_lock() {
            cache.clear_and_unlink();
        }
        let _ = fs::remove_dir_all(&self.inner.preview_dir);
    }
}

async fn wait_for_event<F>(
    rx: &mut tokio::sync::broadcast::Receiver<MpvEvent>,
    timeout: Duration,
    pred: F,
) -> bool
where
    F: Fn(&MpvEvent) -> bool,
{
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return false;
        }
        match tokio::time::timeout(deadline - now, rx.recv()).await {
            Ok(Ok(ev)) => {
                if pred(&ev) {
                    return true;
                }
            }
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
            _ => return false,
        }
    }
}
