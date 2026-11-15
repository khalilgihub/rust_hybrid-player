use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::process::Command;

pub const RELEASE_CHECK_CACHE_MS: u64 = 15 * 60 * 1000;
pub const YTDLP_VERSION_TIMEOUT_MS: u64 = 10_000;
pub const YTDLP_SELF_UPDATE_TIMEOUT_MS: u64 = 90_000;
pub const YTDLP_RELEASES_LATEST_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest";
pub const YTDLP_API_LATEST_URL: &str = "https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest";

#[derive(Debug, Error)]
pub enum UpdaterError {
    #[error("yt-dlp executable not found")]
    BinaryNotFound,
    #[error("Could not resolve latest release tag")]
    TagResolutionFailed,
    #[error("HTTP request failed: {0}")]
    Http(String),
    #[error("Downloaded binary is too small or corrupt")]
    BinaryTooSmall,
    #[error("Checksum verification failed (expected: {expected}, got: {actual})")]
    ChecksumMismatch { expected: String, actual: String },
    #[error("Failed to install updated binary: {0}")]
    InstallFailed(String),
    #[error("Update is already in progress")]
    AlreadyUpdating,
    #[error("Update failed: {0}")]
    SelfUpdateFailed(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateCheckResult {
    pub has_update: bool,
    pub current_version: Option<String>,
    pub latest_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelfUpdateResult {
    pub success: bool,
    pub old_version: Option<String>,
    pub new_version: Option<String>,
    pub message: Option<String>,
    pub error: Option<String>,
}

/// Parses a CalVer string like `2025.03.31` or `v2025.03.31-1` into numeric components.
pub fn parse_calver(ver_str: &str) -> Option<Vec<u32>> {
    let trimmed = ver_str.trim();
    let cleaned = trimmed.strip_prefix('v').unwrap_or(trimmed);
    if cleaned.is_empty() {
        return None;
    }
    let parts: Vec<u32> = cleaned
        .split(['.', '-'])
        .map(|part| {
            let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
            digits.parse::<u32>().unwrap_or(0)
        })
        .collect();
    if parts.is_empty() || parts.iter().all(|&n| n == 0) {
        return None;
    }
    Some(parts)
}

pub fn is_newer_calver(candidate: &str, current: &str) -> bool {
    let Some(c_parts) = parse_calver(candidate) else {
        return false;
    };
    let Some(cur_parts) = parse_calver(current) else {
        return false;
    };
    let len = c_parts.len().max(cur_parts.len());
    for i in 0..len {
        let a = c_parts.get(i).copied().unwrap_or(0);
        let b = cur_parts.get(i).copied().unwrap_or(0);
        if a > b {
            return true;
        }
        if a < b {
            return false;
        }
    }
    false
}

pub fn parse_sha256_sums(sums_text: &str, binary_filename: &str) -> Option<String> {
    for line in sums_text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            let fname = parts[1].trim_start_matches('*');
            if fname == binary_filename {
                return Some(parts[0].to_ascii_lowercase());
            }
        }
    }
    None
}

pub fn compute_sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

pub fn extract_tag_from_redirect_location(location: &str) -> Option<String> {
    let idx = location.find("tag/")?;
    let after = &location[idx + 4..];
    let raw_tag = after.split(['/', '?', '#']).next()?.trim();
    if raw_tag.is_empty() {
        return None;
    }
    Some(raw_tag.strip_prefix('v').unwrap_or(raw_tag).to_string())
}

#[derive(Debug)]
struct CachedCheck {
    result: UpdateCheckResult,
    checked_at: Instant,
}

#[derive(Debug)]
pub struct ReleaseCheckCache {
    ttl: Duration,
    cached: Mutex<Option<CachedCheck>>,
}

impl Default for ReleaseCheckCache {
    fn default() -> Self {
        Self::new(Duration::from_millis(RELEASE_CHECK_CACHE_MS))
    }
}

impl ReleaseCheckCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            cached: Mutex::new(None),
        }
    }

    pub fn get(&self, now: Instant) -> Option<UpdateCheckResult> {
        let guard = self.cached.lock().ok()?;
        let entry = guard.as_ref()?;
        if now.duration_since(entry.checked_at) < self.ttl {
            Some(entry.result.clone())
        } else {
            None
        }
    }

    pub fn set(&self, result: UpdateCheckResult, now: Instant) {
        if let Ok(mut guard) = self.cached.lock() {
            *guard = Some(CachedCheck {
                result,
                checked_at: now,
            });
        }
    }

    pub fn invalidate(&self) {
        if let Ok(mut guard) = self.cached.lock() {
            *guard = None;
        }
    }
}

pub async fn get_ytdlp_version(bin: &Path) -> Option<String> {
    if !bin.is_file() {
        return None;
    }
    let mut cmd = Command::new(bin);
    cmd.arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let output = tokio::time::timeout(
        Duration::from_millis(YTDLP_VERSION_TIMEOUT_MS),
        cmd.output(),
    )
    .await
    .ok()?
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let ver = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if ver.is_empty() {
        None
    } else {
        Some(ver)
    }
}

pub fn build_http_client_no_redirect() -> Result<reqwest::Client, UpdaterError> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .user_agent("HybridPlayer")
        .build()
        .map_err(|e| UpdaterError::Http(e.to_string()))
}

pub fn build_http_client_default() -> Result<reqwest::Client, UpdaterError> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .user_agent("HybridPlayer")
        .build()
        .map_err(|e| UpdaterError::Http(e.to_string()))
}

pub async fn get_latest_ytdlp_release_tag() -> Option<String> {
    get_latest_ytdlp_release_tag_with_urls(YTDLP_RELEASES_LATEST_URL, YTDLP_API_LATEST_URL).await
}

pub async fn get_latest_ytdlp_release_tag_with_urls(
    releases_latest_url: &str,
    api_latest_url: &str,
) -> Option<String> {
    if let Ok(no_redir) = build_http_client_no_redirect() {
        if let Ok(res) = no_redir.get(releases_latest_url).send().await {
            if let Some(loc) = res.headers().get(reqwest::header::LOCATION) {
                if let Ok(loc_str) = loc.to_str() {
                    if let Some(tag) = extract_tag_from_redirect_location(loc_str) {
                        return Some(tag);
                    }
                }
            }
        }
    }

    if let Ok(client) = build_http_client_default() {
        if let Ok(res) = client
            .get(api_latest_url)
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
            if res.status().is_success() {
                if let Ok(json) = res.json::<serde_json::Value>().await {
                    if let Some(tag) = json.get("tag_name").and_then(|v| v.as_str()) {
                        let clean = tag.trim().strip_prefix('v').unwrap_or(tag.trim());
                        if !clean.is_empty() {
                            return Some(clean.to_string());
                        }
                    }
                }
            }
        }
    }

    None
}

pub async fn check_for_ytdlp_update(
    bin: Option<&Path>,
    cache: Option<&ReleaseCheckCache>,
    force: bool,
) -> UpdateCheckResult {
    let now = Instant::now();
    if !force {
        if let Some(c) = cache {
            if let Some(hit) = c.get(now) {
                return hit;
            }
        }
    }

    let current_version = match bin {
        Some(p) => get_ytdlp_version(p).await,
        None => None,
    };
    let latest_version = get_latest_ytdlp_release_tag().await;
    let has_update = match (latest_version.as_deref(), current_version.as_deref()) {
        (Some(latest), Some(cur)) => is_newer_calver(latest, cur),
        _ => false,
    };

    let result = UpdateCheckResult {
        has_update,
        current_version,
        latest_version,
    };
    if let Some(c) = cache {
        c.set(result.clone(), Instant::now());
    }
    result
}

/// Moves `temp_path` into `final_path` with up to 5 attempts and `150 * (attempt + 1)` ms backoff
/// to handle transient Windows `EPERM`/`EBUSY` file locks, falling back from rename to copy+unlink.
pub async fn install_binary_with_retry(
    temp_path: &Path,
    final_path: &Path,
) -> Result<(), UpdaterError> {
    let mut last_err: Option<std::io::Error> = None;

    for attempt in 0..5_usize {
        let attempt_res = match fs::rename(temp_path, final_path) {
            Ok(()) => Ok(()),
            Err(_) => fs::copy(temp_path, final_path).map(|_| ()).map(|()| {
                let _ = fs::remove_file(temp_path);
            }),
        };

        match attempt_res {
            Ok(()) => return Ok(()),
            Err(err) => {
                last_err = Some(err);
                tokio::time::sleep(Duration::from_millis(150 * ((attempt as u64) + 1))).await;
            }
        }
    }

    let _ = fs::remove_file(temp_path);
    Err(UpdaterError::InstallFailed(
        last_err
            .map(|e| e.to_string())
            .unwrap_or_else(|| "unknown error".to_string()),
    ))
}

pub async fn download_ytdlp_update(
    user_data_bin_dir: &Path,
    target_tag: Option<&str>,
    download_base_override: Option<&str>,
) -> Result<PathBuf, UpdaterError> {
    let tag = match target_tag {
        Some(t) if !t.trim().is_empty() => t.trim().to_string(),
        _ => get_latest_ytdlp_release_tag()
            .await
            .ok_or(UpdaterError::TagResolutionFailed)?,
    };

    let binary_filename = if cfg!(windows) {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    };
    fs::create_dir_all(user_data_bin_dir)
        .map_err(|e| UpdaterError::InstallFailed(e.to_string()))?;

    let final_path = user_data_bin_dir.join(binary_filename);
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let temp_path = user_data_bin_dir.join(format!("{binary_filename}.tmp-{now_ms}"));

    let base_url = download_base_override
        .map(|s| s.trim_end_matches('/').to_string())
        .unwrap_or_else(|| format!("https://github.com/yt-dlp/yt-dlp/releases/download/{tag}"));

    let client = build_http_client_default()?;

    // Fetch SHA2-256SUMS if available
    let sums_url = format!("{base_url}/SHA2-256SUMS");
    let mut expected_sha: Option<String> = None;
    if let Ok(res) = client
        .get(&sums_url)
        .timeout(Duration::from_secs(15))
        .send()
        .await
    {
        if res.status().is_success() {
            if let Ok(sums_text) = res.text().await {
                expected_sha = parse_sha256_sums(&sums_text, binary_filename);
            }
        }
    }

    let bin_url = format!("{base_url}/{binary_filename}");
    let bin_res = client
        .get(&bin_url)
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| UpdaterError::Http(e.to_string()))?;

    if !bin_res.status().is_success() {
        return Err(UpdaterError::Http(format!(
            "Failed to download binary: HTTP {}",
            bin_res.status().as_u16()
        )));
    }

    let bytes = bin_res
        .bytes()
        .await
        .map_err(|e| UpdaterError::Http(e.to_string()))?;

    if bytes.len() <= 1024 {
        return Err(UpdaterError::BinaryTooSmall);
    }

    if let Some(expected) = expected_sha {
        let actual = compute_sha256_hex(&bytes);
        if actual != expected {
            return Err(UpdaterError::ChecksumMismatch { expected, actual });
        }
    }

    fs::write(&temp_path, &bytes).map_err(|e| UpdaterError::InstallFailed(e.to_string()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&temp_path, fs::Permissions::from_mode(0o755));
    }

    install_binary_with_retry(&temp_path, &final_path).await?;
    Ok(final_path)
}

pub async fn run_ytdlp_self_update(
    current_binary: &Path,
    user_data_bin_dir: &Path,
    cache: Option<&ReleaseCheckCache>,
) -> SelfUpdateResult {
    if !current_binary.is_file() {
        return SelfUpdateResult {
            success: false,
            old_version: None,
            new_version: None,
            message: None,
            error: Some("yt-dlp executable not found".to_string()),
        };
    }

    let old_version = get_ytdlp_version(current_binary).await;
    let latest_version = get_latest_ytdlp_release_tag().await;

    match download_ytdlp_update(user_data_bin_dir, latest_version.as_deref(), None).await {
        Ok(downloaded_path) => {
            if let Some(c) = cache {
                c.invalidate();
            }
            let new_version = get_ytdlp_version(&downloaded_path)
                .await
                .or_else(|| old_version.clone());
            let ver_label = new_version.as_deref().unwrap_or("latest");
            SelfUpdateResult {
                success: true,
                old_version,
                new_version: new_version.clone(),
                message: Some(format!(
                    "Updated yt-dlp to {ver_label} via verified release download"
                )),
                error: None,
            }
        }
        Err(download_err) => {
            let mut cmd = Command::new(current_binary);
            cmd.arg("-U")
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());

            #[cfg(windows)]
            {
                use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
                cmd.creation_flags(CREATE_NO_WINDOW);
            }

            let res = tokio::time::timeout(
                Duration::from_millis(YTDLP_SELF_UPDATE_TIMEOUT_MS),
                cmd.output(),
            )
            .await;

            match res {
                Ok(Ok(output)) if output.status.success() => {
                    if let Some(c) = cache {
                        c.invalidate();
                    }
                    let stdout_msg = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    let new_version = get_ytdlp_version(current_binary)
                        .await
                        .or_else(|| old_version.clone());
                    SelfUpdateResult {
                        success: true,
                        old_version,
                        new_version,
                        message: Some(if stdout_msg.is_empty() {
                            "Update completed successfully".to_string()
                        } else {
                            stdout_msg
                        }),
                        error: None,
                    }
                }
                Ok(Ok(output)) => {
                    let stderr_msg = String::from_utf8_lossy(&output.stderr).trim().to_string();
                    SelfUpdateResult {
                        success: false,
                        old_version,
                        new_version: None,
                        message: None,
                        error: Some(if stderr_msg.is_empty() {
                            download_err.to_string()
                        } else {
                            stderr_msg
                        }),
                    }
                }
                Ok(Err(err)) => SelfUpdateResult {
                    success: false,
                    old_version,
                    new_version: None,
                    message: None,
                    error: Some(err.to_string()),
                },
                Err(_) => SelfUpdateResult {
                    success: false,
                    old_version,
                    new_version: None,
                    message: None,
                    error: Some("yt-dlp self-update timed out".to_string()),
                },
            }
        }
    }
}
