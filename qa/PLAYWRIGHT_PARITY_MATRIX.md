# Phase 6 Playwright Parity Matrix (29 Specs, 194 Tests)

Complete 1:1 mapping from `hybrid-player/tests/*.spec.js` (`.rust-migration/tokens-and-metrics.md` Section 10) to Rust verification targets across `hybrid-core`, `hybrid-media-services`, `hybrid-mpv-engine`, `hybrid-win32-shell`, `hybrid-shaders`, `hybrid-ui`, `hybrid-app`, and `hybrid-qa` (`qa/src/{pixel_diff,uia_runner}.rs`).

## Golden Visual Baseline (`.rust-migration/golden/`, 27 PNGs)

| Golden Capture | Viewport | Verification Target | Notes |
|---|---|---|---|
| `welcome-{1280x720,900x600,640x360}-{dark,oled,light}.png` (9) | `1280x720`, `900x600`, `640x360` | `pixel_diff` (`ui_parity`) + `hybrid-app --headless-snapshot` | Breakpoint layout, themes, light-mode context flips |
| `shader-{dither,faulty,gridmotion,none}-1280x720.png` (4) | `1280x720` | `pixel_diff` (`default`) + `hybrid-shaders` + `--headless-snapshot` | WebGL -> WGSL `Pass 0` visual parity |
| `shader-particles-1280x720.png` (1) | `1280x720` | `hybrid-shaders` (`all_four_welcome_effects_compile_render_and_determinism`) | Electron capture recorded `nonZeroPixels: 0` (`particles.js:260` TDZ `ReferenceError` on `resumeLoop` during `resize(true)`); flagged via `GoldenCaptureManifest::is_known_blank_capture`; QA verifies Rust renders > `shader-none` pixels |
| `modal-{equalizer,speed,subtitle,audio,settings,networkStream,bgSettings}Modal-1280x720.png` (7) | `1280x720` | `pixel_diff` (`ui_parity`) + `UiaSession` + `--headless-snapshot` | SDF glass blur + modal layout parity |
| `modal-bgAccordionPopover-1280x720.png` (1) | `1280x720` | `pixel_diff` (`ui_parity`) + `hybrid-shaders` (`400x240 @ 30fps`) | `--modal bg --bg-accordion`; 5-card accordion gallery + preview shaders |
| `playlist-open-1280x720.png` (1) | `1280x720` | `pixel_diff` (`ui_parity`) + `hybrid-ui` | `320px` right drawer + welcome resize transition |
| `overlay-{clip-bezel-tab,burst-pill,screenshot-carousel}-1280x720.png` (3) | `1280x720` / `1536x816` | `pixel_diff` (`ui_parity`) + `UiaSession` + `SendInput` | `clip-bezel` (`00:00.0`), `burst-pill` (`1 frame`), `screenshot-carousel` (golden captured at `1920x1020` = `1536x816` @ `1.25x` DPI, matched at `1536x816` to avoid aspect distortion) |
| `playback-sample-1280x720.png` (1) | `1280x720` | `pixel_diff` (`ui_parity`) + `hybrid-mpv-engine` | `DxBridge` DX12/WGL video frame (`hybrid-player/test-sample.mp4` @ `t=0.0s`) + control bar |

---

## Complete 29-Spec / 194-Test Parity Mapping

| # | Playwright Spec | Tests | Rust Crate(s) / Module(s) | Verification Mode |
|---:|---|---:|---|---|
| 1 | `audio-track.spec.js` | 5 | `hybrid-ui` (`audio_modal`), `hybrid-core::tracks`, `hybrid-app::input` (`KeyA`) | Rust Unit + `UiaSession` + `SendInput` (`A`) |
| 2 | `binary-resolver.spec.js` | 3 | `hybrid-core::binary_resolver` (`tests/phase1_core_tests.rs`) | Rust Unit (PASS `#35`) |
| 3 | `burst-screenshot.spec.js` | 8 | `hybrid-mpv-engine::client` (`screenshot_burst_frame`, `finalize_burst_session`), `hybrid-app::input`, `hybrid-ui` | Live `mpv` Spike (PASS `#39`) + `SendInput` (`S` tap/hold, `Mouse4` tap/hold) + `pixel_diff` (`overlay-burst-pill`, `overlay-clip-bezel-tab`) |
| 4 | `custom-shortcuts.spec.js` | 6 | `hybrid-core::shortcuts` (52 catalog entries + `screenshotMouseButton`), `hybrid-ui` (`settings_modal`), `hybrid-app::input` | Rust Unit (PASS `#35`) + `UiaSession` + `SendInput` (`Shift+S`, `Ctrl+U`, `Esc` cancel) |
| 5 | `database-persistence.spec.js` | 7 | `hybrid-core::db` (`100` history cap, `<5s` resume guard, atomic save, `.corrupt-*` backup) | Rust Unit (PASS `#35`) + `LaunchedApp` restart roundtrip |
| 6 | `equalizer-audio.spec.js` | 6 | `hybrid-core::equalizer` (`10->18` bands, `lavfi=[superequalizer=...]`, `200%` volume clamp), `hybrid-mpv-engine::client` | Rust Unit (PASS `#35`) + `UiaSession` + `pixel_diff` (`modal-equalizerModal`) |
| 7 | `full-feature-smoke.spec.js` | 9 | `hybrid-ui`, `hybrid-shaders`, `hybrid-core`, `hybrid-app` | `--headless-snapshot` + `pixel_diff` (all modals/effects) + `UiaSession` smoke |
| 8 | `fullscreen-input.spec.js` | 6 | `hybrid-app::input` (`f`, `Shift+F`, input focus guard, double-click, `Esc`), `hybrid-win32-shell::subclass` | Rust Unit + `SendInput` + `inspect_window_state` |
| 9 | `fullscreen-unfullscreen-maximized.spec.js` | 3 | `hybrid-win32-shell::subclass`, `hybrid-ui` (`titlebar` 3-state maximize icon) | Rust Unit (`win32_integration`) + `UiaSession` + `inspect_window_state` |
| 10 | `gpu-context-loss.spec.js` | 4 | `hybrid-mpv-engine::dx_bridge` (TDR 10-attempt recovery, rapid resize), `hybrid-shaders::scene` (`object-fit: contain`) | Live `mpv` Spike (PASS `#39`) + `hybrid-shaders` (`PASS #25`) |
| 11 | `launch.spec.js` | 12 | `hybrid-win32-shell`, `hybrid-shaders`, `hybrid-ui`, `hybrid-core::db`, `hybrid-app` | `LaunchedApp` + `inspect_window_state` + `pixel_diff` (`welcome-*`) + `UiaSession` |
| 12 | `lock-and-autohide.spec.js` | 4 | `hybrid-app::input` (lock mode gating, `2500ms` chrome auto-hide, modal/drag guards) | Rust Unit + `SendInput` + `UiaSession` + timer verification |
| 13 | `media-folder-resolver.spec.js` | 9 | `hybrid-core::media_scanner::resolve_screenshot_subfolder`, `hybrid-mpv-engine::client::effective_screenshot_dir`, `hybrid-ui` | Rust Unit (PASS `#35`) + `hybrid-mpv-engine` (PASS `#39`) + `UiaSession` |
| 14 | `media-resilience.spec.js` | 9 | `hybrid-mpv-engine::client` (`load_file`, `0`-byte/corrupt/truncated/Unicode/`>260`-char Windows paths, rapid seek), `hybrid-app` | Live `mpv`/Integration Tests + `LaunchedApp` |
| 15 | `multi-monitor-dwm.spec.js` | 5 | `hybrid-win32-shell::{dwm,subclass}` (exclusions before 8-DIP borders, Aero zoom, multi-monitor DPI) | Rust Unit (`win32_integration`, PASS `#33`) + `inspect_window_state` |
| 16 | `network-resilience.spec.js` | 6 | `hybrid-media-services::ytdlp`, `hybrid-mpv-engine::client` (`eof-reached`, `paused-for-cache`), `hybrid-ui` (`LIVE` badge) | Rust Unit + Mock HTTP/Stream Integration + `UiaSession` |
| 17 | `playlist-navigation.spec.js` | 7 | `hybrid-core::playlist`, `hybrid-ui` (`playlist_panel`, welcome resize), `hybrid-app` | Rust Unit (PASS `#35`) + `pixel_diff` (`playlist-open`) + `SendInput` |
| 18 | `playlist-stress.spec.js` | 6 | `hybrid-core::playlist` (`1000` items `<500ms`, shuffle without replacement, repeat `all`), `hybrid-ui` (`>80` virtualization) | Rust Unit (PASS `#35` + `hybrid-ui`) |
| 19 | `rapid-switch-stress.spec.js` | 5 | `hybrid-mpv-engine::{client,dx_bridge}`, `hybrid-app` (30 switches / play-pause / speed / fullscreen) | Stress Integration + `LaunchedApp` |
| 20 | `real-playback.spec.js` | 3 | `hybrid-mpv-engine` (`libmpv-2.dll`, `DxBridge`, `test-sample.mp4` RGBA readback, skip indicator) | Live `mpv`/GPU Spike (PASS `#39`) + `pixel_diff` (`playback-sample`) |
| 21 | `resize-flicker.spec.js` | 4 | `hybrid-win32-shell::{dwm,subclass}` (`WS_CLIPCHILDREN\|WS_CLIPSIBLINGS`, `WVR_REDRAW = 0x0300`, no `SWP_FRAMECHANGED` on resize exit), `hybrid-mpv-engine::dx_bridge` | Rust Unit (PASS `#33`) + Live `DxBridge` Resize (PASS `#39`) + `capture_window_rgba` |
| 22 | `restore-carousel.spec.js` | 10 | `hybrid-app`, `hybrid-ui` (`screenshot_carousel`), `hybrid-win32-shell::os_integration::trash_file` | `SendInput` (`Ctrl+Z`, `Z`, `Enter`, `Esc`, `Ctrl+D`) + `UiaSession` |
| 23 | `screenshot-carousel.spec.js` | 9 | `hybrid-app`, `hybrid-ui` (`screenshot_carousel`), `hybrid-win32-shell::os_integration::trash_file` | `SendInput` (`Ctrl+D`, `ArrowLeft/Right`, `D`, `Z`, `Esc` in fullscreen) + `pixel_diff` (`overlay-screenshot-carousel`) |
| 24 | `subtitle-lifecycle.spec.js` | 9 | `hybrid-core::{tracks,media_scanner}`, `hybrid-mpv-engine::client` (`.srt/.ass/.vtt`, UTF-16 LE / Win-1252 / ISO-8859-1, companion discovery) | Rust Unit + Live `mpv` Sub Integration + `LaunchedApp` |
| 25 | `subtitle-sync.spec.js` | 6 | `hybrid-core::tracks` (`±600s` clamp), `hybrid-ui` (`subtitle_modal`), `hybrid-app::input` (`G`/`H`) | Rust Unit (PASS `#35`) + `UiaSession` + `SendInput` (`G`/`H`) + `pixel_diff` (`modal-subtitleModal`) |
| 26 | `youtube-cookies.spec.js` | 9 | `hybrid-media-services::youtube_auth` (`cookies.txt`, expiry filter, `sign_out_youtube_in_dir`), `hybrid-win32-shell::companion_auth` | Rust Unit (PASS `#35` & `#33`) + `UiaSession` |
| 27 | `youtube-option-a.spec.js` | 10 | `hybrid-media-services::youtube_auth` (`tests/phase1_media_services_tests.rs`, Chrome/Edge locator, Netscape rows, headless CDP) | Rust Integration (PASS `#35`) |
| 28 | `youtube-video-4k.spec.js` | 13 | `hybrid-media-services::ytdlp` (URL validation, 4K/8K format map, 30m cache, in-flight join), `hybrid-ui`, `hybrid-mpv-engine` | Rust Unit (PASS `#35`) + `UiaSession` + Mock `yt-dlp` Integration |
| 29 | `ytdlp-notification.spec.js` | 2 | `hybrid-media-services::ytdlp_updater`, `hybrid-ui` (Obsidian capsule `.toast-update-pill`) | Rust Unit (PASS `#35`) + `UiaSession` |

---

## Per-Test Detailed Traceability

### 1. `audio-track.spec.js` (5 tests)
1. `should verify pause indicator is removed from DOM` -> `hybrid-ui` + `UiaSession` absence check
2. `should render Audio button between Equalizer and Subtitles and open Audio Modal on click` -> `hybrid-ui` layout bounds + `UiaSession` order & `invoke_element` + `pixel_diff` (`modal-audioModal-1280x720.png`)
3. `should populate Audio Modal with Anime presets and track pills` -> `hybrid-ui` + `hybrid-core::tracks` + `UiaSession`
4. `should adjust audio sync delay in modal` -> `hybrid-core::tracks` (`±600_000ms`) + `hybrid-mpv-engine` (`audio-delay`) + `UiaSession`
5. `should trigger Subs to Dubs toggle on A shortcut key` -> `hybrid-core::tracks::compute_subs_dubs_toggle` + `SendInput` (`KeyA`)

### 2. `binary-resolver.spec.js` (3 tests)
1. `prefers bundled ffmpeg resources without PATH lookup` -> `hybrid-core::binary_resolver` (`phase1_core_tests`)
2. `resolves ffmpeg-static in development without PATH lookup` -> `hybrid-core::binary_resolver` (`phase1_core_tests`)
3. `does not fall back to PATH when PATH lookup is disabled` -> `hybrid-core::binary_resolver` (`phase1_core_tests`)

### 3. `burst-screenshot.spec.js` (8 tests)
1. `should trigger single screenshot execution on screenshot button click` -> `hybrid-mpv-engine::client::screenshot` + `UiaSession::invoke_element`
2. `should engage burst capture when holding S and pace in-flight frames` -> `hybrid-app::input` + `SendInput::send_key_hold(KeyS, >200ms)` + `finalize_burst_session`
3. `should trigger standard single screenshot on short tap of S (< 200ms)` -> `SendInput::send_key_tap(KeyS)`
4. `should trigger standard single screenshot on short tap of Mouse 4 (< 200ms)` -> `SendInput::send_mouse_click(..., X1)`
5. `should engage burst capture when holding Mouse 4 (> 200ms) and stop on mouseup` -> `SendInput::send_mouse_hold(..., X1, >200ms)` + `pixel_diff` (`overlay-burst-pill-1280x720.png`)
6. `should allow configuring, persisting, and resetting custom screenshot directory` -> `hybrid-core::db` + `hybrid-mpv-engine::client::set_screenshot_dir` + `UiaSession`
7. `should synchronize custom select UI when screenshot format preference is set to webp` -> `hybrid-mpv-engine::client::set_screenshot_format(Webp)` + `UiaSession`
8. `should ensure clip recording stop button is clickable, elevated above titlebar, and triggers stop` -> `hybrid-ui` z-index + `pixel_diff` (`overlay-clip-bezel-tab-1280x720.png`) + `UiaSession` / `SendInput`

### 4. `custom-shortcuts.spec.js` (6 tests)
1. `should expose complete shortcut catalog and render rows in Settings modal` -> `hybrid-core::shortcuts` (52 entries) + `UiaSession` (`modal-settingsModal-1280x720.png`)
2. `should allow rebinding Screenshot from S to P via Settings UI and support both single tap and burst capture on P` -> `hybrid-core::shortcuts::ShortcutManager` + `SendInput`
3. `should cancel shortcut recording on Escape without closing the Settings modal and support Reset All` -> `hybrid-app::input` + `SendInput(VK_ESCAPE)` + `UiaSession`
4. `should display Delete/Restore Selected Screenshot labels and support configuring Mouse Screenshot Button` -> `hybrid-core::shortcuts::VALID_SCREENSHOT_MOUSE_BUTTONS` + `UiaSession`
5. `should capture lossless PNG screenshot when pressing Shift + S` -> `SendInput(Shift+KeyS)` + `hybrid-mpv-engine::client::screenshot(..., Some(Png))`
6. `should support Show Unlock Button setting, require Ctrl+U combo to unlock, and allow keyboard & mouse screenshots while locked` -> `hybrid-core::shortcuts` + `SendInput(Ctrl+KeyU)` + `UiaSession`

### 5. `database-persistence.spec.js` (7 tests)
1. `History entries are capped at 100 to prevent database bloat` -> `hybrid-core::db` (`phase1_core_tests`)
2. `Resume threshold guards: < 5s is guarded against saving` -> `hybrid-core::db` (`phase1_core_tests`)
3. `Resume position saves and restores accurately for valid mid-video timestamp` -> `hybrid-core::db`
4. `Playback speed is remembered per file across sessions` -> `hybrid-core::db`
5. `User preferences persist and round-trip successfully` -> `hybrid-core::db` (27 prefs)
6. `Clicking a history entry whose file was deleted on disk displays "File not found" warning` -> `hybrid-app` + `UiaSession`
7. `Resume position across app restarts restores 15m 30s (930s)` -> `hybrid-core::db` + `LaunchedApp` restart

### 6. `equalizer-audio.spec.js` (6 tests)
1. `Volume can be boosted up to 200% and correctly updates mpv and UI` -> `hybrid-mpv-engine::config` (`volume-max=200`) + `hybrid-ui` elastic drag
2. `Mute and unmute restores boosted volume accurately` -> `hybrid-mpv-engine::client` + `SendInput(KeyM)`
3. `Lavfi Superequalizer String Generation: All 10 bands and all 9 presets produce valid syntax` -> `hybrid-core::equalizer` (`phase1_core_tests`)
4. `Gain Clamping: Boundary limits (-12 dB to +12 dB) and NaN injection cannot produce invalid syntax` -> `hybrid-core::equalizer` (`phase1_core_tests`)
5. `Equalizer presets apply to mpv without throwing error or corrupting filter chain` -> `hybrid-core::equalizer` + `hybrid-mpv-engine::client`
6. `Audio sync offset adjusts mpv audio-delay property` -> `hybrid-mpv-engine::client`

### 7. `full-feature-smoke.spec.js` (9 tests)
1-9. Welcome Mounts / Shaders Lifecycle / Background Settings & Accordion / Equalizer / Playlist Drawer / General Settings / Network Stream Modal / Volume Gestures / DB & Diagnostics -> `--headless-snapshot` + `pixel_diff` against `.rust-migration/golden/*.png` + `UiaSession`

### 8. `fullscreen-input.spec.js` (6 tests)
1. `should toggle fullscreen when pressing lowercase f key without artificial DOM focus` -> `SendInput(KeyF)` + `inspect_window_state`
2. `should toggle fullscreen when pressing uppercase Shift+F` -> `SendInput(Shift+KeyF)`
3. `should not toggle fullscreen when typing f inside an active input element` -> `hybrid-app::input` + `UiaSession`
4. `should toggle fullscreen when double-clicking directly on videoCanvas surface during playback` -> `SendInput` double-click
5. `should exit fullscreen when pressing Escape` -> `SendInput(VK_ESCAPE)`
6. `should toggle fullscreen when double-clicking on welcome screen when idle` -> `SendInput` double-click

### 9. `fullscreen-unfullscreen-maximized.spec.js` (3 tests)
1-3. Maximized/windowed preservation & 3-state `#btnMaximize` icon updates -> `hybrid-win32-shell::subclass` + `inspect_window_state` + `UiaSession`

### 10. `gpu-context-loss.spec.js` (4 tests)
1. `Video canvas maintains aspect ratio (object-fit: contain) across extreme viewport ratios` -> `hybrid-shaders` (`pass0_clear_colors_and_aspect_fit_video`, PASS `#25`)
2. `Canvas context loss recovery does not crash the renderer` -> `hybrid-shaders` + `hybrid-app`
3. `dx-bridge-wrapper handles device recreation / reset safely without crashing` -> `hybrid-mpv-engine::dx_bridge::request_recreate` (up to 10 TDR attempts)
4. `Rapid viewport resizing during active playback retains stability` -> `hybrid-mpv-engine` (`spike_go_no_go`, PASS `#39`)

### 11. `launch.spec.js` (12 tests)
1-12. Welcome render, background shaders, diagnostics, titlebar hover colors, fullscreen curtain, idle hide, edge lock when maximized/fullscreen, performance snapshot, shortcut catalog, missing recent file prune, persistence failure resilience -> `LaunchedApp` + `inspect_window_state` + `capture_window_rgba` + `pixel_diff` (`welcome-*`) + `hybrid-core::db`

### 12. `lock-and-autohide.spec.js` (4 tests)
1-4. Lock blocking (`Space`, arrows, `M`, sliders, modals), `Ctrl+U` unlock, drag/modal auto-hide guards, `2500ms` idle auto-hide -> `hybrid-app::input` + `SendInput` + `UiaSession`

### 13. `media-folder-resolver.spec.js` (9 tests)
1-8. Scenarios 1-8 (word vs code seasons, fansub tags, Roman numerals, Specials/OVA, dotted names, Movies vs generic recordings, YouTube illegal chars, Trailers) -> `hybrid-core::media_scanner` (`phase1_core_tests`, PASS `#35`) + `hybrid-mpv-engine::client::effective_screenshot_dir` (PASS `#39`)
9. `should render Auto-Organize toggle in Settings and persist preference` -> `UiaSession` + `hybrid-core::db`

### 14. `media-resilience.spec.js` (9 tests)
1-9. Zero-byte, corrupt, missing, audio-only visualizer mode, rapid seek, truncated media, Unicode/emojis, URL chars (`# % & + [ ]`), Windows `>260`-char paths -> `hybrid-mpv-engine::client` + `LaunchedApp`

### 15. `multi-monitor-dwm.spec.js` (5 tests)
1-5. Titlebar exclusions hit-testing, `is-animating-bounds`, maximize/restore, fullscreen icon/state, multi-monitor DPI dragging -> `hybrid-win32-shell::{dwm,subclass}` (`win32_integration`, PASS `#33`) + `inspect_window_state`

### 16. `network-resilience.spec.js` (6 tests)
1-6. Protocol detection, offline/online auto-pause/resume, `LIVE` badge & live-edge seek clamp, corrupt stream error cleanup, manual pause preserved on reconnect, `isLive` reset on new file -> `hybrid-media-services::ytdlp` + `hybrid-mpv-engine::client` + `UiaSession`

### 17. `playlist-navigation.spec.js` (7 tests)
1-7. Next/prev button, video click collapses drawer, single-file vs playlist resume rules, `No media loaded` reset on clear, welcome resize on playlist toggle, rapid toggle stability -> `hybrid-core::playlist` + `hybrid-ui` + `pixel_diff` (`playlist-open-1280x720.png`) + `SendInput`

### 18. `playlist-stress.spec.js` (6 tests)
1-6. 1,000 items `<500ms` + virtualization, rapid shuffle, repeat `all` wrap, search filter, shuffle without replacement, deleting active item -> `hybrid-core::playlist` (`phase1_core_tests`, PASS `#35`) + `hybrid-ui`

### 19. `rapid-switch-stress.spec.js` (5 tests)
1-5. 30 rapid file/next switches, play/pause, speed, fullscreen toggles -> `hybrid-mpv-engine` + `hybrid-app` + `LaunchedApp`

### 20. `real-playback.spec.js` (3 tests)
1-3. Real `libmpv` playback, DirectX/WGL texture bridge RGBA render, skip indicator without loading ring -> `hybrid-mpv-engine` (`spike_go_no_go`, PASS `#39`) + `pixel_diff` (`playback-sample-1280x720.png`)

### 21. `resize-flicker.spec.js` (4 tests)
1-4. `WS_CLIPCHILDREN|WS_CLIPSIBLINGS`, `WM_NCCALCSIZE` `WVR_REDRAW (0x0300)`, zero black-frame drop during dynamic resize, animating bounds cleanup -> `hybrid-win32-shell` (PASS `#33`) + `hybrid-mpv-engine` (PASS `#39`) + `capture_window_rgba`

### 22. `restore-carousel.spec.js` (10 tests)
1-10. `Ctrl+Z` empty guard, `3 / 3` newest default, `ArrowLeft/Right`, `Z`/`Enter`/mouse restore, `Esc` close, `Ctrl+D`<->`Ctrl+Z` mode toggle, rapid toggle, full take->delete->restore->delete->undo lifecycle -> `hybrid-app` + `hybrid-ui` + `SendInput` + `UiaSession`

### 23. `screenshot-carousel.spec.js` (9 tests)
1-9. `Ctrl+D` empty guard, `3 / 3` newest, arrows without seeking, Choice C delete & file removal, `Z` undo, `Esc` cancel, mouse navigation + `D`, top-left notices, `Esc` in fullscreen keeps fullscreen -> `hybrid-app` + `hybrid-ui` + `SendInput` + `pixel_diff` (`overlay-screenshot-carousel-1280x720.png`)

### 24. `subtitle-lifecycle.spec.js` (9 tests)
1-9. Drag-drop sub vs media discrimination, UTF-16 LE / Win-1252 / ISO-8859-1 encodings, delay & visibility, auto-companion `.srt/.ass/.vtt`, idle sub-drop guard, paired drop -> `hybrid-core::{tracks,media_scanner}` + `hybrid-mpv-engine::client` + `hybrid-app`

### 25. `subtitle-sync.spec.js` (6 tests)
1-6. `#subSyncSlider`, seconds badge, steppers, legacy ms conversion, `G`/`H` shortcut keys -> `hybrid-core::tracks` + `hybrid-ui` + `SendInput(KeyG/KeyH)` + `pixel_diff` (`modal-subtitleModal-1280x720.png`)

### 26. `youtube-cookies.spec.js` (9 tests)
1-9. Companion auth UI, `cookies.txt` export, masked status, persistent open, missing browser fallback, Switch/Sign Out, `cookies.txt` + `yt-profile` cleanup, visitor filter, startup sync -> `hybrid-media-services::youtube_auth` (PASS `#35`) + `hybrid-win32-shell::companion_auth` (PASS `#33`) + `UiaSession`

### 27. `youtube-option-a.spec.js` (10 tests)
1-10. Chrome/Edge locator, dedicated `yt-profile`, 7-field Netscape rows, headless CDP, post-exit extraction, skipped Edge test parity, visitor cookie filter, browser launch args, graceful shutdown, sign out -> `hybrid-media-services::youtube_auth` (`phase1_media_services_tests`, PASS `#35`)

### 28. `youtube-video-4k.spec.js` (13 tests)
1-13. YouTube URL validation, 4K/8K height map, format builder, info cache + in-flight join, network stream modal, quality dropdown, `ytdl-format` + timestamp-preserving switch, mpv 4K playback, play/pause/seek, error fallback -> `hybrid-media-services::ytdlp` (PASS `#35`) + `hybrid-mpv-engine` + `UiaSession` + `pixel_diff` (`modal-networkStreamModal-1280x720.png`)

### 29. `ytdlp-notification.spec.js` (2 tests)
1-2. Obsidian capsule `.toast-update-pill` render, one-click update, dismiss -> `hybrid-media-services::ytdlp_updater` (PASS `#35`) + `hybrid-ui` + `UiaSession`
