//! Win32 frameless window subclassing, DWM integration, SMTC, single-instance IPC,
//! shell metadata, temp cleanup, Recycle Bin helper, and native companion auth window.

pub mod clipboard;
pub mod companion_auth;
pub mod dwm;
pub mod error;
pub mod os_integration;
pub mod smtc;
pub mod subclass;

pub use companion_auth::{
    companion_layout, pulse_ease_out, render_companion_vector_layer, CompanionAuthAction,
    CompanionAuthHandle, CompanionAuthOptions, CompanionAuthState, CompanionLayout,
    CompanionSvgAssets, COMPANION_HEIGHT_DIP, COMPANION_WIDTH_DIP, STEP_CHECK_SVG,
    VERIFIED_SHIELD_SVG, YOUTUBE_LOGO_SVG,
};
pub use dwm::{
    apply_dwm_attributes, apply_native_dwm_window_styles, apply_null_brush,
    extract_hwnd_from_winit, hwnd_from_raw, hwnd_to_raw, schedule_native_dwm_window_styles,
    style_child_surfaces, DwmScheduleGuard, ThemeMode, CHILD_WINDOW_STYLE_MASK,
    DWMNCRP_ENABLED_VAL, DWMSBT_NONE_VAL, DWMWA_BORDER_COLOR_ID, DWMWA_CAPTION_COLOR_ID,
    DWMWA_COLOR_NONE_VAL, DWMWA_NCRENDERING_POLICY_ID, DWMWA_SYSTEMBACKDROP_TYPE_ID,
    DWMWA_TEXT_COLOR_ID, DWMWA_TRANSITIONS_ENABLED_VAL, DWMWA_TRANSITIONS_FORCEDISABLED_ID,
    DWMWA_USE_IMMERSIVE_DARK_MODE_ID, DWMWA_USE_IMMERSIVE_DARK_MODE_LEGACY_ID,
    DWMWA_WINDOW_CORNER_PREFERENCE_ID, DWMWCP_DONOTROUND_VAL, FRAMELESS_WINDOW_STYLE_MASK,
    SWP_REFRESH_FRAME_FLAGS,
};
pub use error::{Result, Win32ShellError};
pub use os_integration::{
    cleanup_temp_files, create_start_menu_shortcut, decode_pipe_args,
    default_start_menu_shortcut_path, encode_pipe_args, is_process_running,
    register_hkcu_app_user_model_id, send_args_over_pipe, set_current_process_app_user_model_id,
    set_high_res_timer, setup_windows_shell_integration, trash_file, HighResTimerGuard,
    SingleInstanceOutcome, SingleInstanceServer, APP_ID, APP_NAME, DEFAULT_SINGLE_INSTANCE_MUTEX,
    DEFAULT_SINGLE_INSTANCE_PIPE,
};
pub use smtc::{SmtcCommand, SmtcController, SmtcMetadata, SmtcPlaybackStatus, SmtcTimeline};
pub use subclass::{
    clamp_dragged_bounds, compute_drag_from_maximize_start, compute_drag_restore_size,
    compute_nccalcsize_client_rect, dip_to_physical, dpi_for_window, monitor_info_for_point,
    monitor_info_for_window, physical_to_dip, resolve_nc_hit_test, scale_for_dpi,
    unpack_lparam_point, DipRect, HitTestConfig, MonitorMetrics, ShellWindowEvent,
    TitlebarDragSession, WindowBounds, WindowSubclass, DEFAULT_WINDOWED_HEIGHT,
    DEFAULT_WINDOWED_WIDTH, MIN_RESTORE_HEIGHT, MIN_RESTORE_WIDTH, MIN_WINDOW_HEIGHT_DIP,
    MIN_WINDOW_WIDTH_DIP, RESIZE_BORDER_DIP, TITLEBAR_DRAG_MIN_VISIBLE_HEIGHT,
    TITLEBAR_DRAG_VERTICAL_OFFSET,
};
