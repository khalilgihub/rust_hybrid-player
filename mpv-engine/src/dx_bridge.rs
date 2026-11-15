//! `WGL_NV_DX_interop2` hidden WGL 3.3 Core context + exact LUID-matched D3D11/DX12
//! triple-buffered zero-copy frame bridge.
//!
//! Synchronizes D3D11 -> DX12 (`wgpu`) via a shared `ID3D11Fence` / `ID3D12Fence`
//! NT handle (`ID3D11DeviceContext4::Signal` -> `ID3D12CommandQueue::Wait`), never
//! `IDXGIKeyedMutex`.

use std::collections::VecDeque;
use std::ffi::CStr;
use std::os::raw::{c_char, c_int, c_void};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows::core::{Interface, PCSTR, PCWSTR};
use windows::Win32::Foundation::{
    CloseHandle, GENERIC_ALL, HANDLE, HMODULE, HWND, LPARAM, LRESULT, LUID, WPARAM,
};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL,
    D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11Device5, ID3D11DeviceContext, ID3D11DeviceContext4,
    ID3D11Fence, ID3D11Texture2D, D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_FENCE_FLAG_SHARED, D3D11_RESOURCE_MISC_SHARED,
    D3D11_RESOURCE_MISC_SHARED_NTHANDLE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_DEFAULT,
};
use windows::Win32::Graphics::Direct3D12::{ID3D12Device, ID3D12Fence, ID3D12Resource};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter, IDXGIAdapter1, IDXGIFactory1, IDXGIResource1,
    DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_ERROR_NOT_FOUND, DXGI_SHARED_RESOURCE_READ,
    DXGI_SHARED_RESOURCE_WRITE,
};
use windows::Win32::Graphics::Gdi::{GetDC, ReleaseDC, HDC};
use windows::Win32::Graphics::OpenGL::{
    wglCreateContext, wglDeleteContext, wglGetCurrentContext, wglGetProcAddress, wglMakeCurrent,
    ChoosePixelFormat, SetPixelFormat, HGLRC, PFD_DOUBLEBUFFER, PFD_DRAW_TO_WINDOW, PFD_MAIN_PLANE,
    PFD_SUPPORT_OPENGL, PFD_TYPE_RGBA, PIXELFORMATDESCRIPTOR,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryA};
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForSingleObject, INFINITE};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, WINDOW_EX_STYLE, WNDCLASSW,
    WS_POPUP,
};

use crate::client::MpvClient;
use crate::error::{MpvError, Result};
use crate::ffi::{
    mpv_opengl_fbo, mpv_opengl_init_params, mpv_render_context, mpv_render_param,
    MPV_RENDER_API_TYPE_OPENGL, MPV_RENDER_PARAM_ADVANCED_CONTROL, MPV_RENDER_PARAM_API_TYPE,
    MPV_RENDER_PARAM_FLIP_Y, MPV_RENDER_PARAM_INVALID, MPV_RENDER_PARAM_OPENGL_FBO,
    MPV_RENDER_PARAM_OPENGL_INIT_PARAMS, MPV_RENDER_UPDATE_FRAME,
};

pub const NUM_BUFFERS: usize = 3;
const MAX_RECOVERY_ATTEMPTS: u32 = 10;

// ── OpenGL / WGL constants ──────────────────────────────────
const GL_FRAMEBUFFER: u32 = 0x8D40;
const GL_COLOR_ATTACHMENT0: u32 = 0x8CE0;
const GL_FRAMEBUFFER_COMPLETE: u32 = 0x8CD5;
const GL_TEXTURE_2D: u32 = 0x0DE1;
const GL_RGBA8: c_int = 0x8058;
const GL_TEXTURE_MIN_FILTER: u32 = 0x2801;
const GL_TEXTURE_MAG_FILTER: u32 = 0x2800;
const GL_TEXTURE_WRAP_S: u32 = 0x2802;
const GL_TEXTURE_WRAP_T: u32 = 0x2803;
const GL_CLAMP_TO_EDGE: c_int = 0x812F;
const GL_LINEAR: c_int = 0x2601;
const GL_VENDOR: u32 = 0x1F00;
const GL_RENDERER: u32 = 0x1F01;

const WGL_CONTEXT_MAJOR_VERSION_ARB: c_int = 0x2091;
const WGL_CONTEXT_MINOR_VERSION_ARB: c_int = 0x2092;
const WGL_CONTEXT_PROFILE_MASK_ARB: c_int = 0x9126;
const WGL_CONTEXT_CORE_PROFILE_BIT_ARB: c_int = 0x00000001;
const WGL_ACCESS_READ_WRITE_NV: u32 = 0x00000001;

type FnWglCreateContextAttribsArb =
    unsafe extern "system" fn(hdc: HDC, share: HGLRC, attrib_list: *const c_int) -> HGLRC;
type FnWglDxOpenDeviceNv = unsafe extern "system" fn(dx_device: *mut c_void) -> HANDLE;
type FnWglDxCloseDeviceNv = unsafe extern "system" fn(h_device: HANDLE) -> i32;
type FnWglDxRegisterObjectNv = unsafe extern "system" fn(
    h_device: HANDLE,
    dx_object: *mut c_void,
    name: u32,
    type_: u32,
    access: u32,
) -> HANDLE;
type FnWglDxUnregisterObjectNv =
    unsafe extern "system" fn(h_device: HANDLE, h_object: HANDLE) -> i32;
type FnWglDxLockObjectsNv =
    unsafe extern "system" fn(h_device: HANDLE, count: i32, objects: *mut HANDLE) -> i32;
type FnWglDxUnlockObjectsNv =
    unsafe extern "system" fn(h_device: HANDLE, count: i32, objects: *mut HANDLE) -> i32;

type FnGlGenFramebuffers = unsafe extern "system" fn(n: i32, framebuffers: *mut u32);
type FnGlBindFramebuffer = unsafe extern "system" fn(target: u32, framebuffer: u32);
type FnGlFramebufferTexture2D = unsafe extern "system" fn(
    target: u32,
    attachment: u32,
    textarget: u32,
    texture: u32,
    level: i32,
);
type FnGlCheckFramebufferStatus = unsafe extern "system" fn(target: u32) -> u32;
type FnGlDeleteFramebuffers = unsafe extern "system" fn(n: i32, framebuffers: *const u32);

type FnGlGenTextures = unsafe extern "system" fn(n: i32, textures: *mut u32);
type FnGlDeleteTextures = unsafe extern "system" fn(n: i32, textures: *const u32);
type FnGlBindTexture = unsafe extern "system" fn(target: u32, texture: u32);
type FnGlTexParameteri = unsafe extern "system" fn(target: u32, pname: u32, param: c_int);
type FnGlGetString = unsafe extern "system" fn(name: u32) -> *const u8;

struct GlProcs {
    wgl_dx_open_device: FnWglDxOpenDeviceNv,
    wgl_dx_close_device: FnWglDxCloseDeviceNv,
    wgl_dx_register_object: FnWglDxRegisterObjectNv,
    wgl_dx_unregister_object: FnWglDxUnregisterObjectNv,
    wgl_dx_lock_objects: FnWglDxLockObjectsNv,
    wgl_dx_unlock_objects: FnWglDxUnlockObjectsNv,

    gl_gen_framebuffers: FnGlGenFramebuffers,
    gl_bind_framebuffer: FnGlBindFramebuffer,
    gl_framebuffer_texture_2d: FnGlFramebufferTexture2D,
    gl_check_framebuffer_status: FnGlCheckFramebufferStatus,
    gl_delete_framebuffers: FnGlDeleteFramebuffers,

    gl_gen_textures: FnGlGenTextures,
    gl_delete_textures: FnGlDeleteTextures,
    gl_bind_texture: FnGlBindTexture,
    gl_tex_parameteri: FnGlTexParameteri,
    gl_get_string: FnGlGetString,
}

unsafe fn get_wgl_proc(name: &CStr) -> *mut c_void {
    let p = wglGetProcAddress(PCSTR(name.as_ptr() as *const u8));
    let ptr = p.map(|f| f as *mut c_void).unwrap_or(std::ptr::null_mut());
    let addr = ptr as usize;
    if ptr.is_null() || addr == 1 || addr == 2 || addr == 3 || addr == usize::MAX {
        if let Ok(h_gl) = LoadLibraryA(PCSTR(c"opengl32.dll".as_ptr() as *const u8)) {
            if let Some(f) = GetProcAddress(h_gl, PCSTR(name.as_ptr() as *const u8)) {
                return f as *mut c_void;
            }
        }
        return std::ptr::null_mut();
    }
    ptr
}

unsafe extern "C" fn mpv_get_gl_proc_address(
    _ctx: *mut c_void,
    name: *const c_char,
) -> *mut c_void {
    if name.is_null() {
        return std::ptr::null_mut();
    }
    get_wgl_proc(CStr::from_ptr(name))
}

unsafe fn load_proc<T>(name: &'static [u8]) -> Result<T> {
    let cstr = CStr::from_bytes_with_nul(name)
        .map_err(|_| MpvError::Wgl("invalid proc name".to_owned()))?;
    let p = get_wgl_proc(cstr);
    if p.is_null() {
        return Err(MpvError::Wgl(format!(
            "missing OpenGL/WGL procedure: {}",
            cstr.to_string_lossy()
        )));
    }
    Ok(std::mem::transmute_copy(&p))
}

impl GlProcs {
    unsafe fn load() -> Result<Self> {
        Ok(Self {
            wgl_dx_open_device: load_proc(b"wglDXOpenDeviceNV\0")?,
            wgl_dx_close_device: load_proc(b"wglDXCloseDeviceNV\0")?,
            wgl_dx_register_object: load_proc(b"wglDXRegisterObjectNV\0")?,
            wgl_dx_unregister_object: load_proc(b"wglDXUnregisterObjectNV\0")?,
            wgl_dx_lock_objects: load_proc(b"wglDXLockObjectsNV\0")?,
            wgl_dx_unlock_objects: load_proc(b"wglDXUnlockObjectsNV\0")?,
            gl_gen_framebuffers: load_proc(b"glGenFramebuffers\0")?,
            gl_bind_framebuffer: load_proc(b"glBindFramebuffer\0")?,
            gl_framebuffer_texture_2d: load_proc(b"glFramebufferTexture2D\0")?,
            gl_check_framebuffer_status: load_proc(b"glCheckFramebufferStatus\0")?,
            gl_delete_framebuffers: load_proc(b"glDeleteFramebuffers\0")?,
            gl_gen_textures: load_proc(b"glGenTextures\0")?,
            gl_delete_textures: load_proc(b"glDeleteTextures\0")?,
            gl_bind_texture: load_proc(b"glBindTexture\0")?,
            gl_tex_parameteri: load_proc(b"glTexParameteri\0")?,
            gl_get_string: load_proc(b"glGetString\0")?,
        })
    }
}

/// Public snapshot of an imported `wgpu::Texture` ready for sampling in Pass 0.
#[derive(Clone)]
pub struct ImportedVideoFrame {
    pub texture: Arc<wgpu::Texture>,
    pub view: Arc<wgpu::TextureView>,
    pub width: u32,
    pub height: u32,
    pub generation: u64,
    pub buffer_index: usize,
    pub fence_value: u64,
}

#[derive(Default)]
struct BufferSlot {
    d3d_render_texture: Option<ID3D11Texture2D>,
    d3d_shared_texture: Option<ID3D11Texture2D>,
    nt_handle: isize,
    gl_texture: u32,
    gl_fbo: u32,
    gl_registered_obj: isize,
    wgpu_texture: Option<Arc<wgpu::Texture>>,
    wgpu_view: Option<Arc<wgpu::TextureView>>,
    /// Fence value signaled by `wgpu` when it finished reading this slot.
    wgpu_done_fence: u64,
    in_use: bool,
}

struct RetiredSlot {
    d3d_shared_texture: Option<ID3D11Texture2D>,
    nt_handle: isize,
    wgpu_texture: Option<Arc<wgpu::Texture>>,
    wgpu_view: Option<Arc<wgpu::TextureView>>,
    generation: u64,
    index: usize,
}

struct BridgeWakeEvent(isize);
unsafe impl Send for BridgeWakeEvent {}
unsafe impl Sync for BridgeWakeEvent {}

impl BridgeWakeEvent {
    fn new() -> Result<Arc<Self>> {
        let h = unsafe { CreateEventW(None, false, false, None) }
            .map_err(|e| MpvError::D3dInterop(format!("CreateEventW for DxBridge failed: {e}")))?;
        Ok(Arc::new(Self(h.0 as isize)))
    }

    #[inline]
    fn handle(&self) -> HANDLE {
        HANDLE(self.0 as *mut c_void)
    }

    #[inline]
    fn signal(&self) {
        unsafe {
            let _ = SetEvent(self.handle());
        }
    }
}

impl Drop for BridgeWakeEvent {
    fn drop(&mut self) {
        if self.0 != 0 {
            unsafe {
                let _ = CloseHandle(self.handle());
            }
            self.0 = 0;
        }
    }
}

unsafe extern "C" fn on_mpv_render_update(ctx: *mut c_void) {
    if ctx.is_null() {
        return;
    }
    let ev = &*(ctx as *const BridgeWakeEvent);
    ev.signal();
}

struct SharedState {
    slots: [BufferSlot; NUM_BUFFERS],
    retired: VecDeque<RetiredSlot>,
    latest_frame: Option<ImportedVideoFrame>,
    /// Retained VRAM frame kept alive across seeks/switches until replaced or cleared.
    retained_frame: Option<ImportedVideoFrame>,
    generation: u64,
    current_width: u32,
    current_height: u32,
    buffers_allocated: bool,
    d3d12_fence: Option<ID3D12Fence>,
    next_fence_value: u64,
    wgpu_next_fence_value: u64,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            slots: [
                BufferSlot::default(),
                BufferSlot::default(),
                BufferSlot::default(),
            ],
            retired: VecDeque::new(),
            latest_frame: None,
            retained_frame: None,
            generation: 1,
            current_width: 0,
            current_height: 0,
            buffers_allocated: false,
            d3d12_fence: None,
            next_fence_value: 1,
            wgpu_next_fence_value: 1,
        }
    }
}

/// Optional callback invoked on the render thread when a new video frame is signaled.
pub type FrameReadyCallback = Arc<dyn Fn(&ImportedVideoFrame) + Send + Sync>;

/// Offscreen `WGL_NV_DX_interop2` + D3D11/DX12 zero-copy bridge for `libmpv`.
pub struct DxBridge {
    _mpv_client: Arc<MpvClient>,
    wgpu_device: Arc<wgpu::Device>,
    wgpu_queue: Arc<wgpu::Queue>,
    adapter_info: wgpu::AdapterInfo,
    wake_event: Arc<BridgeWakeEvent>,
    running: Arc<AtomicBool>,
    needs_recreate: Arc<AtomicBool>,
    pending_resize: Arc<AtomicU64>,
    flip_y: Arc<AtomicBool>,
    shared: Arc<(Mutex<SharedState>, Condvar)>,
    render_thread: Mutex<Option<JoinHandle<()>>>,
}

struct RenderThreadContext {
    mpv_client: Arc<MpvClient>,
    wgpu_device: Arc<wgpu::Device>,
    d3d12_device: ID3D12Device,
    adapter_info: wgpu::AdapterInfo,
    wake_event: Arc<BridgeWakeEvent>,
    wake_raw: *const BridgeWakeEvent,
    running: Arc<AtomicBool>,
    needs_recreate: Arc<AtomicBool>,
    pending_resize: Arc<AtomicU64>,
    flip_y: Arc<AtomicBool>,
    shared: Arc<(Mutex<SharedState>, Condvar)>,
    on_frame: Option<FrameReadyCallback>,

    // Thread-local Win32 / WGL / D3D11 handles (never accessed off render thread)
    hwnd: HWND,
    hdc: HDC,
    hglrc: HGLRC,
    procs: Option<GlProcs>,

    d3d11_device: Option<ID3D11Device5>,
    d3d11_context: Option<ID3D11DeviceContext4>,
    d3d11_fence: Option<ID3D11Fence>,
    gl_hdevice: isize,
    mpv_gl_ctx: *mut mpv_render_context,
}

unsafe impl Send for RenderThreadContext {}

impl DxBridge {
    /// Creates and starts the offscreen `WGL_NV_DX_interop2` + D3D11/DX12 bridge.
    ///
    /// `wgpu_device` MUST be backed by the `wgpu_hal::api::Dx12` backend.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        mpv_client: Arc<MpvClient>,
        wgpu_device: Arc<wgpu::Device>,
        wgpu_queue: Arc<wgpu::Queue>,
        adapter: &wgpu::Adapter,
        initial_width: u32,
        initial_height: u32,
        flip_y: bool,
        on_frame: Option<FrameReadyCallback>,
    ) -> Result<Self> {
        if mpv_client.is_subprocess() {
            return Err(MpvError::D3dInterop(
                "DxBridge is unused when running mpv in external --wid subprocess mode".to_owned(),
            ));
        }
        let adapter_info = adapter.get_info();
        let d3d12_device = unsafe {
            wgpu_device
                .as_hal::<wgpu_hal::api::Dx12>()
                .map(|hal_dev| hal_dev.raw_device().clone())
        }
        .ok_or_else(|| {
            MpvError::D3dInterop("wgpu::Device is not backed by wgpu_hal::api::Dx12".to_owned())
        })?;

        let wake_event = BridgeWakeEvent::new()?;
        let wake_raw = Arc::into_raw(wake_event.clone());
        let running = Arc::new(AtomicBool::new(true));
        let needs_recreate = Arc::new(AtomicBool::new(false));
        let w = initial_width.max(1);
        let h = initial_height.max(1);
        let pending_resize = Arc::new(AtomicU64::new(((w as u64) << 32) | (h as u64)));
        let flip_y_atom = Arc::new(AtomicBool::new(flip_y));
        let shared = Arc::new((Mutex::new(SharedState::default()), Condvar::new()));

        let (init_tx, init_rx) = std::sync::mpsc::sync_channel::<Result<()>>(1);

        let mut ctx = RenderThreadContext {
            mpv_client: mpv_client.clone(),
            wgpu_device: wgpu_device.clone(),
            d3d12_device,
            adapter_info: adapter_info.clone(),
            wake_event: wake_event.clone(),
            wake_raw,
            running: running.clone(),
            needs_recreate: needs_recreate.clone(),
            pending_resize: pending_resize.clone(),
            flip_y: flip_y_atom.clone(),
            shared: shared.clone(),
            on_frame,
            hwnd: HWND::default(),
            hdc: HDC::default(),
            hglrc: HGLRC::default(),
            procs: None,
            d3d11_device: None,
            d3d11_context: None,
            d3d11_fence: None,
            gl_hdevice: 0,
            mpv_gl_ctx: std::ptr::null_mut(),
        };

        let handle = thread::Builder::new()
            .name("mpv-dx-interop".to_owned())
            .spawn(move || {
                match ctx.initialize(w, h) {
                    Ok(()) => {
                        let _ = init_tx.send(Ok(()));
                        ctx.render_loop();
                    }
                    Err(e) => {
                        let _ = init_tx.send(Err(e));
                    }
                }
                ctx.teardown();
            })
            .map_err(|e| MpvError::Wgl(format!("failed to spawn dx_bridge render thread: {e}")))?;

        match init_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                running.store(false, Ordering::Release);
                wake_event.signal();
                let _ = handle.join();
                return Err(e);
            }
            Err(_) => {
                running.store(false, Ordering::Release);
                wake_event.signal();
                let _ = handle.join();
                return Err(MpvError::Wgl(
                    "render thread terminated during init".to_owned(),
                ));
            }
        }

        Ok(Self {
            _mpv_client: mpv_client,
            wgpu_device,
            wgpu_queue,
            adapter_info,
            wake_event,
            running,
            needs_recreate,
            pending_resize,
            flip_y: flip_y_atom,
            shared,
            render_thread: Mutex::new(Some(handle)),
        })
    }

    pub fn adapter_info(&self) -> &wgpu::AdapterInfo {
        &self.adapter_info
    }

    pub fn set_flip_y(&self, flip: bool) {
        self.flip_y.store(flip, Ordering::Release);
        self.wake_event.signal();
    }

    /// Flags the render thread to rebuild the D3D11/WGL interop device and surfaces.
    pub fn request_recreate(&self) {
        self.needs_recreate.store(true, Ordering::Release);
        self.wake_event.signal();
    }

    /// Requests an asynchronous texture reallocation on the render thread.
    pub fn resize(&self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        let packed = ((width as u64) << 32) | (height as u64);
        self.pending_resize.store(packed, Ordering::Release);
        self.wake_event.signal();
    }

    /// Clears the retained frame (e.g., when stopping playback to return to Welcome screen).
    pub fn clear_retained_frame(&self) {
        let (lock, cvar) = &*self.shared;
        let mut st = lock.lock().unwrap();
        st.latest_frame = None;
        st.retained_frame = None;
        for slot in &mut st.slots {
            slot.in_use = false;
        }
        cvar.notify_all();
    }

    /// Acquires the newest rendered frame (or falls back to the VRAM retained frame
    /// across seeks/switches) and stages `ID3D12CommandQueue::Wait` on `wgpu::Queue`
    /// before the caller's `queue.submit(...)`!
    ///
    /// Returns `(frame, is_new_frame)`. Call [`Self::finish_frame_use`] after
    /// `wgpu::Queue::submit` so the fence signal is recorded and the slot is released
    /// once the GPU finishes reading it.
    pub fn acquire_frame_for_submit(&self) -> Option<(ImportedVideoFrame, bool)> {
        let (lock, _cvar) = &*self.shared;
        let mut st = lock.lock().unwrap();

        let (frame, is_new) = if let Some(new_frame) = st.latest_frame.take() {
            // Release any previously retained slot if it differs from the new frame
            if let Some(prev) = st.retained_frame.take() {
                if prev.generation == st.generation && prev.buffer_index != new_frame.buffer_index {
                    st.slots[prev.buffer_index].in_use = false;
                }
            }
            st.retained_frame = Some(new_frame.clone());
            (new_frame, true)
        } else {
            let retained = st.retained_frame.clone()?;
            (retained, false)
        };

        if let Some(d3d12_fence) = st.d3d12_fence.clone() {
            if is_new {
                unsafe {
                    if let Some(hal_queue) = self.wgpu_queue.as_hal::<wgpu_hal::api::Dx12>() {
                        hal_queue.add_wait_fence(d3d12_fence, frame.fence_value);
                    }
                }
            }
        }

        Some((frame, is_new))
    }

    /// Stages a post-submit `ID3D12CommandQueue::Signal` on `wgpu::Queue` (call BEFORE
    /// `queue.submit`) OR call this right before `queue.submit` so `wgpu` signals the
    /// shared fence when done sampling `frame`.
    pub fn stage_wgpu_done_signal(&self, frame: &ImportedVideoFrame) {
        let (lock, _cvar) = &*self.shared;
        let mut st = lock.lock().unwrap();
        if let Some(d3d12_fence) = st.d3d12_fence.clone() {
            st.wgpu_next_fence_value = st.wgpu_next_fence_value.saturating_add(1);
            let done_val = st.wgpu_next_fence_value;
            if frame.generation == st.generation && frame.buffer_index < NUM_BUFFERS {
                st.slots[frame.buffer_index].wgpu_done_fence = done_val;
            }
            unsafe {
                if let Some(hal_queue) = self.wgpu_queue.as_hal::<wgpu_hal::api::Dx12>() {
                    hal_queue.add_signal_fence(d3d12_fence, done_val);
                }
            }
        }
    }

    /// Releases a buffer slot explicitly (e.g. if a consumer chose not to retain it).
    pub fn release_buffer(&self, index: usize, generation: u64) {
        if index >= NUM_BUFFERS {
            return;
        }
        let (lock, cvar) = &*self.shared;
        let mut st = lock.lock().unwrap();
        if generation != st.generation {
            st.retired
                .retain(|r| !(r.index == index && r.generation == generation));
            return;
        }
        st.slots[index].in_use = false;
        cvar.notify_all();
        self.wake_event.signal();
    }

    pub fn current_dimensions(&self) -> (u32, u32) {
        let (lock, _) = &*self.shared;
        let st = lock.lock().unwrap();
        (st.current_width, st.current_height)
    }

    pub fn device(&self) -> &Arc<wgpu::Device> {
        &self.wgpu_device
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }
}

impl Drop for DxBridge {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        self.wake_event.signal();
        {
            let (_, cvar) = &*self.shared;
            cvar.notify_all();
        }
        if let Some(jh) = self.render_thread.lock().unwrap().take() {
            let _ = jh.join();
        }
    }
}

impl RenderThreadContext {
    fn initialize(&mut self, initial_w: u32, initial_h: u32) -> Result<()> {
        self.init_gl()?;
        self.init_d3d11()?;
        unsafe {
            let _ = wglMakeCurrent(self.hdc, self.hglrc);
        }
        let alloc_ok = self.allocate_buffers(initial_w, initial_h);
        unsafe {
            let _ = wglMakeCurrent(HDC::default(), HGLRC::default());
        }
        if !alloc_ok {
            return Err(MpvError::D3dInterop(
                "initial allocate_buffers failed".to_owned(),
            ));
        }
        self.init_mpv_render()?;
        Ok(())
    }

    fn init_gl(&mut self) -> Result<()> {
        unsafe {
            let h_inst = GetModuleHandleW(PCWSTR::null())
                .map_err(|e| MpvError::Wgl(format!("GetModuleHandleW: {e}")))?;

            let class_name: Vec<u16> = "HybridDxBridgeGLDummy\0".encode_utf16().collect();
            let title: Vec<u16> = "Dummy\0".encode_utf16().collect();

            unsafe extern "system" fn dummy_wnd_proc(
                hwnd: HWND,
                msg: u32,
                wparam: WPARAM,
                lparam: LPARAM,
            ) -> LRESULT {
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }

            let wc = WNDCLASSW {
                lpfnWndProc: Some(dummy_wnd_proc),
                hInstance: h_inst.into(),
                lpszClassName: PCWSTR(class_name.as_ptr()),
                ..Default::default()
            };
            let _ = RegisterClassW(&wc);

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(class_name.as_ptr()),
                PCWSTR(title.as_ptr()),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(h_inst.into()),
                None,
            )
            .map_err(|e| MpvError::Wgl(format!("CreateWindowExW failed: {e}")))?;

            let hdc = GetDC(Some(hwnd));
            if hdc.is_invalid() {
                let _ = DestroyWindow(hwnd);
                return Err(MpvError::Wgl("GetDC failed".to_owned()));
            }

            let pfd = PIXELFORMATDESCRIPTOR {
                nSize: std::mem::size_of::<PIXELFORMATDESCRIPTOR>() as u16,
                nVersion: 1,
                dwFlags: PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER,
                iPixelType: PFD_TYPE_RGBA,
                cColorBits: 32,
                cDepthBits: 24,
                cStencilBits: 8,
                iLayerType: PFD_MAIN_PLANE.0 as u8,
                ..Default::default()
            };

            let fmt = ChoosePixelFormat(hdc, &pfd);
            if fmt == 0 || SetPixelFormat(hdc, fmt, &pfd).is_err() {
                let _ = ReleaseDC(Some(hwnd), hdc);
                let _ = DestroyWindow(hwnd);
                return Err(MpvError::Wgl(
                    "ChoosePixelFormat/SetPixelFormat failed".to_owned(),
                ));
            }

            let temp_ctx = wglCreateContext(hdc).map_err(|e| {
                let _ = ReleaseDC(Some(hwnd), hdc);
                let _ = DestroyWindow(hwnd);
                MpvError::Wgl(format!("wglCreateContext failed: {e}"))
            })?;

            if wglMakeCurrent(hdc, temp_ctx).is_err() {
                let _ = wglDeleteContext(temp_ctx);
                let _ = ReleaseDC(Some(hwnd), hdc);
                let _ = DestroyWindow(hwnd);
                return Err(MpvError::Wgl("wglMakeCurrent(temp) failed".to_owned()));
            }

            let mut final_ctx = temp_ctx;
            let create_attribs_ptr = get_wgl_proc(c"wglCreateContextAttribsARB");
            if !create_attribs_ptr.is_null() {
                let wgl_create_attribs: FnWglCreateContextAttribsArb =
                    std::mem::transmute_copy(&create_attribs_ptr);
                let attribs = [
                    WGL_CONTEXT_MAJOR_VERSION_ARB,
                    3,
                    WGL_CONTEXT_MINOR_VERSION_ARB,
                    3,
                    WGL_CONTEXT_PROFILE_MASK_ARB,
                    WGL_CONTEXT_CORE_PROFILE_BIT_ARB,
                    0,
                ];
                let core_ctx = wgl_create_attribs(hdc, HGLRC::default(), attribs.as_ptr());
                if !core_ctx.is_invalid() {
                    let _ = wglMakeCurrent(HDC::default(), HGLRC::default());
                    let _ = wglDeleteContext(temp_ctx);
                    final_ctx = core_ctx;
                    let _ = wglMakeCurrent(hdc, final_ctx);
                }
            }

            let procs = GlProcs::load().inspect_err(|_| {
                let _ = wglMakeCurrent(HDC::default(), HGLRC::default());
                let _ = wglDeleteContext(final_ctx);
                let _ = ReleaseDC(Some(hwnd), hdc);
                let _ = DestroyWindow(hwnd);
            })?;

            let vendor_ptr = (procs.gl_get_string)(GL_VENDOR);
            let renderer_ptr = (procs.gl_get_string)(GL_RENDERER);
            let vendor = if vendor_ptr.is_null() {
                "unknown".into()
            } else {
                CStr::from_ptr(vendor_ptr as *const c_char).to_string_lossy()
            };
            let renderer = if renderer_ptr.is_null() {
                "unknown".into()
            } else {
                CStr::from_ptr(renderer_ptr as *const c_char).to_string_lossy()
            };
            eprintln!("[mpv-dx-bridge] OpenGL Vendor: {vendor}, Renderer: {renderer}");

            let _ = wglMakeCurrent(HDC::default(), HGLRC::default());

            self.hwnd = hwnd;
            self.hdc = hdc;
            self.hglrc = final_ctx;
            self.procs = Some(procs);
            Ok(())
        }
    }

    fn init_d3d11(&mut self) -> Result<()> {
        unsafe {
            let _ = wglMakeCurrent(self.hdc, self.hglrc);
            let target_luid: LUID = self.d3d12_device.GetAdapterLuid();

            let procs = self
                .procs
                .as_ref()
                .ok_or_else(|| MpvError::Wgl("GL procs not loaded".to_owned()))?;
            let gl_vendor_ptr = (procs.gl_get_string)(GL_VENDOR);
            let gl_vendor = if gl_vendor_ptr.is_null() {
                String::new()
            } else {
                CStr::from_ptr(gl_vendor_ptr as *const c_char)
                    .to_string_lossy()
                    .into_owned()
            };

            let factory: IDXGIFactory1 = CreateDXGIFactory1()
                .map_err(|e| MpvError::D3dInterop(format!("CreateDXGIFactory1 failed: {e}")))?;

            let mut exact_luid_adapters: Vec<IDXGIAdapter1> = Vec::new();
            let mut vendor_matched: Vec<IDXGIAdapter1> = Vec::new();
            let mut fallback: Vec<IDXGIAdapter1> = Vec::new();

            let mut idx = 0u32;
            loop {
                match factory.EnumAdapters1(idx) {
                    Ok(adapter) => {
                        if let Ok(desc) = adapter.GetDesc1() {
                            if (desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0 {
                                idx += 1;
                                continue;
                            }
                            if desc.AdapterLuid.LowPart == target_luid.LowPart
                                && desc.AdapterLuid.HighPart == target_luid.HighPart
                            {
                                exact_luid_adapters.push(adapter);
                            } else if (desc.VendorId == self.adapter_info.vendor
                                && desc.DeviceId == self.adapter_info.device)
                                || is_gl_vendor_match(&gl_vendor, desc.VendorId)
                            {
                                vendor_matched.push(adapter);
                            } else {
                                fallback.push(adapter);
                            }
                        }
                        idx += 1;
                    }
                    Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
                    Err(_) => break,
                }
            }

            let feature_levels = [
                D3D_FEATURE_LEVEL_11_1,
                D3D_FEATURE_LEVEL_11_0,
                D3D_FEATURE_LEVEL_10_1,
                D3D_FEATURE_LEVEL_10_0,
            ];

            let mut opened = false;
            let try_adapter = |this: &mut Self,
                               adapter: Option<&IDXGIAdapter>,
                               drv_type: D3D_DRIVER_TYPE|
             -> bool {
                let mut dev: Option<ID3D11Device> = None;
                let mut ctx: Option<ID3D11DeviceContext> = None;
                let mut fl = D3D_FEATURE_LEVEL::default();

                let mut hr = D3D11CreateDevice(
                    adapter,
                    drv_type,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    Some(&feature_levels),
                    D3D11_SDK_VERSION,
                    Some(&mut dev),
                    Some(&mut fl),
                    Some(&mut ctx),
                );
                if hr.is_err() {
                    hr = D3D11CreateDevice(
                        adapter,
                        drv_type,
                        HMODULE::default(),
                        D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                        Some(&feature_levels[1..]),
                        D3D11_SDK_VERSION,
                        Some(&mut dev),
                        Some(&mut fl),
                        Some(&mut ctx),
                    );
                }
                if hr.is_err() {
                    return false;
                }
                let (Some(dev), Some(ctx)) = (dev, ctx) else {
                    return false;
                };
                let Ok(dev5) = dev.cast::<ID3D11Device5>() else {
                    return false;
                };
                let Ok(ctx4) = ctx.cast::<ID3D11DeviceContext4>() else {
                    return false;
                };

                let Some(procs) = this.procs.as_ref() else {
                    return false;
                };
                let h_gl_dev = (procs.wgl_dx_open_device)(dev.as_raw());
                if h_gl_dev.is_invalid() {
                    return false;
                }

                // Create shared ID3D11Fence -> NT handle -> ID3D12Fence (no IDXGIKeyedMutex)
                let mut fence_opt: Option<ID3D11Fence> = None;
                let fence: ID3D11Fence = match dev5
                    .CreateFence(0, D3D11_FENCE_FLAG_SHARED, &mut fence_opt)
                    .ok()
                    .and(fence_opt)
                {
                    Some(f) => f,
                    None => {
                        let _ = (procs.wgl_dx_close_device)(h_gl_dev);
                        return false;
                    }
                };
                let fence_nt_handle = match fence.CreateSharedHandle(None, GENERIC_ALL.0, None) {
                    Ok(h) => h,
                    Err(_) => {
                        let _ = (procs.wgl_dx_close_device)(h_gl_dev);
                        return false;
                    }
                };
                let mut d3d12_fence_opt: Option<ID3D12Fence> = None;
                let d3d12_fence_res = this
                    .d3d12_device
                    .OpenSharedHandle(fence_nt_handle, &mut d3d12_fence_opt);
                let _ = CloseHandle(fence_nt_handle);
                let Some(d3d12_fence) = d3d12_fence_res.ok().and(d3d12_fence_opt) else {
                    let _ = (procs.wgl_dx_close_device)(h_gl_dev);
                    return false;
                };

                {
                    let (lock, _) = &*this.shared;
                    let mut st = lock.lock().unwrap();
                    st.d3d12_fence = Some(d3d12_fence);
                }

                this.d3d11_device = Some(dev5);
                this.d3d11_context = Some(ctx4);
                this.d3d11_fence = Some(fence);
                this.gl_hdevice = h_gl_dev.0 as isize;
                true
            };

            for a in exact_luid_adapters
                .iter()
                .chain(vendor_matched.iter())
                .chain(fallback.iter())
            {
                if let Ok(base_adapter) = a.cast::<IDXGIAdapter>() {
                    if try_adapter(self, Some(&base_adapter), D3D_DRIVER_TYPE_UNKNOWN) {
                        opened = true;
                        break;
                    }
                }
            }

            if !opened {
                opened = try_adapter(self, None, D3D_DRIVER_TYPE_HARDWARE);
            }

            let _ = wglMakeCurrent(HDC::default(), HGLRC::default());

            if !opened || self.gl_hdevice == 0 {
                return Err(MpvError::D3dInterop(
                    "failed to create LUID-matched D3D11 device or open WGL_NV_DX_interop2 device"
                        .to_owned(),
                ));
            }
            Ok(())
        }
    }

    fn allocate_buffers(&mut self, width: u32, height: u32) -> bool {
        let target_w = width.max(1);
        let target_h = height.max(1);

        // Wait briefly for in-flight buffers before retiring/reallocating
        {
            let (lock, cvar) = &*self.shared;
            let guard = lock.lock().unwrap();
            let _ = cvar.wait_timeout_while(guard, Duration::from_millis(500), |st| {
                st.slots.iter().any(|s| s.in_use)
            });
        }

        let Some(d3d_dev) = self.d3d11_device.clone() else {
            return false;
        };
        if self.gl_hdevice == 0 {
            return false;
        }
        let gl_hdev = HANDLE(self.gl_hdevice as *mut c_void);

        let (old_gen, d3d12_dev) = {
            let (lock, _) = &*self.shared;
            let mut st = lock.lock().unwrap();
            let old_gen = st.generation;
            for i in 0..NUM_BUFFERS {
                let mut slot = std::mem::take(&mut st.slots[i]);
                Self::cleanup_slot(
                    &mut slot,
                    i,
                    old_gen,
                    true,
                    &mut st.retired,
                    self.procs.as_ref(),
                    gl_hdev,
                    self.hdc,
                    self.hglrc,
                );
            }
            st.buffers_allocated = false;
            st.latest_frame = None;
            st.generation = st.generation.wrapping_add(1);
            (old_gen, self.d3d12_device.clone())
        };
        let _ = old_gen;

        let mut new_slots: [BufferSlot; NUM_BUFFERS] = [
            BufferSlot::default(),
            BufferSlot::default(),
            BufferSlot::default(),
        ];

        for slot_out in &mut new_slots {
            let render_desc = D3D11_TEXTURE2D_DESC {
                Width: target_w,
                Height: target_h,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };

            let mut render_tex: Option<ID3D11Texture2D> = None;
            if let Err(e) =
                unsafe { d3d_dev.CreateTexture2D(&render_desc, None, Some(&mut render_tex)) }
            {
                eprintln!("[mpv-dx-bridge] CreateTexture2D(render) failed: {e}");
                return false;
            }
            let Some(render_tex) = render_tex else {
                return false;
            };

            let mut shared_desc = render_desc;
            shared_desc.MiscFlags =
                (D3D11_RESOURCE_MISC_SHARED.0 | D3D11_RESOURCE_MISC_SHARED_NTHANDLE.0) as u32;

            let mut shared_tex: Option<ID3D11Texture2D> = None;
            if let Err(e) =
                unsafe { d3d_dev.CreateTexture2D(&shared_desc, None, Some(&mut shared_tex)) }
            {
                eprintln!("[mpv-dx-bridge] CreateTexture2D(shared) failed: {e}");
                return false;
            }
            let Some(shared_tex) = shared_tex else {
                return false;
            };

            let Ok(dxgi_res) = shared_tex.cast::<IDXGIResource1>() else {
                eprintln!("[mpv-dx-bridge] shared_tex.cast::<IDXGIResource1>() failed");
                return false;
            };
            let nt_handle = match unsafe {
                dxgi_res.CreateSharedHandle(
                    None,
                    DXGI_SHARED_RESOURCE_READ.0 | DXGI_SHARED_RESOURCE_WRITE.0,
                    None,
                )
            } {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("[mpv-dx-bridge] CreateSharedHandle(shared_tex) failed: {e}");
                    return false;
                }
            };

            let mut d3d12_res_opt: Option<ID3D12Resource> = None;
            let d3d12_res: ID3D12Resource =
                match unsafe { d3d12_dev.OpenSharedHandle(nt_handle, &mut d3d12_res_opt) } {
                    Ok(()) => match d3d12_res_opt {
                        Some(r) => r,
                        None => {
                            unsafe {
                                let _ = CloseHandle(nt_handle);
                            }
                            eprintln!("[mpv-dx-bridge] OpenSharedHandle returned None");
                            return false;
                        }
                    },
                    Err(e) => {
                        unsafe {
                            let _ = CloseHandle(nt_handle);
                        }
                        eprintln!("[mpv-dx-bridge] d3d12 OpenSharedHandle(tex) failed: {e}");
                        return false;
                    }
                };

            let hal_tex = unsafe {
                wgpu_hal::dx12::Device::texture_from_raw(
                    d3d12_res,
                    wgpu::TextureFormat::Rgba8Unorm,
                    wgpu::TextureDimension::D2,
                    wgpu::Extent3d {
                        width: target_w,
                        height: target_h,
                        depth_or_array_layers: 1,
                    },
                    1,
                    1,
                )
            };
            let wgpu_tex = Arc::new(unsafe {
                self.wgpu_device
                    .create_texture_from_hal::<wgpu_hal::api::Dx12>(
                        hal_tex,
                        &wgpu::TextureDescriptor {
                            label: Some("mpv_dx12_shared_frame"),
                            size: wgpu::Extent3d {
                                width: target_w,
                                height: target_h,
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: wgpu::TextureFormat::Rgba8Unorm,
                            usage: wgpu::TextureUsages::TEXTURE_BINDING
                                | wgpu::TextureUsages::COPY_SRC,
                            view_formats: &[],
                        },
                        wgpu::TextureUses::UNINITIALIZED,
                    )
            });
            let wgpu_view = Arc::new(wgpu_tex.create_view(&wgpu::TextureViewDescriptor::default()));

            let Some(procs) = self.procs.as_ref() else {
                unsafe {
                    let _ = CloseHandle(nt_handle);
                }
                return false;
            };

            let mut gl_tex = 0u32;
            let mut gl_fbo = 0u32;
            let reg_obj: HANDLE;
            unsafe {
                (procs.gl_gen_textures)(1, &mut gl_tex);
                (procs.gl_bind_texture)(GL_TEXTURE_2D, gl_tex);
                (procs.gl_tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
                (procs.gl_tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
                (procs.gl_tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
                (procs.gl_tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
                (procs.gl_bind_texture)(GL_TEXTURE_2D, 0);

                reg_obj = (procs.wgl_dx_register_object)(
                    gl_hdev,
                    render_tex.as_raw(),
                    gl_tex,
                    GL_TEXTURE_2D,
                    WGL_ACCESS_READ_WRITE_NV,
                );
                if reg_obj.is_invalid() {
                    eprintln!("[mpv-dx-bridge] wglDXRegisterObjectNV returned invalid handle");
                    (procs.gl_delete_textures)(1, &gl_tex);
                    let _ = CloseHandle(nt_handle);
                    return false;
                }

                let mut reg_arr = [reg_obj];
                let locked = (procs.wgl_dx_lock_objects)(gl_hdev, 1, reg_arr.as_mut_ptr()) != 0;
                if !locked {
                    eprintln!("[mpv-dx-bridge] wglDXLockObjectsNV failed during FBO attach");
                    let _ = (procs.wgl_dx_unregister_object)(gl_hdev, reg_obj);
                    (procs.gl_delete_textures)(1, &gl_tex);
                    let _ = CloseHandle(nt_handle);
                    return false;
                }

                (procs.gl_gen_framebuffers)(1, &mut gl_fbo);
                (procs.gl_bind_framebuffer)(GL_FRAMEBUFFER, gl_fbo);
                (procs.gl_framebuffer_texture_2d)(
                    GL_FRAMEBUFFER,
                    GL_COLOR_ATTACHMENT0,
                    GL_TEXTURE_2D,
                    gl_tex,
                    0,
                );
                let status = (procs.gl_check_framebuffer_status)(GL_FRAMEBUFFER);
                (procs.gl_bind_framebuffer)(GL_FRAMEBUFFER, 0);
                let _ = (procs.wgl_dx_unlock_objects)(gl_hdev, 1, reg_arr.as_mut_ptr());
                if status != GL_FRAMEBUFFER_COMPLETE {
                    eprintln!("[mpv-dx-bridge] glCheckFramebufferStatus failed: 0x{status:x}");
                    let _ = (procs.wgl_dx_unregister_object)(gl_hdev, reg_obj);
                    (procs.gl_delete_framebuffers)(1, &gl_fbo);
                    (procs.gl_delete_textures)(1, &gl_tex);
                    let _ = CloseHandle(nt_handle);
                    return false;
                }
            }

            *slot_out = BufferSlot {
                d3d_render_texture: Some(render_tex),
                d3d_shared_texture: Some(shared_tex),
                nt_handle: nt_handle.0 as isize,
                gl_texture: gl_tex,
                gl_fbo,
                gl_registered_obj: reg_obj.0 as isize,
                wgpu_texture: Some(wgpu_tex),
                wgpu_view: Some(wgpu_view),
                wgpu_done_fence: 0,
                in_use: false,
            };
        }

        let (lock, _) = &*self.shared;
        let mut st = lock.lock().unwrap();
        st.slots = new_slots;
        st.current_width = target_w;
        st.current_height = target_h;
        st.buffers_allocated = true;
        self.needs_recreate.store(false, Ordering::Release);
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn cleanup_slot(
        slot: &mut BufferSlot,
        index: usize,
        generation: u64,
        defer_if_in_use: bool,
        retired: &mut VecDeque<RetiredSlot>,
        procs: Option<&GlProcs>,
        gl_hdev: HANDLE,
        hdc: HDC,
        hglrc: HGLRC,
    ) {
        if let Some(procs) = procs {
            if slot.gl_registered_obj != 0 && !gl_hdev.is_invalid() {
                unsafe {
                    let _ = (procs.wgl_dx_unregister_object)(
                        gl_hdev,
                        HANDLE(slot.gl_registered_obj as *mut c_void),
                    );
                }
                slot.gl_registered_obj = 0;
            }
            unsafe {
                let mut cur =
                    !hdc.is_invalid() && !hglrc.is_invalid() && wglGetCurrentContext() == hglrc;
                if !cur && !hdc.is_invalid() && !hglrc.is_invalid() {
                    cur = wglMakeCurrent(hdc, hglrc).is_ok();
                }
                if cur {
                    if slot.gl_fbo != 0 {
                        (procs.gl_delete_framebuffers)(1, &slot.gl_fbo);
                        slot.gl_fbo = 0;
                    }
                    if slot.gl_texture != 0 {
                        (procs.gl_delete_textures)(1, &slot.gl_texture);
                        slot.gl_texture = 0;
                    }
                }
            }
        }
        slot.d3d_render_texture = None;

        if defer_if_in_use
            && slot.in_use
            && (slot.nt_handle != 0
                || slot.d3d_shared_texture.is_some()
                || slot.wgpu_texture.is_some())
        {
            retired.push_back(RetiredSlot {
                d3d_shared_texture: slot.d3d_shared_texture.take(),
                nt_handle: std::mem::take(&mut slot.nt_handle),
                wgpu_texture: slot.wgpu_texture.take(),
                wgpu_view: slot.wgpu_view.take(),
                generation,
                index,
            });
            while retired.len() > 16 {
                if let Some(mut old) = retired.pop_front() {
                    Self::cleanup_retired_slot(&mut old);
                }
            }
        } else {
            slot.wgpu_view = None;
            slot.wgpu_texture = None;
            if slot.nt_handle != 0 {
                unsafe {
                    let _ = CloseHandle(HANDLE(slot.nt_handle as *mut c_void));
                }
                slot.nt_handle = 0;
            }
            slot.d3d_shared_texture = None;
        }
        slot.in_use = false;
    }

    fn cleanup_retired_slot(rb: &mut RetiredSlot) {
        rb.wgpu_view = None;
        rb.wgpu_texture = None;
        if rb.nt_handle != 0 {
            unsafe {
                let _ = CloseHandle(HANDLE(rb.nt_handle as *mut c_void));
            }
            rb.nt_handle = 0;
        }
        rb.d3d_shared_texture = None;
    }

    fn cleanup_d3d11(&mut self, force_all_retired: bool) {
        let gl_hdev = HANDLE(self.gl_hdevice as *mut c_void);
        {
            let (lock, _) = &*self.shared;
            let mut st = lock.lock().unwrap();
            let gen = st.generation;
            for i in 0..NUM_BUFFERS {
                let mut slot = std::mem::take(&mut st.slots[i]);
                Self::cleanup_slot(
                    &mut slot,
                    i,
                    gen,
                    !force_all_retired,
                    &mut st.retired,
                    self.procs.as_ref(),
                    gl_hdev,
                    self.hdc,
                    self.hglrc,
                );
            }
            if force_all_retired {
                while let Some(mut rb) = st.retired.pop_front() {
                    Self::cleanup_retired_slot(&mut rb);
                }
            }
            st.buffers_allocated = false;
            st.d3d12_fence = None;
        }

        if self.gl_hdevice != 0 {
            if let Some(procs) = self.procs.as_ref() {
                unsafe {
                    let _ = (procs.wgl_dx_close_device)(gl_hdev);
                }
            }
            self.gl_hdevice = 0;
        }
        self.d3d11_fence = None;
        self.d3d11_context = None;
        self.d3d11_device = None;
    }

    fn recover_device(&mut self) -> bool {
        let (target_w, target_h) = {
            let (lock, cvar) = &*self.shared;
            let guard = lock.lock().unwrap();
            let guard = match cvar.wait_timeout_while(guard, Duration::from_millis(500), |st| {
                st.slots.iter().any(|s| s.in_use)
            }) {
                Ok((g, _)) => g,
                Err(e) => e.into_inner().0,
            };
            (
                if guard.current_width > 0 {
                    guard.current_width
                } else {
                    1280
                },
                if guard.current_height > 0 {
                    guard.current_height
                } else {
                    720
                },
            )
        };

        self.cleanup_d3d11(false);

        if self.init_d3d11().is_err() {
            return false;
        }
        unsafe {
            let _ = wglMakeCurrent(self.hdc, self.hglrc);
        }
        self.allocate_buffers(target_w, target_h)
    }

    fn init_mpv_render(&mut self) -> Result<()> {
        unsafe {
            let _ = wglMakeCurrent(self.hdc, self.hglrc);

            let mut gl_init = mpv_opengl_init_params {
                get_proc_address: Some(mpv_get_gl_proc_address),
                get_proc_address_ctx: std::ptr::null_mut(),
            };
            let mut advanced_control: c_int = 1;
            let api_type = MPV_RENDER_API_TYPE_OPENGL.as_ptr() as *mut c_void;

            let mut params = [
                mpv_render_param {
                    type_: MPV_RENDER_PARAM_API_TYPE,
                    data: api_type,
                },
                mpv_render_param {
                    type_: MPV_RENDER_PARAM_OPENGL_INIT_PARAMS,
                    data: (&mut gl_init as *mut mpv_opengl_init_params).cast(),
                },
                mpv_render_param {
                    type_: MPV_RENDER_PARAM_ADVANCED_CONTROL,
                    data: (&mut advanced_control as *mut c_int).cast(),
                },
                mpv_render_param {
                    type_: MPV_RENDER_PARAM_INVALID,
                    data: std::ptr::null_mut(),
                },
            ];

            let api = self.mpv_client.api();
            let mut ctx: *mut mpv_render_context = std::ptr::null_mut();
            let mut err = (api.render_context_create)(
                &mut ctx,
                self.mpv_client.raw_handle(),
                params.as_mut_ptr(),
            );
            if err < 0 {
                let mut fallback = [
                    mpv_render_param {
                        type_: MPV_RENDER_PARAM_API_TYPE,
                        data: api_type,
                    },
                    mpv_render_param {
                        type_: MPV_RENDER_PARAM_OPENGL_INIT_PARAMS,
                        data: (&mut gl_init as *mut mpv_opengl_init_params).cast(),
                    },
                    mpv_render_param {
                        type_: MPV_RENDER_PARAM_INVALID,
                        data: std::ptr::null_mut(),
                    },
                ];
                err = (api.render_context_create)(
                    &mut ctx,
                    self.mpv_client.raw_handle(),
                    fallback.as_mut_ptr(),
                );
            }

            if err < 0 || ctx.is_null() {
                let _ = wglMakeCurrent(HDC::default(), HGLRC::default());
                return Err(MpvError::Api {
                    code: err,
                    message: format!(
                        "mpv_render_context_create failed: {}",
                        api.error_message(err)
                    ),
                });
            }

            (api.render_context_set_update_callback)(
                ctx,
                Some(on_mpv_render_update),
                self.wake_raw as *mut c_void,
            );
            let _ = wglMakeCurrent(HDC::default(), HGLRC::default());
            self.mpv_gl_ctx = ctx;
            Ok(())
        }
    }

    fn render_loop(&mut self) {
        unsafe {
            let _ = wglMakeCurrent(self.hdc, self.hglrc);
        }

        let mut consecutive_recover_failures: u32 = 0;

        while self.running.load(Ordering::Acquire) {
            let timeout = if self.needs_recreate.load(Ordering::Relaxed) {
                16
            } else {
                INFINITE
            };
            unsafe {
                let _ = WaitForSingleObject(self.wake_event.handle(), timeout);
            }
            if !self.running.load(Ordering::Acquire) {
                break;
            }

            // 1. Check DXGI_ERROR_DEVICE_REMOVED / TDR
            if let Some(dev) = &self.d3d11_device {
                let reason = unsafe { dev.GetDeviceRemovedReason() };
                if reason.is_err() {
                    eprintln!("[mpv-dx-bridge] D3D11 device removed detected: {reason:?}");
                    if !self.recover_device() {
                        consecutive_recover_failures += 1;
                        if consecutive_recover_failures >= MAX_RECOVERY_ATTEMPTS {
                            eprintln!(
                                "[mpv-dx-bridge] Exceeded max recovery attempts ({MAX_RECOVERY_ATTEMPTS}); stopping render loop."
                            );
                            self.running.store(false, Ordering::Release);
                            break;
                        }
                        self.needs_recreate.store(true, Ordering::Release);
                        thread::sleep(Duration::from_millis(50));
                        continue;
                    }
                    consecutive_recover_failures = 0;
                }
            }

            // 2. Process atomic resize requests
            let packed = self.pending_resize.swap(0, Ordering::AcqRel);
            if packed != 0 {
                let req_w = (packed >> 32) as u32;
                let req_h = (packed & 0xFFFF_FFFF) as u32;
                let needs_alloc = {
                    let (lock, _) = &*self.shared;
                    let st = lock.lock().unwrap();
                    req_w > 0
                        && req_h > 0
                        && (req_w != st.current_width
                            || req_h != st.current_height
                            || !st.buffers_allocated)
                };
                if needs_alloc && !self.allocate_buffers(req_w, req_h) {
                    self.needs_recreate.store(true, Ordering::Release);
                }
            }

            if self.mpv_gl_ctx.is_null() {
                continue;
            }

            let update_flags =
                unsafe { (self.mpv_client.api().render_context_update)(self.mpv_gl_ctx) };
            if (update_flags & MPV_RENDER_UPDATE_FRAME) == 0 {
                continue;
            }

            let (allocated, mut buf_idx) = {
                let (lock, _) = &*self.shared;
                let st = lock.lock().unwrap();
                if !st.buffers_allocated {
                    (false, None)
                } else {
                    let completed = self
                        .d3d11_fence
                        .as_ref()
                        .map(|f| unsafe { f.GetCompletedValue() })
                        .unwrap_or(u64::MAX);
                    let idx = (0..NUM_BUFFERS).find(|&i| {
                        !st.slots[i].in_use
                            && st.slots[i].gl_registered_obj != 0
                            && completed >= st.slots[i].wgpu_done_fence
                    });
                    (true, idx)
                }
            };
            if !allocated {
                continue;
            }

            if buf_idx.is_none() {
                let (lock, cvar) = &*self.shared;
                let guard = lock.lock().unwrap_or_else(|e| e.into_inner());
                let (guard, _) =
                    match cvar.wait_timeout_while(guard, Duration::from_millis(12), |st| {
                        let completed = self
                            .d3d11_fence
                            .as_ref()
                            .map(|f| unsafe { f.GetCompletedValue() })
                            .unwrap_or(u64::MAX);
                        !(0..NUM_BUFFERS).any(|i| {
                            !st.slots[i].in_use
                                && st.slots[i].gl_registered_obj != 0
                                && completed >= st.slots[i].wgpu_done_fence
                        })
                    }) {
                        Ok(v) => v,
                        Err(e) => e.into_inner(),
                    };
                let completed = self
                    .d3d11_fence
                    .as_ref()
                    .map(|f| unsafe { f.GetCompletedValue() })
                    .unwrap_or(u64::MAX);
                buf_idx = (0..NUM_BUFFERS).find(|&i| {
                    !guard.slots[i].in_use
                        && guard.slots[i].gl_registered_obj != 0
                        && completed >= guard.slots[i].wgpu_done_fence
                });
                // Fallback: if wgpu hasn't released an idle slot yet, pick any non-in-use slot and GPU-wait
                if buf_idx.is_none() {
                    buf_idx = (0..NUM_BUFFERS)
                        .find(|&i| !guard.slots[i].in_use && guard.slots[i].gl_registered_obj != 0);
                }
            }

            let Some(idx) = buf_idx else {
                continue;
            };

            let (reg_obj, gl_fbo, width, height, generation, render_tex, shared_tex, wgpu_done) = {
                let (lock, _) = &*self.shared;
                let st = lock.lock().unwrap();
                let s = &st.slots[idx];
                (
                    HANDLE(s.gl_registered_obj as *mut c_void),
                    s.gl_fbo,
                    st.current_width,
                    st.current_height,
                    st.generation,
                    s.d3d_render_texture.clone(),
                    s.d3d_shared_texture.clone(),
                    s.wgpu_done_fence,
                )
            };

            if reg_obj.is_invalid() {
                continue;
            }

            let Some(procs) = self.procs.as_ref() else {
                continue;
            };

            let mut reg_arr = [reg_obj];
            let locked = unsafe {
                (procs.wgl_dx_lock_objects)(
                    HANDLE(self.gl_hdevice as *mut c_void),
                    1,
                    reg_arr.as_mut_ptr(),
                )
            };
            if locked == 0 {
                if let Some(dev) = &self.d3d11_device {
                    if unsafe { dev.GetDeviceRemovedReason() }.is_err() {
                        let _ = self.recover_device();
                    }
                }
                continue;
            }

            let mut fbo = mpv_opengl_fbo {
                fbo: gl_fbo as c_int,
                w: width as c_int,
                h: height as c_int,
                internal_format: GL_RGBA8,
            };
            let mut flip: c_int = if self.flip_y.load(Ordering::Acquire) {
                1
            } else {
                0
            };
            let mut render_params = [
                mpv_render_param {
                    type_: MPV_RENDER_PARAM_OPENGL_FBO,
                    data: (&mut fbo as *mut mpv_opengl_fbo).cast(),
                },
                mpv_render_param {
                    type_: MPV_RENDER_PARAM_FLIP_Y,
                    data: (&mut flip as *mut c_int).cast(),
                },
                mpv_render_param {
                    type_: MPV_RENDER_PARAM_INVALID,
                    data: std::ptr::null_mut(),
                },
            ];

            unsafe {
                (self.mpv_client.api().render_context_render)(
                    self.mpv_gl_ctx,
                    render_params.as_mut_ptr(),
                );
                let _ = (procs.wgl_dx_unlock_objects)(
                    HANDLE(self.gl_hdevice as *mut c_void),
                    1,
                    reg_arr.as_mut_ptr(),
                );
            }

            let mut signaled_val = 0u64;
            if let (Some(ctx4), Some(d3d_fence), Some(dst), Some(src)) = (
                &self.d3d11_context,
                &self.d3d11_fence,
                &shared_tex,
                &render_tex,
            ) {
                unsafe {
                    if wgpu_done > 0 {
                        let _ = ctx4.Wait(d3d_fence, wgpu_done);
                    }
                    ctx4.CopyResource(dst, src);
                    {
                        let (lock, _) = &*self.shared;
                        let mut st = lock.lock().unwrap();
                        st.next_fence_value = st.next_fence_value.saturating_add(1);
                        signaled_val = st.next_fence_value;
                    }
                    let _ = ctx4.Signal(d3d_fence, signaled_val);
                    ctx4.Flush();
                }
            }

            let published = {
                let (lock, _) = &*self.shared;
                let mut st = lock.lock().unwrap();
                if st.generation == generation
                    && st.slots[idx].wgpu_texture.is_some()
                    && st.slots[idx].wgpu_view.is_some()
                {
                    st.slots[idx].in_use = true;
                    let frame = ImportedVideoFrame {
                        texture: st.slots[idx].wgpu_texture.clone().unwrap(),
                        view: st.slots[idx].wgpu_view.clone().unwrap(),
                        width,
                        height,
                        generation,
                        buffer_index: idx,
                        fence_value: signaled_val,
                    };
                    // Release an older unconsumed `latest_frame` slot if different from retained
                    if let Some(prev_unconsumed) = st.latest_frame.replace(frame.clone()) {
                        let keep_retained = st
                            .retained_frame
                            .as_ref()
                            .map(|r| {
                                r.generation == prev_unconsumed.generation
                                    && r.buffer_index == prev_unconsumed.buffer_index
                            })
                            .unwrap_or(false);
                        if !keep_retained && prev_unconsumed.generation == st.generation {
                            st.slots[prev_unconsumed.buffer_index].in_use = false;
                        }
                    }
                    Some(frame)
                } else {
                    None
                }
            };

            if let (Some(cb), Some(frame)) = (&self.on_frame, &published) {
                cb(frame);
            }
        }

        unsafe {
            let _ = wglMakeCurrent(HDC::default(), HGLRC::default());
        }
    }

    fn teardown(&mut self) {
        unsafe {
            if !self.mpv_gl_ctx.is_null() {
                (self.mpv_client.api().render_context_set_update_callback)(
                    self.mpv_gl_ctx,
                    None,
                    std::ptr::null_mut(),
                );
            }
            let _ = wglMakeCurrent(self.hdc, self.hglrc);
            if !self.mpv_gl_ctx.is_null() {
                (self.mpv_client.api().render_context_free)(self.mpv_gl_ctx);
                self.mpv_gl_ctx = std::ptr::null_mut();
            }
        }

        self.cleanup_d3d11(true);

        unsafe {
            let _ = wglMakeCurrent(HDC::default(), HGLRC::default());
            if !self.hglrc.is_invalid() {
                let _ = wglDeleteContext(self.hglrc);
                self.hglrc = HGLRC::default();
            }
            if !self.hdc.is_invalid() && !self.hwnd.is_invalid() {
                let _ = ReleaseDC(Some(self.hwnd), self.hdc);
                self.hdc = HDC::default();
            }
            if !self.hwnd.is_invalid() {
                let _ = DestroyWindow(self.hwnd);
                self.hwnd = HWND::default();
            }
            if !self.wake_raw.is_null() {
                drop(Arc::from_raw(self.wake_raw));
                self.wake_raw = std::ptr::null();
            }
        }
    }
}

fn is_gl_vendor_match(gl_vendor: &str, dxgi_vendor_id: u32) -> bool {
    let v = gl_vendor.to_ascii_lowercase();
    if v.contains("intel") && dxgi_vendor_id == 0x8086 {
        return true;
    }
    if v.contains("nvidia") && dxgi_vendor_id == 0x10DE {
        return true;
    }
    if (v.contains("amd") || v.contains("ati") || v.contains("advanced micro devices"))
        && (dxgi_vendor_id == 0x1002 || dxgi_vendor_id == 0x1022)
    {
        return true;
    }
    false
}
