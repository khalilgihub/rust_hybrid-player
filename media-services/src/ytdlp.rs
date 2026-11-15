use once_cell_like::LazyRegex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use url::Url;

pub const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/132.0.0.0 Safari/537.36";
pub const YTDLP_EXTRACTOR_ARGS: &str = "";
pub const YT_DLP_TIMEOUT_MS: u64 = 20_000;
pub const YT_DLP_PLAYLIST_TIMEOUT_MS: u64 = 45_000;
pub const YT_DLP_PLAYLIST_MAX_ENTRIES: usize = 300;
pub const YT_DLP_INFO_CACHE_TTL_MS: u64 = 30 * 60 * 1000;
const MAX_URL_LENGTH: usize = 4096;
const MAX_STDOUT_BYTES: usize = 16 * 1024 * 1024;

pub const YOUTUBE_HOSTS: [&str; 7] = [
    "youtube.com",
    "www.youtube.com",
    "m.youtube.com",
    "music.youtube.com",
    "youtu.be",
    "youtube-nocookie.com",
    "www.youtube-nocookie.com",
];

mod once_cell_like {
    use regex::Regex;
    use std::sync::OnceLock;

    pub struct LazyRegex(OnceLock<Regex>, &'static str);

    impl LazyRegex {
        pub const fn new(pattern: &'static str) -> Self {
            Self(OnceLock::new(), pattern)
        }

        pub fn get(&self) -> &Regex {
            self.0
                .get_or_init(|| Regex::new(self.1).expect("valid static regex"))
        }
    }
}

#[derive(Debug, Error)]
pub enum YtDlpError {
    #[error("Invalid or non-YouTube URL")]
    InvalidUrl,
    #[error("Missing playlist ID in URL")]
    MissingPlaylistId,
    #[error("yt-dlp binary not found")]
    BinaryNotFound,
    #[error("Failed to execute yt-dlp: {0}")]
    SpawnFailed(String),
    #[error("yt-dlp timed out after {0}ms")]
    Timeout(u64),
    #[error("yt-dlp returned invalid JSON: {0}")]
    InvalidJson(String),
    #[error("yt-dlp failed: {0}")]
    CommandFailed(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct YoutubeVideoInfo {
    pub video_id: Option<String>,
    pub heights: Vec<u32>,
    pub title: Option<String>,
    pub uploader: Option<String>,
    pub duration: Option<f64>,
    pub is_live: bool,
    pub live_status: Option<String>,
    pub thumbnail: Option<String>,
    pub webpage_url: String,
    pub error: Option<String>,
    pub code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct YoutubePlaylistEntry {
    pub id: String,
    pub url: String,
    pub title: Option<String>,
    pub duration: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct YoutubePlaylistInfo {
    pub playlist_id: String,
    pub title: Option<String>,
    pub entries: Vec<YoutubePlaylistEntry>,
    pub has_more: bool,
    pub total_estimated: usize,
}

pub fn is_valid_youtube_id(id: &str) -> bool {
    static RE_ID: LazyRegex = LazyRegex::new(r"^[a-zA-Z0-9_-]{11}$");
    RE_ID.get().is_match(id)
}

pub fn normalize_youtube_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_URL_LENGTH {
        return None;
    }
    let parsed = Url::parse(trimmed).ok()?;
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let host = parsed.host_str()?.to_ascii_lowercase();
    if !YOUTUBE_HOSTS.contains(&host.as_str()) {
        return None;
    }
    Some(parsed.into())
}

pub fn extract_youtube_id(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if is_valid_youtube_id(trimmed) {
        return Some(trimmed.to_string());
    }
    let normalized = normalize_youtube_url(trimmed)?;
    let parsed = Url::parse(&normalized).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();

    if host == "youtu.be" {
        let seg = parsed.path_segments()?.next()?;
        if is_valid_youtube_id(seg) {
            return Some(seg.to_string());
        }
    }

    for (k, v) in parsed.query_pairs() {
        if k == "v" && is_valid_youtube_id(&v) {
            return Some(v.into_owned());
        }
    }

    let segments: Vec<&str> = parsed.path_segments()?.filter(|s| !s.is_empty()).collect();
    if segments.len() >= 2
        && matches!(segments[0], "shorts" | "embed" | "live" | "v")
        && is_valid_youtube_id(segments[1])
    {
        return Some(segments[1].to_string());
    }
    None
}

pub fn extract_youtube_playlist_id(raw: &str) -> Option<String> {
    static RE_LIST: LazyRegex = LazyRegex::new(r"^[a-zA-Z0-9_-]{4,128}$");
    let normalized = normalize_youtube_url(raw)?;
    let parsed = Url::parse(&normalized).ok()?;
    for (k, v) in parsed.query_pairs() {
        if k == "list" && RE_LIST.get().is_match(&v) {
            return Some(v.into_owned());
        }
    }
    None
}

pub fn extractor_arg_pair() -> Vec<String> {
    if YTDLP_EXTRACTOR_ARGS.is_empty() {
        Vec::new()
    } else {
        vec![
            "--extractor-args".to_string(),
            YTDLP_EXTRACTOR_ARGS.to_string(),
        ]
    }
}

pub fn playlist_items_range_for_url(target_url: &str) -> String {
    if let Ok(parsed) = Url::parse(target_url) {
        for (k, v) in parsed.query_pairs() {
            if k == "index" {
                if let Ok(idx) = v.parse::<usize>() {
                    if idx > 0 {
                        let half_window = YT_DLP_PLAYLIST_MAX_ENTRIES / 2;
                        let start = idx.saturating_sub(half_window).max(1);
                        let end = start + YT_DLP_PLAYLIST_MAX_ENTRIES - 1;
                        return format!("{start}-{end}");
                    }
                }
            }
        }
    }
    format!("1-{YT_DLP_PLAYLIST_MAX_ENTRIES}")
}

pub fn build_ytdlp_video_info_args(target_url: &str, cookies_path: Option<&Path>) -> Vec<String> {
    let mut args = vec![
        "-J".to_string(),
        "--no-warnings".to_string(),
        "--no-playlist".to_string(),
        "--user-agent".to_string(),
        DEFAULT_USER_AGENT.to_string(),
    ];
    args.extend(extractor_arg_pair());
    if let Some(cookies) = cookies_path {
        if cookies.is_file() {
            args.push("--cookies".to_string());
            args.push(cookies.to_string_lossy().into_owned());
        }
    }
    args.push("--".to_string());
    args.push(target_url.to_string());
    args
}

pub fn build_ytdlp_playlist_args(
    playlist_id: &str,
    items_range: &str,
    cookies_path: Option<&Path>,
) -> Vec<String> {
    let mut args = vec![
        "-J".to_string(),
        "--no-warnings".to_string(),
        "--flat-playlist".to_string(),
        "--yes-playlist".to_string(),
        "--playlist-items".to_string(),
        items_range.to_string(),
        "--user-agent".to_string(),
        DEFAULT_USER_AGENT.to_string(),
    ];
    args.extend(extractor_arg_pair());
    if let Some(cookies) = cookies_path {
        if cookies.is_file() {
            args.push("--cookies".to_string());
            args.push(cookies.to_string_lossy().into_owned());
        }
    }
    let encoded_list: String =
        url::form_urlencoded::byte_serialize(playlist_id.as_bytes()).collect();
    args.push("--".to_string());
    args.push(format!(
        "https://www.youtube.com/playlist?list={encoded_list}"
    ));
    args
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct YtDlpFormatPayload {
    #[serde(default)]
    pub vcodec: Option<String>,
    #[serde(default)]
    pub format_note: Option<String>,
    #[serde(default)]
    pub width: Option<f64>,
    #[serde(default)]
    pub height: Option<f64>,
}

pub fn map_format_to_standard_height(format: &YtDlpFormatPayload) -> Option<u32> {
    static RE_NOTE_HEIGHT: LazyRegex = LazyRegex::new(r"^(\d{3,4})p");
    if format.vcodec.as_deref() == Some("none") {
        return None;
    }
    let note = format
        .format_note
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    if let Some(caps) = RE_NOTE_HEIGHT.get().captures(&note) {
        if let Ok(parsed) = caps[1].parse::<u32>() {
            if parsed > 0 {
                return Some(parsed);
            }
        }
    }
    if note.contains("4k") || note.contains("2160p") {
        return Some(2160);
    }
    if note.contains("8k") || note.contains("4320p") {
        return Some(4320);
    }

    let h = format
        .height
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(0.0);
    let w = format
        .width
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(0.0);
    let long_side = h.max(w);
    let short_side = if h > 0.0 && w > 0.0 {
        h.min(w)
    } else {
        h.max(w)
    };
    if short_side <= 0.0 {
        return None;
    }

    if short_side >= 3200.0 || long_side >= 6000.0 {
        Some(4320)
    } else if short_side >= 1800.0 || long_side >= 3200.0 {
        Some(2160)
    } else if short_side >= 1200.0 || long_side >= 2200.0 {
        Some(1440)
    } else if short_side >= 900.0 || long_side >= 1600.0 {
        Some(1080)
    } else if short_side >= 576.0 || long_side >= 1000.0 {
        Some(720)
    } else if short_side >= 400.0 || long_side >= 700.0 {
        Some(480)
    } else if short_side >= 300.0 || long_side >= 500.0 {
        Some(360)
    } else if short_side >= 180.0 || long_side >= 300.0 {
        Some(240)
    } else {
        Some(144)
    }
}

#[derive(Debug)]
struct CachedInfoEntry {
    info: YoutubeVideoInfo,
    fetched_at: Instant,
}

#[derive(Debug)]
pub struct YoutubeInfoCache {
    ttl: Duration,
    entries: Mutex<HashMap<String, CachedInfoEntry>>,
}

impl Default for YoutubeInfoCache {
    fn default() -> Self {
        Self::new(Duration::from_millis(YT_DLP_INFO_CACHE_TTL_MS))
    }
}

impl YoutubeInfoCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub fn get(&self, video_id: &str) -> Option<YoutubeVideoInfo> {
        self.get_at(video_id, Instant::now())
    }

    pub fn get_at(&self, video_id: &str, now: Instant) -> Option<YoutubeVideoInfo> {
        if video_id.is_empty() {
            return None;
        }
        let mut guard = self.entries.lock().ok()?;
        let entry = guard.get(video_id)?;
        if now.duration_since(entry.fetched_at) > self.ttl {
            guard.remove(video_id);
            return None;
        }
        Some(entry.info.clone())
    }

    pub fn insert(&self, video_id: impl Into<String>, info: YoutubeVideoInfo) {
        self.insert_at(video_id, info, Instant::now());
    }

    pub fn insert_at(&self, video_id: impl Into<String>, info: YoutubeVideoInfo, now: Instant) {
        let key = video_id.into();
        if key.is_empty() || info.error.is_some() {
            return;
        }
        if let Ok(mut guard) = self.entries.lock() {
            guard.insert(
                key,
                CachedInfoEntry {
                    info,
                    fetched_at: now,
                },
            );
        }
    }

    pub fn clear(&self) {
        if let Ok(mut guard) = self.entries.lock() {
            guard.clear();
        }
    }
}

pub fn parse_ytdlp_video_payload(
    payload: &serde_json::Value,
    target_url: &str,
    video_id: Option<&str>,
) -> YoutubeVideoInfo {
    let Some(formats) = payload.get("formats").and_then(|v| v.as_array()) else {
        return YoutubeVideoInfo {
            video_id: video_id.map(str::to_string),
            heights: Vec::new(),
            title: None,
            uploader: None,
            duration: None,
            is_live: false,
            live_status: None,
            thumbnail: None,
            webpage_url: target_url.to_string(),
            error: Some("Video unavailable or sign-in required".to_string()),
            code: Some("YT_EXTRACT_FAILED".to_string()),
        };
    };

    let mut heights = BTreeSet::new();
    for raw_fmt in formats {
        if let Ok(fmt) = serde_json::from_value::<YtDlpFormatPayload>(raw_fmt.clone()) {
            if let Some(h) = map_format_to_standard_height(&fmt) {
                heights.insert(h);
            }
        }
    }
    let sorted_heights: Vec<u32> = heights.into_iter().rev().collect();

    let resolved_id = video_id.map(str::to_string).or_else(|| {
        payload
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
    });

    let uploader = payload
        .get("uploader")
        .and_then(|v| v.as_str())
        .or_else(|| payload.get("channel").and_then(|v| v.as_str()))
        .map(str::to_string);

    YoutubeVideoInfo {
        video_id: resolved_id,
        heights: sorted_heights,
        title: payload
            .get("title")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        uploader,
        duration: payload
            .get("duration")
            .and_then(|v| v.as_f64())
            .filter(|d| d.is_finite() && *d > 0.0),
        is_live: payload.get("is_live").and_then(|v| v.as_bool()) == Some(true),
        live_status: payload
            .get("live_status")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        thumbnail: payload
            .get("thumbnail")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        webpage_url: target_url.to_string(),
        error: None,
        code: None,
    }
}

pub fn parse_ytdlp_playlist_payload(
    payload: &serde_json::Value,
    playlist_id: &str,
) -> Option<YoutubePlaylistInfo> {
    let raw_entries = payload.get("entries")?.as_array()?;
    let total_estimated = payload
        .get("playlist_count")
        .and_then(|v| v.as_u64())
        .or_else(|| payload.get("n_entries").and_then(|v| v.as_u64()))
        .map(|n| n as usize)
        .unwrap_or(raw_entries.len());
    let has_more = total_estimated > YT_DLP_PLAYLIST_MAX_ENTRIES
        || raw_entries.len() >= YT_DLP_PLAYLIST_MAX_ENTRIES;

    let mut entries = Vec::new();
    for raw in raw_entries {
        let id = raw
            .get("id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| {
                raw.get("url")
                    .or_else(|| raw.get("webpage_url"))
                    .and_then(|v| v.as_str())
                    .and_then(extract_youtube_id)
            });
        let Some(id) = id else {
            continue;
        };
        entries.push(YoutubePlaylistEntry {
            url: format!("https://www.youtube.com/watch?v={id}"),
            id,
            title: raw
                .get("title")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            duration: raw
                .get("duration")
                .and_then(|v| v.as_f64())
                .filter(|d| d.is_finite() && *d > 0.0),
        });
        if entries.len() >= YT_DLP_PLAYLIST_MAX_ENTRIES {
            break;
        }
    }

    Some(YoutubePlaylistInfo {
        playlist_id: playlist_id.to_string(),
        title: payload
            .get("title")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        entries,
        has_more,
        total_estimated,
    })
}

pub async fn run_ytdlp_json(
    bin: &Path,
    args: &[String],
    timeout_ms: u64,
) -> Result<serde_json::Value, YtDlpError> {
    if !bin.is_file() {
        return Err(YtDlpError::BinaryNotFound);
    }

    let mut cmd = Command::new(bin);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| YtDlpError::SpawnFailed(e.to_string()))?;

    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();

    let stdout_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        if let Some(ref mut pipe) = stdout_pipe {
            let mut chunk = [0u8; 8192];
            loop {
                match pipe.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let rem = MAX_STDOUT_BYTES.saturating_sub(buf.len());
                        if rem > 0 {
                            buf.extend_from_slice(&chunk[..n.min(rem)]);
                        }
                    }
                }
            }
        }
        buf
    });

    let stderr_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        if let Some(ref mut pipe) = stderr_pipe {
            let mut chunk = [0u8; 4096];
            loop {
                match pipe.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let rem = 16_384_usize.saturating_sub(buf.len());
                        if rem > 0 {
                            buf.extend_from_slice(&chunk[..n.min(rem)]);
                        }
                    }
                }
            }
        }
        String::from_utf8_lossy(&buf).into_owned()
    });

    let wait_res = tokio::time::timeout(Duration::from_millis(timeout_ms), child.wait()).await;
    match wait_res {
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            Err(YtDlpError::Timeout(timeout_ms))
        }
        Ok(Err(e)) => Err(YtDlpError::CommandFailed(e.to_string())),
        Ok(Ok(status)) => {
            let stdout_bytes = stdout_task.await.unwrap_or_default();
            let stderr_msg = stderr_task.await.unwrap_or_default();
            if !status.success() {
                return Err(YtDlpError::CommandFailed(if stderr_msg.trim().is_empty() {
                    format!("exited with code {:?}", status.code())
                } else {
                    stderr_msg.trim().to_string()
                }));
            }
            serde_json::from_slice(&stdout_bytes)
                .map_err(|e| YtDlpError::InvalidJson(e.to_string()))
        }
    }
}

pub async fn get_youtube_video_info(
    ytdlp_bin: &Path,
    cookies_path: Option<&Path>,
    url: &str,
    cache: Option<&YoutubeInfoCache>,
) -> Result<YoutubeVideoInfo, YtDlpError> {
    let target = normalize_youtube_url(url).ok_or(YtDlpError::InvalidUrl)?;
    let video_id = extract_youtube_id(&target);

    if let (Some(c), Some(vid)) = (cache, video_id.as_deref()) {
        if let Some(hit) = c.get(vid) {
            return Ok(hit);
        }
    }

    let args = build_ytdlp_video_info_args(&target, cookies_path);
    match run_ytdlp_json(ytdlp_bin, &args, YT_DLP_TIMEOUT_MS).await {
        Ok(payload) => {
            let info = parse_ytdlp_video_payload(&payload, &target, video_id.as_deref());
            if info.error.is_none() {
                if let (Some(c), Some(vid)) = (cache, info.video_id.as_deref()) {
                    c.insert(vid, info.clone());
                }
            }
            Ok(info)
        }
        Err(_) => {
            // Failures are never cached.
            Ok(YoutubeVideoInfo {
                video_id,
                heights: Vec::new(),
                title: None,
                uploader: None,
                duration: None,
                is_live: false,
                live_status: None,
                thumbnail: None,
                webpage_url: target,
                error: Some("Video unavailable or sign-in required".to_string()),
                code: Some("YT_EXTRACT_FAILED".to_string()),
            })
        }
    }
}

pub async fn get_youtube_playlist(
    ytdlp_bin: &Path,
    cookies_path: Option<&Path>,
    url: &str,
) -> Result<YoutubePlaylistInfo, YtDlpError> {
    let target = normalize_youtube_url(url).ok_or(YtDlpError::InvalidUrl)?;
    let playlist_id = extract_youtube_playlist_id(&target).ok_or(YtDlpError::MissingPlaylistId)?;
    let range = playlist_items_range_for_url(&target);
    let args = build_ytdlp_playlist_args(&playlist_id, &range, cookies_path);
    let payload = run_ytdlp_json(ytdlp_bin, &args, YT_DLP_PLAYLIST_TIMEOUT_MS).await?;
    parse_ytdlp_playlist_payload(&payload, &playlist_id)
        .ok_or_else(|| YtDlpError::InvalidJson("no entries in playlist payload".to_string()))
}

pub fn normalize_cookies_path_candidate(p: Option<PathBuf>) -> Option<PathBuf> {
    p.filter(|path| path.is_file())
}
