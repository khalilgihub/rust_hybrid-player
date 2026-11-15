//! Dynamic C-FFI bindings to `libmpv-2.dll` via `libloading`.
//!
//! Layout and enum values match `hybrid-player/mpv/include/mpv/{client.h,render.h,render_gl.h}`.

#![allow(non_camel_case_types, non_snake_case, dead_code)]

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_double, c_int, c_ulong, c_void};
use std::path::Path;
use std::sync::Arc;

use libloading::Library;
use serde_json::{Map, Value};

use crate::error::{MpvError, Result};

// ── Opaque Handles ──────────────────────────────────────────

#[repr(C)]
pub struct mpv_handle {
    _private: [u8; 0],
}

#[repr(C)]
pub struct mpv_render_context {
    _private: [u8; 0],
}

// ── client.h Enums & Structs ────────────────────────────────

pub type mpv_format = c_int;
pub const MPV_FORMAT_NONE: mpv_format = 0;
pub const MPV_FORMAT_STRING: mpv_format = 1;
pub const MPV_FORMAT_OSD_STRING: mpv_format = 2;
pub const MPV_FORMAT_FLAG: mpv_format = 3;
pub const MPV_FORMAT_INT64: mpv_format = 4;
pub const MPV_FORMAT_DOUBLE: mpv_format = 5;
pub const MPV_FORMAT_NODE: mpv_format = 6;
pub const MPV_FORMAT_NODE_ARRAY: mpv_format = 7;
pub const MPV_FORMAT_NODE_MAP: mpv_format = 8;
pub const MPV_FORMAT_BYTE_ARRAY: mpv_format = 9;

pub type mpv_event_id = c_int;
pub const MPV_EVENT_NONE: mpv_event_id = 0;
pub const MPV_EVENT_SHUTDOWN: mpv_event_id = 1;
pub const MPV_EVENT_LOG_MESSAGE: mpv_event_id = 2;
pub const MPV_EVENT_GET_PROPERTY_REPLY: mpv_event_id = 3;
pub const MPV_EVENT_SET_PROPERTY_REPLY: mpv_event_id = 4;
pub const MPV_EVENT_COMMAND_REPLY: mpv_event_id = 5;
pub const MPV_EVENT_START_FILE: mpv_event_id = 6;
pub const MPV_EVENT_END_FILE: mpv_event_id = 7;
pub const MPV_EVENT_FILE_LOADED: mpv_event_id = 8;
pub const MPV_EVENT_IDLE: mpv_event_id = 11;
pub const MPV_EVENT_TICK: mpv_event_id = 14;
pub const MPV_EVENT_CLIENT_MESSAGE: mpv_event_id = 16;
pub const MPV_EVENT_VIDEO_RECONFIG: mpv_event_id = 17;
pub const MPV_EVENT_AUDIO_RECONFIG: mpv_event_id = 18;
pub const MPV_EVENT_SEEK: mpv_event_id = 20;
pub const MPV_EVENT_PLAYBACK_RESTART: mpv_event_id = 21;
pub const MPV_EVENT_PROPERTY_CHANGE: mpv_event_id = 22;
pub const MPV_EVENT_QUEUE_OVERFLOW: mpv_event_id = 24;
pub const MPV_EVENT_HOOK: mpv_event_id = 25;

pub type mpv_end_file_reason = c_int;
pub const MPV_END_FILE_REASON_EOF: mpv_end_file_reason = 0;
pub const MPV_END_FILE_REASON_STOP: mpv_end_file_reason = 2;
pub const MPV_END_FILE_REASON_QUIT: mpv_end_file_reason = 3;
pub const MPV_END_FILE_REASON_ERROR: mpv_end_file_reason = 4;
pub const MPV_END_FILE_REASON_REDIRECT: mpv_end_file_reason = 5;

#[repr(C)]
#[derive(Copy, Clone)]
pub union mpv_node_union {
    pub string: *mut c_char,
    pub flag: c_int,
    pub int64: i64,
    pub double_: c_double,
    pub list: *mut mpv_node_list,
    pub ba: *mut mpv_byte_array,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct mpv_node {
    pub u: mpv_node_union,
    pub format: mpv_format,
}

#[repr(C)]
pub struct mpv_node_list {
    pub num: c_int,
    pub values: *mut mpv_node,
    pub keys: *mut *mut c_char,
}

#[repr(C)]
pub struct mpv_byte_array {
    pub data: *mut c_void,
    pub size: usize,
}

#[repr(C)]
pub struct mpv_event_property {
    pub name: *const c_char,
    pub format: mpv_format,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct mpv_event_log_message {
    pub prefix: *const c_char,
    pub level: *const c_char,
    pub text: *const c_char,
    pub log_level: c_int,
}

#[repr(C)]
pub struct mpv_event_start_file {
    pub playlist_entry_id: i64,
}

#[repr(C)]
pub struct mpv_event_end_file {
    pub reason: mpv_end_file_reason,
    pub error: c_int,
    pub playlist_entry_id: i64,
    pub playlist_insert_id: i64,
    pub playlist_insert_num_entries: c_int,
}

#[repr(C)]
pub struct mpv_event_client_message {
    pub num_args: c_int,
    pub args: *const *const c_char,
}

#[repr(C)]
pub struct mpv_event_command {
    pub result: mpv_node,
}

#[repr(C)]
pub struct mpv_event {
    pub event_id: mpv_event_id,
    pub error: c_int,
    pub reply_userdata: u64,
    pub data: *mut c_void,
}

// ── render.h & render_gl.h Enums & Structs ──────────────────

pub type mpv_render_param_type = c_int;
pub const MPV_RENDER_PARAM_INVALID: mpv_render_param_type = 0;
pub const MPV_RENDER_PARAM_API_TYPE: mpv_render_param_type = 1;
pub const MPV_RENDER_PARAM_OPENGL_INIT_PARAMS: mpv_render_param_type = 2;
pub const MPV_RENDER_PARAM_OPENGL_FBO: mpv_render_param_type = 3;
pub const MPV_RENDER_PARAM_FLIP_Y: mpv_render_param_type = 4;
pub const MPV_RENDER_PARAM_DEPTH: mpv_render_param_type = 5;
pub const MPV_RENDER_PARAM_ADVANCED_CONTROL: mpv_render_param_type = 10;
pub const MPV_RENDER_PARAM_NEXT_FRAME_INFO: mpv_render_param_type = 11;
pub const MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME: mpv_render_param_type = 12;
pub const MPV_RENDER_PARAM_SKIP_RENDERING: mpv_render_param_type = 13;

pub const MPV_RENDER_UPDATE_FRAME: u64 = 1 << 0;
pub const MPV_RENDER_API_TYPE_OPENGL: &[u8] = b"opengl\0";

#[repr(C)]
#[derive(Copy, Clone)]
pub struct mpv_render_param {
    pub type_: mpv_render_param_type,
    pub data: *mut c_void,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct mpv_opengl_init_params {
    pub get_proc_address:
        Option<unsafe extern "C" fn(ctx: *mut c_void, name: *const c_char) -> *mut c_void>,
    pub get_proc_address_ctx: *mut c_void,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct mpv_opengl_fbo {
    pub fbo: c_int,
    pub w: c_int,
    pub h: c_int,
    pub internal_format: c_int,
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
pub struct mpv_render_frame_info {
    pub flags: u64,
    pub target_time: i64,
}

// ── Function Pointer Types ──────────────────────────────────

pub type FnMpvClientApiVersion = unsafe extern "C" fn() -> c_ulong;
pub type FnMpvErrorString = unsafe extern "C" fn(error: c_int) -> *const c_char;
pub type FnMpvFree = unsafe extern "C" fn(data: *mut c_void);
pub type FnMpvFreeNodeContents = unsafe extern "C" fn(node: *mut mpv_node);
pub type FnMpvCreate = unsafe extern "C" fn() -> *mut mpv_handle;
pub type FnMpvInitialize = unsafe extern "C" fn(ctx: *mut mpv_handle) -> c_int;
pub type FnMpvDestroy = unsafe extern "C" fn(ctx: *mut mpv_handle);
pub type FnMpvTerminateDestroy = unsafe extern "C" fn(ctx: *mut mpv_handle);
pub type FnMpvSetOptionString =
    unsafe extern "C" fn(ctx: *mut mpv_handle, name: *const c_char, data: *const c_char) -> c_int;
pub type FnMpvCommand =
    unsafe extern "C" fn(ctx: *mut mpv_handle, args: *const *const c_char) -> c_int;
pub type FnMpvCommandAsync = unsafe extern "C" fn(
    ctx: *mut mpv_handle,
    reply_userdata: u64,
    args: *const *const c_char,
) -> c_int;
pub type FnMpvCommandRet = unsafe extern "C" fn(
    ctx: *mut mpv_handle,
    args: *const *const c_char,
    result: *mut mpv_node,
) -> c_int;
pub type FnMpvCommandString =
    unsafe extern "C" fn(ctx: *mut mpv_handle, args: *const c_char) -> c_int;
pub type FnMpvSetProperty = unsafe extern "C" fn(
    ctx: *mut mpv_handle,
    name: *const c_char,
    format: mpv_format,
    data: *mut c_void,
) -> c_int;
pub type FnMpvSetPropertyString =
    unsafe extern "C" fn(ctx: *mut mpv_handle, name: *const c_char, data: *const c_char) -> c_int;
pub type FnMpvSetPropertyAsync = unsafe extern "C" fn(
    ctx: *mut mpv_handle,
    reply_userdata: u64,
    name: *const c_char,
    format: mpv_format,
    data: *mut c_void,
) -> c_int;
pub type FnMpvGetProperty = unsafe extern "C" fn(
    ctx: *mut mpv_handle,
    name: *const c_char,
    format: mpv_format,
    data: *mut c_void,
) -> c_int;
pub type FnMpvGetPropertyString =
    unsafe extern "C" fn(ctx: *mut mpv_handle, name: *const c_char) -> *mut c_char;
pub type FnMpvGetPropertyAsync = unsafe extern "C" fn(
    ctx: *mut mpv_handle,
    reply_userdata: u64,
    name: *const c_char,
    format: mpv_format,
) -> c_int;
pub type FnMpvObserveProperty = unsafe extern "C" fn(
    mpv: *mut mpv_handle,
    reply_userdata: u64,
    name: *const c_char,
    format: mpv_format,
) -> c_int;
pub type FnMpvUnobserveProperty =
    unsafe extern "C" fn(mpv: *mut mpv_handle, registered_reply_userdata: u64) -> c_int;
pub type FnMpvEventName = unsafe extern "C" fn(event: mpv_event_id) -> *const c_char;
pub type FnMpvRequestLogMessages =
    unsafe extern "C" fn(ctx: *mut mpv_handle, min_level: *const c_char) -> c_int;
pub type FnMpvWaitEvent =
    unsafe extern "C" fn(ctx: *mut mpv_handle, timeout: c_double) -> *mut mpv_event;
pub type FnMpvWakeup = unsafe extern "C" fn(ctx: *mut mpv_handle);
pub type FnMpvSetWakeupCallback = unsafe extern "C" fn(
    ctx: *mut mpv_handle,
    cb: Option<unsafe extern "C" fn(d: *mut c_void)>,
    d: *mut c_void,
);

pub type FnMpvRenderContextCreate = unsafe extern "C" fn(
    res: *mut *mut mpv_render_context,
    mpv: *mut mpv_handle,
    params: *mut mpv_render_param,
) -> c_int;
pub type FnMpvRenderContextSetParameter =
    unsafe extern "C" fn(ctx: *mut mpv_render_context, param: mpv_render_param) -> c_int;
pub type FnMpvRenderContextGetInfo =
    unsafe extern "C" fn(ctx: *mut mpv_render_context, param: mpv_render_param) -> c_int;
pub type FnMpvRenderContextSetUpdateCallback = unsafe extern "C" fn(
    ctx: *mut mpv_render_context,
    callback: Option<unsafe extern "C" fn(cb_ctx: *mut c_void)>,
    callback_ctx: *mut c_void,
);
pub type FnMpvRenderContextUpdate = unsafe extern "C" fn(ctx: *mut mpv_render_context) -> u64;
pub type FnMpvRenderContextRender =
    unsafe extern "C" fn(ctx: *mut mpv_render_context, params: *mut mpv_render_param) -> c_int;
pub type FnMpvRenderContextReportSwap = unsafe extern "C" fn(ctx: *mut mpv_render_context);
pub type FnMpvRenderContextFree = unsafe extern "C" fn(ctx: *mut mpv_render_context);

/// Dynamically loaded `libmpv-2.dll` symbol table.
pub struct MpvApi {
    _lib: Library,
    pub client_api_version: FnMpvClientApiVersion,
    pub error_string: FnMpvErrorString,
    pub free: FnMpvFree,
    pub free_node_contents: FnMpvFreeNodeContents,
    pub create: FnMpvCreate,
    pub initialize: FnMpvInitialize,
    pub destroy: FnMpvDestroy,
    pub terminate_destroy: Option<FnMpvTerminateDestroy>,
    pub set_option_string: FnMpvSetOptionString,
    pub command: FnMpvCommand,
    pub command_async: FnMpvCommandAsync,
    pub command_ret: FnMpvCommandRet,
    pub command_string: FnMpvCommandString,
    pub set_property: FnMpvSetProperty,
    pub set_property_string: FnMpvSetPropertyString,
    pub set_property_async: FnMpvSetPropertyAsync,
    pub get_property: FnMpvGetProperty,
    pub get_property_string: FnMpvGetPropertyString,
    pub get_property_async: FnMpvGetPropertyAsync,
    pub observe_property: FnMpvObserveProperty,
    pub unobserve_property: FnMpvUnobserveProperty,
    pub event_name: FnMpvEventName,
    pub request_log_messages: FnMpvRequestLogMessages,
    pub wait_event: FnMpvWaitEvent,
    pub wakeup: FnMpvWakeup,
    pub set_wakeup_callback: FnMpvSetWakeupCallback,

    pub render_context_create: FnMpvRenderContextCreate,
    pub render_context_set_parameter: FnMpvRenderContextSetParameter,
    pub render_context_get_info: FnMpvRenderContextGetInfo,
    pub render_context_set_update_callback: FnMpvRenderContextSetUpdateCallback,
    pub render_context_update: FnMpvRenderContextUpdate,
    pub render_context_render: FnMpvRenderContextRender,
    pub render_context_report_swap: FnMpvRenderContextReportSwap,
    pub render_context_free: FnMpvRenderContextFree,
}

unsafe impl Send for MpvApi {}
unsafe impl Sync for MpvApi {}

unsafe fn load_sym<T: Copy>(lib: &Library, name: &'static str, bytes: &[u8]) -> Result<T> {
    let sym = lib
        .get::<T>(bytes)
        .map_err(|_| MpvError::MissingSymbol(name))?;
    Ok(*sym)
}

impl MpvApi {
    /// Load `libmpv-2.dll` from an explicit path or DLL search path.
    pub fn load(dll_path: &Path) -> Result<Arc<Self>> {
        let lib = unsafe { Library::new(dll_path) }
            .map_err(|e| MpvError::LibraryLoad(format!("{}: {e}", dll_path.display())))?;

        unsafe {
            let api = Self {
                client_api_version: load_sym(
                    &lib,
                    "mpv_client_api_version",
                    b"mpv_client_api_version\0",
                )?,
                error_string: load_sym(&lib, "mpv_error_string", b"mpv_error_string\0")?,
                free: load_sym(&lib, "mpv_free", b"mpv_free\0")?,
                free_node_contents: load_sym(
                    &lib,
                    "mpv_free_node_contents",
                    b"mpv_free_node_contents\0",
                )?,
                create: load_sym(&lib, "mpv_create", b"mpv_create\0")?,
                initialize: load_sym(&lib, "mpv_initialize", b"mpv_initialize\0")?,
                destroy: load_sym(&lib, "mpv_destroy", b"mpv_destroy\0")?,
                terminate_destroy: lib
                    .get::<FnMpvTerminateDestroy>(b"mpv_terminate_destroy\0")
                    .ok()
                    .map(|s| *s),
                set_option_string: load_sym(
                    &lib,
                    "mpv_set_option_string",
                    b"mpv_set_option_string\0",
                )?,
                command: load_sym(&lib, "mpv_command", b"mpv_command\0")?,
                command_async: load_sym(&lib, "mpv_command_async", b"mpv_command_async\0")?,
                command_ret: load_sym(&lib, "mpv_command_ret", b"mpv_command_ret\0")?,
                command_string: load_sym(&lib, "mpv_command_string", b"mpv_command_string\0")?,
                set_property: load_sym(&lib, "mpv_set_property", b"mpv_set_property\0")?,
                set_property_string: load_sym(
                    &lib,
                    "mpv_set_property_string",
                    b"mpv_set_property_string\0",
                )?,
                set_property_async: load_sym(
                    &lib,
                    "mpv_set_property_async",
                    b"mpv_set_property_async\0",
                )?,
                get_property: load_sym(&lib, "mpv_get_property", b"mpv_get_property\0")?,
                get_property_string: load_sym(
                    &lib,
                    "mpv_get_property_string",
                    b"mpv_get_property_string\0",
                )?,
                get_property_async: load_sym(
                    &lib,
                    "mpv_get_property_async",
                    b"mpv_get_property_async\0",
                )?,
                observe_property: load_sym(
                    &lib,
                    "mpv_observe_property",
                    b"mpv_observe_property\0",
                )?,
                unobserve_property: load_sym(
                    &lib,
                    "mpv_unobserve_property",
                    b"mpv_unobserve_property\0",
                )?,
                event_name: load_sym(&lib, "mpv_event_name", b"mpv_event_name\0")?,
                request_log_messages: load_sym(
                    &lib,
                    "mpv_request_log_messages",
                    b"mpv_request_log_messages\0",
                )?,
                wait_event: load_sym(&lib, "mpv_wait_event", b"mpv_wait_event\0")?,
                wakeup: load_sym(&lib, "mpv_wakeup", b"mpv_wakeup\0")?,
                set_wakeup_callback: load_sym(
                    &lib,
                    "mpv_set_wakeup_callback",
                    b"mpv_set_wakeup_callback\0",
                )?,
                render_context_create: load_sym(
                    &lib,
                    "mpv_render_context_create",
                    b"mpv_render_context_create\0",
                )?,
                render_context_set_parameter: load_sym(
                    &lib,
                    "mpv_render_context_set_parameter",
                    b"mpv_render_context_set_parameter\0",
                )?,
                render_context_get_info: load_sym(
                    &lib,
                    "mpv_render_context_get_info",
                    b"mpv_render_context_get_info\0",
                )?,
                render_context_set_update_callback: load_sym(
                    &lib,
                    "mpv_render_context_set_update_callback",
                    b"mpv_render_context_set_update_callback\0",
                )?,
                render_context_update: load_sym(
                    &lib,
                    "mpv_render_context_update",
                    b"mpv_render_context_update\0",
                )?,
                render_context_render: load_sym(
                    &lib,
                    "mpv_render_context_render",
                    b"mpv_render_context_render\0",
                )?,
                render_context_report_swap: load_sym(
                    &lib,
                    "mpv_render_context_report_swap",
                    b"mpv_render_context_report_swap\0",
                )?,
                render_context_free: load_sym(
                    &lib,
                    "mpv_render_context_free",
                    b"mpv_render_context_free\0",
                )?,
                _lib: lib,
            };
            Ok(Arc::new(api))
        }
    }

    pub fn error_message(&self, code: c_int) -> String {
        unsafe {
            let ptr = (self.error_string)(code);
            if ptr.is_null() {
                format!("unknown error ({code})")
            } else {
                CStr::from_ptr(ptr).to_string_lossy().into_owned()
            }
        }
    }

    pub fn check(&self, code: c_int) -> Result<()> {
        if code >= 0 {
            Ok(())
        } else {
            Err(MpvError::Api {
                code,
                message: self.error_message(code),
            })
        }
    }

    pub fn client_api_version_tuple(&self) -> (u16, u16) {
        let ver = unsafe { (self.client_api_version)() };
        (((ver >> 16) & 0xFFFF) as u16, (ver & 0xFFFF) as u16)
    }
}

pub fn to_cstring(s: &str) -> Result<CString> {
    CString::new(s).map_err(|_| MpvError::InvalidString(s.to_owned()))
}

/// Convert a `mpv_node` returned by libmpv into a `serde_json::Value`.
///
/// # Safety
/// `node` and all nested pointers must remain valid for the duration of the call.
pub unsafe fn mpv_node_to_json(node: &mpv_node) -> Value {
    match node.format {
        MPV_FORMAT_NONE => Value::Null,
        MPV_FORMAT_STRING | MPV_FORMAT_OSD_STRING => {
            let ptr = node.u.string;
            if ptr.is_null() {
                Value::Null
            } else {
                Value::String(CStr::from_ptr(ptr).to_string_lossy().into_owned())
            }
        }
        MPV_FORMAT_FLAG => Value::Bool(node.u.flag != 0),
        MPV_FORMAT_INT64 => Value::Number(serde_json::Number::from(node.u.int64)),
        MPV_FORMAT_DOUBLE => serde_json::Number::from_f64(node.u.double_)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        MPV_FORMAT_NODE_ARRAY => {
            let list = node.u.list;
            if list.is_null() || (*list).num <= 0 || (*list).values.is_null() {
                return Value::Array(Vec::new());
            }
            let num = (*list).num as usize;
            let values = std::slice::from_raw_parts((*list).values, num);
            Value::Array(values.iter().map(|v| mpv_node_to_json(v)).collect())
        }
        MPV_FORMAT_NODE_MAP => {
            let list = node.u.list;
            if list.is_null()
                || (*list).num <= 0
                || (*list).values.is_null()
                || (*list).keys.is_null()
            {
                return Value::Object(Map::new());
            }
            let num = (*list).num as usize;
            let values = std::slice::from_raw_parts((*list).values, num);
            let keys = std::slice::from_raw_parts((*list).keys, num);
            let mut map = Map::with_capacity(num);
            for (k_ptr, val) in keys.iter().zip(values.iter()) {
                if !k_ptr.is_null() {
                    let key = CStr::from_ptr(*k_ptr).to_string_lossy().into_owned();
                    map.insert(key, mpv_node_to_json(val));
                }
            }
            Value::Object(map)
        }
        MPV_FORMAT_BYTE_ARRAY => {
            let ba = node.u.ba;
            if ba.is_null() || (*ba).data.is_null() || (*ba).size == 0 {
                return Value::Array(Vec::new());
            }
            let bytes = std::slice::from_raw_parts((*ba).data as *const u8, (*ba).size);
            Value::Array(bytes.iter().map(|b| Value::from(*b)).collect())
        }
        _ => Value::Null,
    }
}
