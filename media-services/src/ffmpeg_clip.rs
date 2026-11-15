use hybrid_core::media_scanner::{is_media_extension, is_unc_or_network_path};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

pub const CLIP_EXPORT_TIMEOUT_MS: u64 = 5 * 60 * 1000;
pub const CLIP_MAX_DURATION_SECONDS: f64 = 6.0 * 3600.0;
pub const CLIP_MAX_START_SECONDS: f64 = 30.0 * 86_400.0;
const MAX_STDERR_BYTES: usize = 16_384;

#[derive(Debug, Error)]
pub enum ClipError {
    #[error("Source media file not found or unsupported")]
    InvalidSource,
    #[error("Clip start time is invalid")]
    InvalidStartTime,
    #[error("Clip duration must be greater than zero")]
    InvalidDuration,
    #[error("Clip duration is too long")]
    DurationTooLong,
    #[error("ffmpeg binary not found")]
    FfmpegNotFound,
    #[error("Failed to start ffmpeg: {0}")]
    SpawnFailed(String),
    #[error("Clip export timed out")]
    Timeout,
    #[error("{0}")]
    FfmpegFailed(String),
}

#[derive(Debug, Clone)]
pub struct ClipRequest {
    pub file_path: String,
    pub start_time: f64,
    pub duration: f64,
    pub output_dir: Option<PathBuf>,
    pub ffmpeg_binary: Option<PathBuf>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClipResult {
    pub output_path: PathBuf,
    pub start_time: f64,
    pub duration: f64,
}

pub fn format_ffmpeg_timestamp(seconds: f64) -> String {
    let clamped = if seconds.is_finite() {
        seconds.max(0.0)
    } else {
        0.0
    };
    format!("{clamped:.3}")
}

/// Validates and resolves a local media file path, blocking UNC/SMB and `file://` edge cases.
pub fn resolve_existing_local_file(file_path: &str) -> Option<PathBuf> {
    let mut source = file_path.trim().to_string();
    if source.is_empty() || source.len() > 4096 || source.contains('\0') {
        return None;
    }
    if source.to_ascii_lowercase().starts_with("file://") {
        if let Ok(parsed) = url::Url::parse(&source) {
            if let Ok(p) = parsed.to_file_path() {
                source = p.to_string_lossy().into_owned();
            } else {
                return None;
            }
        } else {
            return None;
        }
    }
    if is_unc_or_network_path(&source) {
        return None;
    }
    let path = PathBuf::from(&source);
    let resolved = fs::canonicalize(&path).unwrap_or(path);
    let meta = fs::metadata(&resolved).ok()?;
    if !meta.is_file() || !is_media_extension(&resolved) {
        return None;
    }
    Some(resolved)
}

pub fn create_clip_output_path(source_path: &Path, output_dir: Option<&Path>) -> PathBuf {
    let stem = source_path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("clip");
    let ext = source_path
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| !e.is_empty())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();

    let stamp = format_utc_stamp();
    let dest_dir = output_dir
        .or_else(|| source_path.parent())
        .unwrap_or_else(|| Path::new("."));
    let _ = fs::create_dir_all(dest_dir);

    let mut candidate = dest_dir.join(format!("{stem}-clip-{stamp}{ext}"));
    let mut counter = 1u32;
    while candidate.exists() {
        counter += 1;
        candidate = dest_dir.join(format!("{stem}-clip-{stamp}-{counter}{ext}"));
    }
    candidate
}

fn format_utc_stamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let hour = rem / 3_600;
    let min = (rem % 3_600) / 60;
    let sec = rem % 60;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}{month:02}{day:02}-{hour:02}{min:02}{sec:02}")
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = y + if m <= 2 { 1 } else { 0 };
    (y, m as u32, d as u32)
}

/// Builds the exact lossless FFmpeg argument vector matching `hybrid-player/src/main/main.js:349-363`.
pub fn build_ffmpeg_clip_args(
    target_path: &Path,
    output_path: &Path,
    start_time: f64,
    duration: f64,
) -> Vec<String> {
    vec![
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
        "-protocol_whitelist".to_string(),
        "file,crypto,data".to_string(),
        "-ss".to_string(),
        format_ffmpeg_timestamp(start_time),
        "-i".to_string(),
        target_path.to_string_lossy().into_owned(),
        "-t".to_string(),
        format_ffmpeg_timestamp(duration),
        "-map".to_string(),
        "0:v:0?".to_string(),
        "-map".to_string(),
        "0:a?".to_string(),
        "-sn".to_string(),
        "-dn".to_string(),
        "-avoid_negative_ts".to_string(),
        "make_zero".to_string(),
        "-c".to_string(),
        "copy".to_string(),
        output_path.to_string_lossy().into_owned(),
    ]
}

#[cfg(windows)]
mod win32_job {
    use std::mem::{size_of, zeroed};
    use std::ptr::null;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    pub struct ProcessTreeJob {
        handle: HANDLE,
    }

    unsafe impl Send for ProcessTreeJob {}
    unsafe impl Sync for ProcessTreeJob {}

    impl ProcessTreeJob {
        pub fn new() -> Option<Self> {
            unsafe {
                let handle = CreateJobObjectW(null(), null());
                if handle.is_null() {
                    return None;
                }
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let ok = SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const _,
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                if ok == 0 {
                    CloseHandle(handle);
                    return None;
                }
                Some(Self { handle })
            }
        }

        /// # Safety
        /// `process_handle` must be a valid Windows process handle.
        pub unsafe fn assign_process(&self, process_handle: HANDLE) -> bool {
            if self.handle.is_null() || process_handle.is_null() {
                return false;
            }
            unsafe { AssignProcessToJobObject(self.handle, process_handle) != 0 }
        }

        pub fn terminate_tree(&self) {
            if !self.handle.is_null() {
                unsafe {
                    TerminateJobObject(self.handle, 1);
                }
            }
        }
    }

    impl Drop for ProcessTreeJob {
        fn drop(&mut self) {
            if !self.handle.is_null() {
                unsafe {
                    CloseHandle(self.handle);
                }
            }
        }
    }
}

#[cfg(not(windows))]
mod win32_job {
    pub struct ProcessTreeJob;
    impl ProcessTreeJob {
        pub fn new() -> Option<Self> {
            None
        }
        pub fn terminate_tree(&self) {}
    }
}

pub use win32_job::ProcessTreeJob;

pub async fn clip_media_segment(request: ClipRequest) -> Result<ClipResult, ClipError> {
    let target_path =
        resolve_existing_local_file(&request.file_path).ok_or(ClipError::InvalidSource)?;

    if !request.start_time.is_finite()
        || request.start_time < 0.0
        || request.start_time > CLIP_MAX_START_SECONDS
    {
        return Err(ClipError::InvalidStartTime);
    }
    if !request.duration.is_finite() || request.duration <= 0.0 {
        return Err(ClipError::InvalidDuration);
    }
    if request.duration > CLIP_MAX_DURATION_SECONDS {
        return Err(ClipError::DurationTooLong);
    }

    let ffmpeg_binary = request
        .ffmpeg_binary
        .filter(|p| p.is_file())
        .ok_or(ClipError::FfmpegNotFound)?;

    let safe_start = request.start_time.max(0.0);
    let safe_duration = request.duration.max(0.0);
    let output_path = create_clip_output_path(&target_path, request.output_dir.as_deref());
    let args = build_ffmpeg_clip_args(&target_path, &output_path, safe_start, safe_duration);

    let mut cmd = Command::new(&ffmpeg_binary);
    cmd.args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| ClipError::SpawnFailed(e.to_string()))?;

    let job = ProcessTreeJob::new();
    #[cfg(windows)]
    if let (Some(ref j), Some(raw)) = (&job, child.raw_handle()) {
        unsafe {
            j.assign_process(raw as windows_sys::Win32::Foundation::HANDLE);
        }
    }

    let mut stderr_pipe = child.stderr.take();
    let stderr_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        if let Some(ref mut pipe) = stderr_pipe {
            let mut chunk = [0u8; 4096];
            loop {
                match pipe.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let remaining = MAX_STDERR_BYTES.saturating_sub(buf.len());
                        if remaining > 0 {
                            buf.extend_from_slice(&chunk[..n.min(remaining)]);
                        }
                    }
                }
            }
        }
        String::from_utf8_lossy(&buf).into_owned()
    });

    let timeout_dur = Duration::from_millis(request.timeout_ms.unwrap_or(CLIP_EXPORT_TIMEOUT_MS));
    let wait_res = tokio::time::timeout(timeout_dur, child.wait()).await;

    match wait_res {
        Err(_) => {
            if let Some(ref j) = job {
                j.terminate_tree();
            }
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = fs::remove_file(&output_path);
            Err(ClipError::Timeout)
        }
        Ok(Err(e)) => {
            let _ = fs::remove_file(&output_path);
            Err(ClipError::FfmpegFailed(e.to_string()))
        }
        Ok(Ok(status)) => {
            let stderr_msg = stderr_task.await.unwrap_or_default();
            if status.success() {
                Ok(ClipResult {
                    output_path,
                    start_time: safe_start,
                    duration: safe_duration,
                })
            } else {
                let _ = fs::remove_file(&output_path);
                let msg = stderr_msg.trim();
                let detail = if msg.is_empty() {
                    format!("ffmpeg exited with code {:?}", status.code())
                } else {
                    msg.to_string()
                };
                Err(ClipError::FfmpegFailed(detail))
            }
        }
    }
}
