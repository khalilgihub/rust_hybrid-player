# Phase 0 Golden Reference Captures

Captured from the live Electron `hybrid-player` application (`v12.6.0`, Electron `^40.10.2`, Playwright `_electron.launch`) on `2026-11-12` into `.rust-migration/golden/`. Full machine-readable metadata is in [capture-manifest.json](file:///c:/Users/abdul/OneDrive/Documents/rust%20vidplayer/.rust-migration/golden/capture-manifest.json).

## Captured Screenshots (27 PNGs)

### 1. Responsive Breakpoints x Themes (Welcome Screen, `bg=dither`)
| File | Viewport (`innerWidth x innerHeight`) | Theme | Bytes |
| --- | --- | --- | --- |
| `welcome-1280x720-dark.png` | `1280x720` | `dark` | `37408` |
| `welcome-1280x720-oled.png` | `1280x720` | `oled` | `36395` |
| `welcome-1280x720-light.png` | `1280x720` | `light` | `35300` |
| `welcome-900x600-dark.png` | `900x600` | `dark` | `28200` |
| `welcome-900x600-oled.png` | `900x600` | `oled` | `27761` |
| `welcome-900x600-light.png` | `900x600` | `light` | `28162` |
| `welcome-640x360-dark.png` | `640x360` | `dark` | `21766` |
| `welcome-640x360-oled.png` | `640x360` | `oled` | `21170` |
| `welcome-640x360-light.png` | `640x360` | `light` | `21297` |

### 2. Welcome Background Shaders (`1280x720`, `dark`)
| File | Background | Bytes | Notes |
| --- | --- | --- | --- |
| `shader-dither-1280x720.png` | `dither` | `39036` | Full 8x8 Bayer Perlin wave render verified visually |
| `shader-particles-1280x720.png` | `particles` | `21502` | See **Known Capture Limitation** below (`nonZeroPixels: 0`) |
| `shader-faulty-1280x720.png` | `faulty` | `241501` | CRT dot-matrix + scanlines + curvature render verified visually |
| `shader-gridmotion-1280x720.png` | `gridmotion` | `817146` | `-12deg` tilted gradient/text lattice render verified visually |
| `shader-none-1280x720.png` | `none` | `21502` | Pure `#000000` welcome background |

### 3. Modals & Background Accordion Popover (`1280x720`, `dark`, `bg=dither`)
| File | Element | Bytes |
| --- | --- | --- |
| `modal-equalizerModal-1280x720.png` | `#equalizerModal` | `44028` |
| `modal-speedModal-1280x720.png` | `#speedModal` | `65385` |
| `modal-subtitleModal-1280x720.png` | `#subtitleModal` | `68634` |
| `modal-audioModal-1280x720.png` | `#audioModal` | `60977` |
| `modal-settingsModal-1280x720.png` | `#settingsModal` | `74720` |
| `modal-networkStreamModal-1280x720.png` | `#networkStreamModal` | `46510` |
| `modal-bgSettingsModal-1280x720.png` | `#bgSettingsModal` | `43794` |
| `modal-bgAccordionPopover-1280x720.png` | `#bgSettingsModal` + `#bgAccordionPopover` | `93246` |

### 4. Drawer, Overlays & Sample Playback (`1280x720`, `dark`, `bg=dither`)
| File | State / Overlay | Bytes |
| --- | --- | --- |
| `playlist-open-1280x720.png` | Open `#playlistPanel` (`320px` right drawer) | `56598` |
| `overlay-clip-bezel-tab-1280x720.png` | Top bezel clip tab (`#recordingIndicator.active.clip-active`, `00:00.0` + stop button) | `42755` |
| `overlay-burst-pill-1280x720.png` | Burst screenshot recording pill (`#recordingIndicator.active`, count `12`) | `48301` |
| `overlay-screenshot-carousel-1280x720.png` | `#screenshotPreview.carousel-mode` (`2 / 2` synthetic SMPTE-style test PNG) | `63045` |
| `playback-sample-1280x720.png` | Active `libmpv` video playback (`hybrid-player/test-sample.mp4`) + full player control bar | `292064` |

## Known Capture Limitation (`shader-particles-1280x720.png`)

When switching to `particles` inside Electron in this environment, even with `win.show(); win.focus()`, `document.hasFocus() === true`, `#particlesMount` computed `opacity === "1"` and `zIndex === "1"`, and the WebGL canvas mounted at `1536x816` (DPR `1.2` scaled or `1.0` at `1280x680` body), `gl.readPixels(...)` across the entire `particlesMount` canvas returned `nonZeroPixels: 0` (solid `#000000`, matching `shader-none-1280x720.png` at `21502` bytes). No capture script error was thrown (`errors: []` in `capture-manifest.json`). Therefore, `crates/shaders` (`particles.wgsl`) must follow the authoritative WebGL vertex/fragment specification in `hybrid-player/src/renderer/modules/particles.js` (documented in [tokens-and-metrics.md](file:///c:/Users/abdul/OneDrive/Documents/rust%20vidplayer/.rust-migration/tokens-and-metrics.md)) rather than visually sampling `shader-particles-1280x720.png`.
