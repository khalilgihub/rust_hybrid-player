# Hybrid Player — Phase 0 Baseline & Design Tokens (`tokens-and-metrics.md`)

> Baseline reference extracted 1:1 from `hybrid-player/` (`v12.6.0`).  
> Scope: design tokens, motion profiles, `@keyframes`, stacking order, `backdrop-filter` surfaces, Chameleon skins, pixel geometry, responsive breakpoints, SVGs/fonts, shaders/defaults, and Phase 6 Playwright test coverage.

---

## 1. Toolchain, Packaging & Runtime Baseline

| Item | Observed Value | Source |
|---|---|---|
| Rust compiler | `rustc 1.97.1 (8bab26f4f 2026-07-14)` | `rustc --version` |
| Cargo | `cargo 1.97.1 (c980f4866 2026-06-30)` | `cargo --version` |
| Git status | Workspace root is not a git repository | `git status` |
| App package | `hybrid-player` `12.6.0` | `hybrid-player/package.json` |
| Electron / Playwright | `electron ^40.10.2`, `@playwright/test ^1.60.0`, `electron-builder ^26.15.3` | `hybrid-player/package.json` |
| Runtime dependencies | `ffmpeg-static ^5.3.0`, `gsap ^3.15.0`, `koffi ^3.3.3`, `ogl ^1.0.11`, `postprocessing ^6.39.1`, `three ^0.183.2` | `hybrid-player/package.json` |
| Bundler config | `appId: com.hybridplayer.app`, `productName: Hybrid Player`, NSIS x64 | `hybrid-player/electron-builder.json` |
| File associations (10) | `.mp4`, `.mkv`, `.webm`, `.avi`, `.mov`, `.mp3`, `.flac`, `.wav`, `.ogg`, `.m4a` | `hybrid-player/electron-builder.json` |
| Bundled `mpv/` binaries | `libmpv-2.dll` (115,794,944 B), `mpv.exe` (115,799,552 B), `mpv.com` (3,584 B), `d3dcompiler_43.dll` (4,481,992 B), `libmpv.dll.a` (172,988 B), `yt-dlp.exe` (18,618,344 B), `updater.bat` (818 B); subdirs `doc`, `include`, `installer`, `mpv` | `hybrid-player/mpv/` & `electron-builder.json` (`extraResources` excludes `include`, `doc`, `installer`, `*.old` and copies `ffmpeg*`) |

---

## 2. Assets Inventory (Fonts, Brand Icons, All 64 Main UI SVGs + 3 Companion Auth SVGs)

### 2.1 Fonts (`hybrid-player/assets/fonts/` + `themes.css:6-57, 256-259`)
| `@font-face` Family | Weight | File | Bytes |
|---|---|---|---|
| `Sora Local` | 400 | `assets/fonts/Sora-Regular.ttf` | 46,040 |
| `Sora Local` | 600 | `assets/fonts/Sora-SemiBold.ttf` | 46,216 |
| `Sora Local` | 700 | `assets/fonts/Sora-Bold.ttf` | 46,152 |
| `IBM Plex Sans Local` | 400 | `assets/fonts/IBMPlexSans-Regular.ttf` | 205,252 |
| `IBM Plex Sans Local` | 500 | `assets/fonts/IBMPlexSans-Medium.ttf` | 205,292 |
| `IBM Plex Sans Local` | 600 | `assets/fonts/IBMPlexSans-SemiBold.ttf` | 205,284 |

- `--font-display: 'Sora Local', 'Segoe UI', sans-serif;`
- `--font-ui: 'IBM Plex Sans Local', 'Segoe UI', sans-serif;`
- `--font-mono: 'Consolas', 'SF Mono', monospace;`
- `body.brand-font-disabled`: `--font-display: 'Segoe UI', sans-serif; --font-ui: 'Segoe UI', sans-serif;` (`themes.css:256-259`, toggled via `#settBrandFontEnabled`, default `true`).

### 2.2 Icon Files (`hybrid-player/assets/icons/`)
- `brand-mark.svg` (12,515 B, `width="512" height="512" viewBox="0 0 512 512"`, `<g transform="translate(96, 96) scale(0.8)">`, pure white `#ffffff` dot-matrix play triangle)
- `brand-lockup.svg` (12,517 B, `width="740" height="220" viewBox="0 0 740 220"`, `<g transform="translate(280, 20) scale(0.45)">`)
- `icon.svg` (12,574 B, `512x512` with `<rect width="512" height="512" rx="128" fill="#11161E"/>` backing)
- `icon.png` (38,564 B), `icon-linux.png` (38,564 B), `icon.ico` (450,171 B), `video.ico` (450,171 B)

> **Brand mark vs lockup rule:** The UI uses the **dotted-triangle mark (`brand-mark.svg` geometry)** directly in the titlebar (`index.html:44-266`, `20x20`, `viewBox="-52 0 508 400"`, `fill="currentColor"` colored by `--accent`) and does **not** render `brand-lockup.svg` anywhere in the welcome screen or titlebar.

### 2.3 Complete SVG Inventory (64 Main UI + 3 Companion Auth = 67 Total)

#### A. Static HTML SVGs in `hybrid-player/src/renderer/index.html` (48 SVGs)
| # | Line | Container / Element | Dimensions / ViewBox | Purpose |
|---:|---:|---|---|---|
| 1 | 44 | `.titlebar-icon` | `20x20`, `-52 0 508 400` | Dotted-triangle brand mark |
| 2 | 273 | `#btnMinimize` | `12x12`, `0 0 12 12` | Minimize window dash |
| 3 | 278 | `#btnMaximize` | `12x12`, `0 0 12 12` | Maximize window square |
| 4 | 284 | `#btnClose` | `12x12`, `0 0 12 12` | Close window X |
| 5 | 313 | `#btnOpenFile` | `18x18`, `0 0 24 24` | Folder open icon |
| 6 | 319 | `#btnOpenLink` | `18x18`, `0 0 24 24` | Chain link icon |
| 7 | 363 | `#recordingIconCam` | CSS-sized, `0 0 24 24` | Recording camera badge |
| 8 | 381 | `#recordingStopSquare` | CSS `9x9`, `0 0 24 24` | Clip stop square |
| 9 | 384 | `#recordingStopFolder` | CSS-sized, `0 0 24 24` | Morph-to-folder icon on stop |
| 10 | 398 | `#recordingIconFolder` | CSS-sized, `0 0 24 24` | Saved burst/clip folder icon |
| 11 | 411 | `#unlockOverlay` | `18x18`, `0 0 24 24` | Padlock unlock icon |
| 12 | 439 | `#btnPrev` | `22x22`, `0 0 24 24` | Previous track |
| 13 | 444 | `#btnPlay .icon-play` | `32x32`, `0 0 24 24` | Play triangle |
| 14 | 447 | `#btnPlay .icon-pause` | `32x32`, `0 0 24 24` | Pause bars (initially hidden) |
| 15 | 453 | `#btnNext` | `22x22`, `0 0 24 24` | Next track |
| 16 | 461 | `#btnVolume .icon-vol-high` | `22x22`, `0 0 24 24` | Volume high |
| 17 | 465 | `#btnVolume .icon-vol-low` | `22x22`, `0 0 24 24` | Volume low (hidden) |
| 18 | 469 | `#btnVolume .icon-vol-mute` | `22x22`, `0 0 24 24` | Volume mute (hidden) |
| 19 | 494 | `#bgSettingsToggle` | `22x22`, `0 0 24 24` | Sparkles (background effects) |
| 20 | 510 | `#btnEqualizer` | `22x22`, `0 0 24 24` | 3-bar equalizer |
| 21 | 517 | `#btnAudioTrack` | `22x22`, `0 0 24 24` | Headphones (Subs/Dubs & audio) |
| 22 | 524 | `#btnSubtitles` | `22x22`, `0 0 24 24` | CC box |
| 23 | 533 | `#btnPlaylist` | `20x20`, `0 0 24 24` | Playlist lines |
| 24 | 539 | `#btnRepeat .icon-repeat` | `24x12`, `0 0 438 194` | Infinity / Repeat-One custom path |
| 25 | 546 | `#btnShuffle` | `20x20`, `0 0 24 24` | Crossing arrows |
| 26 | 554 | `#btnABLoop .icon-ab-loop` | `20x20`, `0 0 24 24` | Loop arrows + `A`/`AB` badge |
| 27 | 563 | `#btnLock` | `22x22`, `0 0 24 24` | Lock icon |
| 28 | 580 | `#btnSettings` | `22x22`, `0 0 24 24` | Gear icon |
| 29 | 588 | `#btnFullscreen .icon-fullscreen-enter` | `22x22`, `0 0 24 24` | Fullscreen enter |
| 30 | 591 | `#btnFullscreen .icon-fullscreen-exit` | `22x22`, `0 0 24 24` | Fullscreen exit (hidden) |
| 31 | 607 | `#btnClosePlaylist` | `18x18`, `0 0 24 24` | Sidebar close X |
| 32 | 617 | `#btnAddFiles` | `18x18`, `0 0 24 24` | Playlist plus |
| 33 | 623 | `#btnAddFolder` | `18x18`, `0 0 24 24` | Playlist folder |
| 34 | 630 | `#btnPlaylistRepeat .icon-repeat` | `22x11`, `0 0 438 194` | Sidebar repeat icon |
| 35 | 637 | `#btnClearPlaylist` | `18x18`, `0 0 24 24` | Sidebar trash |
| 36 | 661 | `#equalizerModal .modal-close` | `12x12`, `0 0 12 12` | Modal close X |
| 37 | 698 | `#speedModal .modal-close` | `12x12`, `0 0 12 12` | Modal close X |
| 38 | 744 | `#subtitleModal .modal-close` | `12x12`, `0 0 12 12` | Modal close X |
| 39 | 863 | `#settingsModal .modal-close` | `12x12`, `0 0 12 12` | Modal close X |
| 40 | 1007 | `#ytCookiesCard .yt-real-logo` | `0 0 28 20` | YouTube play badge (`#FF0000` + white triangle) |
| 41 | 1060 | `#networkStreamModal .modal-close` | `12x12`, `0 0 12 12` | Modal close X |
| 42 | 1121 | `.bg-accordion-item[data-bg="dither"]` | `14x14`, `0 0 24 24` | Active checkmark |
| 43 | 1130 | `.bg-accordion-item[data-bg="particles"]` | `14x14`, `0 0 24 24` | Active checkmark |
| 44 | 1139 | `.bg-accordion-item[data-bg="faulty"]` | `14x14`, `0 0 24 24` | Active checkmark |
| 45 | 1148 | `.bg-accordion-item[data-bg="gridmotion"]` | `14x14`, `0 0 24 24` | Active checkmark |
| 46 | 1157 | `.bg-accordion-item[data-bg="none"]` | `14x14`, `0 0 24 24` | Active checkmark |
| 47 | 1170 | `#bgSettingsModal .modal-close` | `12x12`, `0 0 12 12` | Modal close X |
| 48 | 1185 | `#bgEffectPickerBtn .bg-effect-picker-chevron` | `14x14`, `0 0 24 24` | Picker chevron down |

#### B. Dynamically Created Main UI SVGs in Renderer JS (16 SVGs)
| File | Lines | Count | Purpose |
|---|---:|---:|---|
| `src/components/toast.js` | 90, 126, 144, 150, 195 | 5 | Error pill close X (`11x11`), update download badge (`13x13`), "Update Now" button icon (`12x12`), update pill close X (`11x11`), success checkmark (`14x14`, `#22c55e`) |
| `src/renderer/app.js` | 416, 426, 435, 882 | 4 | `#btnMaximize` dynamic states: exit-fullscreen monitor (`12x12`), restore stacked squares (`12x12`), maximize square (`12x12`); plus `_loadRecentFiles()` play triangle (`16x16`) |
| `src/renderer/modules/controls.js` | 1579, 1586 | 2 | `#skip-indicator` forward double-arrow (`18x18`) & backward double-arrow (`18x18`) |
| `src/renderer/modules/player.js` | 1784, 1798 | 2 | Screenshot carousel `.carousel-nav-btn.prev` (`13x13`) & `.next` (`13x13`) |
| `src/renderer/modules/settings.js` | 1529, 2175 | 2 | Custom select option checkmark (`12x12`) & per-shortcut row reset arrow (`12x12`) |
| `src/renderer/modules/playlist.js` | 543 | 1 | Playlist empty-state list icon (`40x40`, `0 0 24 24`) |

#### C. Companion Auth Window SVGs (`hybrid-player/src/main/youtube-auth.js`, 3 SVGs)
| Line | Element | ViewBox / Size | Purpose |
|---:|---|---|---|
| 888 | `.yt-logo-svg` | `0 0 28 20` | YouTube header logo (`#FF0000` pill + white play) |
| 899 | `.shield-badge` | `12x12`, `0 0 20 20` | Verified security shield |
| 908 | `.step-num-done` | `10x10`, `0 0 16 16` | Step 1 completed checkmark |

---

## 3. Theme Tokens, Context Flips & Dynamic Accent Derivation

### 3.1 `:root` Shared Tokens (`themes.css:54-70`)
```css
--font-display: 'Sora Local', 'Segoe UI', sans-serif;
--font-ui: 'IBM Plex Sans Local', 'Segoe UI', sans-serif;
--font-mono: 'Consolas', 'SF Mono', monospace;
--brand-logo-size-sm: 20px;
--brand-logo-size-md: 34px;
--brand-logo-size-lg: 64px;
--brand-corner-radius: 14px;
--brand-glow-strength: 0.28;
--ytdlp-info: #00f2fe;
--ytdlp-ok: #4ade80;
--ytdlp-pending: #facc15;
--ytdlp-error: #ef4444;
```

### 3.2 Theme Palettes (`themes.css:73-208`)
| Token | `dark` (default) | `oled` | `light` |
|---|---|---|---|
| `--bg-primary` | `#0a0a0f` | `#000000` | `#f5f5fa` |
| `--bg-secondary` | `#12121a` | `#050505` | `#ffffff` |
| `--bg-tertiary` | `#1a1a25` | `#0a0a0a` | `#eaeaf0` |
| `--bg-elevated` | `#22222f` | `#111111` | `#ffffff` |
| `--bg-glass` | `rgba(18, 18, 26, 0.85)` | `rgba(5, 5, 5, 0.9)` | `rgba(255, 255, 255, 0.85)` |
| `--bg-glass-heavy` | `rgba(10, 10, 15, 0.95)` | `rgba(0, 0, 0, 0.98)` | `rgba(245, 245, 250, 0.95)` |
| `--bg-controls` | `linear-gradient(to top, rgba(0,0,0,0.95) 0%, rgba(0,0,0,0.7) 50%, transparent 100%)` | `linear-gradient(to top, rgba(0,0,0,1) 0%, rgba(0,0,0,0.8) 50%, transparent 100%)` | `linear-gradient(to top, rgba(0,0,0,0.85) 0%, rgba(0,0,0,0.5) 50%, transparent 100%)` |
| `--bg-titlebar` | `linear-gradient(to bottom, rgba(0,0,0,0.8) 0%, transparent 100%)` | `linear-gradient(to bottom, rgba(0,0,0,0.9) 0%, transparent 100%)` | `linear-gradient(to bottom, rgba(255,255,255,0.9) 0%, transparent 100%)` |
| `--text-primary` | `#f0f0f5` | `#e8e8ed` | `#1a1a2e` |
| `--text-secondary` | `#b8b8cc` | `#a8a8b2` | `#4e4e64` |
| `--text-tertiary` | `#8a8aa0` | `#7a7a82` | `#6e6e86` |
| `--border-color` | `rgba(255, 255, 255, 0.08)` | `rgba(255, 255, 255, 0.12)` | `rgba(0, 0, 0, 0.08)` |
| `--border-focus` | `rgba(255, 255, 255, 0.2)` | `rgba(255, 255, 255, 0.15)` | `rgba(0, 0, 0, 0.2)` |
| `--accent` | `#e50914` | `#e50914` | `#e50914` |
| `--accent-hover` | `#ff1a26` | `#ff1a26` | `#cc0812` |
| `--accent-dim` | `rgba(229, 9, 20, 0.3)` | `rgba(229, 9, 20, 0.25)` | `rgba(229, 9, 20, 0.15)` |
| `--progress-bg` | `rgba(255, 255, 255, 0.15)` | `rgba(255, 255, 255, 0.1)` | `rgba(255, 255, 255, 0.25)` |
| `--progress-buffer` | `rgba(255, 255, 255, 0.25)` | `rgba(255, 255, 255, 0.2)` | `rgba(255, 255, 255, 0.4)` |
| `--scrollbar-bg` | `transparent` | `transparent` | `transparent` |
| `--scrollbar-thumb` | `rgba(255, 255, 255, 0.15)` | `rgba(255, 255, 255, 0.1)` | `rgba(0, 0, 0, 0.15)` |
| `--scrollbar-thumb-hover` | `rgba(255, 255, 255, 0.3)` | `rgba(255, 255, 255, 0.2)` | `rgba(0, 0, 0, 0.3)` |
| `--shadow-sm` | `0 2px 8px rgba(0, 0, 0, 0.3)` | `0 2px 8px rgba(0, 0, 0, 0.5)` | `0 2px 8px rgba(0, 0, 0, 0.08)` |
| `--shadow-md` | `0 4px 20px rgba(0, 0, 0, 0.5)` | `0 4px 20px rgba(0, 0, 0, 0.7)` | `0 4px 20px rgba(0, 0, 0, 0.12)` |
| `--shadow-lg` | `0 8px 40px rgba(0, 0, 0, 0.7)` | `0 8px 40px rgba(0, 0, 0, 0.9)` | `0 8px 40px rgba(0, 0, 0, 0.18)` |
| `--shadow-glow` | `0 0 20px var(--accent-dim)` | `0 0 20px var(--accent-dim)` | `0 0 20px var(--accent-dim)` |
| `--glass-blur` | `blur(20px)` | `blur(25px)` | `blur(20px)` |
| `--glass-border` | `1px solid rgba(255, 255, 255, 0.06)` | `1px solid rgba(255, 255, 255, 0.04)` | `1px solid rgba(0, 0, 0, 0.06)` |
| `--brand-glass-opacity` | `0.62` | `0.56` | `0.75` |
| `--brand-text-high` | `#e8edf6` | `#e8edf6` | `#1d2430` |
| `--brand-text-mid` | `#b8c3d6` | `#a4b1c7` | `#425470` |
| `--brand-card-bg` | `#111923` | `#0d141d` | `#f3f7fd` |
| `--brand-card-top` | `#1f2c3d` | `#182231` | `#dde7f7` |
| `--ytdlp-info` | *(inherits `:root` `#00f2fe`)* | *(inherits `:root` `#00f2fe`)* | `#0369a1` |
| `--ytdlp-ok` | *(inherits `:root` `#4ade80`)* | *(inherits `:root` `#4ade80`)* | `#15803d` |
| `--ytdlp-pending` | *(inherits `:root` `#facc15`)* | *(inherits `:root` `#facc15`)* | `#854d0e` |
| `--ytdlp-error` | *(inherits `:root` `#ef4444`)* | *(inherits `:root` `#ef4444`)* | `#b91c1c` |

### 3.3 Dynamic Accent Derivation (`settings.js:615-617, 1734-1736, 2780-2787`)
When `accentColor` (hex `#RRGGBB`, default `#e50914`) is chosen:
- `--accent`: `hex`
- `--accent-hover`: `_lighten(hex, 15)` -> adds `Math.round(2.55 * 15) = 38` (`0x26`) clamped to `255` per RGB channel
- `--accent-dim`: `${hex}4D` (exact 30% alpha suffix `0x4D` appended to 6-digit hex)

### 3.4 Light-Mode Context Flips & Always-Dark Top-Right Volume OSD
- **Titlebar over video vs light UI (`themes.css:211-225`):**
  - Default `[data-theme="light"] .titlebar`: `color: #ffffff; text-shadow: 0 1px 4px rgba(0, 0, 0, 0.75);`, `.titlebar-btn`: `color: rgba(255, 255, 255, 0.9);` (stays legible over dark video frames).
  - Flip when `body.welcome-active`, `body.modal-open`, or `body:has(.modal-overlay:not([hidden]))` in `[data-theme="light"]`: `.titlebar` flips to `color: var(--text-primary) !important; text-shadow: none !important;`, and `.titlebar-btn` flips to `color: var(--text-secondary) !important;`.
- **Always-dark Top-Right Volume OSD (`themes.css:227-232`, `animations.css:428-475`):**
  - Default dark/oled: `background: rgba(0, 0, 0, 0.72); border: 1px solid rgba(255, 255, 255, 0.14); color: #ffffff; box-shadow: 0 4px 18px rgba(0, 0, 0, 0.4);`, `.volume-osd-pct`: `color: #38bdf8; text-shadow: 0 0 12px rgba(56, 189, 248, 0.45);`.
  - Light theme override (`themes.css:227-232`): stays dark plate (`background: rgba(15, 23, 42, 0.85); border: 1px solid rgba(255, 255, 255, 0.22); color: #ffffff; box-shadow: 0 4px 20px rgba(0, 0, 0, 0.3);`).
- **Other Light-Mode Overrides:**
  - `.sidebar-playlist`: `background: var(--bg-secondary)` (`#ffffff`) (`themes.css:226`)
  - `.setting-version-badge`: `background: rgba(0,0,0,0.06); border: 1px solid rgba(0,0,0,0.18); color: #0f172a; font-weight: 600;` (`themes.css:233-238`)
  - `.recent-item`: `color: #1a1a2e; background: rgba(0,0,0,0.04);` hover `rgba(0,0,0,0.08)` / `#000000` (`themes.css:239-246`)
  - `.modal-close`: `background: rgba(0,0,0,0.04); border: 1px solid var(--border-color);` hover `rgba(0,0,0,0.08)` / `rgba(0,0,0,0.18)` (`themes.css:247-254`)
  - `.toast`: `background: rgba(255,255,255,0.98); border: 1px solid rgba(20,24,36,0.12); color: #1a1a2e; box-shadow: 0 8px 24px rgba(20,24,36,0.18);` (`animations.css:206-211`)
  - `.toast-update-pill`: light surface `rgba(255,255,255,0.94)` (`animations.css:386-417`), and `[data-theme="oled"] .toast-update-pill`: `#000000` (`animations.css:420-424`)
  - `[data-theme="light"] .bg-settings-panel, body[data-theme="light"][data-welcome-background] .bg-settings-panel, [data-theme="light"] .bg-accordion-popover`: `background: rgba(248,249,252,0.94) !important; border-color: rgba(0,0,0,0.12) !important; color: #11131a !important;` (`settings.css:2282-2293`)

---

## 4. Motion Profiles & All 38 Main CSS `@keyframes` (+ Companion Auth)

### 4.1 `motionProfile` Tokens (`main.css:14-84`)
| Profile | `--motion-fast` | `--motion-medium` | `--motion-slow` | Extra Behavior |
|---|---|---|---|---|
| `balanced` (default) | `120ms` | `200ms` | `320ms` | `--motion-ease-standard: cubic-bezier(0.2, 0.7, 0.2, 1)`, `--motion-ease-emphasized: cubic-bezier(0.18, 0.84, 0.24, 1)` |
| `reduced` (or `@media (prefers-reduced-motion: reduce)` when unset) | `90ms` | `120ms` | `180ms` | `.welcome-screen * { animation-duration: 1ms !important; animation-delay: 0ms !important; transition-duration: var(--motion-fast) !important; }`, disables cold-start/unfocused transforms & filters |
| `showcase` | `140ms` | `240ms` | `360ms` | Emphasized welcome entrance timing |

### 4.2 All 38 Main CSS `@keyframes` + Companion Auth Keyframes
1. **`animations.css` (19 keyframes):**
   - `fadeIn` (`6`): opacity `0 -> 1`
   - `fadeOut` (`11`): opacity `1 -> 0`
   - `modalSlideIn` (`16`): `scale(0.95) translateY(10px) opacity 0 -> scale(1) translateY(0) opacity 1`
   - `modalSlideOut` (`27`): reverse of `modalSlideIn`
   - `slideUp` (`38`): `translateY(20px) opacity 0 -> 0 / 1`
   - `slideDown` (`49`): `translateY(-20px) opacity 0 -> 0 / 1`
   - `slideInRight` (`60`): `translateX(20px) opacity 0 -> 0 / 1`
   - `skipIndicatorForwardPop` (`71`): `translate(-50%,-50%) scale(0.86) translateX(-6px)` -> `scale(1.04) translateX(2px)` -> `scale(1) 0`
   - `skipIndicatorBackwardPop` (`86`): mirrored X (`6px` -> `-2px` -> `0`)
   - `skipArrowWaveForward` (`101`): opacity `0.45 -> 1 -> 0.45`, `translateX(0 -> 2px -> 0)`
   - `skipArrowWaveBackward` (`112`): opacity `0.45 -> 1 -> 0.45`, `translateX(0 -> -2px -> 0)`
   - `pulse` (`123`): opacity `1 -> 0.6 -> 1`
   - `pulse-glow` (`128`): `drop-shadow(0 0 10px var(--accent-dim)) scale(1) -> drop-shadow(0 0 20px var(--accent)) scale(1.05)`
   - `spin` (`139`): `rotate(0deg) -> rotate(360deg)`
   - `ripple` (`144`): `scale(0) opacity 0.5 -> scale(2.5) opacity 0`
   - `toastIn` (`155`): `translateY(20px) opacity 0 -> 0 / 1`
   - `toastOut` (`166`): `translateY(0) opacity 1 -> translateY(-20px) opacity 0`
   - `pulseToastDot` (`278`): `scale(1) opacity 1 -> scale(1.3) opacity 0.55`
   - `toastSpin` (`360`): `to { transform: rotate(360deg); }`
2. **`controls.css` (2 keyframes):**
   - `hover-thumb-in` (`158`): hover seek thumbnail entrance
   - `hover-thumb-loading` (`163`): shimmer/loading state for hover seek thumbnail
3. **`main.css` (12 keyframes):**
   - `recordingPillIn` (`676`): `translate(-50%, -10px) scale(0.92) opacity 0 -> translate(-50%, 0) scale(1) opacity 1`
   - `recordingRubyBreath` (`754`): `scale(1) opacity 1` + `drop-shadow(0 0 6px rgba(255,59,48,0.75))` <-> `scale(0.94) opacity 0.68`
   - `recordingBtnPop` (`896`): `scale(1) -> 0.86 -> 1.08 -> 1`
   - `recordingDigitFlash` (`1015`): burst counter digit flash
   - `previewDeletePop` (`1143`): `scale(1) -> scale(1.04) rotate(-1deg) -> scale(0.75) translateY(24px) rotate(2deg) opacity 0`
   - `previewRestorePop` (`1149`): `scale(0.85) translateY(10px) opacity 0 -> scale(1.02) translateY(-2px) opacity 1 -> scale(1) translateY(0)`
   - `badgePopIn` (`1191`): screenshot preview badge pop-in
   - `screenshotNoticeIn` (`1464`): `translateY(-8px) scale(0.94) opacity 0 -> 0 / scale(1) / 1`
   - `faultyPickerSweep` (`1965`): `translateY(-100%) -> translateY(100%)`
   - `colorBendsPickerSpin` (`1975`): `rotate(8deg) -> rotate(368deg)`
   - `welcomeTitlebarIn` (`2423`): `opacity 0 translateY(-10px) -> 1 / 0`
   - `welcomeControlIn` (`2435`): `opacity 0 translateY(10px) -> 1 / 0`
4. **`settings.css` (5 keyframes):**
   - `yt-pulse` (`306`): YouTube live/connected status dot pulse
   - `dropdownFadeIn` (`924`): custom select menu open fade
   - `bgSectionIn` (`1078`): background settings section switch fade
   - `bgAccordionPopoverIn` (`1594`): `opacity 0 translateX(20px) scale(0.96) -> 1 / 0 / 1`
   - `shortcutPulse` (`2185`): shortcut key capture / highlight pulse
5. **Companion Auth Window (`youtube-auth.js`):**
   - `spin` (`744`): `to { transform: rotate(360deg); }` (same name as `animations.css:139`)
   - `pulse-ring` (`821`): unique companion-auth pulsing halo keyframe

---

## 5. Exact Z-Index Stack (`-100` to `10060`)

| Z-Index | Selector(s) | Source |
|---:|---|---|
| `-100` | Settings background fallback override (`!important`) | `settings.css:820` |
| `0` | `.mpv-container`, inactive welcome mounts | `main.css:495, 1503` |
| `1` | `#videoCanvas`, `.paused-frame-overlay`, active welcome mount, `.welcome-bg-control`, `.bg-accordion-live-mount`, `body[data-welcome-background='faulty'] .bg-settings-panel::after` | `main.css:370, 502, 1517, 1737, 2024`; `settings.css:1421, 1763` |
| `2` | `.welcome-content`, `.bg-settings-panel .modal-header/.bg-settings-body`, `.bg-accordion-preview` | `main.css:1541`; `settings.css:1465, 1713` |
| `3` | `.bg-accordion-check` | `settings.css:1847` |
| `5` | `.bg-accordion-content` | `settings.css:1805` |
| `10` | `.video-curtain`, `.screenshot-preview-badge`, `.carousel-nav-btn`, `.subtitle-overlay`, `.chapter-markers` | `main.css:400, 1160, 1365, 2591`; `controls.css:525` |
| `12` | `.loading-spinner`, `.carousel-mode .screenshot-preview-badge`, `.carousel-close-btn` | `animations.css:555`; `main.css:1258, 1303` |
| `15` | `.center-play-overlay`, `.welcome-screen` | `animations.css:532`; `main.css:1482` |
| `22` | `.network-loading-spinner` | `main.css:385` |
| `30` | `.seek-indicator`, `.progress-hover-thumb` | `animations.css:508`; `controls.css:107` |
| `40` | `.quality-dropdown`, `.welcome-bg-select` focus/popup layer | `controls.css:294`; `main.css:2064` |
| `50` | `.stats-overlay`, `.ab-loop-indicator` | `main.css:2612, 2643` |
| `54` | `.recording-indicator` (burst dynamic island pill) | `main.css:639` |
| `55` | `#skip-indicator` | `main.css:538` |
| `56` | `.unlock-overlay` | `main.css:1077` |
| `60` | `.top-right-volume-osd` | `animations.css:432` |
| `1000` | `.controls-bar`, `.custom-select-dropdown` | `controls.css:209`; `settings.css:896` |
| `1310` | `#btnClosePlaylist` | `playlist.css:67` |
| `1350` | `.controls-wrapper` | `controls.css:15` |
| `1400` | `.titlebar` | `main.css:172` |
| `1800` | `.sidebar-playlist` | `playlist.css:26` |
| `2000` | `.modal-overlay`, `.recording-indicator.clip-active` (`!important`) | `main.css:2671, 784` |
| `2100` | `.bg-accordion-popover` (`!important`) | `settings.css:1577` |
| `2200` | `.screenshot-preview` (standard preview) | `main.css:1110` |
| `2501` | `.recording-stop-btn` (`!important`, above titlebar `1400` & clip bezel `2000`) | `main.css:864` |
| `3000` | `.toast-container` | `animations.css:183` |
| `10005` | `.screenshot-preview.carousel-mode` | `main.css:1215` |
| `10015` | `.screenshot-notice` | `main.css:1422` |
| `10060` | `.drop-overlay` | `main.css:2898` |

---

## 6. Backdrop-Filter Inventory (23 Blur Rules + 7 `none` Overrides)

### 6.1 Blur Rules (23)
1. `animations.css:224`: `.toast-update-pill` -> `blur(28px) saturate(190%) !important`
2. `animations.css:446`: `.top-right-volume-osd` -> `blur(10px)`
3. `controls.css:748`: `.speed-panel` -> `blur(28px)`
4. `controls.css:900`: `.subtitle-panel` -> `blur(28px)`
5. `main.css:633`: `.recording-indicator` -> `blur(32px) saturate(190%)`
6. `main.css:780`: `.recording-indicator.clip-active` -> `blur(12px) saturate(150%) !important`
7. `main.css:1217`: `.screenshot-preview.carousel-mode` -> `blur(14px)`
8. `main.css:1265`: `.screenshot-preview.carousel-mode .screenshot-preview-badge` -> `blur(8px)`
9. `main.css:1309`: `.carousel-close-btn` -> `blur(6px)`
10. `main.css:1357`: `.carousel-nav-btn` -> `blur(8px)`
11. `main.css:1430`: `.screenshot-notice` -> `blur(14px)`
12. `main.css:1723`: `body[data-welcome-quality='medium'] .welcome-bg-settings` -> `blur(10px) saturate(125%)`
13. `main.css:1842`: `body[data-welcome-background='faulty'] .welcome-bg-settings` -> `blur(4px) saturate(110%)`
14. `main.css:1881`: `body[data-welcome-background='particles'] .welcome-bg-settings` -> `blur(20px) saturate(150%)`
15. `main.css:1948`: `body[data-welcome-background='none'] .welcome-bg-settings` -> `blur(10px)`
16. `main.css:2389`: `.welcome-recent` -> `blur(8px)`
17. `main.css:2614`: `.stats-overlay` -> `var(--glass-blur)`
18. `main.css:2691`: `.modal-panel` -> `var(--glass-blur)`
19. `main.css:2900`: `.drop-overlay` -> `blur(10px)`
20. `settings.css:15`: `.settings-modal` -> `blur(28px)`
21. `settings.css:684`: `.bg-settings-panel` -> `var(--welcome-picker-panel-blur, var(--glass-blur)) !important`
22. `settings.css:891`: `.custom-select-dropdown` -> `blur(16px)`
23. `settings.css:1574`: `.bg-accordion-popover` -> `var(--welcome-picker-panel-blur, var(--glass-blur))`

### 6.2 `backdrop-filter: none` Overrides (7)
1. `main.css:163`: `.titlebar` -> `none !important`
2. `main.css:196`: `.titlebar-drag` -> `none !important`
3. `main.css:1727`: `body[data-welcome-quality='low'] .welcome-bg-settings` -> `none`
4. `main.css:1812`: `body[data-welcome-background='dither'] .welcome-bg-settings` -> `none`
5. `main.css:2667`: `.modal-overlay` -> `none`
6. `main.css:2979`: `#bgSettingsModal.modal-overlay` -> `none`
7. `settings.css:1683`: `.bg-accordion-item` -> `none`

---

## 7. Chameleon Skins (`main.css:1568-1963`, `settings.css:1388-1459`)

`.bg-settings-panel` and `.bg-accordion-popover` skin themselves to the active `body[data-welcome-background]` attribute:

| Effect | `--bgp-accent` | `--bgp-fg-dim` | Panel Background / Overlay (`settings.css:1388-1459`) |
|---|---|---|---|
| `dither` | `#b8c4ff` | `rgba(232, 238, 255, 0.6)` | Dual 1px/3px repeating-linear scanlines (`0.03` / `0.025`) over `var(--bg-glass-heavy)` padding-box + `1px` border-box gradient; `--welcome-picker-panel-shadow: none` |
| `faulty` | `#A7EF9E` | `rgba(167, 239, 158, 0.6)` | Horizontal CRT scanlines over `rgba(7,18,11,0.95)->rgba(10,26,17,0.86)` + mint border gradient + `::after` `faultyPickerSweep 2.8s linear infinite` (`opacity: 0.25`) + monospace body (`'Consolas', 'Courier New', monospace`) |
| `particles` | `#7ec8ff` | `rgba(190, 220, 255, 0.62)` | Deep navy glass `rgba(8,14,31,0.94)->rgba(8,18,39,0.82)` + cyan/blue border gradient (`rgba(95,219,255,0.46)`) |
| `gridmotion` | `#4ae7d2` | `rgba(180, 235, 230, 0.62)` | Teal obsidian glass `rgba(8,18,20,0.95)->rgba(6,12,14,0.85)` + `#4ae7d2`/`#00c3ff` border gradient |
| `none` | `var(--accent, #e50914)` | `var(--text-secondary, #a0a0b0)` | `var(--bg-glass-heavy)`, `var(--glass-border)`, `var(--shadow-md)` |

- `#bgSettingsModal.modal-overlay` uses dimmed transparent backdrop (`background-color: rgba(0, 0, 0, 0.2); backdrop-filter: none;`, `main.css:2977-2980`) and hides `.welcome-content` (`opacity: 0; pointer-events: none;`, `main.css:2983-2986`) so live background shader edits remain clearly visible.

---

## 8. Pixel Geometry & Responsive Breakpoints

### 8.1 Titlebar (`main.css:155-299`)
- Height: `36px`, fixed top (`top: 0; left: 0; right: 0; z-index: 1400`), `background: transparent !important`, `backdrop-filter: none !important`.
- `.titlebar-drag`: `gap: 10px; padding-left: 14px;` (`padding-right: 140px` when `body.has-native-window-controls`).
- `.titlebar-icon`: `20x20` SVG mark (`color: var(--accent)`).
- `.titlebar-title`: `font-size: 12px; font-weight: 600; opacity: 0.9; letter-spacing: 0.3px; font-family: var(--font-display);`.
- `.yt-live-badge`: `margin-left: 8px; padding: 1px 7px; border-radius: 4px; font-size: 10px; font-weight: 700; letter-spacing: 1px; line-height: 16px;`.
- `.titlebar-btn`: `width: 46px; height: 100%;` (`width: 38px` at `@media (max-width: 720px)`).
  - Hover: `rgba(255, 255, 255, 0.16) !important`; Close hover: `#e81123 !important; color: #fff`.
- `body.playlist-open .titlebar`: `right: 320px; opacity: 1 !important;`.

### 8.2 Welcome Screen (`main.css:1476-1565, 2345-2515`)
- Fullscreen absolute under `.video-container` (`z-index: 15; background: #000000`).
- Canvas mounts (`z-index: 0` inactive -> `z-index: 1; opacity: 1` active): `#particlesMount`, `#faultyMount`, `#ditherMount`, dynamic `.gridmotion-mount`.
- `.welcome-content` (`z-index: 2`): `justify-content: flex-end; align-items: center; gap: 12px; padding: 0 18px 96px;` (`width: calc(100% - 320px)` on `body.playlist-open`).
- `.welcome-actions`: `gap: 10px`; `#btnOpenFile` (`.btn-primary`: `padding: 12px 28px; border-radius: 12px; font-size: 15px; font-weight: 600`), `#btnOpenLink` (`.btn-open-link`).
- `.welcome-recent`: `width: min(440px, calc(100% - 12px)); max-height: min(32vh, calc(100vh - 260px)); padding: 12px; border-radius: 14px;` (up to 5 recent items).
- `@media (max-height: 620px)`: `.welcome-content` `gap: 8px; padding-bottom: 88px;`, `.welcome-recent` `max-height: 26vh; padding: 10px;`, buttons shrink (`9px 20px` / `min-height: 40px`, `14px` font).

### 8.3 Control Bar & Progress Bar (`index.html:425-598`, `controls.css:7-740`)
- `.controls-wrapper`: `z-index: 1350; padding: 0 14px 12px; background: var(--bg-controls);`.
- `.progress-bar-container`: `height: 4px` (`6px` hover), `border-radius: 2px; margin-bottom: 10px;` (`margin-right: 320px` on `playlist-open`).
  - `.progress-handle`: `14x14` circle; `.progress-hover-thumb`: `bottom: 22px; width: min(192px, 60vw); aspect-ratio: 16 / 9; max-height: 108px; z-index: 30;`.
- `.controls-bar`: `z-index: 1000`, `width: calc(100% - 320px); margin-right: 320px` on `playlist-open`.
- **Exact Left Controls (`.controls-left`, `gap: 4px`):**
  1. `#btnPrev` (`40x40`)
  2. `#btnPlay` (`.ctrl-btn-play`: `44x44`)
  3. `#btnNext` (`40x40`)
  4. `#volumeContainer` (`margin-left: 10px`): `#btnVolume` (`40x40`) + `#volumeSliderTrack` (`width: 80px; margin-left: 8px`, range `0..200`) + `#volumeValue` (`width: 40px; margin-left: 12px`)
  5. `.time-display` (`#currentTime` / `#totalTime`, `font-size: 13px; margin-left: 8px; gap: 4px`)
- **Exact Right Controls (`.controls-right`, `gap: 4px`):**
  1. `#bgSettingsToggle`
  2. `#btnSpeed` (`.ctrl-btn-text`: `padding: 0 10px; font-size: 13px`)
  3. `#btnEqualizer`
  4. `#btnAudioTrack`
  5. `#btnSubtitles`
  6. `#btnPlaylist` (`aria-expanded`)
  7. `#btnRepeat` (`data-repeat="none|all|one"`)
  8. `#btnShuffle`
  9. `#btnABLoop`
  10. `#btnScreenshot` (**intentionally hidden in DOM** at `index.html:559` via inline `style="display:none !important; position:absolute; width:0; height:0; ..."` — triggered via hotkey/mouse/API)
  11. `#btnLock`
  12. `#youtubeQualityControl` (`hidden` when non-YouTube; `#btnYoutubeQuality` + `#youtubeQualityDropdown` `min-width: 160px; max-height: 240px; z-index: 40`)
  13. `#btnSettings`
  14. `#btnFullscreen`
  *(Note: `#btnPiP` and `#btnRecordClip` exist in `controls.css` suppression selectors for legacy compatibility, not in `index.html`.)*

#### Responsive Breakpoints (`controls.css`, `main.css`, `settings.css`)
| Breakpoint | Component | Exact Rule Changes |
|---|---|---|
| `playlist-open` (all widths) | Controls / Titlebar / Welcome / Panels | Hides `#btnRepeat`, `#btnShuffle`, `#btnABLoop`, `#btnLock`, `#bgSettingsToggle` (`controls.css:248-256`); `.titlebar` `right: 320px`; `.controls-bar` `width: calc(100% - 320px)`; `.progress-bar-container` `margin-right: 320px`; `.welcome-content` `width: calc(100% - 320px)`; `.top-right-volume-osd` & `.ab-loop-indicator` `right: 348px`; `.bg-settings-panel` & `.bg-accordion-popover` `--bgp-right: 340px` |
| `@media (max-width: 1150px)` + `playlist-open` | Control Bar | Hides `#btnEqualizer`, `#btnAudioTrack`, `#btnSubtitles`, `#btnScreenshot`, `#btnSpeed`, `.volume-value`; `.ctrl-btn` `24x24`, `.ctrl-btn-play` `28x28`, `.volume-slider-track` `34px`, `.time-display` `10.5px` |
| `@media (max-width: 1000px)` | Control Bar | `.controls-wrapper` `padding: 0 8px 8px`; `.ctrl-btn` `34x34` (`18x18` svg); `.ctrl-btn-play` `38x38` (`24x24` svg); `.volume-slider-track` `60px`; `.volume-value` `34px; 11px`; `.time-display` `11.5px` |
| `@media (max-width: 950px)` | Background Settings & Accordion | `.bg-settings-panel` `right: 16px; bottom: 72px; width: min(44vw, 300px); max-height: calc(100vh - 116px)`; `.bg-accordion-popover` `left: 16px; right: calc(16px + min(44vw, 300px) + 12px); bottom: 72px; width: auto; max-width: none`; `.bg-accordion-gallery` `height: min(200px, calc(100vh - 210px))` |
| `@media (max-width: 860px)` | Control Bar | Hides `#btnRepeat`, `#btnShuffle`, `#btnABLoop`, `#btnLock`, `#bgSettingsToggle` (`!important`); `.ctrl-btn` `32x32`; `.ctrl-btn-text` `padding: 0 5px; 11px`; `.volume-slider-track` `50px` |
| `@media (max-width: 760px)` | Control Bar | `.controls-bar` `padding: 4px 8px`; `.ctrl-btn` `28x28`; `.ctrl-btn-play` `32x32`; `.volume-slider-track` `40px`; hides `.volume-value`, `#btnRepeat`, `#btnShuffle`, `#btnABLoop`, `#btnLock`, `#btnAudioTrack`, `#btnYoutubeQuality`, `#bgSettingsToggle`; `.time-display` `11px` (`10px` when `playlist-open`, also hides `#btnPrev` & `#btnNext`) |
| `@media (max-width: 720px)` | Titlebar & Legacy Picker | `.titlebar-btn` `width: 38px`; `.welcome-bg-settings` `top: 42px; right: 10px` |
| `@media (max-width: 640px)` | Accordion & Playlist Drawer | `.bg-accordion-gallery` switches `flex-direction: column; height: auto`; `.bg-accordion-item` `min-height: 64px; flex-direction: row`; `.sidebar-playlist` `width: min(320px, 100vw)`; `playlist-open` hides `.time-display` & `.volume-value` |
| `@media (max-height: 620px)` | Equalizer & Welcome | Equalizer `.eq-slider-wrap` `height: 110px`, tighter paddings; Welcome content `gap: 8px; padding-bottom: 88px;`, smaller buttons & `26vh` recent list |

### 8.4 Playlist Drawer (`playlist.css:11-286`, `playlist.js:16-18, 476-537`)
- Width: `320px` (`min(320px, 100vw)` at `<=640px`), `z-index: 1800`, GPU `transform: translateX(100%)` when `.collapsed` (`0.35s cubic-bezier(0.16, 1, 0.3, 1)`).
- `.sidebar-header`: `height: 36px; padding: 0 14px;` matching titlebar height; `#btnClosePlaylist` `28x28`, `z-index: 1310`.
- `.sidebar-toolbar`: `padding: 6px 16px; gap: 6px;`; `.sidebar-btn` `36x36`.
- `.playlist-item`: `height: 39px; margin-bottom: 1px; padding: 0 16px; border-left: 3px solid transparent;` -> **exact `ITEM_HEIGHT = 40` px stride**.
- **Virtualization (`playlist.js:476-537`):**
  - `filtered.length <= 80`: renders all items in a single `DocumentFragment`.
  - `filtered.length > 80`: spacer `height = totalHeight` + absolutely positioned slice from `Math.max(0, Math.floor(scrollTop / 40) - 5)` to `Math.min(len, Math.ceil((scrollTop + clientHeight) / 40) + 5)` (`±5` item overscan buffer).

### 8.5 OSDs, Recording Bezel Tab & Screenshot Carousel
- **Top-Right Volume OSD (`.top-right-volume-osd`, `animations.css:428-501`):** `top: 48px; right: 28px; z-index: 60; padding: 6px 14px; border-radius: 20px; font-size: 28px; font-weight: 900` (`.volume-osd-pct` `16px; #38bdf8`). Shifts `right: 348px` when `playlist-open`, `top: 28px` when `is-fullscreen`, `top: 92px` when `ab-loop-active`.
- **Skip Indicator (`#skip-indicator`, `main.css:525-585`, `controls.js:1550-1617`):** Centered pill (`z-index: 55`), `padding: 8px 20px 8px 10px; border-radius: 999px;`, `34x34` badge, accumulates same-direction skips within `800ms`, hides after `850ms`.
- **Recording / Burst Indicator & Clip Bezel Tab (`.recording-indicator`, `main.css:622-915`):**
  - Burst mode: `top: 48px; left: 50%; height: 30px; border-radius: 999px; z-index: 54`.
  - Clip Bezel Tab (`.clip-active`): `position: fixed; top: 0; left: 50%; height: 28px; border-radius: 0 0 10px 10px; background: rgba(10,12,18,0.92); backdrop-filter: blur(12px) saturate(150%); z-index: 2000`.
  - `.recording-stop-btn`: `20x20; border-radius: 5px; background: #e11d48; z-index: 2501` with `::after { inset: -6px }` expanded click hitbox and `-webkit-app-region: no-drag`. Morphs green (`#10b981`) on stop.
- **Unlock Overlay (`.unlock-overlay`, `main.css:1070-1093`):** `top: 54px; right: 18px; z-index: 56; padding: 10px 14px; border-radius: 999px;`.
- **Stats Overlay (`.stats-overlay`, `main.css:2608-2621`):** `top: 50px; left: 15px; z-index: 50; padding: 14px 18px; border-radius: 12px; min-width: 220px; font-size: 12px` monospace.
- **A-B Loop Indicator (`.ab-loop-indicator`, `main.css:2637-2660`):** `top: 102px; right: 15px; z-index: 50; padding: 6px 14px; border-radius: 20px;` (`right: 348px` on `playlist-open`).
- **Subtitle Overlay (`.subtitle-overlay`, `main.css:2590-2605`):** `bottom: 80px; max-width: 80%; z-index: 10; font-size: 28px; line-height: 1.4`.
- **Screenshot Preview, Carousel & Notice (`main.css:1103-1463`):**
  - Standard preview: `position: fixed; top: 86px; left: 24px; width: 260px; max-width: 32vw; padding: 4px; border-radius: 12px; z-index: 2200`.
  - Carousel mode (`.screenshot-preview.carousel-mode`, `Ctrl+D` / `Ctrl+Z`): `z-index: 10005; opacity: 0.82` (hover `1`), `16:9` wrapper, badge (`top: 10px; left: 10px; z-index: 12`), `.carousel-close-btn` (`22x22; top: 10px; right: 10px; z-index: 12`), `.carousel-nav-btn` prev/next (`26x26; z-index: 10`).
  - Notice pill (`.screenshot-notice`): `top: 86px; left: 24px; max-width: 260px; padding: 7px 14px; border-radius: 9px; z-index: 10015`.
  - When `#statsOverlay:not([hidden])`, both `.screenshot-preview` and `.screenshot-notice` shift to `top: 296px`.
- **Toasts (`animations.css:178-424`, `toast.js`):** `.toast-container` `bottom: 80px; z-index: 3000; gap: 8px;` (max 3 simultaneous). Standard `.toast` `padding: 10px 20px; border-radius: 10px; font-size: 13px`. `.toast-update-pill` pill (`border-radius: 9999px; padding: 6px 9px 6px 12px; gap: 11px`).

### 8.6 All 7 Modals + `#bgAccordionPopover`

| ID | Panel Class & Geometry | Key Controls / Contents |
|---|---|---|
| `#equalizerModal` | `.equalizer-panel`: `max-width: 920px; width: min(94vw, 920px); max-height: min(88vh, 700px); min-height: 360px` bands | Preset select (`flat`, `bass-boost`, `treble-boost`, `vocal`, `rock`, `pop`, `jazz`, `classical`, `electronic`, `custom`), 10 vertical sliders (`31Hz`..`16K`, `-12..12` dB step `0.5`), `#eqReset` |
| `#speedModal` | `.speed-panel`: `max-width: 440px; width: 92%; border-radius: 20px` | Slow (`0.25x, 0.5x, 0.75x`), Normal (`1.0x`), Fast (`1.25x, 1.5x, 1.75x, 2x, 3x, 4x`), `#customSpeedSlider` (`0.1..4` step `0.05`) |
| `#subtitleModal` | `.subtitle-panel`: `max-width: 470px; width: 92%; max-height: 88vh; border-radius: 20px` | Track pills + `#btnLoadSubtitle`, sync slider `#subSyncSlider` (`-10..+10s` step `0.1` + `-0.5/-0.1/Reset/+0.1/+0.5`), `#subFontSize` (`12..64px`), `#subFontColor`, `#subBgColor`, `#subBgOpacity` (`0..100%`) |
| `#audioModal` | `.audio-panel.subtitle-panel` (`max-width: 470px; width: 92%; max-height: 88vh`) | `#btnQuickSubs`, `#btnQuickDubs`, `#audioTracks` pills, sync `-100ms` / `Reset` / `+100ms` |
| `#settingsModal` | `.settings-modal`: `max-width: 520px; width: 92%; max-height: 88vh; border-radius: 20px` | Appearance (`#settTheme`, `#settAccentColor`, `#settBrandFontEnabled`), Behavior (`#settAutoResume`, `#settMotionProfile`, `#settSeekStep`, `#settDoubleClickFullscreen`, `#settShowUnlockButton`), Screenshots (folder path, `#settScreenshotFormat`, `#settAutoOrganizeShots`, `#settScreenshotMouseBtn`), Shortcuts search + rebinding + Reset All, `yt-dlp` status + `#ytCookiesCard` (`Sign in to YouTube` / `Switch Account` / `Sign Out`), Player Actions (`Open File/Multiple/Folder/Stream/Quit`) |
| `#networkStreamModal` | `.modal-small.network-stream-modal`: `max-width: 520px; width: 90%; border-radius: 16px` | `#networkStreamInput` URL field, Cancel & Play actions |
| `#bgSettingsModal` | `.modal-small.bg-settings-panel`: `right: 20px; bottom: 80px; width: min(92vw, 360px); max-height: min(calc(100vh - 124px), 640px)` | `#bgEffectPickerBtn`, hidden `#welcomeBackgroundSelect`, Quality selector (`low`, `medium`, `high`, `custom`), plus all **27 effect controls** (`BG_CONTROLS`) & Grid Pictures uploader |
| `#bgAccordionPopover` | `.bg-accordion-popover`: `z-index: 2100; right: calc(var(--bgp-right, 20px) + min(92vw, 360px) + 14px); bottom: 80px; width: min(92vw, 860px, calc(100vw - 32px)); max-width: min(calc(100vw - 410px), calc(100vw - 32px))` | 5 radio cards (`dither`, `particles`, `faulty`, `gridmotion`, `none`) in `.bg-accordion-gallery` (`height: 240px`), `.bg-accordion-item` (`min-width: 44px`, flex `1` -> `2.0` hover -> `3.8` active), live `400x240` WebGL/GSAP preview mounts |

### 8.7 All 27 Welcome Background Controls, Defaults & Quality Presets (`settings.js:22-84`, `index.html:1206-1348`)

#### `BG_CONTROLS` (7 Dither + 6 Particles + 8 Faulty + 6 Grid Motion = 27)
| Effect | ID | Label | Type | Range / Options | Default (`BG_DEFAULTS`) |
|---|---|---|---|---|---|
| `dither` | `bgDitherSpeed` | Speed | `range` | `0.01..0.2`, step `0.01` | `0.05` |
| `dither` | `bgDitherFrequency` | Frequency | `range` | `1..8`, step `0.5` | `3` |
| `dither` | `bgDitherAmplitude` | Amplitude | `range` | `0.1..0.8`, step `0.05` | `0.3` |
| `dither` | `bgDitherColor` | Color | `color` | `#RRGGBB` | `#808080` |
| `dither` | `bgDitherBgColor` *(injected by `settings.js:340`)* | Background Color | `color` | `#RRGGBB` | `#000000` |
| `dither` | `bgDitherPixelSize` | Pixel Size | `range` | `1..8`, step `1` | `1` *(HTML attr `2`, synced via `BG_DEFAULTS` `1`)* |
| `dither` | `bgDitherColorNum` | Color Depth | `range` | `2..8`, step `1` | `4` |
| `particles` | `bgParticlesCount` | Count | `range` | `50..800`, step `50` | `300` |
| `particles` | `bgParticlesSpeed` | Speed | `range` | `0.02..0.5`, step `0.02` | `0.1` |
| `particles` | `bgParticlesSpread` | Spread | `range` | `2..20`, step `1` | `10` |
| `particles` | `bgParticlesColor` | Color | `color` | `#RRGGBB` | `#ffffff` |
| `particles` | `bgParticlesSize` | Size | `range` | `20..300`, step `10` | `100` |
| `particles` | `bgParticlesAlpha` | Alpha | `checkbox` | boolean | `false` |
| `faulty` | `bgFaultyScale` | Scale | `range` | `0.5..8`, step `0.1` | `1.2` |
| `faulty` | `bgFaultyGlitch` | Glitch | `range` | `0..3`, step `0.1` | `1` |
| `faulty` | `bgFaultyScanlines` | Scanlines | `range` | `0..1.5`, step `0.05` | `0.7` |
| `faulty` | `bgFaultyFlicker` | Flicker | `range` | `0..2`, step `0.1` | `1` |
| `faulty` | `bgFaultyAberration` | Aberration | `range` | `0..5`, step `0.5` | `0` |
| `faulty` | `bgFaultyCurvature` | Curvature | `range` | `0..0.3`, step `0.02` | `0.2` *(HTML attr `0.1`, synced via `BG_DEFAULTS` `0.2`)* |
| `faulty` | `bgFaultyTint` | Tint | `color` | `#RRGGBB` | `#A7EF9E` |
| `faulty` | `bgFaultyBrightness` | Brightness | `range` | `0.2..1.5`, step `0.05` | `0.8` |
| `gridmotion` *(injected by `settings.js:173`)* | `bgGridmotionAutoMove` | Auto Move | `checkbox` | boolean | `false` |
| `gridmotion` | `bgGridmotionScale` | Zoom | `range` | `0.4..1.4`, step `0.05` | `1.0` |
| `gridmotion` | `bgGridmotionAspectRatio` | Card Ratio | `select` | `16:9`, `1:1`, `3:4`, `4:3`, `auto` | `16:9` |
| `gridmotion` | `bgGridmotionSpeed` | Speed | `range` | `0.1..3.0`, step `0.1` | `1.0` |
| `gridmotion` | `bgGridmotionMaxMove` | Max Distance | `range` | `50..600`, step `10` | `300` |
| `gridmotion` | `bgGridmotionGradientColor` | Background Color | `color` | `#RRGGBB` | `#000000` |

*(Plus `#bgGridmotionFileInput`, `#bgGridmotionSelectBtn`, `#bgGridmotionClearBtn`, `#bgGridmotionThumbnails` custom image uploader for `customPictures[]`.)*

#### `BG_QUALITY_PRESETS` (`low`, `medium` [default], `high`)
- **`low`**: `dither` (`freq: 2.55, amp: 0.26, pixelSize: 4`), `particles` (`count: 135, speed: 0.09, spread: 9, size: 85`), `faulty` (`scale: 0.8, glitch: 0.65, scanlines: 0.46, flicker: 0.65, curvature: 0.16`), `gridmotion` (`speed: 0.5, maxMove: 150`).
- **`medium` (default)**: `dither` (`freq: 2.85, amp: 0.29, pixelSize: 3`), `particles` (`count: 210, speed: 0.1, spread: 10, size: 95`), `faulty` (`scale: 1.2, glitch: 0.85, scanlines: 0.6, flicker: 0.85, curvature: 0.18`), `gridmotion` (`speed: 1.0, maxMove: 300`).
- **`high`**: `dither` (`freq: 3, amp: 0.3, pixelSize: 1`), `particles` (`count: 300, speed: 0.1, spread: 10, size: 100`), `faulty` (`scale: 1.4, glitch: 1, scanlines: 0.7, flicker: 1, curvature: 0.2`), `gridmotion` (`speed: 1.5, maxMove: 450`).

---

## 9. Keyboard Shortcuts (`SHORTCUT_CATALOG` — 52 Actions) & `screenshotMouseButton`

Source: `hybrid-player/src/renderer/modules/shortcuts.js:19-83, 144, 164, 177-189` and `index.html:965-970` (`#settScreenshotMouseBtn`).

### 9.1 `screenshotMouseButton` Allowed Values & Default
- Valid integer values: `[-1, 1, 3, 4]` (default `3` per `shortcuts.js:144, 164, 178`):
  - `3`: `Mouse 4 (Back Button)` (`selected` default in `index.html:966`)
  - `4`: `Mouse 5 (Forward Button)` (`index.html:967`)
  - `1`: `Middle Click` (`index.html:968`)
  - `-1`: `Off` (`index.html:969`)
- Any persisted preference outside `[-1, 1, 3, 4]` falls back to `3`.

### 9.2 Complete 52-Entry `SHORTCUT_CATALOG` (6 Categories)

| # | Category | `id` | `action` | `label` | `defaultCombo` |
|---:|---|---|---|---|---|
| 1 | Screenshots & Recording | `screenshot` | `screenshot` | Take Screenshot (Hold for Burst) | `KeyS` |
| 2 | Screenshots & Recording | `screenshot-png` | `screenshot-png` | Take Screenshot (PNG) | `Shift+KeyS` |
| 3 | Screenshots & Recording | `delete-latest-screenshot` | `delete-latest-screenshot` | Delete Latest Screenshot | `KeyD` |
| 4 | Screenshots & Recording | `restore-latest-screenshot` | `restore-latest-screenshot` | Restore Latest Screenshot | `KeyZ` |
| 5 | Screenshots & Recording | `toggle-screenshot-carousel` | `toggle-screenshot-carousel` | Delete Selected Screenshot | `Ctrl+KeyD` |
| 6 | Screenshots & Recording | `toggle-restore-carousel` | `toggle-restore-carousel` | Restore Selected Screenshot | `Ctrl+KeyZ` |
| 7 | Screenshots & Recording | `toggle-recording` | `toggle-recording` | Record Video Clip | `Ctrl+KeyS` |
| 8 | Playback | `toggle-play` | `toggle-play` | Play / Pause | `Space` |
| 9 | Playback | `toggle-play-alt` | `toggle-play` | Play / Pause (Secondary) | `KeyK` |
| 10 | Playback | `stop` | `stop` | Stop Playback | *(empty `''`)* |
| 11 | Playback | `previous-track` | `previous-track` | Previous Track | `KeyP` |
| 12 | Playback | `next-track` | `next-track` | Next Track | `KeyN` |
| 13 | Playback | `speed-down` | `speed-down` | Decrease Speed (-0.25x) | `BracketLeft` |
| 14 | Playback | `speed-up` | `speed-up` | Increase Speed (+0.25x) | `BracketRight` |
| 15 | Playback | `frame-backward` | `frame-backward` | Previous Frame | `Comma` |
| 16 | Playback | `frame-forward` | `frame-forward` | Next Frame | `Period` |
| 17 | Playback | `toggle-repeat` | `toggle-repeat` | Cycle Repeat Mode | `KeyL` |
| 18 | Playback | `toggle-ab-loop` | `toggle-ab-loop` | A-B Loop (Set A / Set B / Clear) | `KeyR` |
| 19 | Seeking & Navigation | `seek-back-5` | `seek-back-5` | Seek Backward | `ArrowLeft` |
| 20 | Seeking & Navigation | `seek-forward-5` | `seek-forward-5` | Seek Forward | `ArrowRight` |
| 21 | Seeking & Navigation | `seek-0` | `seek-0` | Jump to 0% | `Digit0` |
| 22 | Seeking & Navigation | `seek-10` | `seek-10` | Jump to 10% | `Digit1` |
| 23 | Seeking & Navigation | `seek-20` | `seek-20` | Jump to 20% | `Digit2` |
| 24 | Seeking & Navigation | `seek-30` | `seek-30` | Jump to 30% | `Digit3` |
| 25 | Seeking & Navigation | `seek-40` | `seek-40` | Jump to 40% | `Digit4` |
| 26 | Seeking & Navigation | `seek-50` | `seek-50` | Jump to 50% | `Digit5` |
| 27 | Seeking & Navigation | `seek-60` | `seek-60` | Jump to 60% | `Digit6` |
| 28 | Seeking & Navigation | `seek-70` | `seek-70` | Jump to 70% | `Digit7` |
| 29 | Seeking & Navigation | `seek-80` | `seek-80` | Jump to 80% | `Digit8` |
| 30 | Seeking & Navigation | `seek-90` | `seek-90` | Jump to 90% | `Digit9` |
| 31 | Audio & Subtitles | `volume-up` | `volume-up` | Volume Up (+5%) | `ArrowUp` |
| 32 | Audio & Subtitles | `volume-down` | `volume-down` | Volume Down (-5%) | `ArrowDown` |
| 33 | Audio & Subtitles | `toggle-mute` | `toggle-mute` | Mute / Unmute | `KeyM` |
| 34 | Audio & Subtitles | `cycle-audio` | `cycle-audio` | Cycle Audio Track (Subs / Dubs) | `KeyA` |
| 35 | Audio & Subtitles | `toggle-equalizer` | `toggle-equalizer` | Audio Equalizer | `KeyE` |
| 36 | Audio & Subtitles | `toggle-subtitles` | `toggle-subtitles` | Subtitles Panel | `KeyC` |
| 37 | Audio & Subtitles | `sub-delay-down` | `sub-delay-down` | Subtitle Delay -0.1s | `KeyG` |
| 38 | Audio & Subtitles | `sub-delay-up` | `sub-delay-up` | Subtitle Delay +0.1s | `KeyH` |
| 39 | View & Interface | `toggle-fullscreen` | `toggle-fullscreen` | Toggle Fullscreen | `KeyF` |
| 40 | View & Interface | `exit-fullscreen` | `exit-fullscreen` | Exit Fullscreen | `Escape` |
| 41 | View & Interface | `toggle-playlist` | `toggle-playlist` | Toggle Playlist Sidebar | `Ctrl+KeyL` |
| 42 | View & Interface | `toggle-stats` | `toggle-stats` | Stats for Nerds | `KeyI` |
| 43 | View & Interface | `toggle-time-format` | `toggle-time-format` | Toggle Time Format | `KeyT` |
| 44 | View & Interface | `toggle-bg-settings` | `toggle-bg-settings` | Welcome Background Settings | `KeyB` |
| 45 | View & Interface | `unlock-ui` | `unlock-ui` | Unlock Controls (When Locked) | `Ctrl+KeyU` |
| 46 | View & Interface | `toggle-lock` | `toggle-lock` | Lock / Unlock Controls | *(empty `''`)* |
| 47 | File & Application | `open-file` | `open-file` | Open File | `Ctrl+KeyO` |
| 48 | File & Application | `open-multiple-files` | `open-multiple-files` | Open Multiple Files | `Ctrl+Shift+KeyO` |
| 49 | File & Application | `open-folder` | `open-folder` | Open Folder | `Ctrl+KeyF` |
| 50 | File & Application | `open-network-stream` | `open-network-stream` | Open Network Stream / URL | `Ctrl+KeyN` |
| 51 | File & Application | `toggle-settings` | `toggle-settings` | Open Settings | `Ctrl+Comma` |
| 52 | File & Application | `quit` | `quit` | Quit Player | `Ctrl+KeyQ` |

*(Count summary: 7 Screenshots & Recording + 11 Playback + 12 Seeking & Navigation + 8 Audio & Subtitles + 8 View & Interface + 6 File & Application = **52 entries**.)*

---

## 10. Phase 6 Playwright Test Coverage Matrix (29 Specs, 194 Tests)

Helper: `hybrid-player/tests/evidence-utils.js` + `hybrid-player/playwright.config.js` (`workers: 2`, `HYBRID_MUTE_AUDIO=1`, `NODE_ENV=test`, `--mute-audio`).

| Spec File | Tests | Primary Feature / Migration Phase Parity |
|---|---:|---|
| `audio-track.spec.js` | 5 | Audio modal button order, anime Subs/Dubs presets, track pills, sync delay, `A` hotkey |
| `binary-resolver.spec.js` | 3 | Bundled `ffmpeg`/`ffprobe` path resolution without `PATH` fallback |
| `burst-screenshot.spec.js` | 8 | Single tap (`<200ms`) vs hold burst (`S` / Mouse 4), custom dir, WebP sync, clip stop bezel z-index |
| `custom-shortcuts.spec.js` | 6 | Shortcut catalog, live rebinding (`S->P`), `Esc` cancel, `Reset All`, Shift+S PNG, locked-mode screenshots & `Ctrl+U` |
| `database-persistence.spec.js` | 7 | 100-entry history cap, `<5s` resume guard, per-file speed, missing file handling, restart resume (`930s`) |
| `equalizer-audio.spec.js` | 6 | 200% boost + mute restore, `lavfi=[superequalizer=...]` syntax for all 10 bands / 9 presets, gain clamping, mpv sync delay |
| `full-feature-smoke.spec.js` | 9 | Welcome mounts, background shaders lifecycle, accordion gallery, EQ, playlist drawer, settings, stream modal, volume, DB |
| `fullscreen-input.spec.js` | 6 | `f` / `Shift+F`, input focus guard, double-click `#videoCanvas` & welcome screen, `Esc` exit |
| `fullscreen-unfullscreen-maximized.spec.js` | 3 | Maximized/windowed restoration across fullscreen transitions, 3-state `#btnMaximize` SVG updates |
| `gpu-context-loss.spec.js` | 4 | Aspect ratio (`object-fit: contain`) across extreme viewports, WebGL/DirectX bridge context reset & rapid resize |
| `launch.spec.js` | 12 | Welcome screen, background modules, diagnostics, titlebar hover colors, fullscreen curtain, idle hide, edge lock, missing recent files |
| `lock-and-autohide.spec.js` | 4 | Lock mode interaction/shortcut blocking, drag/modal auto-hide guards, 2.5s auto-hide timer |
| `media-folder-resolver.spec.js` | 9 | Anime episode/season/OVA/movie/YouTube folder resolution + auto-organize setting persistence |
| `media-resilience.spec.js` | 9 | 0-byte/corrupt/truncated/missing files, audio visualizer mode, rapid seek, Unicode/URL chars/`>260`-char Windows paths |
| `multi-monitor-dwm.spec.js` | 5 | DPI titlebar hit-testing exclusions, `is-animating-bounds`, maximize/restore, multi-monitor bounds |
| `network-resilience.spec.js` | 6 | Stream protocol detection, offline/online auto-pause/resume, `LIVE` badge & live-edge seek clamping |
| `playlist-navigation.spec.js` | 7 | Next/prev, click-video-to-collapse drawer, playlist vs single-file resume rules, clear title reset, welcome width transition |
| `playlist-stress.spec.js` | 6 | 1,000-item `<500ms` load & `>80` virtualization, shuffle without replacement, repeat `all` wrap, search, delete active |
| `rapid-switch-stress.spec.js` | 5 | 30 rapid file/next switches, rapid play/pause, speed changes, fullscreen toggles |
| `real-playback.spec.js` | 3 | Real `mpv` playback, DirectX texture bridge frame rendering, seek skip indicator without spinner |
| `resize-flicker.spec.js` | 4 | `WS_CLIPCHILDREN`/`WS_CLIPSIBLINGS`, `WM_NCCALCSIZE` `WVR_REDRAW`, frame continuity during resize |
| `restore-carousel.spec.js` | 10 | `Ctrl+Z` restore carousel navigation, empty warning notice, `Z`/`Enter`/mouse restore, `Ctrl+D`<->`Ctrl+Z` mode switch |
| `screenshot-carousel.spec.js` | 9 | `Ctrl+D` delete carousel, `3 / 3` newest default, arrow navigation without seeking, `D` delete, `Z` undo, `Esc` in fullscreen |
| `subtitle-lifecycle.spec.js` | 9 | Drag-drop discrimination, UTF-16 LE / Win-1252 / ISO-8859-1 encodings, auto-companion `.srt`/`.ass`/`.vtt`, delay & visibility |
| `subtitle-sync.spec.js` | 6 | `#subSyncSlider`, seconds badge, steppers, `G`/`H` shortcut keys |
| `youtube-cookies.spec.js` | 9 | YouTube sign-in UI, cookie export, signed-in badge/Switch/Sign Out, startup session sync |
| `youtube-option-a.spec.js` | 10 | Chrome/Edge locator, Netscape cookie rows, headless CDP extraction, graceful close |
| `youtube-video-4k.spec.js` | 13 | URL validation, 4K/8K format building, quality dropdown, timestamp-preserving quality switch, `mpv` 4K playback |
| `ytdlp-notification.spec.js` | 2 | Obsidian capsule `.toast-update-pill` rendering, update action, dismiss button |
