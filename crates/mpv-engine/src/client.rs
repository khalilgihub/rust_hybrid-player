//! Event-driven `libmpv` client wrapper using `mpv_set_wakeup_callback` (zero polling).

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::os::raw::c_void;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rand::RngExt;
use serde_json::Value;
use tokio::sync::{broadcast, oneshot};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Pipes::PeekNamedPipe;
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForSingleObject, INFINITE};
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

use crate::config::{
    build_ytdl_raw_options, MpvStartupConfig, ScreenshotFormat, ScreenshotMode, OBSERVED_PROPERTIES,
};
use crate::error::{MpvError, Result};
use crate::ffi::{
    mpv_event_client_message, mpv_event_command, mpv_event_end_file, mpv_event_log_message,
    mpv_event_property, mpv_handle, mpv_node, mpv_node_to_json, to_cstring, MpvApi,
    MPV_END_FILE_REASON_EOF, MPV_END_FILE_REASON_ERROR, MPV_END_FILE_REASON_QUIT,
    MPV_END_FILE_REASON_REDIRECT, MPV_END_FILE_REASON_STOP, MPV_EVENT_CLIENT_MESSAGE,
    MPV_EVENT_COMMAND_REPLY, MPV_EVENT_END_FILE, MPV_EVENT_FILE_LOADED,
    MPV_EVENT_GET_PROPERTY_REPLY, MPV_EVENT_LOG_MESSAGE, MPV_EVENT_NONE,
    MPV_EVENT_PLAYBACK_RESTART, MPV_EVENT_PROPERTY_CHANGE, MPV_EVENT_SEEK,
    MPV_EVENT_SET_PROPERTY_REPLY, MPV_EVENT_SHUTDOWN, MPV_EVENT_VIDEO_RECONFIG, MPV_FORMAT_FLAG,
    MPV_FORMAT_NODE, MPV_FORMAT_STRING,
};

#[derive(Debug, Clone)]
pub enum EndFileReason {
    Eof,
    Stop,
    Quit,
    Error,
    Redirect,
    Unknown(i32),
}

impl EndFileReason {
    fn from_raw(raw: i32) -> Self {
        match raw {
            MPV_END_FILE_REASON_EOF => Self::Eof,
            MPV_END_FILE_REASON_STOP => Self::Stop,
            MPV_END_FILE_REASON_QUIT => Self::Quit,
            MPV_END_FILE_REASON_ERROR => Self::Error,
            MPV_END_FILE_REASON_REDIRECT => Self::Redirect,
            other => Self::Unknown(other),
        }
    }
}

#[derive(Debug, Clone)]
pub enum MpvEvent {
    PropertyChange {
        name: String,
        value: Value,
        userdata: u64,
    },
    FileLoaded,
    EndFile {
        reason: EndFileReason,
        error: Option<String>,
    },
    Seek,
    PlaybackRestart,
    VideoReconfig,
    ClientMessage {
        args: Vec<String>,
    },
    LogMessage {
        prefix: String,
        level: String,
        text: String,
    },
    Shutdown,
}

#[derive(Debug, Clone)]
pub struct BurstFrameInfo {
    pub frame_path: PathBuf,
    pub file_name: String,
    pub burst_folder: String,
    pub seq_num: u32,
}

#[derive(Debug, Clone)]
pub struct BurstFileEntry {
    pub file_path: PathBuf,
    pub file_name: String,
}

#[derive(Debug, Clone)]
pub struct BurstFinalizeResult {
    pub in_subfolder: bool,
    pub count: usize,
    pub folder_name: String,
    pub files: Vec<BurstFileEntry>,
}

#[derive(Debug, Clone)]
struct BurstSession {
    base_dir: PathBuf,
    burst_folder: String,
    burst_dir_path: PathBuf,
    format: ScreenshotFormat,
}

struct WakeupSignal {
    event: isize,
    stopping: AtomicBool,
}

impl WakeupSignal {
    fn new() -> Result<Arc<Self>> {
        let handle =
            unsafe { CreateEventW(None, false, false, None) }.map_err(|e| MpvError::Api {
                code: -1,
                message: format!("CreateEventW failed: {e}"),
            })?;
        Ok(Arc::new(Self {
            event: handle.0 as isize,
            stopping: AtomicBool::new(false),
        }))
    }

    #[inline]
    fn handle(&self) -> HANDLE {
        HANDLE(self.event as *mut c_void)
    }

    #[inline]
    fn signal(&self) {
        unsafe {
            let _ = SetEvent(self.handle());
        }
    }
}

impl Drop for WakeupSignal {
    fn drop(&mut self) {
        if self.event != 0 {
            unsafe {
                let _ = CloseHandle(self.handle());
            }
            self.event = 0;
        }
    }
}

unsafe extern "C" fn on_mpv_wakeup(ctx: *mut c_void) {
    if ctx.is_null() {
        return;
    }
    let sig = &*(ctx as *const WakeupSignal);
    sig.signal();
}

struct HandlePtr(*mut mpv_handle);
unsafe impl Send for HandlePtr {}
unsafe impl Sync for HandlePtr {}

struct MpvClientInner {
    api: Arc<MpvApi>,
    handle: *mut mpv_handle,
    signal: Arc<WakeupSignal>,
    _signal_raw: *const WakeupSignal,
    events_tx: broadcast::Sender<MpvEvent>,
    pending_replies: Mutex<HashMap<u64, oneshot::Sender<Result<Value>>>>,
    next_userdata: AtomicU64,
    destroyed: AtomicBool,
}

unsafe impl Send for MpvClientInner {}
unsafe impl Sync for MpvClientInner {}

struct SubprocessIpcInner {
    child: Mutex<Option<Child>>,
    pipe_writer: Mutex<Option<fs::File>>,
    events_tx: broadcast::Sender<MpvEvent>,
    pending_sync: Mutex<HashMap<u64, std::sync::mpsc::Sender<Result<Value>>>>,
    pending_async: Mutex<HashMap<u64, oneshot::Sender<Result<Value>>>>,
    next_request_id: AtomicU64,
    stopping: AtomicBool,
}

enum MpvClientBackend {
    LibMpv(Arc<MpvClientInner>),
    Subprocess(Arc<SubprocessIpcInner>),
}

#[derive(Debug)]
pub enum ParsedIpcLine {
    Reply {
        request_id: u64,
        result: std::result::Result<Value, String>,
    },
    Event(MpvEvent),
}

/// Parses one newline-delimited JSON IPC message emitted by `mpv --input-ipc-server`.
pub fn parse_subprocess_ipc_line(line: &str) -> Option<ParsedIpcLine> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let v: Value = serde_json::from_str(trimmed).ok()?;
    if let Some(req_id) = v.get("request_id").and_then(|r| r.as_u64()) {
        let err = v.get("error").and_then(|e| e.as_str()).unwrap_or("success");
        let result = if err == "success" {
            Ok(v.get("data").cloned().unwrap_or(Value::Null))
        } else {
            Err(err.to_owned())
        };
        return Some(ParsedIpcLine::Reply {
            request_id: req_id,
            result,
        });
    }

    let ev_name = v.get("event").and_then(|e| e.as_str())?;
    let ev = match ev_name {
        "file-loaded" => MpvEvent::FileLoaded,
        "playback-restart" => MpvEvent::PlaybackRestart,
        "seek" => MpvEvent::Seek,
        "video-reconfig" => MpvEvent::VideoReconfig,
        "shutdown" => MpvEvent::Shutdown,
        "end-file" => {
            let reason_str = v.get("reason").and_then(|r| r.as_str()).unwrap_or("eof");
            let reason = match reason_str {
                "eof" => EndFileReason::Eof,
                "stop" => EndFileReason::Stop,
                "quit" => EndFileReason::Quit,
                "error" => EndFileReason::Error,
                "redirect" => EndFileReason::Redirect,
                _ => EndFileReason::Unknown(-1),
            };
            let error = v
                .get("file_error")
                .or_else(|| v.get("error"))
                .and_then(|e| e.as_str())
                .map(|s| s.to_owned());
            MpvEvent::EndFile { reason, error }
        }
        "property-change" => {
            let name = v
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or_default()
                .to_owned();
            let value = v.get("data").cloned().unwrap_or(Value::Null);
            let userdata = v.get("id").and_then(|i| i.as_u64()).unwrap_or(0);
            MpvEvent::PropertyChange {
                name,
                value,
                userdata,
            }
        }
        "client-message" => {
            let args = v
                .get("args")
                .and_then(|a| a.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str().map(ToOwned::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            MpvEvent::ClientMessage { args }
        }
        "log-message" => MpvEvent::LogMessage {
            prefix: v
                .get("prefix")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_owned(),
            level: v
                .get("level")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_owned(),
            text: v
                .get("text")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_owned(),
        },
        _ => return None,
    };
    Some(ParsedIpcLine::Event(ev))
}

/// In-process `libmpv` client (or `--wid` subprocess IPC fallback) driven by wakeup/IPC events.
pub struct MpvClient {
    backend: MpvClientBackend,
    event_thread: Mutex<Option<JoinHandle<()>>>,
    current_file: Mutex<Option<String>>,
    screenshot_dir: Mutex<PathBuf>,
    default_screenshot_dir: PathBuf,
    screenshot_format: Mutex<ScreenshotFormat>,
    auto_organize_screenshots: AtomicBool,
    burst_sessions: Mutex<HashMap<String, BurstSession>>,
    user_agent: String,
}

impl MpvClient {
    /// Creates and initializes a `libmpv` instance with the exact startup options
    /// from `mpv-process.js:216-298` (or a headless configuration).
    pub fn new(api: Arc<MpvApi>, config: &MpvStartupConfig) -> Result<Arc<Self>> {
        let handle = unsafe { (api.create)() };
        if handle.is_null() {
            return Err(MpvError::CreateFailed);
        }

        if let Err(err) = fs::create_dir_all(&config.screenshot_dir) {
            // Non-fatal: mpv can still run, screenshots will attempt creation later
            eprintln!(
                "[mpv-engine] failed to create screenshot dir {}: {err}",
                config.screenshot_dir.display()
            );
        }

        for (name, val) in config.build_option_pairs() {
            let c_name = to_cstring(&name)?;
            let c_val = to_cstring(&val)?;
            let code = unsafe { (api.set_option_string)(handle, c_name.as_ptr(), c_val.as_ptr()) };
            if code < 0 {
                let msg = api.error_message(code);
                unsafe { (api.destroy)(handle) };
                return Err(MpvError::Api {
                    code,
                    message: format!("set_option_string({name}={val}): {msg}"),
                });
            }
        }

        let init_code = unsafe { (api.initialize)(handle) };
        if init_code < 0 {
            let msg = api.error_message(init_code);
            unsafe { (api.destroy)(handle) };
            return Err(MpvError::Api {
                code: init_code,
                message: format!("mpv_initialize failed: {msg}"),
            });
        }

        Self::from_initialized_handle(api, handle, config, true)
    }

    /// Creates a headless secondary `libmpv` instance (used by `SeekThumbnailer`).
    pub fn new_headless(api: Arc<MpvApi>, screenshot_dir: &Path) -> Result<Arc<Self>> {
        let handle = unsafe { (api.create)() };
        if handle.is_null() {
            return Err(MpvError::CreateFailed);
        }

        let _ = fs::create_dir_all(screenshot_dir);

        let opts = [
            ("config", "no"),
            ("force-window", "no"),
            ("mute", "yes"),
            ("pause", "yes"),
            ("audio", "no"),
            ("ao", "null"),
            ("vo", "null"),
            ("hwdec", "no"),
            ("osc", "no"),
            ("osd-bar", "no"),
            ("osd-level", "0"),
            ("idle", "yes"),
            ("keep-open", "yes"),
            ("msg-level", "all=warn"),
            ("input-default-bindings", "no"),
            ("screenshot-format", "jpg"),
            ("screenshot-jpeg-quality", "85"),
            ("screenshot-directory", &screenshot_dir.to_string_lossy()),
        ];

        for (name, val) in opts {
            let c_name = to_cstring(name)?;
            let c_val = to_cstring(val)?;
            let code = unsafe { (api.set_option_string)(handle, c_name.as_ptr(), c_val.as_ptr()) };
            if code < 0 {
                let msg = api.error_message(code);
                unsafe { (api.destroy)(handle) };
                return Err(MpvError::Api {
                    code,
                    message: format!("headless set_option_string({name}={val}): {msg}"),
                });
            }
        }

        let init_code = unsafe { (api.initialize)(handle) };
        if init_code < 0 {
            let msg = api.error_message(init_code);
            unsafe { (api.destroy)(handle) };
            return Err(MpvError::Api {
                code: init_code,
                message: format!("headless mpv_initialize failed: {msg}"),
            });
        }

        let cfg = MpvStartupConfig {
            screenshot_dir: screenshot_dir.to_path_buf(),
            screenshot_format: ScreenshotFormat::Jpg,
            ..Default::default()
        };
        Self::from_initialized_handle(api, handle, &cfg, false)
    }

    fn from_initialized_handle(
        api: Arc<MpvApi>,
        handle: *mut mpv_handle,
        config: &MpvStartupConfig,
        observe_defaults: bool,
    ) -> Result<Arc<Self>> {
        let signal = WakeupSignal::new()?;
        let signal_raw = Arc::into_raw(signal.clone());
        let (events_tx, _) = broadcast::channel(512);

        let inner = Arc::new(MpvClientInner {
            api: api.clone(),
            handle,
            signal: signal.clone(),
            _signal_raw: signal_raw,
            events_tx,
            pending_replies: Mutex::new(HashMap::new()),
            next_userdata: AtomicU64::new(1),
            destroyed: AtomicBool::new(false),
        });

        unsafe {
            (api.set_wakeup_callback)(handle, Some(on_mpv_wakeup), signal_raw as *mut c_void);
        }

        let thread_inner = inner.clone();
        let handle_wrap = HandlePtr(handle);
        let event_thread = thread::Builder::new()
            .name("mpv-event-loop".to_owned())
            .spawn(move || {
                Self::run_event_loop(thread_inner, handle_wrap);
            })
            .map_err(|e| MpvError::Api {
                code: -1,
                message: format!("failed to spawn mpv event thread: {e}"),
            })?;

        let client = Arc::new(Self {
            backend: MpvClientBackend::LibMpv(inner),
            event_thread: Mutex::new(Some(event_thread)),
            current_file: Mutex::new(None),
            screenshot_dir: Mutex::new(config.screenshot_dir.clone()),
            default_screenshot_dir: config.screenshot_dir.clone(),
            screenshot_format: Mutex::new(config.screenshot_format),
            auto_organize_screenshots: AtomicBool::new(config.auto_organize_screenshots),
            burst_sessions: Mutex::new(HashMap::new()),
            user_agent: config.user_agent.clone(),
        });

        if observe_defaults {
            if let MpvClientBackend::LibMpv(ref lib) = client.backend {
                let warn = to_cstring("warn")?;
                unsafe {
                    let _ = (lib.api.request_log_messages)(handle, warn.as_ptr());
                }
            }
            for prop in OBSERVED_PROPERTIES {
                client.observe_property(prop)?;
            }
            // Extra telemetry properties for Stats for Nerds (`player.js:2092`).
            client.observe_property("video-codec")?;
            client.observe_property("hwdec-current")?;
        }

        // Trigger initial drain in case startup queued events before callback registration.
        signal.signal();

        Ok(client)
    }

    /// Spawns external `mpv.exe --wid=<hwnd>` and attaches via `--input-ipc-server` named pipe
    /// when `libmpv-2.dll` / `DxBridge` is unavailable (`mpv-process.js:414-510`).
    pub fn new_subprocess(
        mpv_exe: &Path,
        wid_hwnd: Option<isize>,
        config: &MpvStartupConfig,
    ) -> Result<Arc<Self>> {
        if let Err(err) = fs::create_dir_all(&config.screenshot_dir) {
            eprintln!(
                "[mpv-engine] failed to create screenshot dir {}: {err}",
                config.screenshot_dir.display()
            );
        }

        let nonce: u32 = rand::rng().random();
        let pipe_name = format!(r"\\.\pipe\hybrid-mpv-{}-{nonce:08x}", std::process::id());
        let args = config.build_subprocess_args(&pipe_name, wid_hwnd);

        let mut cmd = Command::new(mpv_exe);
        cmd.args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW);

        let mut child = cmd.spawn().map_err(|e| {
            MpvError::LibraryLoad(format!("failed to spawn {}: {e}", mpv_exe.display()))
        })?;

        let deadline = Instant::now() + Duration::from_secs(5);
        let pipe_file = loop {
            match OpenOptions::new().read(true).write(true).open(&pipe_name) {
                Ok(f) => break f,
                Err(err) => {
                    if let Ok(Some(status)) = child.try_wait() {
                        return Err(MpvError::LibraryLoad(format!(
                            "mpv.exe exited early ({status}) before creating IPC pipe"
                        )));
                    }
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(MpvError::Timeout(format!(
                            "timed out waiting for mpv IPC pipe {pipe_name}: {err}"
                        )));
                    }
                    thread::sleep(Duration::from_millis(25));
                }
            }
        };

        let mut pipe_reader = pipe_file.try_clone()?;
        let (events_tx, _) = broadcast::channel(512);
        let sub_inner = Arc::new(SubprocessIpcInner {
            child: Mutex::new(Some(child)),
            pipe_writer: Mutex::new(Some(pipe_file)),
            events_tx,
            pending_sync: Mutex::new(HashMap::new()),
            pending_async: Mutex::new(HashMap::new()),
            next_request_id: AtomicU64::new(1),
            stopping: AtomicBool::new(false),
        });

        let thread_sub = Arc::clone(&sub_inner);
        let event_thread = thread::Builder::new()
            .name("mpv-ipc-event-loop".to_owned())
            .spawn(move || {
                Self::run_subprocess_ipc_loop(thread_sub, &mut pipe_reader);
            })
            .map_err(|e| MpvError::Api {
                code: -1,
                message: format!("failed to spawn mpv IPC event thread: {e}"),
            })?;

        let client = Arc::new(Self {
            backend: MpvClientBackend::Subprocess(sub_inner),
            event_thread: Mutex::new(Some(event_thread)),
            current_file: Mutex::new(None),
            screenshot_dir: Mutex::new(config.screenshot_dir.clone()),
            default_screenshot_dir: config.screenshot_dir.clone(),
            screenshot_format: Mutex::new(config.screenshot_format),
            auto_organize_screenshots: AtomicBool::new(config.auto_organize_screenshots),
            burst_sessions: Mutex::new(HashMap::new()),
            user_agent: config.user_agent.clone(),
        });

        let _ = client.command(&["request_log_messages", "warn"]);
        for prop in OBSERVED_PROPERTIES {
            let _ = client.observe_property(prop);
        }
        let _ = client.observe_property("video-codec");
        let _ = client.observe_property("hwdec-current");

        Ok(client)
    }

    fn run_subprocess_ipc_loop(inner: Arc<SubprocessIpcInner>, reader: &mut fs::File) {
        let mut buf = Vec::with_capacity(4096);
        let mut chunk = [0u8; 4096];
        let raw_handle = HANDLE(reader.as_raw_handle() as *mut c_void);

        while !inner.stopping.load(Ordering::Acquire) {
            let mut avail = 0u32;
            let peek_ok =
                unsafe { PeekNamedPipe(raw_handle, None, 0, None, Some(&mut avail), None) };
            if peek_ok.is_err() {
                break;
            }
            if avail == 0 {
                thread::sleep(Duration::from_millis(4));
                continue;
            }
            let to_read = (avail as usize).min(chunk.len());
            match reader.read(&mut chunk[..to_read]) {
                Ok(0) => break,
                Ok(n) => {
                    buf.extend_from_slice(&chunk[..n]);
                    while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                        let line_bytes: Vec<u8> = buf.drain(..=pos).collect();
                        let line = String::from_utf8_lossy(&line_bytes);
                        if let Some(parsed) = parse_subprocess_ipc_line(&line) {
                            match parsed {
                                ParsedIpcLine::Reply { request_id, result } => {
                                    let mapped = result.map_err(|msg| MpvError::Api {
                                        code: -1,
                                        message: msg,
                                    });
                                    if let Some(tx) =
                                        inner.pending_sync.lock().unwrap().remove(&request_id)
                                    {
                                        let _ = tx.send(mapped);
                                    } else if let Some(tx) =
                                        inner.pending_async.lock().unwrap().remove(&request_id)
                                    {
                                        let _ = tx.send(mapped);
                                    }
                                }
                                ParsedIpcLine::Event(ev) => {
                                    let is_shutdown = matches!(ev, MpvEvent::Shutdown);
                                    let _ = inner.events_tx.send(ev);
                                    if is_shutdown {
                                        return;
                                    }
                                }
                            }
                        }
                    }
                }
                Err(_) => break,
            }
        }
    }

    fn send_subprocess_ipc(
        &self,
        sub: &SubprocessIpcInner,
        command: Value,
        request_id: u64,
    ) -> Result<()> {
        let payload = serde_json::json!({
            "command": command,
            "request_id": request_id,
        });
        let mut line = serde_json::to_string(&payload).map_err(|e| MpvError::Api {
            code: -1,
            message: format!("serialize IPC command: {e}"),
        })?;
        line.push('\n');

        let mut writer_guard = sub.pipe_writer.lock().unwrap();
        let writer = writer_guard.as_mut().ok_or_else(|| MpvError::Api {
            code: -1,
            message: "mpv subprocess IPC pipe closed".to_owned(),
        })?;
        writer.write_all(line.as_bytes())?;
        writer.flush()?;
        Ok(())
    }

    fn subprocess_command_sync(&self, sub: &SubprocessIpcInner, command: Value) -> Result<Value> {
        let id = sub.next_request_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = std::sync::mpsc::channel();
        sub.pending_sync.lock().unwrap().insert(id, tx);
        if let Err(err) = self.send_subprocess_ipc(sub, command, id) {
            sub.pending_sync.lock().unwrap().remove(&id);
            return Err(err);
        }
        match rx.recv_timeout(Duration::from_secs(3)) {
            Ok(res) => res,
            Err(_) => {
                sub.pending_sync.lock().unwrap().remove(&id);
                Err(MpvError::Timeout(
                    "mpv subprocess IPC command timed out".to_owned(),
                ))
            }
        }
    }

    async fn subprocess_command_async(
        &self,
        sub: &SubprocessIpcInner,
        command: Value,
    ) -> Result<Value> {
        let id = sub.next_request_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        sub.pending_async.lock().unwrap().insert(id, tx);
        if let Err(err) = self.send_subprocess_ipc(sub, command, id) {
            sub.pending_async.lock().unwrap().remove(&id);
            return Err(err);
        }
        rx.await.map_err(|_| MpvError::Api {
            code: -1,
            message: "mpv subprocess async reply channel closed".to_owned(),
        })?
    }

    fn run_event_loop(inner: Arc<MpvClientInner>, handle_wrap: HandlePtr) {
        let handle = handle_wrap.0;
        let _ = &handle_wrap;
        loop {
            // Drain all queued events with timeout=0 before sleeping.
            loop {
                if inner.destroyed.load(Ordering::Acquire) {
                    return;
                }
                let ev_ptr = unsafe { (inner.api.wait_event)(handle, 0.0) };
                if ev_ptr.is_null() {
                    break;
                }
                let ev = unsafe { &*ev_ptr };
                if ev.event_id == MPV_EVENT_NONE {
                    break;
                }
                Self::dispatch_event(&inner, ev);
                if ev.event_id == MPV_EVENT_SHUTDOWN {
                    return;
                }
            }

            if inner.signal.stopping.load(Ordering::Acquire)
                || inner.destroyed.load(Ordering::Acquire)
            {
                return;
            }

            let res = unsafe { WaitForSingleObject(inner.signal.handle(), INFINITE) };
            if res != WAIT_OBJECT_0 {
                return;
            }
            if inner.signal.stopping.load(Ordering::Acquire)
                || inner.destroyed.load(Ordering::Acquire)
            {
                return;
            }
        }
    }

    fn dispatch_event(inner: &MpvClientInner, ev: &crate::ffi::mpv_event) {
        match ev.event_id {
            MPV_EVENT_COMMAND_REPLY => {
                let result = if ev.error < 0 {
                    Err(MpvError::Api {
                        code: ev.error,
                        message: inner.api.error_message(ev.error),
                    })
                } else if !ev.data.is_null() {
                    let cmd = unsafe { &*(ev.data as *const mpv_event_command) };
                    Ok(unsafe { mpv_node_to_json(&cmd.result) })
                } else {
                    Ok(Value::Null)
                };
                if let Some(tx) = inner
                    .pending_replies
                    .lock()
                    .unwrap()
                    .remove(&ev.reply_userdata)
                {
                    let _ = tx.send(result);
                }
            }
            MPV_EVENT_GET_PROPERTY_REPLY => {
                let result = if ev.error < 0 {
                    Err(MpvError::Api {
                        code: ev.error,
                        message: inner.api.error_message(ev.error),
                    })
                } else if !ev.data.is_null() {
                    let prop = unsafe { &*(ev.data as *const mpv_event_property) };
                    if prop.format == MPV_FORMAT_NODE && !prop.data.is_null() {
                        Ok(unsafe { mpv_node_to_json(&*(prop.data as *const mpv_node)) })
                    } else {
                        Ok(Value::Null)
                    }
                } else {
                    Ok(Value::Null)
                };
                if let Some(tx) = inner
                    .pending_replies
                    .lock()
                    .unwrap()
                    .remove(&ev.reply_userdata)
                {
                    let _ = tx.send(result);
                }
            }
            MPV_EVENT_SET_PROPERTY_REPLY => {
                let result = if ev.error < 0 {
                    Err(MpvError::Api {
                        code: ev.error,
                        message: inner.api.error_message(ev.error),
                    })
                } else {
                    Ok(Value::Bool(true))
                };
                if let Some(tx) = inner
                    .pending_replies
                    .lock()
                    .unwrap()
                    .remove(&ev.reply_userdata)
                {
                    let _ = tx.send(result);
                }
            }
            MPV_EVENT_PROPERTY_CHANGE => {
                if !ev.data.is_null() {
                    let prop = unsafe { &*(ev.data as *const mpv_event_property) };
                    let name = if prop.name.is_null() {
                        String::new()
                    } else {
                        unsafe { CStr::from_ptr(prop.name) }
                            .to_string_lossy()
                            .into_owned()
                    };
                    let value = if prop.format == MPV_FORMAT_NODE && !prop.data.is_null() {
                        unsafe { mpv_node_to_json(&*(prop.data as *const mpv_node)) }
                    } else if prop.format == MPV_FORMAT_STRING && !prop.data.is_null() {
                        let str_ptr = unsafe { *(prop.data as *const *const std::os::raw::c_char) };
                        if str_ptr.is_null() {
                            Value::Null
                        } else {
                            Value::String(
                                unsafe { CStr::from_ptr(str_ptr) }
                                    .to_string_lossy()
                                    .into_owned(),
                            )
                        }
                    } else if prop.format == MPV_FORMAT_FLAG && !prop.data.is_null() {
                        let v = unsafe { *(prop.data as *const i32) };
                        Value::Bool(v != 0)
                    } else {
                        Value::Null
                    };

                    let _ = inner.events_tx.send(MpvEvent::PropertyChange {
                        name,
                        value,
                        userdata: ev.reply_userdata,
                    });
                }
            }
            MPV_EVENT_FILE_LOADED => {
                let _ = inner.events_tx.send(MpvEvent::FileLoaded);
            }
            MPV_EVENT_END_FILE => {
                let (reason, error) = if !ev.data.is_null() {
                    let ef = unsafe { &*(ev.data as *const mpv_event_end_file) };
                    let err = if ef.error < 0 {
                        Some(inner.api.error_message(ef.error))
                    } else {
                        None
                    };
                    (EndFileReason::from_raw(ef.reason), err)
                } else {
                    (EndFileReason::Eof, None)
                };
                let _ = inner.events_tx.send(MpvEvent::EndFile { reason, error });
            }
            MPV_EVENT_SEEK => {
                let _ = inner.events_tx.send(MpvEvent::Seek);
            }
            MPV_EVENT_PLAYBACK_RESTART => {
                let _ = inner.events_tx.send(MpvEvent::PlaybackRestart);
            }
            MPV_EVENT_VIDEO_RECONFIG => {
                let _ = inner.events_tx.send(MpvEvent::VideoReconfig);
            }
            MPV_EVENT_CLIENT_MESSAGE => {
                if !ev.data.is_null() {
                    let cm = unsafe { &*(ev.data as *const mpv_event_client_message) };
                    let mut args = Vec::new();
                    if cm.num_args > 0 && !cm.args.is_null() {
                        let ptrs =
                            unsafe { std::slice::from_raw_parts(cm.args, cm.num_args as usize) };
                        for &p in ptrs {
                            if !p.is_null() {
                                args.push(
                                    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned(),
                                );
                            }
                        }
                    }
                    let _ = inner.events_tx.send(MpvEvent::ClientMessage { args });
                }
            }
            MPV_EVENT_LOG_MESSAGE => {
                if !ev.data.is_null() {
                    let lm = unsafe { &*(ev.data as *const mpv_event_log_message) };
                    let prefix = if lm.prefix.is_null() {
                        String::new()
                    } else {
                        unsafe { CStr::from_ptr(lm.prefix) }
                            .to_string_lossy()
                            .into_owned()
                    };
                    let level = if lm.level.is_null() {
                        String::new()
                    } else {
                        unsafe { CStr::from_ptr(lm.level) }
                            .to_string_lossy()
                            .into_owned()
                    };
                    let text = if lm.text.is_null() {
                        String::new()
                    } else {
                        unsafe { CStr::from_ptr(lm.text) }
                            .to_string_lossy()
                            .into_owned()
                    };
                    let _ = inner.events_tx.send(MpvEvent::LogMessage {
                        prefix,
                        level,
                        text,
                    });
                }
            }
            MPV_EVENT_SHUTDOWN => {
                let _ = inner.events_tx.send(MpvEvent::Shutdown);
            }
            _ => {}
        }
    }

    pub fn is_subprocess(&self) -> bool {
        matches!(self.backend, MpvClientBackend::Subprocess(_))
    }

    pub fn try_api(&self) -> Option<&Arc<MpvApi>> {
        match &self.backend {
            MpvClientBackend::LibMpv(inner) => Some(&inner.api),
            MpvClientBackend::Subprocess(_) => None,
        }
    }

    pub fn api(&self) -> &Arc<MpvApi> {
        self.try_api()
            .expect("MpvClient::api() is only valid in libmpv mode")
    }

    /// Raw `mpv_handle*` for creating `mpv_render_context`.
    /// Must only be used while `Arc<MpvClient>` is kept alive.
    pub fn raw_handle(&self) -> *mut mpv_handle {
        match &self.backend {
            MpvClientBackend::LibMpv(inner) => inner.handle,
            MpvClientBackend::Subprocess(_) => std::ptr::null_mut(),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<MpvEvent> {
        match &self.backend {
            MpvClientBackend::LibMpv(inner) => inner.events_tx.subscribe(),
            MpvClientBackend::Subprocess(sub) => sub.events_tx.subscribe(),
        }
    }

    pub fn current_file(&self) -> Option<String> {
        self.current_file.lock().unwrap().clone()
    }

    pub fn observe_property(&self, name: &str) -> Result<u64> {
        match &self.backend {
            MpvClientBackend::LibMpv(inner) => {
                let id = inner.next_userdata.fetch_add(1, Ordering::Relaxed);
                let c_name = to_cstring(name)?;
                let code = unsafe {
                    (inner.api.observe_property)(inner.handle, id, c_name.as_ptr(), MPV_FORMAT_NODE)
                };
                inner.api.check(code)?;
                Ok(id)
            }
            MpvClientBackend::Subprocess(sub) => {
                let id = sub.next_request_id.fetch_add(1, Ordering::Relaxed);
                let _ = self.subprocess_command_sync(
                    sub,
                    serde_json::json!(["observe_property", id, name]),
                )?;
                Ok(id)
            }
        }
    }

    pub fn unobserve_property(&self, id: u64) -> Result<()> {
        match &self.backend {
            MpvClientBackend::LibMpv(inner) => {
                let code = unsafe { (inner.api.unobserve_property)(inner.handle, id) };
                inner.api.check(code)
            }
            MpvClientBackend::Subprocess(sub) => {
                let _ = self
                    .subprocess_command_sync(sub, serde_json::json!(["unobserve_property", id]))?;
                Ok(())
            }
        }
    }

    pub fn command(&self, args: &[&str]) -> Result<()> {
        match &self.backend {
            MpvClientBackend::LibMpv(inner) => {
                let c_args: Vec<CString> = args
                    .iter()
                    .map(|a| to_cstring(a))
                    .collect::<Result<Vec<_>>>()?;
                let mut ptrs: Vec<*const std::os::raw::c_char> =
                    c_args.iter().map(|s| s.as_ptr()).collect();
                ptrs.push(std::ptr::null());
                let code = unsafe { (inner.api.command)(inner.handle, ptrs.as_ptr()) };
                inner.api.check(code)
            }
            MpvClientBackend::Subprocess(sub) => {
                let arr: Vec<Value> = args
                    .iter()
                    .map(|s| Value::String((*s).to_owned()))
                    .collect();
                let _ = self.subprocess_command_sync(sub, Value::Array(arr))?;
                Ok(())
            }
        }
    }

    pub async fn command_async(&self, args: &[&str]) -> Result<Value> {
        match &self.backend {
            MpvClientBackend::LibMpv(inner) => {
                let id = inner.next_userdata.fetch_add(1, Ordering::Relaxed);
                let (tx, rx) = oneshot::channel();
                inner.pending_replies.lock().unwrap().insert(id, tx);

                let code = {
                    let c_args: Vec<CString> = args
                        .iter()
                        .map(|a| to_cstring(a))
                        .collect::<Result<Vec<_>>>()?;
                    let mut ptrs: Vec<*const std::os::raw::c_char> =
                        c_args.iter().map(|s| s.as_ptr()).collect();
                    ptrs.push(std::ptr::null());
                    unsafe { (inner.api.command_async)(inner.handle, id, ptrs.as_ptr()) }
                };

                if code < 0 {
                    inner.pending_replies.lock().unwrap().remove(&id);
                    return Err(MpvError::Api {
                        code,
                        message: inner.api.error_message(code),
                    });
                }

                rx.await.map_err(|_| MpvError::Api {
                    code: -1,
                    message: "mpv async reply channel closed".to_owned(),
                })?
            }
            MpvClientBackend::Subprocess(sub) => {
                let arr: Vec<Value> = args
                    .iter()
                    .map(|s| Value::String((*s).to_owned()))
                    .collect();
                self.subprocess_command_async(sub, Value::Array(arr)).await
            }
        }
    }

    pub fn set_property_string(&self, name: &str, value: &str) -> Result<()> {
        match &self.backend {
            MpvClientBackend::LibMpv(inner) => {
                let c_name = to_cstring(name)?;
                let c_val = to_cstring(value)?;
                let code = unsafe {
                    (inner.api.set_property_string)(inner.handle, c_name.as_ptr(), c_val.as_ptr())
                };
                inner.api.check(code)
            }
            MpvClientBackend::Subprocess(sub) => {
                let _ = self.subprocess_command_sync(
                    sub,
                    serde_json::json!(["set_property", name, value]),
                )?;
                Ok(())
            }
        }
    }

    pub fn set_property_bool(&self, name: &str, value: bool) -> Result<()> {
        match &self.backend {
            MpvClientBackend::LibMpv(_) => {
                self.set_property_string(name, if value { "yes" } else { "no" })
            }
            MpvClientBackend::Subprocess(sub) => {
                let _ = self.subprocess_command_sync(
                    sub,
                    serde_json::json!(["set_property", name, value]),
                )?;
                Ok(())
            }
        }
    }

    pub fn set_property_f64(&self, name: &str, value: f64) -> Result<()> {
        match &self.backend {
            MpvClientBackend::LibMpv(_) => self.set_property_string(name, &value.to_string()),
            MpvClientBackend::Subprocess(sub) => {
                let _ = self.subprocess_command_sync(
                    sub,
                    serde_json::json!(["set_property", name, value]),
                )?;
                Ok(())
            }
        }
    }

    pub fn set_property_i64(&self, name: &str, value: i64) -> Result<()> {
        match &self.backend {
            MpvClientBackend::LibMpv(_) => self.set_property_string(name, &value.to_string()),
            MpvClientBackend::Subprocess(sub) => {
                let _ = self.subprocess_command_sync(
                    sub,
                    serde_json::json!(["set_property", name, value]),
                )?;
                Ok(())
            }
        }
    }

    pub fn get_property_json(&self, name: &str) -> Result<Value> {
        match &self.backend {
            MpvClientBackend::LibMpv(inner) => {
                let c_name = to_cstring(name)?;
                let mut node = mpv_node {
                    u: crate::ffi::mpv_node_union { int64: 0 },
                    format: 0,
                };
                let code = unsafe {
                    (inner.api.get_property)(
                        inner.handle,
                        c_name.as_ptr(),
                        MPV_FORMAT_NODE,
                        (&mut node as *mut mpv_node).cast(),
                    )
                };
                inner.api.check(code)?;
                let val = unsafe { mpv_node_to_json(&node) };
                unsafe { (inner.api.free_node_contents)(&mut node) };
                Ok(val)
            }
            MpvClientBackend::Subprocess(sub) => {
                self.subprocess_command_sync(sub, serde_json::json!(["get_property", name]))
            }
        }
    }

    pub fn get_property_string(&self, name: &str) -> Result<Option<String>> {
        match &self.backend {
            MpvClientBackend::LibMpv(inner) => {
                let c_name = to_cstring(name)?;
                let ptr = unsafe { (inner.api.get_property_string)(inner.handle, c_name.as_ptr()) };
                if ptr.is_null() {
                    return Ok(None);
                }
                let s = unsafe { CStr::from_ptr(ptr) }
                    .to_string_lossy()
                    .into_owned();
                unsafe { (inner.api.free)(ptr.cast()) };
                Ok(Some(s))
            }
            MpvClientBackend::Subprocess(sub) => {
                match self
                    .subprocess_command_sync(sub, serde_json::json!(["get_property_string", name]))
                {
                    Ok(Value::Null) => Ok(None),
                    Ok(Value::String(s)) => Ok(Some(s)),
                    Ok(other) => Ok(Some(other.to_string())),
                    Err(_) => Ok(None),
                }
            }
        }
    }

    // ── Convenience Playback Controls ────────────────────────

    pub async fn load_file(&self, file_path: &str) -> Result<()> {
        self.load_file_at(file_path, None).await
    }

    pub async fn load_file_at(&self, file_path: &str, start_secs: Option<f64>) -> Result<()> {
        *self.current_file.lock().unwrap() = Some(file_path.to_owned());
        let start_val = match start_secs {
            Some(s) if s.is_finite() && s > 0.0 => format!("{s:.3}"),
            _ => "0".to_string(),
        };
        let _ = self.set_property_string("options/start", &start_val);
        self.command_async(&["loadfile", file_path, "replace"])
            .await?;
        Ok(())
    }

    pub fn load_file_sync(&self, file_path: &str) -> Result<()> {
        self.load_file_sync_at(file_path, None)
    }

    pub fn load_file_sync_at(&self, file_path: &str, start_secs: Option<f64>) -> Result<()> {
        *self.current_file.lock().unwrap() = Some(file_path.to_owned());
        let start_val = match start_secs {
            Some(s) if s.is_finite() && s > 0.0 => format!("{s:.3}"),
            _ => "0".to_string(),
        };
        let _ = self.set_property_string("options/start", &start_val);
        self.command(&["loadfile", file_path, "replace"])
    }

    pub fn play(&self) -> Result<()> {
        self.set_property_bool("pause", false)
    }

    pub fn pause(&self) -> Result<()> {
        self.set_property_bool("pause", true)
    }

    pub fn toggle_pause(&self) -> Result<bool> {
        self.command(&["cycle", "pause"])?;
        Ok(self
            .get_property_json("pause")
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(false))
    }

    pub fn stop(&self) -> Result<()> {
        *self.current_file.lock().unwrap() = None;
        self.command(&["stop"])
    }

    pub fn seek(&self, seconds: f64, flags: &str) -> Result<()> {
        let sec_str = format!("{seconds:.6}");
        self.command(&["seek", &sec_str, flags])
    }

    pub async fn seek_async(&self, seconds: f64, flags: &str) -> Result<()> {
        let sec_str = format!("{seconds:.6}");
        self.command_async(&["seek", &sec_str, flags]).await?;
        Ok(())
    }

    pub fn seek_relative(&self, seconds: f64) -> Result<()> {
        self.seek(seconds, "relative+exact")
    }

    pub fn seek_percent(&self, pct: f64) -> Result<()> {
        self.seek(pct, "absolute-percent+exact")
    }

    pub fn set_volume(&self, vol: f64) -> Result<()> {
        self.set_property_f64("volume", vol.clamp(0.0, 200.0))
    }

    pub fn set_mute(&self, muted: bool) -> Result<()> {
        self.set_property_bool("mute", muted)
    }

    pub fn set_speed(&self, speed: f64) -> Result<()> {
        self.set_property_f64("speed", speed.clamp(0.1, 16.0))
    }

    pub fn cycle_subtitles(&self) -> Result<()> {
        self.command(&["cycle", "sub"])
    }

    pub fn set_sub(&self, track_id: &str) -> Result<()> {
        self.set_property_string("sid", track_id)
    }

    pub fn set_sub_delay(&self, sec: f64) -> Result<()> {
        self.set_property_f64("sub-delay", sec)
    }

    pub fn set_sub_visibility(&self, vis: bool) -> Result<()> {
        self.set_property_bool("sub-visibility", vis)
    }

    pub fn add_sub_file(&self, path: &str) -> Result<()> {
        self.command(&["sub-add", path, "select"])
    }

    pub fn cycle_audio(&self) -> Result<()> {
        self.command(&["cycle", "audio"])
    }

    pub fn set_audio(&self, track_id: &str) -> Result<()> {
        self.set_property_string("aid", track_id)
    }

    pub fn set_audio_delay(&self, sec: f64) -> Result<()> {
        self.set_property_f64("audio-delay", sec)
    }

    pub fn set_chapter(&self, idx: i64) -> Result<()> {
        self.set_property_i64("chapter", idx)
    }

    pub fn next_chapter(&self) -> Result<()> {
        self.command(&["add", "chapter", "1"])
    }

    pub fn prev_chapter(&self) -> Result<()> {
        self.command(&["add", "chapter", "-1"])
    }

    pub fn frame_step(&self) -> Result<()> {
        self.command(&["frame-step"])
    }

    pub fn frame_back_step(&self) -> Result<()> {
        self.command(&["frame-back-step"])
    }

    pub fn set_ab_loop_a(&self, time: f64) -> Result<()> {
        self.set_property_f64("ab-loop-a", time)
    }

    pub fn set_ab_loop_b(&self, time: f64) -> Result<()> {
        self.set_property_f64("ab-loop-b", time)
    }

    pub fn clear_ab_loop(&self) -> Result<()> {
        self.set_property_string("ab-loop-a", "no")?;
        self.set_property_string("ab-loop-b", "no")
    }

    pub fn refresh_ytdl_options(
        &self,
        extractor_args: &str,
        cookies_path: Option<&Path>,
    ) -> Result<()> {
        let raw = build_ytdl_raw_options(extractor_args, &self.user_agent, cookies_path);
        self.set_property_string("ytdl-raw-options", &raw)
    }

    // ── Screenshots & Burst Capture ─────────────────────────

    pub fn set_screenshot_dir(&self, dir: Option<&Path>) -> Result<()> {
        let target = match dir {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => self.default_screenshot_dir.clone(),
        };
        fs::create_dir_all(&target)?;
        *self.screenshot_dir.lock().unwrap() = target.clone();
        let _ = self.set_property_string("screenshot-directory", &target.to_string_lossy());
        Ok(())
    }

    pub fn screenshot_dir(&self) -> PathBuf {
        self.screenshot_dir.lock().unwrap().clone()
    }

    pub fn set_screenshot_format(&self, fmt: ScreenshotFormat) -> Result<()> {
        *self.screenshot_format.lock().unwrap() = fmt;
        self.set_property_string("screenshot-format", fmt.as_str())?;
        if fmt == ScreenshotFormat::Webp {
            let _ = self.set_property_string("screenshot-webp-quality", "80");
            let _ = self.set_property_string("screenshot-webp-compression", "0");
            let _ = self.set_property_string("screenshot-webp-lossless", "no");
        }
        Ok(())
    }

    pub fn set_auto_organize_screenshots(&self, enabled: bool) {
        self.auto_organize_screenshots
            .store(enabled, Ordering::Relaxed);
    }

    pub fn effective_screenshot_dir(&self) -> PathBuf {
        let base = self.screenshot_dir.lock().unwrap().clone();
        if !self.auto_organize_screenshots.load(Ordering::Relaxed) {
            return base;
        }
        let current_path = self
            .get_property_string("path")
            .ok()
            .flatten()
            .filter(|s| !s.is_empty())
            .or_else(|| self.current_file.lock().unwrap().clone())
            .unwrap_or_default();
        let media_title = self
            .get_property_string("media-title")
            .ok()
            .flatten()
            .unwrap_or_default();
        let subfolder =
            hybrid_core::media_scanner::resolve_screenshot_subfolder(&current_path, &media_title);
        if !subfolder.as_os_str().is_empty() {
            let full_dir = base.join(subfolder);
            let _ = fs::create_dir_all(&full_dir);
            return full_dir;
        }
        let _ = fs::create_dir_all(&base);
        base
    }

    pub async fn screenshot(
        &self,
        mode: ScreenshotMode,
        format_override: Option<ScreenshotFormat>,
    ) -> Result<PathBuf> {
        let dir = self.effective_screenshot_dir();
        fs::create_dir_all(&dir)?;
        let fmt = format_override.unwrap_or(*self.screenshot_format.lock().unwrap());
        let name = generate_screenshot_filename(fmt);
        let path = dir.join(name);
        let path_str = path.to_string_lossy().into_owned();
        self.command_async(&["screenshot-to-file", &path_str, mode.as_str()])
            .await?;
        Ok(path)
    }

    /// Captures a single frame in a burst session (`mpv-process.js:1045-1074`).
    pub async fn screenshot_burst_frame(
        &self,
        session_id: &str,
        seq_num: u32,
        mode: ScreenshotMode,
    ) -> Result<BurstFrameInfo> {
        let session = {
            let mut map = self.burst_sessions.lock().unwrap();
            if let Some(existing) = map.get(session_id) {
                existing.clone()
            } else {
                let base_dir = self.effective_screenshot_dir();
                let fmt = *self.screenshot_format.lock().unwrap();
                let burst_folder = format!("burst_{session_id}");
                let burst_dir_path = base_dir.join(&burst_folder);
                fs::create_dir_all(&burst_dir_path)?;
                let created = BurstSession {
                    base_dir,
                    burst_folder,
                    burst_dir_path,
                    format: fmt,
                };
                map.insert(session_id.to_owned(), created.clone());
                created
            }
        };

        let file_name = format!("frame-{seq_num:05}.{}", session.format.as_str());
        let expected_path = session.burst_dir_path.join(&file_name);
        let path_str = expected_path.to_string_lossy().into_owned();
        self.command_async(&["screenshot-to-file", &path_str, mode.as_str()])
            .await?;

        Ok(BurstFrameInfo {
            frame_path: expected_path,
            file_name,
            burst_folder: session.burst_folder,
            seq_num,
        })
    }

    /// Finalizes a burst session (`mpv-process.js:1083-1177`):
    /// - Polls up to 2500ms (400ms if `total_count == 0`) for non-zero sizes and Windows write-lock release.
    /// - If fewer than 20 frames: flattens to `hybrid-burst-<session_id>_<file>` in parent dir & removes subfolder.
    /// - If >= 20 frames: keeps organized inside `burst_<session_id>`.
    pub fn finalize_burst_session(
        &self,
        session_id: &str,
        total_count: usize,
    ) -> BurstFinalizeResult {
        let session = self.burst_sessions.lock().unwrap().remove(session_id);
        let base_dir = session
            .as_ref()
            .map(|s| s.base_dir.clone())
            .unwrap_or_else(|| self.effective_screenshot_dir());
        let burst_folder = session
            .as_ref()
            .map(|s| s.burst_folder.clone())
            .unwrap_or_else(|| format!("burst_{session_id}"));
        let burst_dir_path = session
            .as_ref()
            .map(|s| s.burst_dir_path.clone())
            .unwrap_or_else(|| base_dir.join(&burst_folder));

        if !burst_dir_path.exists() {
            return BurstFinalizeResult {
                in_subfolder: false,
                count: 0,
                folder_name: String::new(),
                files: Vec::new(),
            };
        }

        let wait_timeout = Duration::from_millis(if total_count > 0 { 2500 } else { 400 });
        let start = Instant::now();

        while start.elapsed() < wait_timeout {
            let mut entries = match list_dir_files(&burst_dir_path) {
                Ok(e) => e,
                Err(_) => break,
            };
            entries.sort();
            if !entries.is_empty() && entries.len() >= total_count {
                let all_ready = entries.iter().all(|path| {
                    let size_ok = fs::metadata(path).map(|m| m.len() > 0).unwrap_or(false);
                    if !size_ok {
                        return false;
                    }
                    // Verify Windows write-lock release (`r+` exclusive check)
                    OpenOptions::new().read(true).write(true).open(path).is_ok()
                });
                if all_ready {
                    break;
                }
            }
            thread::sleep(Duration::from_millis(25));
        }

        let mut files = list_dir_files(&burst_dir_path).unwrap_or_default();
        files.sort();
        let count = files.len();

        if count < 20 {
            let mut moved_files = Vec::with_capacity(count);
            for src_path in files {
                let Some(orig_name) = src_path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                let dest_name = format!("hybrid-burst-{session_id}_{orig_name}");
                let dest_path = base_dir.join(&dest_name);
                let mut moved = false;
                for _attempt in 0..5 {
                    if fs::rename(&src_path, &dest_path).is_ok() {
                        moved = true;
                        break;
                    }
                    if fs::copy(&src_path, &dest_path).is_ok() && fs::remove_file(&src_path).is_ok()
                    {
                        moved = true;
                        break;
                    }
                    thread::sleep(Duration::from_millis(40));
                }
                if moved {
                    moved_files.push(BurstFileEntry {
                        file_path: dest_path,
                        file_name: dest_name,
                    });
                }
            }
            let _ = fs::remove_dir(&burst_dir_path);
            return BurstFinalizeResult {
                in_subfolder: false,
                count: moved_files.len(),
                folder_name: String::new(),
                files: moved_files,
            };
        }

        let subfolder_files = files
            .into_iter()
            .filter_map(|p| {
                let name = p.file_name()?.to_str()?.to_owned();
                Some(BurstFileEntry {
                    file_path: p,
                    file_name: name,
                })
            })
            .collect();

        BurstFinalizeResult {
            in_subfolder: true,
            count,
            folder_name: burst_folder,
            files: subfolder_files,
        }
    }

    pub fn shutdown(&self) {
        match &self.backend {
            MpvClientBackend::LibMpv(inner) => {
                if inner.destroyed.swap(true, Ordering::AcqRel) {
                    return;
                }
                inner.signal.stopping.store(true, Ordering::Release);
                unsafe {
                    (inner.api.set_wakeup_callback)(inner.handle, None, std::ptr::null_mut());
                    (inner.api.wakeup)(inner.handle);
                }
                inner.signal.signal();

                if let Some(jh) = self.event_thread.lock().unwrap().take() {
                    let _ = jh.join();
                }

                let mut pending = inner.pending_replies.lock().unwrap();
                pending.clear();
            }
            MpvClientBackend::Subprocess(sub) => {
                if sub.stopping.swap(true, Ordering::AcqRel) {
                    return;
                }
                let _ = self.send_subprocess_ipc(sub, serde_json::json!(["quit"]), u64::MAX);
                drop(sub.pipe_writer.lock().unwrap().take());
                if let Some(jh) = self.event_thread.lock().unwrap().take() {
                    let _ = jh.join();
                }
                if let Some(mut child) = sub.child.lock().unwrap().take() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                sub.pending_sync.lock().unwrap().clear();
                sub.pending_async.lock().unwrap().clear();
            }
        }
    }
}

impl Drop for MpvClient {
    fn drop(&mut self) {
        self.shutdown();
        if let MpvClientBackend::LibMpv(ref inner) = self.backend {
            if !inner.handle.is_null() {
                unsafe {
                    if let Some(term) = inner.api.terminate_destroy {
                        term(inner.handle);
                    } else {
                        (inner.api.destroy)(inner.handle);
                    }
                }
            }
            if !inner._signal_raw.is_null() {
                unsafe {
                    drop(Arc::from_raw(inner._signal_raw));
                }
            }
        }
    }
}

fn list_dir_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let p = entry.path();
        if p.is_file() {
            out.push(p);
        }
    }
    Ok(out)
}

fn generate_screenshot_filename(fmt: ScreenshotFormat) -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let nonce: u32 = rand::rng().random();
    format!("hybrid-player-{millis}-{nonce:06x}.{}", fmt.as_str())
}

#[cfg(test)]
mod workstream_d_tests {
    use super::*;
    use crate::config::MpvStartupConfig;

    #[test]
    fn subprocess_args_and_ipc_line_parsing_work() {
        let cfg = MpvStartupConfig::default();
        let wid_args = cfg.build_subprocess_args(r"\\.\pipe\hybrid-test-ipc", Some(0x1234));
        assert!(wid_args.contains(&"--input-ipc-server=\\\\.\\pipe\\hybrid-test-ipc".to_string()));
        assert!(wid_args.contains(&"--vo=gpu".to_string()));
        assert!(wid_args.contains(&"--wid=4660".to_string()));

        let null_args = cfg.build_subprocess_args(r"\\.\pipe\hybrid-test-ipc", None);
        assert!(null_args.contains(&"--vo=null".to_string()));
        assert!(null_args.contains(&"--force-window=no".to_string()));

        match parse_subprocess_ipc_line(r#"{"request_id":7,"error":"success","data":42}"#) {
            Some(ParsedIpcLine::Reply {
                request_id: 7,
                result: Ok(v),
            }) => {
                assert_eq!(v, serde_json::json!(42));
            }
            other => panic!("unexpected parsed reply: {other:?}"),
        }

        match parse_subprocess_ipc_line(r#"{"request_id":9,"error":"property unavailable"}"#) {
            Some(ParsedIpcLine::Reply {
                request_id: 9,
                result: Err(e),
            }) => {
                assert_eq!(e, "property unavailable");
            }
            other => panic!("unexpected error reply: {other:?}"),
        }

        match parse_subprocess_ipc_line(
            r#"{"event":"end-file","reason":"error","file_error":"loading failed"}"#,
        ) {
            Some(ParsedIpcLine::Event(MpvEvent::EndFile {
                reason: EndFileReason::Error,
                error,
            })) => {
                assert_eq!(error.as_deref(), Some("loading failed"));
            }
            other => panic!("unexpected end-file event: {other:?}"),
        }
    }
}
