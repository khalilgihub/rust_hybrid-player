# Phase 6 Live Windows Verification Plan (Post-`crates/app` Build)

This document defines the live Windows automation test harness to execute once `crates/app` (`hybrid-app.exe`) compiles and `win-unpacked` / NSIS installer artifacts are built. It replaces Playwright/Electron DOM probes (`hybrid-player/tests/*.spec.js`) with native Windows UI Automation (`IUIAutomation` over `accesskit_winit`), real `SendInput` keyboard/mouse events, live `HWND`/DWM inspection, installer/registry verification, and pixel-diff golden comparison.

---

## 1. Prerequisites & Environment

- **Build binary**: `cargo build --release -p hybrid-app`
- **Stage & verify resources**:
  `powershell -ExecutionPolicy Bypass -File packaging/stage-and-verify.ps1 -Stage`
- **Build NSIS installer**:
  - Preferred (once installed via `cargo install cargo-packager`):
    `cargo packager --release --config packaging/Packager.toml --formats nsis`
  - Fallback using cached NSIS (`%LOCALAPPDATA%\electron-builder\Cache\nsis\nsis-3.0.4.1\makensis.exe`): generate setup `.exe` wrapping `packaging/staging/win-unpacked/` + `packaging/nsis/installer.nsh`.
- **Sample media**: `hybrid-player/test-sample.mp4` and `hybrid-player/test-audio.mp3`.

---

## 2. Live `HWND` & DWM Parity Suite (Ported from `resize-flicker.spec.js`, `multi-monitor-dwm.spec.js`, `fullscreen-unfullscreen-maximized.spec.js`)

Target: a Rust/Win32 test harness (`crates/app/tests/live_windows_e2e.rs` or standalone verification runner) launching `Hybrid Player.exe` with an isolated `--user-data-dir`:

1. **Startup & Zero White Flash**:
   - Assert main window class background brush (`GetClassLongPtrW(hwnd, GCLP_HBRBACKGROUND)`) == `GetStockObject(NULL_BRUSH)` (`0` or `NULL_BRUSH` handle).
   - Assert initial `WS_THICKFRAME | WS_CAPTION | WS_MINIMIZEBOX | WS_MAXIMIZEBOX | WS_SYSMENU | WS_CLIPCHILDREN | WS_CLIPSIBLINGS` bitmask matches `hybrid_win32_shell::FRAMELESS_WINDOW_STYLE_MASK`.
   - Assert `DwmGetWindowAttribute(hwnd, DWMWA_NCRENDERING_ENABLED)` == `TRUE`.
   - Capture first visible frame before `app_cold_start` clears and assert clear pixel color matches `#000000` (cold-start/welcome) and `#0c0d12` (idle frame).
2. **Frameless Resize & Zero `SWP_FRAMECHANGED` Flicker**:
   - Simulate interactive edge drag (`SendMessageW` / `SendInput` mouse-down on `HTRIGHT`/`HTBOTTOMRIGHT` border, move `+200px`, mouse-up).
   - Verify no `WS_CAPTION` re-flash, no `SWP_FRAMECHANGED` inside `WM_EXITSIZEMOVE`, and continuous DWM extended frame bounds tracking.
3. **Hit-Test (`WM_NCHITTEST`), Win11 Snap Layouts & Drag-From-Maximize**:
   - Probe `WM_NCHITTEST` at screen coordinates for:
     - Maximize button rect -> `HTMAXBUTTON` (`9`) (required for Win11 Snap Layouts).
     - Minimize/Close/Titlebar controls (`y: 0..32` right corner) -> `HTCLIENT` (`1`) even inside 8-DIP top-right border.
     - Titlebar drag strip -> `HTCAPTION` (`2`).
     - Unexcluded 8-DIP corners/edges -> `HTTOPLEFT`..`HTBOTTOMRIGHT`.
   - Double-click caption -> `IsZoomed(hwnd) != 0`; drag caption down > threshold -> restores windowed bounds centered proportionally under cursor X and clamped within monitor work area.
4. **Fullscreen <-> Maximized <-> Windowed State Machine**:
   - Toggle fullscreen (`F` / `F11`) from maximized state -> verify `WS_CAPTION` stripped (`GetWindowLongPtrW(hwnd, GWL_STYLE) & WS_CAPTION == 0`) and window fills `MONITORINFO.rcMonitor`.
   - Press `Escape` -> returns cleanly to maximized state with `IsZoomed(hwnd) != 0` and restored `FRAMELESS_WINDOW_STYLE_MASK`.
5. **Multi-Monitor DPI (`WM_DPICHANGED`)**:
   - If >= 2 monitors exist (or via simulated `WM_DPICHANGED` with `96 -> 144 DPI`), verify `GetDpiForWindow(hwnd)` scales client layout cleanly and titlebar hit-test DIP coordinates remain accurate.

---

## 3. Windows UI Automation (`IUIAutomation` over `accesskit_winit`)

Using `windows::Win32::UI::Accessibility::IUIAutomation` (`CUIAutomation`):

1. **Root & Tree Discovery**:
   - Locate `Hybrid Player` top-level `IUIAutomationElement` by `PID` (`UIA_ProcessIdPropertyId`).
   - Verify non-empty `ControlType` + `Name` nodes for:
     - Welcome Screen (`Open File`, `Open Folder`, `Open URL`, Effect pills: `Dither Waves`, `Particles`, `Faulty Terminal`, `Grid Motion`, Quality selector).
     - Titlebar (`Minimize`, `Maximize`/`Restore`, `Close`, Pin/Settings/Playlist buttons).
     - Playback controls (`Play/Pause`, `Previous`, `Next`, `Seek` slider with `RangeValuePattern`, `Volume` slider, `Subtitle` & `Audio` track popovers, `Equalizer`, `Fullscreen`).
2. **Keyboard Focus & Modal Traversal**:
   - Open Settings modal (`Ctrl+,`), Shortcuts modal (`Ctrl+K`), Open URL modal (`Ctrl+U`), Help/Shortcuts sheet (`?`), Confirm modal, Chapter/Clip modal, and verify `IUIAutomation::GetFocusedElement` stays trapped inside the active modal and returns to the trigger button on `Escape`.
3. **Live Regions & OSD Accessibility**:
   - Trigger Volume change / Speed change / Screenshot -> verify OSD status element emits UIA `LiveRegionChanged` / updated `Name` text (e.g., `"Volume 85%"`, `"Speed 1.25x"`).

---

## 4. Real `SendInput` Interaction Suite (Ported from `full-feature-smoke.spec.js`, `custom-shortcuts.spec.js`, `lock-and-autohide.spec.js`, `burst-screenshot.spec.js`)

After focusing the `Hybrid Player` HWND (`SetForegroundWindow`), synthesize real OS hardware events via Win32 `SendInput`:

1. **Media Launch & Playback Controls**:
   - Pass `hybrid-player/test-sample.mp4` on CLI -> verify `libmpv` playback starts (non-zero frame timestamp progression + non-black DX12 video surface).
   - `Space` toggles Play/Pause; `ArrowRight`/`ArrowLeft` seeks; `[` / `]` adjusts speed; `M` toggles mute; Mouse Wheel up/down changes volume by exact `±5%`.
2. **Lock Mode, Idle Cursor (`2000ms`), and Chrome Auto-Hide (`2500ms`)**:
   - While playing with pointer idle > `2100ms`, verify cursor hidden state and after `2600ms` verify chrome enters auto-hidden state; keep pointer over toolbar or open a modal/dropdown and assert chrome does NOT auto-hide.
   - Toggle UI Lock (`L`) -> verify shortcuts except unlock remain ignored (`shortcut_lock` guard).
3. **Screenshots, Burst Hold (`S`), and Carousel Undo (`Ctrl+D` / `Ctrl+Z` / `Escape`)**:
   - Tap `S` -> single screenshot saved into resolved subfolder (`auto_organize_screenshots = true`).
   - Hold `S` (`KEYEVENTF_KEYDOWN` -> sleep `800ms` -> `KEYEVENTF_KEYUP`) -> burst shots saved (`<20` flattened vs `>=20` subfolder).
   - Press `Ctrl+D` / `Ctrl+Z` -> screenshot deleted to Windows Recycle Bin (`trash_file` / `SHFileOperationW` `FOF_ALLOWUNDO`), not permanently unlinked.
   - Verify `Escape` priority order: dismiss Screenshot Carousel -> close active Modal -> exit Fullscreen.
4. **Custom Shortcuts & Mouse Screenshot Button**:
   - Open Shortcuts modal, rebind a shortcut cleanly (displacement check), verify `screenshotMouseButton` default `3` (`XButton1`) / `4` (`XButton2`) / `1` (`Left`) behavior.
5. **Drag-and-Drop Routing**:
   - Send `WM_DROPFILES` with `.mp4` -> replaces/appends media queue; with `.srt` -> loads external subtitle track without resetting video playback.

---

## 5. OS Integration, Single-Instance, Companion Auth & Packaged Smoke

1. **Single-Instance Mutex + Named Pipe Forwarding**:
   - Launch instance #1; launch instance #2 with `test-audio.mp3` CLI argument -> assert instance #2 exits with code `0` within `< 1500ms` and instance #1 receives & loads `test-audio.mp3` over the single-instance named pipe.
2. **AUMID, SMTC & Temp Cleanup**:
   - Verify `GetCurrentProcessExplicitAppUserModelID` == `"com.hybridplayer.app"`, HKCU `Software\Classes\AppUserModelId\com.hybridplayer.app` exists, and stale PID temp files are cleaned on clean exit while live PID temp files remain untouched.
3. **Native Pure-Rust Companion Auth Window (`500x405`)**:
   - Trigger YouTube Option A sign-in flow -> verify native Win32 child/owned window (`"Sign in to YouTube"`, `500x405` DIPs, zero WebView2/Chromium renderer processes spawned for the status dialog) renders and Cancel/Finish buttons respond cleanly.
4. **Installer & File Association Smoke (NSIS Sandbox / VM)**:
   - Run `powershell -ExecutionPolicy Bypass -File packaging/stage-and-verify.ps1 -Stage`.
   - Install NSIS package silently (`/S /D=C:\Temp\HybridPlayerInstallTest`) -> verify:
     - `C:\Temp\HybridPlayerInstallTest\Hybrid Player.exe`, `resources\mpv\libmpv-2.dll`, `d3dcompiler_43.dll`, `yt-dlp.exe`, `resources\ffmpeg\ffmpeg.exe`, `resources\assets\fonts\*`, `resources\assets\icons\*` all present.
     - Desktop & Start Menu `.lnk` shortcuts created.
     - Registry `Software\Classes\HybridPlayer.<ext>` + `.OpenWithProgids` + `Software\HybridPlayer\Capabilities\FileAssociations` populated for all 10 extensions (`mp4, mkv, webm, avi, mov, mp3, flac, wav, ogg, m4a`).
   - Launch installed `Hybrid Player.exe` -> verify `Welcome` + `libmpv` + DX12 ready -> close cleanly -> run `uninstall.exe /S` -> verify registry keys & shortcuts removed cleanly.

---

## 6. Headless Visual Regression Against `.rust-migration/golden/`

1. Run `Hybrid Player.exe --headless-snapshot --snapshot-dir target/e2e-snapshots` across all 5 breakpoints (`1920x1080`, `1440x900`, `1280x720`, `720x540`, `540x400`), Light/Dark/OLED themes, Chameleon Glass skins (`none`, `dither`, `faulty`, `particles`, `gridmotion`), Welcome Screen effects, Settings 6 tabs, Playlist panel, Equalizer modal, and OSD overlays.
2. Compare against `.rust-migration/golden/` PNGs via perceptual/pixel diff threshold (`<= 1.5%` structural delta, ignoring anti-aliased subpixel font edges & animated effect phase when seeded deterministically).
