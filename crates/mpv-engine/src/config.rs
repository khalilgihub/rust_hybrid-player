//! Startup configuration, observed property list, and libmpv path resolution.
//!
//! Mirrors `hybrid-player/src/main/mpv-process.js:216-298` and `:703-725` without
//! `--wid` or `input.conf` hacks.

use std::path::{Path, PathBuf};

pub const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/132.0.0.0 Safari/537.36";

/// All 21 properties observed on startup (`mpv-process.js:703-725`).
pub const OBSERVED_PROPERTIES: [&str; 21] = [
    "time-pos",
    "duration",
    "pause",
    "volume",
    "mute",
    "speed",
    "eof-reached",
    "track-list",
    "chapter-list",
    "chapter",
    "media-title",
    "video-params",
    "demuxer-cache-state",
    "sub-delay",
    "sub-visibility",
    "estimated-vf-fps",
    "video-bitrate",
    "audio-bitrate",
    "drop-frame-count",
    "paused-for-cache",
    "seeking",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenshotFormat {
    Jpg,
    Png,
    Webp,
}

impl ScreenshotFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Jpg => "jpg",
            Self::Png => "png",
            Self::Webp => "webp",
        }
    }

    pub fn from_str_lossy(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "jpg" | "jpeg" => Some(Self::Jpg),
            "png" => Some(Self::Png),
            "webp" => Some(Self::Webp),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenshotMode {
    Video,
    Subtitles,
    Window,
}

impl ScreenshotMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Subtitles => "subtitles",
            Self::Window => "window",
        }
    }
}

#[derive(Debug, Clone)]
pub struct MpvStartupConfig {
    pub libmpv_path: Option<PathBuf>,
    pub ytdl_path: Option<PathBuf>,
    pub cookies_path: Option<PathBuf>,
    pub enable_ytdl_raw_options: bool,
    pub ytdl_extractor_args: String,
    pub user_agent: String,
    pub screenshot_dir: PathBuf,
    pub screenshot_format: ScreenshotFormat,
    pub auto_organize_screenshots: bool,
    pub hwdec: String,
    pub hwdec_codecs: String,
    pub sub_ass_override: String,
    pub mute_audio_output: bool,
}

impl Default for MpvStartupConfig {
    fn default() -> Self {
        let screenshot_dir = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("screenshots");
        Self {
            libmpv_path: None,
            ytdl_path: None,
            cookies_path: None,
            enable_ytdl_raw_options: false,
            ytdl_extractor_args: String::new(),
            user_agent: DEFAULT_USER_AGENT.to_owned(),
            screenshot_dir,
            screenshot_format: ScreenshotFormat::Jpg,
            auto_organize_screenshots: true,
            hwdec: "d3d11va-zero-copy".to_owned(),
            hwdec_codecs: "h264,h265,vp9,av1".to_owned(),
            sub_ass_override: "scale".to_owned(),
            mute_audio_output: false,
        }
    }
}

impl MpvStartupConfig {
    /// Returns the exact ordered `(name, value)` option pairs from
    /// `mpv-process.js:216-298` for offscreen `vo=libmpv` rendering.
    pub fn build_option_pairs(&self) -> Vec<(String, String)> {
        let mut opts = Vec::with_capacity(36);
        let push = |opts: &mut Vec<(String, String)>, k: &str, v: &str| {
            opts.push((k.to_owned(), v.to_owned()));
        };

        push(&mut opts, "config", "no");
        push(&mut opts, "vo", "libmpv");
        push(&mut opts, "hwdec", &self.hwdec);
        push(&mut opts, "hwdec-codecs", &self.hwdec_codecs);

        push(&mut opts, "keepaspect-window", "no");
        push(&mut opts, "auto-window-resize", "no");
        push(&mut opts, "osc", "no");
        push(&mut opts, "osd-bar", "no");
        push(&mut opts, "osd-level", "0");
        push(&mut opts, "idle", "yes");
        push(&mut opts, "keep-open", "yes");
        push(&mut opts, "msg-level", "all=warn");
        push(&mut opts, "ytdl", "yes");
        push(&mut opts, "sub-auto", "fuzzy");
        push(&mut opts, "sub-file-paths", "subs:subtitles");
        push(&mut opts, "sub-fix-timing", "no");
        push(&mut opts, "demuxer-mkv-subtitle-preroll", "yes");
        push(&mut opts, "sub-ass-use-video-data", "all");

        let ass_override = match self.sub_ass_override.as_str() {
            "no" | "yes" | "scale" | "force" | "strip" => self.sub_ass_override.as_str(),
            _ => "scale",
        };
        push(&mut opts, "sub-ass-override", ass_override);

        let screenshot_dir_str = self.screenshot_dir.to_string_lossy().into_owned();
        push(&mut opts, "screenshot-directory", &screenshot_dir_str);
        push(
            &mut opts,
            "screenshot-template",
            "hybrid-player-%tY-%tm-%td-%tH-%tM-%tS",
        );
        push(
            &mut opts,
            "screenshot-format",
            self.screenshot_format.as_str(),
        );
        push(&mut opts, "screenshot-jpeg-quality", "92");
        push(&mut opts, "screenshot-webp-quality", "80");
        push(&mut opts, "screenshot-webp-compression", "0");
        push(&mut opts, "screenshot-webp-lossless", "no");

        push(&mut opts, "cache", "yes");
        push(&mut opts, "demuxer-max-bytes", "150MiB");
        push(&mut opts, "demuxer-max-back-bytes", "50MiB");

        push(&mut opts, "input-default-bindings", "no");
        push(&mut opts, "cursor-autohide", "no");
        push(&mut opts, "hr-seek", "yes");
        push(&mut opts, "volume-max", "200");
        push(&mut opts, "sub-codepage", "auto");
        push(&mut opts, "user-agent", &self.user_agent);

        if self.mute_audio_output {
            push(&mut opts, "ao", "null");
        }

        if let Some(ytdl_path) = &self.ytdl_path {
            let normalized = ytdl_path
                .to_string_lossy()
                .replace('\\', "/")
                .replace(',', "\\,");
            push(
                &mut opts,
                "script-opts",
                &format!("ytdl_hook-ytdl_path={normalized}"),
            );
        }

        if self.enable_ytdl_raw_options {
            let raw = build_ytdl_raw_options(
                &self.ytdl_extractor_args,
                &self.user_agent,
                self.cookies_path.as_deref(),
            );
            if !raw.is_empty() {
                push(&mut opts, "ytdl-raw-options", &raw);
            }
        }

        opts
    }

    /// Returns CLI flags for launching external `mpv.exe --wid=<hwnd>` fallback (`mpv-process.js:420-506`).
    pub fn build_subprocess_args(&self, pipe_name: &str, wid_hwnd: Option<isize>) -> Vec<String> {
        let safe_ua: String = self
            .user_agent
            .chars()
            .map(|c| {
                if matches!(c, '\r' | '\n' | ',') {
                    ' '
                } else {
                    c
                }
            })
            .take(512)
            .collect();
        let ass_override = match self.sub_ass_override.as_str() {
            "no" | "yes" | "scale" | "force" | "strip" => self.sub_ass_override.as_str(),
            _ => "scale",
        };
        let mut args = vec![
            "--idle=yes".to_owned(),
            "--keep-open=yes".to_owned(),
            "--no-terminal".to_owned(),
            "--msg-level=all=warn".to_owned(),
            "--no-osc".to_owned(),
            "--no-osd-bar".to_owned(),
            "--osd-level=0".to_owned(),
            format!("--input-ipc-server={pipe_name}"),
            format!("--hwdec={}", self.hwdec),
            format!("--hwdec-codecs={}", self.hwdec_codecs),
            if wid_hwnd.is_some() {
                "--vo=gpu".to_owned()
            } else {
                "--vo=null".to_owned()
            },
            "--ytdl=yes".to_owned(),
            "--sub-auto=fuzzy".to_owned(),
            "--sub-file-paths=subs:subtitles".to_owned(),
            "--sub-fix-timing=no".to_owned(),
            "--demuxer-mkv-subtitle-preroll=yes".to_owned(),
            "--sub-ass-use-video-data=all".to_owned(),
            format!("--sub-ass-override={ass_override}"),
            format!(
                "--screenshot-directory={}",
                self.screenshot_dir.to_string_lossy()
            ),
            "--screenshot-template=hybrid-player-%tY-%tm-%td-%tH-%tM-%tS".to_owned(),
            format!("--screenshot-format={}", self.screenshot_format.as_str()),
            "--screenshot-jpeg-quality=92".to_owned(),
            "--screenshot-webp-quality=80".to_owned(),
            "--screenshot-webp-compression=0".to_owned(),
            "--screenshot-webp-lossless=no".to_owned(),
            "--cache=yes".to_owned(),
            "--demuxer-max-bytes=150MiB".to_owned(),
            "--demuxer-max-back-bytes=50MiB".to_owned(),
            "--no-config".to_owned(),
            "--input-default-bindings=no".to_owned(),
            "--input-vo-keyboard=no".to_owned(),
            "--cursor-autohide=no".to_owned(),
            "--hr-seek=yes".to_owned(),
            "--hr-seek-framedrop=no".to_owned(),
            "--volume-max=200".to_owned(),
            "--sub-codepage=auto".to_owned(),
            format!("--user-agent={safe_ua}"),
        ];
        if self.mute_audio_output {
            args.push("--ao=null".to_owned());
        }
        if let Some(ytdl_path) = &self.ytdl_path {
            let normalized = ytdl_path
                .to_string_lossy()
                .replace('\\', "/")
                .replace(',', "\\,");
            args.push(format!("--script-opts=ytdl_hook-ytdl_path={normalized}"));
        }
        if self.enable_ytdl_raw_options {
            let raw = build_ytdl_raw_options(
                &self.ytdl_extractor_args,
                &self.user_agent,
                self.cookies_path.as_deref(),
            );
            if !raw.is_empty() {
                args.push(format!("--ytdl-raw-options={raw}"));
            }
        }
        if let Some(hwnd) = wid_hwnd {
            args.push(format!("--wid={hwnd}"));
        } else {
            args.push("--force-window=no".to_owned());
            args.push("--mute=yes".to_owned());
        }
        args
    }
}

/// Escapes `ytdl-raw-options` values matching `mpv-process.js:277-283` (`%<bytes>%<str>`).
pub fn format_ytdl_option_value(val: &str) -> String {
    if val
        .bytes()
        .any(|b| matches!(b, b',' | b'=' | b'"' | b'[' | b']'))
    {
        format!("%{}%{}", val.len(), val)
    } else {
        val.to_owned()
    }
}

pub fn build_ytdl_raw_options(
    extractor_args: &str,
    user_agent: &str,
    cookies_path: Option<&Path>,
) -> String {
    let mut parts = Vec::new();
    if !extractor_args.trim().is_empty() {
        parts.push(format!(
            "extractor-args={}",
            format_ytdl_option_value(extractor_args)
        ));
    }
    if !user_agent.trim().is_empty() {
        parts.push(format!(
            "user-agent={}",
            format_ytdl_option_value(user_agent)
        ));
    }
    if let Some(cookies) = cookies_path {
        if cookies.is_file() {
            let s = cookies.to_string_lossy();
            parts.push(format!("cookies={}", format_ytdl_option_value(&s)));
        }
    }
    parts.join(",")
}

/// Resolves `libmpv-2.dll` / `mpv-2.dll` from explicit hint, executable directory,
/// workspace `hybrid-player/mpv`, or `PATH`.
pub fn resolve_libmpv_path(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = explicit {
        if p.is_file() {
            return Some(p.to_path_buf());
        }
    }

    let names = ["libmpv-2.dll", "mpv-2.dll"];
    let mut candidates = Vec::new();

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for name in names {
                candidates.push(dir.join(name));
                candidates.push(dir.join("mpv").join(name));
            }
            // Walk up a few ancestors (covers `target/debug/deps` in cargo test)
            let mut cur = dir;
            for _ in 0..5 {
                if let Some(parent) = cur.parent() {
                    for name in names {
                        candidates.push(parent.join("mpv").join(name));
                        candidates.push(parent.join("hybrid-player").join("mpv").join(name));
                    }
                    cur = parent;
                }
            }
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        for name in names {
            candidates.push(cwd.join(name));
            candidates.push(cwd.join("mpv").join(name));
            candidates.push(cwd.join("hybrid-player").join("mpv").join(name));
        }
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if let Some(ws_root) = manifest_dir.parent().and_then(|p| p.parent()) {
        for name in names {
            candidates.push(ws_root.join("hybrid-player").join("mpv").join(name));
            candidates.push(ws_root.join("mpv").join(name));
        }
    }

    for cand in candidates {
        if cand.is_file() {
            return Some(cand);
        }
    }

    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            for name in names {
                let cand = dir.join(name);
                if cand.is_file() {
                    return Some(cand);
                }
            }
        }
    }

    None
}
