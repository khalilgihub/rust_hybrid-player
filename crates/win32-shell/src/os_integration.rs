//! OS integration for Windows:
//! - High-resolution multimedia timer (`timeBeginPeriod(1)` / `timeEndPeriod(1)`)
//! - Single-instance `CreateMutexW` + named-pipe forwarding of CLI args
//! - `SetCurrentProcessExplicitAppUserModelID("com.hybridplayer.app")`
//! - HKCU `AppUserModelId` registry keys & Start Menu `.lnk` creation
//! - PID-scoped temp file cleanup on startup/exit
//! - Windows Recycle Bin / Shell trash helper (`SHFileOperationW`) for screenshot undo

use std::ffi::c_void;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime};

use windows::core::{Interface, GUID, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, ERROR_PIPE_CONNECTED, HANDLE, HWND,
    PROPERTYKEY, STILL_ACTIVE,
};
use windows::Win32::Media::{timeBeginPeriod, timeEndPeriod};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FlushFileBuffers, ReadFile, WriteFile, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ,
    FILE_GENERIC_WRITE, FILE_SHARE_NONE, OPEN_EXISTING, PIPE_ACCESS_INBOUND,
};
use windows::Win32::System::Com::StructuredStorage::{
    PropVariantClear, PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemAlloc, CoUninitialize, IPersistFile,
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, STGM_READWRITE,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE,
    PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_WRITE,
    REG_OPTION_NON_VOLATILE, REG_SZ,
};
use windows::Win32::System::Threading::{
    CreateMutexW, GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::System::Variant::VT_LPWSTR;
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{
    IShellLinkW, SHChangeNotify, SHFileOperationW, SetCurrentProcessExplicitAppUserModelID,
    ShellLink, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FO_DELETE,
    SHCNE_ASSOCCHANGED, SHCNF_FLUSH, SHFILEOPSTRUCTW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE,
};

use crate::error::{Result, Win32ShellError};

pub const APP_ID: &str = "com.hybridplayer.app";
pub const APP_NAME: &str = "Hybrid Player";
pub const DEFAULT_SINGLE_INSTANCE_MUTEX: &str = "Local\\com.hybridplayer.app.single-instance";
pub const DEFAULT_SINGLE_INSTANCE_PIPE: &str = "\\\\.\\pipe\\hybrid-player-single-instance";

// ─── High-Resolution Timer (timeBeginPeriod(1)) ──────────────────────────────

static HIGH_RES_REFCOUNT: AtomicUsize = AtomicUsize::new(0);
static HIGH_RES_ACTIVE_FLAG: AtomicBool = AtomicBool::new(false);

/// RAII guard calling `timeBeginPeriod(1)` on creation and `timeEndPeriod(1)` on drop.
#[derive(Debug)]
pub struct HighResTimerGuard {
    _private: (),
}

impl HighResTimerGuard {
    pub fn acquire() -> Self {
        if HIGH_RES_REFCOUNT.fetch_add(1, Ordering::AcqRel) == 0 {
            unsafe {
                timeBeginPeriod(1);
            }
        }
        Self { _private: () }
    }
}

impl Drop for HighResTimerGuard {
    fn drop(&mut self) {
        if HIGH_RES_REFCOUNT.fetch_sub(1, Ordering::AcqRel) == 1 {
            unsafe {
                timeEndPeriod(1);
            }
        }
    }
}

/// Imperative toggle matching `setHighResTimer(active)` in `ipc-handlers.js:462-473`.
pub fn set_high_res_timer(active: bool) {
    if active {
        if !HIGH_RES_ACTIVE_FLAG.swap(true, Ordering::AcqRel)
            && HIGH_RES_REFCOUNT.fetch_add(1, Ordering::AcqRel) == 0
        {
            unsafe {
                timeBeginPeriod(1);
            }
        }
    } else if HIGH_RES_ACTIVE_FLAG.swap(false, Ordering::AcqRel)
        && HIGH_RES_REFCOUNT.fetch_sub(1, Ordering::AcqRel) == 1
    {
        unsafe {
            timeEndPeriod(1);
        }
    }
}

// ─── Single-Instance Mutex + Named Pipe CLI Forwarding ───────────────────────

fn to_wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Encode CLI arguments into a length-prefixed binary packet:
/// `[u32 count][u32 len_0][utf8_0]...`
pub fn encode_pipe_args(args: &[String]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(64);
    buf.extend_from_slice(&(args.len() as u32).to_le_bytes());
    for arg in args {
        let bytes = arg.as_bytes();
        buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        buf.extend_from_slice(bytes);
    }
    buf
}

/// Decode CLI arguments from a length-prefixed pipe payload.
pub fn decode_pipe_args(bytes: &[u8]) -> Option<Vec<String>> {
    if bytes.len() < 4 {
        return None;
    }
    let count = u32::from_le_bytes(bytes[0..4].try_into().ok()?) as usize;
    if count > 4096 {
        return None;
    }
    let mut offset = 4usize;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        if offset + 4 > bytes.len() {
            return None;
        }
        let len = u32::from_le_bytes(bytes[offset..offset + 4].try_into().ok()?) as usize;
        offset += 4;
        if offset + len > bytes.len() {
            return None;
        }
        let s = std::str::from_utf8(&bytes[offset..offset + len]).ok()?;
        out.push(s.to_string());
        offset += len;
    }
    Some(out)
}

/// Role returned by [`SingleInstance::acquire`].
pub enum SingleInstanceOutcome {
    /// This process acquired the mutex and now owns the named-pipe server.
    Primary(SingleInstanceServer),
    /// Another instance already holds the mutex; args were forwarded over the pipe.
    SecondaryForwarded,
}

/// Owns the single-instance Win32 mutex and background named-pipe listener.
pub struct SingleInstanceServer {
    mutex_handle: isize,
    pipe_name: String,
    stop: Arc<AtomicBool>,
    listener_thread: Mutex<Option<JoinHandle<()>>>,
}

impl SingleInstanceServer {
    /// Acquire the named mutex; if already owned, forward `args_to_forward` to `pipe_name`
    /// and return `SingleInstanceOutcome::SecondaryForwarded`.
    pub fn acquire(
        mutex_name: &str,
        pipe_name: &str,
        args_to_forward: &[String],
    ) -> Result<SingleInstanceOutcome> {
        let wide_mutex = to_wide_null(mutex_name);
        let handle =
            unsafe { CreateMutexW(None, false, PCWSTR(wide_mutex.as_ptr())) }.map_err(|e| {
                Win32ShellError::HResult {
                    api: "CreateMutexW",
                    hr: e.code().0,
                    message: e.message(),
                }
            })?;

        let last_err = unsafe { GetLastError() };
        if last_err == ERROR_ALREADY_EXISTS {
            unsafe {
                let _ = CloseHandle(handle);
            }
            send_args_over_pipe(pipe_name, args_to_forward)?;
            return Ok(SingleInstanceOutcome::SecondaryForwarded);
        }

        Ok(SingleInstanceOutcome::Primary(Self {
            mutex_handle: handle.0 as isize,
            pipe_name: pipe_name.to_string(),
            stop: Arc::new(AtomicBool::new(false)),
            listener_thread: Mutex::new(None),
        }))
    }

    /// Start the background named-pipe server thread. When args arrive, invokes `on_args`
    /// and optionally restores/focuses `focus_hwnd`.
    pub fn start_listener<F>(&self, focus_hwnd: Option<HWND>, mut on_args: F)
    where
        F: FnMut(Vec<String>) + Send + 'static,
    {
        let pipe_name = self.pipe_name.clone();
        let stop = Arc::clone(&self.stop);
        let raw_hwnd = focus_hwnd.map(|h| h.0 as isize).unwrap_or(0);

        let handle = std::thread::spawn(move || {
            let wide_pipe = to_wide_null(&pipe_name);
            while !stop.load(Ordering::Acquire) {
                let pipe = unsafe {
                    CreateNamedPipeW(
                        PCWSTR(wide_pipe.as_ptr()),
                        PIPE_ACCESS_INBOUND,
                        PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                        PIPE_UNLIMITED_INSTANCES,
                        4096,
                        4096,
                        0,
                        None,
                    )
                };
                if pipe.is_invalid() {
                    std::thread::sleep(Duration::from_millis(50));
                    continue;
                }

                let connected = match unsafe { ConnectNamedPipe(pipe, None) } {
                    Ok(()) => true,
                    Err(e) => e.code().0 as u32 == ERROR_PIPE_CONNECTED.0,
                };

                if stop.load(Ordering::Acquire) {
                    unsafe {
                        let _ = DisconnectNamedPipe(pipe);
                        let _ = CloseHandle(pipe);
                    }
                    break;
                }

                if connected {
                    let mut payload = Vec::new();
                    let mut chunk = [0u8; 4096];
                    loop {
                        let mut read_bytes = 0u32;
                        let ok = unsafe {
                            ReadFile(pipe, Some(&mut chunk), Some(&mut read_bytes), None)
                        };
                        if ok.is_err() || read_bytes == 0 {
                            break;
                        }
                        payload.extend_from_slice(&chunk[..read_bytes as usize]);
                        if payload.len() > 256 * 1024 {
                            break;
                        }
                    }

                    if !payload.is_empty() {
                        if let Some(args) = decode_pipe_args(&payload) {
                            if raw_hwnd != 0 {
                                let hwnd = HWND(raw_hwnd as *mut c_void);
                                unsafe {
                                    if IsIconic(hwnd).as_bool() {
                                        let _ = ShowWindow(hwnd, SW_RESTORE);
                                    }
                                    let _ = SetForegroundWindow(hwnd);
                                }
                            }
                            on_args(args);
                        }
                    }
                }

                unsafe {
                    let _ = DisconnectNamedPipe(pipe);
                    let _ = CloseHandle(pipe);
                }
            }
        });

        if let Ok(mut guard) = self.listener_thread.lock() {
            *guard = Some(handle);
        }
    }
}

impl Drop for SingleInstanceServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // Poke the pipe so blocking ConnectNamedPipe wakes immediately.
        let _ = send_args_over_pipe(&self.pipe_name, &[]);
        if let Ok(mut guard) = self.listener_thread.lock() {
            if let Some(handle) = guard.take() {
                let _ = handle.join();
            }
        }
        if self.mutex_handle != 0 {
            unsafe {
                let _ = CloseHandle(HANDLE(self.mutex_handle as *mut c_void));
            }
            self.mutex_handle = 0;
        }
    }
}

/// Connect to an existing instance's named pipe and forward CLI arguments.
pub fn send_args_over_pipe(pipe_name: &str, args: &[String]) -> Result<()> {
    let wide_pipe = to_wide_null(pipe_name);
    let mut last_err = None;
    for attempt in 0..10 {
        let handle = unsafe {
            CreateFileW(
                PCWSTR(wide_pipe.as_ptr()),
                FILE_GENERIC_WRITE.0,
                FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
        };
        match handle {
            Ok(h) => {
                let packet = encode_pipe_args(args);
                let mut written = 0u32;
                let write_res = unsafe { WriteFile(h, Some(&packet), Some(&mut written), None) };
                unsafe {
                    let _ = FlushFileBuffers(h);
                    let _ = CloseHandle(h);
                }
                write_res.map_err(|e| Win32ShellError::HResult {
                    api: "WriteFile",
                    hr: e.code().0,
                    message: e.message(),
                })?;
                return Ok(());
            }
            Err(e) => {
                last_err = Some(e);
                if attempt < 9 {
                    std::thread::sleep(Duration::from_millis(25));
                }
            }
        }
    }

    Err(match last_err {
        Some(e) => Win32ShellError::HResult {
            api: "CreateFileW(pipe)",
            hr: e.code().0,
            message: e.message(),
        },
        None => Win32ShellError::Io(io::Error::new(
            io::ErrorKind::NotFound,
            "failed to connect to single-instance pipe",
        )),
    })
}

/// Read-only helper check matching `FILE_GENERIC_READ` usage if needed.
#[allow(dead_code)]
fn _keep_read_import_used() {
    let _ = FILE_GENERIC_READ;
}

// ─── AppUserModelId, HKCU Registry, and Start Menu .lnk ──────────────────────

/// Set the process-wide Windows AppUserModelId (`com.hybridplayer.app`).
pub fn set_current_process_app_user_model_id(app_id: &str) -> Result<()> {
    let wide = to_wide_null(app_id);
    unsafe { SetCurrentProcessExplicitAppUserModelID(PCWSTR(wide.as_ptr())) }.map_err(|e| {
        Win32ShellError::HResult {
            api: "SetCurrentProcessExplicitAppUserModelID",
            hr: e.code().0,
            message: e.message(),
        }
    })
}

fn set_hkcu_reg_sz(hkey: HKEY, name: &str, value: &str) -> Result<()> {
    let wide_name = to_wide_null(name);
    let wide_val = to_wide_null(value);
    let byte_len = (wide_val.len() * std::mem::size_of::<u16>()) as u32;
    let bytes =
        unsafe { std::slice::from_raw_parts(wide_val.as_ptr() as *const u8, byte_len as usize) };
    let status = unsafe {
        RegSetValueExW(
            hkey,
            PCWSTR(wide_name.as_ptr()),
            Some(0),
            REG_SZ,
            Some(bytes),
        )
    };
    if status.is_err() {
        return Err(Win32ShellError::Win32 {
            api: "RegSetValueExW",
            code: status.0,
        });
    }
    Ok(())
}

/// Register HKCU `Software\Classes\AppUserModelId\<app_id>` metadata (`DisplayName`,
/// `IconUri`, `SmallLogo`, `Square44x44Logo`, `Square150x150Logo`) so Windows 11
/// SMTC displays "Hybrid Player" and the official logo instead of "Unknown app".
pub fn register_hkcu_app_user_model_id(
    app_id: &str,
    display_name: &str,
    icon_png_path: &Path,
) -> Result<()> {
    let subkey = format!("Software\\Classes\\AppUserModelId\\{app_id}");
    let wide_subkey = to_wide_null(&subkey);
    let mut hkey = HKEY::default();

    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(wide_subkey.as_ptr()),
            Some(0),
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut hkey,
            None,
        )
    };
    if status.is_err() {
        return Err(Win32ShellError::Win32 {
            api: "RegCreateKeyExW",
            code: status.0,
        });
    }

    let icon_str = icon_png_path.to_string_lossy().to_string();
    let res = (|| {
        set_hkcu_reg_sz(hkey, "DisplayName", display_name)?;
        set_hkcu_reg_sz(hkey, "IconUri", &icon_str)?;
        set_hkcu_reg_sz(hkey, "SmallLogo", &icon_str)?;
        set_hkcu_reg_sz(hkey, "Square44x44Logo", &icon_str)?;
        set_hkcu_reg_sz(hkey, "Square150x150Logo", &icon_str)?;
        Ok(())
    })();

    unsafe {
        let _ = RegCloseKey(hkey);
    }
    res
}

struct ComApartmentGuard(bool);

impl ComApartmentGuard {
    fn enter() -> Self {
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        Self(hr.is_ok())
    }
}

impl Drop for ComApartmentGuard {
    fn drop(&mut self) {
        if self.0 {
            unsafe {
                CoUninitialize();
            }
        }
    }
}

/// Create or update a Start Menu `.lnk` shortcut and stamp `System.AppUserModel.ID`
/// via COM `IShellLinkW` + `IPropertyStore` + `IPersistFile` (zero PowerShell).
pub fn create_start_menu_shortcut(
    shortcut_path: &Path,
    target_exe: &Path,
    icon_ico_path: &Path,
    description: &str,
    app_id: &str,
) -> Result<()> {
    let _com = ComApartmentGuard::enter();

    if let Some(parent) = shortcut_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let shell_link: IShellLinkW = unsafe {
        CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
    }
    .map_err(|e| Win32ShellError::HResult {
        api: "CoCreateInstance(ShellLink)",
        hr: e.code().0,
        message: e.message(),
    })?;

    let target_wide = to_wide_null(&target_exe.to_string_lossy());
    let icon_wide = to_wide_null(&icon_ico_path.to_string_lossy());
    let desc_wide = to_wide_null(description);
    let link_wide = to_wide_null(&shortcut_path.to_string_lossy());
    let app_id_wide = to_wide_null(app_id);

    unsafe {
        shell_link
            .SetPath(PCWSTR(target_wide.as_ptr()))
            .map_err(|e| Win32ShellError::HResult {
                api: "IShellLinkW::SetPath",
                hr: e.code().0,
                message: e.message(),
            })?;
        shell_link
            .SetIconLocation(PCWSTR(icon_wide.as_ptr()), 0)
            .map_err(|e| Win32ShellError::HResult {
                api: "IShellLinkW::SetIconLocation",
                hr: e.code().0,
                message: e.message(),
            })?;
        shell_link
            .SetDescription(PCWSTR(desc_wide.as_ptr()))
            .map_err(|e| Win32ShellError::HResult {
                api: "IShellLinkW::SetDescription",
                hr: e.code().0,
                message: e.message(),
            })?;

        // PKEY_AppUserModel_ID = {9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3}, pid 5
        let pkey = PROPERTYKEY {
            fmtid: GUID::from_u128(0x9F4C2855_9F79_4B39_A8D0_E1D42DE1D5F3),
            pid: 5,
        };
        let byte_len = app_id_wide.len() * std::mem::size_of::<u16>();
        let mem = CoTaskMemAlloc(byte_len) as *mut u16;
        if mem.is_null() {
            return Err(Win32ShellError::Win32 {
                api: "CoTaskMemAlloc",
                code: 0,
            });
        }
        std::ptr::copy_nonoverlapping(app_id_wide.as_ptr(), mem, app_id_wide.len());

        let mut propvar = PROPVARIANT {
            Anonymous: PROPVARIANT_0 {
                Anonymous: std::mem::ManuallyDrop::new(PROPVARIANT_0_0 {
                    vt: VT_LPWSTR,
                    wReserved1: 0,
                    wReserved2: 0,
                    wReserved3: 0,
                    Anonymous: PROPVARIANT_0_0_0 {
                        pwszVal: PWSTR(mem),
                    },
                }),
            },
        };

        let store: IPropertyStore = shell_link.cast().map_err(|e| Win32ShellError::HResult {
            api: "IShellLinkW::cast(IPropertyStore)",
            hr: e.code().0,
            message: e.message(),
        })?;
        let set_res = store.SetValue(&pkey, &propvar);
        let _ = PropVariantClear(&mut propvar);
        set_res.map_err(|e| Win32ShellError::HResult {
            api: "IPropertyStore::SetValue",
            hr: e.code().0,
            message: e.message(),
        })?;
        store.Commit().map_err(|e| Win32ShellError::HResult {
            api: "IPropertyStore::Commit",
            hr: e.code().0,
            message: e.message(),
        })?;

        let persist: IPersistFile = shell_link.cast().map_err(|e| Win32ShellError::HResult {
            api: "IShellLinkW::cast(IPersistFile)",
            hr: e.code().0,
            message: e.message(),
        })?;
        persist
            .Save(PCWSTR(link_wide.as_ptr()), true)
            .map_err(|e| Win32ShellError::HResult {
                api: "IPersistFile::Save",
                hr: e.code().0,
                message: e.message(),
            })?;

        SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_FLUSH, None, None);
    }

    let _ = HSTRING::new();
    let _ = STGM_READWRITE;
    Ok(())
}

/// Default Start Menu Programs shortcut path (`%APPDATA%\Microsoft\Windows\Start Menu\Programs\Hybrid Player.lnk`).
pub fn default_start_menu_shortcut_path() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(
        PathBuf::from(appdata)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join(format!("{APP_NAME}.lnk")),
    )
}

/// Full Windows shell integration setup (`SetCurrentProcessExplicitAppUserModelID`,
/// HKCU registry keys, and optional Start Menu `.lnk`).
pub fn setup_windows_shell_integration(
    icon_ico_path: Option<&Path>,
    icon_png_path: Option<&Path>,
    create_shortcut: bool,
) -> Result<()> {
    let _ = set_current_process_app_user_model_id(APP_ID);

    if let Some(png_or_ico) = icon_png_path
        .filter(|p| p.exists())
        .or_else(|| icon_ico_path.filter(|p| p.exists()))
    {
        let _ = register_hkcu_app_user_model_id(APP_ID, APP_NAME, png_or_ico);
    }

    if create_shortcut {
        if let (Some(shortcut), Some(ico), Ok(exe)) = (
            default_start_menu_shortcut_path(),
            icon_ico_path.filter(|p| p.exists()),
            std::env::current_exe(),
        ) {
            let _ = create_start_menu_shortcut(&shortcut, &exe, ico, APP_NAME, APP_ID);
        }
    }

    Ok(())
}

// ─── Recycle Bin / Shell Trash Helper ────────────────────────────────────────

/// Move a file to the Windows Recycle Bin using `SHFileOperationW`
/// (`FO_DELETE`, `FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT`).
pub fn trash_file(path: &Path) -> Result<()> {
    if !path.exists() {
        return Err(Win32ShellError::Io(io::Error::new(
            io::ErrorKind::NotFound,
            format!("file not found: {}", path.display()),
        )));
    }

    // SHFileOperationW requires double-null-terminated string.
    let mut wide: Vec<u16> = path.to_string_lossy().encode_utf16().collect();
    wide.push(0);
    wide.push(0);

    let mut op = SHFILEOPSTRUCTW {
        hwnd: HWND::default(),
        wFunc: FO_DELETE,
        pFrom: PCWSTR(wide.as_ptr()),
        pTo: PCWSTR::null(),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT).0 as u16,
        fAnyOperationsAborted: Default::default(),
        hNameMappings: std::ptr::null_mut(),
        lpszProgressTitle: PCWSTR::null(),
    };

    let code = unsafe { SHFileOperationW(&mut op) };
    if code != 0 || op.fAnyOperationsAborted.as_bool() {
        return Err(Win32ShellError::Win32 {
            api: "SHFileOperationW(FO_DELETE)",
            code: code as u32,
        });
    }
    Ok(())
}

// ─── PID-Scoped Temp Cleanup ─────────────────────────────────────────────────

/// Check whether a Windows process with `pid` is currently alive.
pub fn is_process_running(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) };
    match handle {
        Ok(h) => {
            let mut exit_code = 0u32;
            let alive = unsafe { GetExitCodeProcess(h, &mut exit_code).is_ok() }
                && exit_code == STILL_ACTIVE.0 as u32;
            unsafe {
                let _ = CloseHandle(h);
            }
            alive
        }
        Err(_) => false,
    }
}

fn parse_pid_prefix(rest: &str) -> Option<u32> {
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let after = &rest[digits.len()..];
    if after.is_empty() || after.starts_with('-') || after.starts_with('_') {
        digits.parse::<u32>().ok().filter(|&p| p > 0)
    } else {
        None
    }
}

/// Clean up orphaned or own-PID temporary directories (`hybrid-player-thumbs-*`,
/// `hybrid-player-input-*`, `hybrid_screenshot_undo*`, `hybrid-player-paused-frames*`),
/// matching `main.js:2374-2444`.
pub fn cleanup_temp_files(temp_dir: &Path, current_pid: u32, is_shutting_down: bool) {
    let Ok(entries) = fs::read_dir(temp_dir) else {
        return;
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let full_path = entry.path();

        let should_clean_pid = |owner_pid: u32| -> bool {
            (is_shutting_down && owner_pid == current_pid)
                || (owner_pid != current_pid && !is_process_running(owner_pid))
        };

        if let Some(rest) = name
            .strip_prefix("hybrid-player-thumbs-")
            .or_else(|| name.strip_prefix("hybrid-player-input-"))
        {
            if let Some(owner_pid) = parse_pid_prefix(rest) {
                if should_clean_pid(owner_pid) {
                    let _ = fs::remove_dir_all(&full_path);
                }
            } else {
                let _ = fs::remove_dir_all(&full_path);
            }
        } else if name.starts_with("hybrid_screenshot_undo") {
            let pid_opt = name
                .strip_prefix("hybrid_screenshot_undo_")
                .or_else(|| name.strip_prefix("hybrid_screenshot_undo-"))
                .and_then(parse_pid_prefix);
            if let Some(owner_pid) = pid_opt {
                if should_clean_pid(owner_pid) {
                    if let Ok(children) = fs::read_dir(&full_path) {
                        for child in children.flatten() {
                            let child_path = child.path();
                            if trash_file(&child_path).is_err() {
                                let _ = fs::remove_file(&child_path);
                            }
                        }
                    }
                    if fs::remove_dir(&full_path).is_err() {
                        let _ = fs::remove_dir_all(&full_path);
                    }
                }
            }
        } else if name.starts_with("hybrid-player-paused-frames") {
            let pid_opt = name
                .strip_prefix("hybrid-player-paused-frames-")
                .or_else(|| name.strip_prefix("hybrid-player-paused-frames_"))
                .and_then(parse_pid_prefix);
            if let Some(owner_pid) = pid_opt {
                if should_clean_pid(owner_pid) {
                    let _ = fs::remove_dir_all(&full_path);
                }
            } else if let Ok(children) = fs::read_dir(&full_path) {
                for child in children.flatten() {
                    let child_name = child.file_name().to_string_lossy().to_string();
                    let child_path = child.path();
                    if let Some(child_pid) =
                        child_name.strip_prefix("pid-").and_then(parse_pid_prefix)
                    {
                        if should_clean_pid(child_pid) {
                            let _ = fs::remove_file(&child_path);
                        }
                    } else if !is_shutting_down {
                        if let Ok(meta) = fs::metadata(&child_path) {
                            if let Ok(modified) = meta.modified() {
                                if SystemTime::now()
                                    .duration_since(modified)
                                    .unwrap_or_default()
                                    > Duration::from_secs(60)
                                {
                                    let _ = fs::remove_file(&child_path);
                                }
                            }
                        }
                    }
                }
                let _ = fs::remove_dir(&full_path);
            }
        }
    }
}
