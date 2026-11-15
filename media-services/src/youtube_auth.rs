use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::process::Command;
use tokio::sync::watch;

pub const SIGNIN_URL: &str =
    "https://accounts.google.com/ServiceLogin?service=youtube&continue=https%3A%2F%2Fwww.youtube.com%2F&hl=en";
pub const BROWSER_IMPORT_PROBE_URL: &str = "https://www.youtube.com/watch?v=BaW_jenozKc";
pub const BROWSER_IMPORT_TIMEOUT_MS: u64 = 45_000;
pub const DEVTOOLS_PORT_TIMEOUT_MS: u64 = 20_000;
pub const CDP_CALL_TIMEOUT_MS: u64 = 20_000;

pub const BROWSER_CANDIDATES: [&str; 7] = [
    "firefox", "chrome", "edge", "brave", "chromium", "opera", "vivaldi",
];

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("No supported Chromium browser (Chrome or Edge) found")]
    BrowserNotFound,
    #[error("Failed to launch browser: {0}")]
    LaunchFailed(String),
    #[error("Timed out waiting for DevToolsActivePort file")]
    DevToolsPortTimeout,
    #[error("Invalid DevToolsActivePort contents")]
    InvalidDevToolsPort,
    #[error("CDP WebSocket error: {0}")]
    CdpError(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChromiumBrowser {
    Chrome,
    Edge,
}

impl ChromiumBrowser {
    pub fn as_str(self) -> &'static str {
        match self {
            ChromiumBrowser::Chrome => "chrome",
            ChromiumBrowser::Edge => "edge",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            ChromiumBrowser::Chrome => "Chrome",
            ChromiumBrowser::Edge => "Edge",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChromiumInstall {
    pub browser: ChromiumBrowser,
    pub exe_path: PathBuf,
    pub profile_base: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetscapeCookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub host_only: bool,
    pub path: String,
    pub secure: bool,
    pub session: bool,
    pub expiration_date: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CookieSessionState {
    pub exists: bool,
    pub logged_in: bool,
    pub has_youtube_cookies: bool,
    pub cookie_count: usize,
}

/// Pure-data companion status state consumed by `win32-shell` to render the native Rust
/// companion window (zero WebView2 / zero HTML / zero JS).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompanionAuthState {
    Idle,
    Launched {
        browser: ChromiumBrowser,
        profile_dir: PathBuf,
    },
    WaitingForSignIn {
        browser: ChromiumBrowser,
    },
    Extracting {
        browser: ChromiumBrowser,
    },
    Complete {
        cookie_count: usize,
        logged_in: bool,
    },
    Cancelled,
    Error(String),
}

#[derive(Debug, Clone)]
pub struct CompanionAuthHandle {
    state_rx: watch::Receiver<CompanionAuthState>,
    finish_requested: Arc<AtomicBool>,
    cancel_requested: Arc<AtomicBool>,
}

impl CompanionAuthHandle {
    pub fn state(&self) -> CompanionAuthState {
        self.state_rx.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<CompanionAuthState> {
        self.state_rx.clone()
    }

    pub fn request_finish(&self) {
        self.finish_requested.store(true, Ordering::SeqCst);
    }

    pub fn request_cancel(&self) {
        self.cancel_requested.store(true, Ordering::SeqCst);
    }
}

#[derive(Debug)]
pub struct CompanionAuthController {
    state_tx: watch::Sender<CompanionAuthState>,
    finish_requested: Arc<AtomicBool>,
    cancel_requested: Arc<AtomicBool>,
}

impl Default for CompanionAuthController {
    fn default() -> Self {
        Self::new()
    }
}

impl CompanionAuthController {
    pub fn new() -> Self {
        let (state_tx, _) = watch::channel(CompanionAuthState::Idle);
        Self {
            state_tx,
            finish_requested: Arc::new(AtomicBool::new(false)),
            cancel_requested: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn handle(&self) -> CompanionAuthHandle {
        CompanionAuthHandle {
            state_rx: self.state_tx.subscribe(),
            finish_requested: Arc::clone(&self.finish_requested),
            cancel_requested: Arc::clone(&self.cancel_requested),
        }
    }

    pub fn set_state(&self, next: CompanionAuthState) {
        let _ = self.state_tx.send(next);
    }

    pub fn is_finish_requested(&self) -> bool {
        self.finish_requested.load(Ordering::SeqCst)
    }

    pub fn is_cancel_requested(&self) -> bool {
        self.cancel_requested.load(Ordering::SeqCst)
    }
}

pub fn get_cookie_file_path(user_data_dir: &Path) -> PathBuf {
    user_data_dir.join("cookies.txt")
}

pub fn get_youtube_dedicated_profile_dir(user_data_dir: &Path) -> PathBuf {
    user_data_dir.join("yt-profile")
}

pub fn is_youtube_related_domain(domain: &str) -> bool {
    let clean = domain.trim().trim_start_matches('.').to_ascii_lowercase();
    const BASES: [&str; 7] = [
        "youtube.com",
        "google.com",
        "youtu.be",
        "googlevideo.com",
        "ggpht.com",
        "ytimg.com",
        "googleusercontent.com",
    ];
    BASES
        .iter()
        .any(|base| clean == *base || clean.ends_with(&format!(".{base}")))
}

pub fn build_netscape_cookie_file(cookies: &[NetscapeCookie]) -> String {
    let mut lines = vec![
        "# Netscape HTTP Cookie File".to_string(),
        "# Generated by Hybrid Player".to_string(),
        String::new(),
    ];

    for c in cookies {
        if c.domain.is_empty() || c.name.is_empty() {
            continue;
        }
        let include_subdomains = if c.host_only || !c.domain.starts_with('.') {
            "FALSE"
        } else {
            "TRUE"
        };
        let path = if c.path.is_empty() { "/" } else { &c.path };
        let secure = if c.secure { "TRUE" } else { "FALSE" };
        let expiry = if c.session {
            0
        } else {
            c.expiration_date.unwrap_or(0).max(0)
        };
        lines.push(format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            c.domain, include_subdomains, path, secure, expiry, c.name, c.value
        ));
    }

    lines.push(String::new());
    lines.join("\n")
}

pub fn has_youtube_cookie_lines(content: &str) -> bool {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = trimmed.split('\t').collect();
        if parts.len() >= 7 && is_youtube_related_domain(parts[0]) {
            return true;
        }
    }
    false
}

pub fn has_authenticated_youtube_cookies(cookies: &[NetscapeCookie]) -> bool {
    cookies
        .iter()
        .any(|c| matches!(c.name.as_str(), "LOGIN_INFO" | "SAPISID" | "__Secure-3PSID"))
}

pub fn get_cookie_file_session_state(cookie_path: &Path) -> CookieSessionState {
    let Ok(content) = fs::read_to_string(cookie_path) else {
        return CookieSessionState {
            exists: false,
            logged_in: false,
            has_youtube_cookies: false,
            cookie_count: 0,
        };
    };

    let now_sec = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let mut cookie_count = 0usize;
    let mut has_youtube_cookies = false;
    let mut logged_in = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = trimmed.split('\t').collect();
        if parts.len() >= 7 {
            let expires = parts[4].trim().parse::<i64>().unwrap_or(0);
            if expires > 0 && expires < now_sec {
                continue;
            }
            cookie_count += 1;
            if is_youtube_related_domain(parts[0]) {
                has_youtube_cookies = true;
            }
            if matches!(parts[5], "LOGIN_INFO" | "SAPISID" | "__Secure-3PSID") {
                logged_in = true;
            }
        }
    }

    CookieSessionState {
        exists: true,
        logged_in,
        has_youtube_cookies,
        cookie_count,
    }
}

pub fn find_chromium_install(browser: ChromiumBrowser) -> Option<ChromiumInstall> {
    let local_app_data = env::var("LOCALAPPDATA").unwrap_or_default();
    let prog_files = env::var("PROGRAMFILES").unwrap_or_else(|_| r"C:\Program Files".to_string());
    let prog_files_x86 =
        env::var("PROGRAMFILES(X86)").unwrap_or_else(|_| r"C:\Program Files (x86)".to_string());

    let candidates: Vec<PathBuf> = if cfg!(windows) {
        match browser {
            ChromiumBrowser::Chrome => vec![
                PathBuf::from(&prog_files).join(r"Google\Chrome\Application\chrome.exe"),
                PathBuf::from(&prog_files_x86).join(r"Google\Chrome\Application\chrome.exe"),
                PathBuf::from(&local_app_data).join(r"Google\Chrome\Application\chrome.exe"),
                PathBuf::from(r"C:\Program Files\Google\Chrome\Application\chrome.exe"),
                PathBuf::from(r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe"),
            ],
            ChromiumBrowser::Edge => vec![
                PathBuf::from(&prog_files_x86).join(r"Microsoft\Edge\Application\msedge.exe"),
                PathBuf::from(&prog_files).join(r"Microsoft\Edge\Application\msedge.exe"),
                PathBuf::from(&local_app_data).join(r"Microsoft\Edge\Application\msedge.exe"),
                PathBuf::from(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"),
                PathBuf::from(r"C:\Program Files\Microsoft\Edge\Application\msedge.exe"),
            ],
        }
    } else if cfg!(target_os = "macos") {
        match browser {
            ChromiumBrowser::Chrome => {
                vec![PathBuf::from(
                    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
                )]
            }
            ChromiumBrowser::Edge => {
                vec![PathBuf::from(
                    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
                )]
            }
        }
    } else {
        match browser {
            ChromiumBrowser::Chrome => vec![
                PathBuf::from("/usr/bin/google-chrome"),
                PathBuf::from("/usr/bin/google-chrome-stable"),
                PathBuf::from("/usr/bin/chromium"),
                PathBuf::from("/usr/bin/chromium-browser"),
            ],
            ChromiumBrowser::Edge => vec![
                PathBuf::from("/usr/bin/microsoft-edge"),
                PathBuf::from("/usr/bin/microsoft-edge-stable"),
            ],
        }
    };

    let exe_path = candidates.into_iter().find(|p| p.is_file())?;

    let profile_base = if cfg!(windows) && !local_app_data.is_empty() {
        let base = match browser {
            ChromiumBrowser::Chrome => {
                PathBuf::from(&local_app_data).join(r"Google\Chrome\User Data")
            }
            ChromiumBrowser::Edge => {
                PathBuf::from(&local_app_data).join(r"Microsoft\Edge\User Data")
            }
        };
        if base.is_dir() {
            Some(base)
        } else {
            None
        }
    } else {
        None
    };

    Some(ChromiumInstall {
        browser,
        exe_path,
        profile_base,
    })
}

pub fn find_installed_chromium_browser() -> Option<ChromiumInstall> {
    for browser in [ChromiumBrowser::Chrome, ChromiumBrowser::Edge] {
        if let Some(install) = find_chromium_install(browser) {
            return Some(install);
        }
    }
    None
}

pub fn pick_chromium_profile_name(profile_base: &Path) -> String {
    let local_state_path = profile_base.join("Local State");
    if let Ok(raw) = fs::read_to_string(local_state_path) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) {
            if let Some(used) = json
                .get("profile")
                .and_then(|p| p.get("last_used"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                return used.to_string();
            }
            if let Some(first) = json
                .get("profile")
                .and_then(|p| p.get("last_active_profiles"))
                .and_then(|a| a.as_array())
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                return first.to_string();
            }
        }
    }
    "Default".to_string()
}

pub fn get_browser_launch_args(profile_dir: &Path, signin_url: Option<&str>) -> Vec<String> {
    vec![
        format!("--user-data-dir={}", profile_dir.to_string_lossy()),
        "--new-window".to_string(),
        "--start-maximized".to_string(),
        "--window-size=1024,768".to_string(),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
        "--disable-background-mode".to_string(),
        "--disable-extensions".to_string(),
        "--disable-default-apps".to_string(),
        "--disable-sync".to_string(),
        "--disable-features=EdgeStartupBoost,Translate".to_string(),
        signin_url.unwrap_or(SIGNIN_URL).to_string(),
    ]
}

pub fn build_headless_cdp_args(profile_dir: &Path, profile_directory: Option<&str>) -> Vec<String> {
    let mut args = vec![format!("--user-data-dir={}", profile_dir.to_string_lossy())];
    if let Some(name) = profile_directory {
        args.push(format!("--profile-directory={name}"));
    }
    args.extend([
        "--headless".to_string(),
        "--remote-debugging-port=0".to_string(),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
        "--disable-background-mode".to_string(),
        "--disable-features=EdgeStartupBoost".to_string(),
        "--disable-gpu".to_string(),
        "--disable-sync".to_string(),
        "--disable-background-networking".to_string(),
        "--disable-extensions".to_string(),
        "--disable-component-update".to_string(),
        "--window-size=960,720".to_string(),
        "about:blank".to_string(),
    ]);
    args
}

pub async fn read_devtools_active_port(
    port_file: &Path,
    timeout_ms: u64,
) -> Result<(u16, String), AuthError> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        if let Ok(content) = fs::read_to_string(port_file) {
            let mut lines = content.lines().map(str::trim).filter(|l| !l.is_empty());
            if let (Some(port_str), Some(ws_path)) = (lines.next(), lines.next()) {
                if let Ok(port) = port_str.parse::<u16>() {
                    if port > 0 && ws_path.starts_with('/') {
                        return Ok((port, ws_path.to_string()));
                    }
                }
            }
        }
        if Instant::now() >= deadline {
            return Err(AuthError::DevToolsPortTimeout);
        }
        tokio::time::sleep(Duration::from_millis(120)).await;
    }
}

pub async fn cdp_call(
    port: u16,
    ws_path: &str,
    method: &str,
    params: serde_json::Value,
    timeout_ms: u64,
) -> Result<serde_json::Value, AuthError> {
    let ws_url = format!("ws://127.0.0.1:{port}{ws_path}");
    let method = method.to_string();
    let req_id: u64 = 1;

    let fut = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, AuthError> {
        use tokio_tungstenite::tungstenite::{connect, Message};

        let (mut socket, _) = connect(&ws_url).map_err(|e| AuthError::CdpError(e.to_string()))?;
        let payload = serde_json::json!({
            "id": req_id,
            "method": method,
            "params": params,
        });
        socket
            .send(Message::Text(payload.to_string().into()))
            .map_err(|e| AuthError::CdpError(e.to_string()))?;

        loop {
            let msg = socket
                .read()
                .map_err(|e| AuthError::CdpError(e.to_string()))?;
            match msg {
                Message::Text(text) => {
                    let parsed: serde_json::Value = serde_json::from_str(&text)
                        .map_err(|e| AuthError::CdpError(e.to_string()))?;
                    if parsed.get("id").and_then(|v| v.as_u64()) == Some(req_id) {
                        let _ = socket.close(None);
                        if let Some(err) = parsed.get("error") {
                            return Err(AuthError::CdpError(err.to_string()));
                        }
                        return Ok(parsed
                            .get("result")
                            .cloned()
                            .unwrap_or_else(|| serde_json::json!({})));
                    }
                }
                Message::Close(_) => {
                    return Ok(serde_json::json!({}));
                }
                _ => {}
            }
        }
    });

    match tokio::time::timeout(Duration::from_millis(timeout_ms), fut).await {
        Ok(Ok(res)) => res,
        Ok(Err(join_err)) => Err(AuthError::CdpError(join_err.to_string())),
        Err(_) => Err(AuthError::CdpError(format!(
            "CDP call timed out after {timeout_ms}ms"
        ))),
    }
}

pub fn kill_process_tree(pid: u32) {
    if pid == 0 {
        return;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
        let _ = StdCommand::new("taskkill")
            .args(["/pid", &pid.to_string(), "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(windows))]
    {
        let _ = StdCommand::new("kill")
            .args(["-9", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

pub fn is_process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return false;
            }
            let mut exit_code: u32 = 0;
            let ok = GetExitCodeProcess(handle, &mut exit_code);
            CloseHandle(handle);
            ok != 0 && exit_code == STILL_ACTIVE as u32
        }
    }
    #[cfg(not(windows))]
    {
        StdCommand::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

pub fn close_process_gracefully(pid: u32, timeout_ms: u64) {
    if pid == 0 {
        return;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
        let _ = StdCommand::new("taskkill")
            .args(["/pid", &pid.to_string()])
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(windows))]
    {
        let _ = StdCommand::new("kill")
            .args(["-TERM", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    while Instant::now() < deadline {
        if !is_process_alive(pid) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    kill_process_tree(pid);
}

pub fn sanitize_profile_window_placement(profile_dir: &Path) {
    let prefs_path = profile_dir.join("Default").join("Preferences");
    if let Ok(raw) = fs::read_to_string(&prefs_path) {
        if let Ok(mut prefs) = serde_json::from_str::<serde_json::Value>(&raw) {
            let mut modified = false;
            if let Some(profile) = prefs.get_mut("profile").and_then(|v| v.as_object_mut()) {
                if profile.get("exit_type").and_then(|v| v.as_str()) != Some("Normal") {
                    profile.insert("exit_type".to_string(), serde_json::json!("Normal"));
                    modified = true;
                }
                if profile.get("exited_cleanly").and_then(|v| v.as_bool()) == Some(false) {
                    profile.insert("exited_cleanly".to_string(), serde_json::json!(true));
                    modified = true;
                }
            }
            if let Some(browser) = prefs.get_mut("browser").and_then(|v| v.as_object_mut()) {
                if browser.remove("window_placement").is_some() {
                    modified = true;
                }
                if browser.remove("window_placement_popup").is_some() {
                    modified = true;
                }
            }
            if let Some(root) = prefs.as_object_mut() {
                if root.remove("app_window_placement").is_some() {
                    modified = true;
                }
            }
            if let Some(apps) = prefs.get_mut("apps").and_then(|v| v.as_object_mut()) {
                if apps.remove("window_placement").is_some() {
                    modified = true;
                }
            }
            if modified {
                let _ = fs::write(&prefs_path, prefs.to_string());
            }
        }
    }

    let local_state_path = profile_dir.join("Local State");
    if let Ok(raw) = fs::read_to_string(&local_state_path) {
        if let Ok(mut state) = serde_json::from_str::<serde_json::Value>(&raw) {
            let mut modified = false;
            if let Some(stability) = state
                .get_mut("user_experience_metrics")
                .and_then(|v| v.get_mut("stability"))
                .and_then(|v| v.as_object_mut())
            {
                if stability.get("exited_cleanly").and_then(|v| v.as_bool()) == Some(false) {
                    stability.insert("exited_cleanly".to_string(), serde_json::json!(true));
                    modified = true;
                }
            }
            if let Some(browser) = state.get_mut("browser").and_then(|v| v.as_object_mut()) {
                if browser.remove("window_placement").is_some() {
                    modified = true;
                }
                if browser.remove("window_placement_popup").is_some() {
                    modified = true;
                }
            }
            if modified {
                let _ = fs::write(&local_state_path, state.to_string());
            }
        }
    }
}

fn cdp_cookie_value_to_netscape(all: &[serde_json::Value]) -> Vec<NetscapeCookie> {
    let mut out = Vec::new();
    for c in all {
        let domain = c.get("domain").and_then(|v| v.as_str()).unwrap_or("");
        if !is_youtube_related_domain(domain) {
            continue;
        }
        let name = c.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let value = c.get("value").and_then(|v| v.as_str()).unwrap_or("");
        if name.is_empty() {
            continue;
        }
        let expires_f64 = c.get("expires").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let has_expiry = expires_f64.is_finite() && expires_f64 > 0.0;
        let session = c.get("session").and_then(|v| v.as_bool()) == Some(true) || !has_expiry;

        out.push(NetscapeCookie {
            name: name.to_string(),
            value: value.to_string(),
            domain: domain.to_string(),
            host_only: !domain.starts_with('.'),
            path: c
                .get("path")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("/")
                .to_string(),
            secure: c.get("secure").and_then(|v| v.as_bool()) == Some(true),
            session,
            expiration_date: if has_expiry {
                Some(expires_f64.floor() as i64)
            } else {
                None
            },
        });
    }
    out
}

struct ChildTreeGuard(Option<u32>);

impl Drop for ChildTreeGuard {
    fn drop(&mut self) {
        if let Some(pid) = self.0.take() {
            kill_process_tree(pid);
        }
    }
}

pub async fn extract_cookies_from_dedicated_profile(
    exe_path: &Path,
    dedicated_profile_dir: &Path,
) -> Result<Vec<NetscapeCookie>, AuthError> {
    extract_cookies_from_profile_internal(exe_path, dedicated_profile_dir, None).await
}

async fn extract_cookies_from_profile_internal(
    exe_path: &Path,
    profile_dir: &Path,
    profile_directory: Option<&str>,
) -> Result<Vec<NetscapeCookie>, AuthError> {
    let port_file = profile_dir.join("DevToolsActivePort");
    let _ = fs::remove_file(&port_file);

    let args = build_headless_cdp_args(profile_dir, profile_directory);
    let mut cmd = Command::new(exe_path);
    cmd.args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| AuthError::LaunchFailed(e.to_string()))?;
    let mut guard = ChildTreeGuard(child.id());

    let res = async {
        let (port, ws_path) =
            read_devtools_active_port(&port_file, DEVTOOLS_PORT_TIMEOUT_MS).await?;
        let cdp_res = cdp_call(
            port,
            &ws_path,
            "Storage.getCookies",
            serde_json::json!({}),
            CDP_CALL_TIMEOUT_MS,
        )
        .await?;
        let _ = cdp_call(
            port,
            &ws_path,
            "Browser.close",
            serde_json::json!({}),
            5_000,
        )
        .await;
        Ok::<serde_json::Value, AuthError>(cdp_res)
    }
    .await;

    if let Some(pid) = guard.0.take() {
        kill_process_tree(pid);
    }
    let _ = child.wait().await;
    let _ = fs::remove_file(&port_file);
    sanitize_profile_window_placement(profile_dir);

    let cdp_res = res?;
    let all = cdp_res
        .get("cookies")
        .and_then(|v| v.as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    Ok(cdp_cookie_value_to_netscape(all))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserImportAttempt {
    pub browser: String,
    pub error: String,
    pub locked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserImportResult {
    pub success: bool,
    pub browser: Option<String>,
    pub attempts: Vec<BrowserImportAttempt>,
    pub error: Option<String>,
}

fn is_lock_error(err: &std::io::Error) -> bool {
    if err.kind() == ErrorKind::PermissionDenied {
        return true;
    }
    #[cfg(windows)]
    if matches!(err.raw_os_error(), Some(32) | Some(33) | Some(5)) {
        return true;
    }
    let msg = err.to_string().to_ascii_lowercase();
    msg.contains("busy or locked") || msg.contains("being used by another process")
}

async fn copy_with_retry<F>(mut op: F, attempts: usize, delay_ms: u64) -> std::io::Result<()>
where
    F: FnMut() -> std::io::Result<()>,
{
    let mut last_err = None;
    for i in 0..attempts {
        match op() {
            Ok(()) => return Ok(()),
            Err(err) => {
                last_err = Some(err);
                if i + 1 < attempts {
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                }
            }
        }
    }
    Err(last_err.unwrap_or_else(|| std::io::Error::other("copy failed")))
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if ty.is_file() {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

pub async fn import_chromium_cookies_via_cdp(
    install: &ChromiumInstall,
) -> Result<Vec<NetscapeCookie>, (String, bool)> {
    let Some(ref profile_base) = install.profile_base else {
        return Err(("No Chromium profile directory found".to_string(), false));
    };

    let tmp_root = env::temp_dir().join(format!(
        "hybrid-{}-cookies-{}",
        install.browser.as_str(),
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp_root);
    let tmp_user_data = tmp_root.join("User Data");
    fs::create_dir_all(&tmp_user_data).map_err(|e| (e.to_string(), false))?;

    let profile_name = pick_chromium_profile_name(profile_base);
    let network_src = profile_base.join(&profile_name).join("Network");
    let network_dst = tmp_user_data.join(&profile_name).join("Network");

    let local_state_src = profile_base.join("Local State");
    let local_state_dst = tmp_user_data.join("Local State");

    if let Err(err) = copy_with_retry(
        || fs::copy(&local_state_src, &local_state_dst).map(|_| ()),
        5,
        600,
    )
    .await
    {
        let locked = is_lock_error(&err);
        let _ = fs::remove_dir_all(&tmp_root);
        return Err((
            if locked {
                format!(
                    "{} is currently open and locks its cookie database — close {} and try again, or use \"Sign in to YouTube\"",
                    install.browser.as_str(),
                    install.browser.display_name()
                )
            } else {
                format!("Could not read {} profile: {err}", install.browser.as_str())
            },
            locked,
        ));
    }

    if let Err(err) =
        copy_with_retry(|| copy_dir_recursive(&network_src, &network_dst), 5, 600).await
    {
        let locked = is_lock_error(&err);
        let _ = fs::remove_dir_all(&tmp_root);
        return Err((
            if locked {
                format!(
                    "{} is currently open and locks its cookie database — close {} and try again, or use \"Sign in to YouTube\"",
                    install.browser.as_str(),
                    install.browser.display_name()
                )
            } else {
                format!("Could not read {} profile: {err}", install.browser.as_str())
            },
            locked,
        ));
    }

    let extracted = extract_cookies_from_profile_internal(
        &install.exe_path,
        &tmp_user_data,
        Some(&profile_name),
    )
    .await;
    let _ = fs::remove_dir_all(&tmp_root);
    extracted.map_err(|e| (e.to_string(), false))
}

pub fn describe_browser_import_failure(attempts: &[BrowserImportAttempt]) -> String {
    if attempts.is_empty() {
        return "No browser profiles could be read".to_string();
    }
    if let Some(locked) = attempts.iter().find(|a| a.locked) {
        let label = if locked.browser == "edge" {
            "Edge"
        } else {
            "Chrome"
        };
        return format!(
            "{label} is currently open and locks its cookie database — close it and try again, or use \"Sign in to YouTube\""
        );
    }
    let app_bound = attempts.iter().any(|a| {
        let lower = a.error.to_ascii_lowercase();
        lower.contains("app-bound") || lower.contains("dpapi") || lower.contains("decrypt")
    });
    let none_found = attempts.iter().all(|a| {
        let lower = a.error.to_ascii_lowercase();
        lower.contains("not found")
            || lower.contains("no such")
            || lower.contains("cannot find")
            || lower.contains("could not find")
    });
    if app_bound {
        return "Chrome and Edge block third-party apps from reading cookies (App-Bound Encryption). Install/sign in via Firefox, or use \"Sign in to YouTube\" instead".to_string();
    }
    if none_found {
        return "No supported browser profile found (Firefox works best). Use \"Sign in to YouTube\" instead".to_string();
    }
    let summary: Vec<String> = attempts
        .iter()
        .map(|a| {
            let short: String = a.error.chars().take(80).collect();
            format!("{} ({short})", a.browser)
        })
        .collect();
    format!("Could not read cookies: {}", summary.join("; "))
}

pub async fn import_cookies_from_browser(
    user_data_dir: &Path,
    ytdlp_bin: Option<&Path>,
) -> BrowserImportResult {
    let _ = fs::create_dir_all(user_data_dir);
    let dest = get_cookie_file_path(user_data_dir);
    let mut attempts = Vec::new();

    for browser in [ChromiumBrowser::Chrome, ChromiumBrowser::Edge] {
        let Some(install) = find_chromium_install(browser) else {
            continue;
        };
        if install.profile_base.is_none() {
            continue;
        }
        match import_chromium_cookies_via_cdp(&install).await {
            Ok(cookies) if !cookies.is_empty() => {
                let text = build_netscape_cookie_file(&cookies);
                if fs::write(&dest, text).is_ok() {
                    if let Ok(saved) = fs::read_to_string(&dest) {
                        if has_youtube_cookie_lines(&saved) {
                            return BrowserImportResult {
                                success: true,
                                browser: Some(browser.as_str().to_string()),
                                attempts,
                                error: None,
                            };
                        }
                    }
                }
                attempts.push(BrowserImportAttempt {
                    browser: browser.as_str().to_string(),
                    error: "No YouTube/Google cookies in this profile".to_string(),
                    locked: false,
                });
            }
            Ok(_) => {
                attempts.push(BrowserImportAttempt {
                    browser: browser.as_str().to_string(),
                    error: "No YouTube/Google cookies in this profile".to_string(),
                    locked: false,
                });
            }
            Err((msg, locked)) => {
                let short: String = msg.chars().take(220).collect();
                attempts.push(BrowserImportAttempt {
                    browser: browser.as_str().to_string(),
                    error: short,
                    locked,
                });
            }
        }
    }

    if let Some(bin) = ytdlp_bin.filter(|p| p.is_file()) {
        for browser in BROWSER_CANDIDATES
            .iter()
            .copied()
            .filter(|b| *b != "chrome" && *b != "edge")
        {
            let args = vec![
                "--cookies-from-browser".to_string(),
                browser.to_string(),
                "--cookies".to_string(),
                dest.to_string_lossy().into_owned(),
                "--no-warnings".to_string(),
                "--simulate".to_string(),
                "--".to_string(),
                BROWSER_IMPORT_PROBE_URL.to_string(),
            ];

            let mut cmd = Command::new(bin);
            cmd.args(&args)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());

            #[cfg(windows)]
            {
                use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
                cmd.creation_flags(CREATE_NO_WINDOW);
            }

            let out = tokio::time::timeout(
                Duration::from_millis(BROWSER_IMPORT_TIMEOUT_MS),
                cmd.output(),
            )
            .await;

            if let Ok(saved) = fs::read_to_string(&dest) {
                if has_youtube_cookie_lines(&saved) {
                    return BrowserImportResult {
                        success: true,
                        browser: Some(browser.to_string()),
                        attempts,
                        error: None,
                    };
                }
            }

            let detail = match out {
                Ok(Ok(o)) => String::from_utf8_lossy(&o.stderr).trim().to_string(),
                Ok(Err(e)) => e.to_string(),
                Err(_) => "yt-dlp timed out".to_string(),
            };
            let short: String = detail.chars().take(220).collect();
            attempts.push(BrowserImportAttempt {
                browser: browser.to_string(),
                error: short,
                locked: false,
            });
        }
    }

    let error_msg = describe_browser_import_failure(&attempts);
    BrowserImportResult {
        success: false,
        browser: None,
        attempts,
        error: Some(error_msg),
    }
}

pub fn sign_out_youtube(cookie_path: &Path) -> std::io::Result<()> {
    if cookie_path.exists() {
        fs::remove_file(cookie_path)?;
    }
    Ok(())
}

/// Removes `user_data_dir/cookies.txt` and dedicated `yt-profile/Default/(Network/)Cookies*` files
/// matching `hybrid-player/src/main/youtube-auth.js:1474-1519`.
pub fn sign_out_youtube_in_dir(user_data_dir: &Path) -> std::io::Result<bool> {
    let cookie_path = get_cookie_file_path(user_data_dir);
    let mut removed_file = false;
    if cookie_path.exists() {
        fs::remove_file(&cookie_path)?;
        removed_file = true;
    }

    let profile_dir = get_youtube_dedicated_profile_dir(user_data_dir);
    let candidate_files = [
        profile_dir.join("Default").join("Network").join("Cookies"),
        profile_dir
            .join("Default")
            .join("Network")
            .join("Cookies-journal"),
        profile_dir.join("Default").join("Cookies"),
        profile_dir.join("Default").join("Cookies-journal"),
    ];
    for file in candidate_files {
        if file.exists() {
            let _ = fs::remove_file(file);
        }
    }

    Ok(removed_file)
}

/// Runs the Option A browser sign-in flow headlessly-coordinated via `CompanionAuthController`:
/// launches genuine Chrome/Edge on `userData/yt-profile`, updates pure-data `CompanionAuthState`
/// while waiting for browser close or `controller.request_finish()` / `request_cancel()`,
/// extracts cookies via headless CDP `Storage.getCookies`, and writes Netscape `cookies.txt`.
pub async fn run_option_a_sign_in(
    user_data_dir: &Path,
    controller: &CompanionAuthController,
) -> Result<CookieSessionState, AuthError> {
    let install = find_installed_chromium_browser().ok_or(AuthError::BrowserNotFound)?;
    let profile_dir = get_youtube_dedicated_profile_dir(user_data_dir);
    let cookie_path = get_cookie_file_path(user_data_dir);
    fs::create_dir_all(&profile_dir)?;

    let args = get_browser_launch_args(&profile_dir, None);
    let mut cmd = Command::new(&install.exe_path);
    cmd.args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let mut child = cmd
        .spawn()
        .map_err(|e| AuthError::LaunchFailed(e.to_string()))?;

    controller.set_state(CompanionAuthState::Launched {
        browser: install.browser,
        profile_dir: profile_dir.clone(),
    });
    controller.set_state(CompanionAuthState::WaitingForSignIn {
        browser: install.browser,
    });

    loop {
        if controller.is_cancel_requested() {
            if let Some(pid) = child.id() {
                kill_process_tree(pid);
            }
            let _ = child.wait().await;
            controller.set_state(CompanionAuthState::Cancelled);
            return Ok(get_cookie_file_session_state(&cookie_path));
        }

        if controller.is_finish_requested() {
            if let Some(pid) = child.id() {
                close_process_gracefully(pid, 2_500);
            }
            let _ = child.wait().await;
            break;
        }

        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => tokio::time::sleep(Duration::from_millis(200)).await,
            Err(_) => break,
        }
    }

    tokio::time::sleep(Duration::from_millis(600)).await;
    controller.set_state(CompanionAuthState::Extracting {
        browser: install.browser,
    });

    match extract_cookies_from_dedicated_profile(&install.exe_path, &profile_dir).await {
        Ok(cookies) => {
            let text = build_netscape_cookie_file(&cookies);
            fs::write(&cookie_path, text)?;
            let state = get_cookie_file_session_state(&cookie_path);
            controller.set_state(CompanionAuthState::Complete {
                cookie_count: state.cookie_count,
                logged_in: state.logged_in,
            });
            Ok(state)
        }
        Err(err) => {
            controller.set_state(CompanionAuthState::Error(err.to_string()));
            Err(err)
        }
    }
}
