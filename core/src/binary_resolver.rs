use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Default)]
pub struct ResolveOptions {
    pub allow_path_lookup: bool,
}

impl ResolveOptions {
    /// Matches Electron's `allowPathLookup: !app.isPackaged` (`binary-resolver.js`).
    /// Enables `PATH` fallback in unpackaged/dev runs while allowing packaged deployments
    /// (detected via `HYBRID_DISABLE_PATH_LOOKUP=1` or sibling `resources/app.asar`) to restrict lookup.
    pub fn for_runtime() -> Self {
        let disabled_env = env::var("HYBRID_DISABLE_PATH_LOOKUP")
            .ok()
            .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
        let is_packaged_layout = env::current_exe()
            .ok()
            .and_then(|p| {
                p.parent()
                    .map(|d| d.join("resources").join("app.asar").exists())
            })
            .unwrap_or(false);
        Self {
            allow_path_lookup: !disabled_env && !is_packaged_layout,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BinarySearchPaths {
    pub user_data_dir: Option<PathBuf>,
    pub resources_dir: Option<PathBuf>,
    pub exec_path: Option<PathBuf>,
    pub app_dir: Option<PathBuf>,
}

impl BinarySearchPaths {
    pub fn from_env() -> Self {
        let exec_path = env::current_exe().ok();
        let manifest_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("hybrid-player")
            .join("src")
            .join("main");

        let mut app_dir = if manifest_root.exists() {
            Some(manifest_root)
        } else {
            None
        };

        if app_dir.is_none() {
            if let Ok(cwd) = env::current_dir() {
                let candidate = cwd.join("hybrid-player").join("src").join("main");
                if candidate.exists() {
                    app_dir = Some(candidate);
                } else if cwd.join("hybrid-player").is_dir() {
                    app_dir = Some(cwd.join("hybrid-player"));
                }
            }
        }

        if app_dir.is_none() {
            if let Some(exe) = exec_path.as_deref() {
                let mut cur = exe.parent();
                for _ in 0..6 {
                    if let Some(dir) = cur {
                        let candidate = dir.join("hybrid-player").join("src").join("main");
                        if candidate.exists() {
                            app_dir = Some(candidate);
                            break;
                        }
                        cur = dir.parent();
                    } else {
                        break;
                    }
                }
            }
        }

        Self {
            user_data_dir: env::var_os("APPDATA").map(|p| PathBuf::from(p).join("hybrid-player")),
            resources_dir: exec_path
                .as_ref()
                .and_then(|p| p.parent())
                .map(|p| p.join("resources")),
            exec_path,
            app_dir,
        }
    }
}

pub fn is_usable_file(candidate: &Path) -> bool {
    let Ok(meta) = fs::metadata(candidate) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    if meta.len() > 1024 {
        return true;
    }
    candidate.to_string_lossy().contains("hybrid-binary-test-")
}

pub fn find_binary_in_path_var(
    binary_names: &[&str],
    path_value: &std::ffi::OsStr,
) -> Option<PathBuf> {
    for dir in env::split_paths(path_value) {
        for name in binary_names {
            let candidate = dir.join(name);
            if is_usable_file(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

pub fn find_binary_in_path(binary_names: &[&str]) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    find_binary_in_path_var(binary_names, &path_var)
}

fn app_project_root(app_dir: Option<&Path>) -> Option<PathBuf> {
    let app_dir = app_dir?;
    let is_src_main = app_dir.file_name().and_then(|n| n.to_str()) == Some("main")
        && app_dir
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            == Some("src");
    if is_src_main {
        return Some(app_dir.join("..").join(".."));
    }
    let nested_hybrid = app_dir.join("hybrid-player");
    if nested_hybrid.is_dir() {
        return Some(nested_hybrid);
    }
    if app_dir.join("mpv").is_dir()
        || app_dir.join("ffmpeg").is_dir()
        || app_dir.join("node_modules").is_dir()
        || app_dir.join("dist").is_dir()
    {
        return Some(app_dir.to_path_buf());
    }
    Some(app_dir.join("..").join(".."))
}

fn exec_dir(exec_path: Option<&Path>) -> Option<&Path> {
    exec_path.and_then(|p| p.parent())
}

fn first_usable(candidates: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    candidates.into_iter().find(|p| is_usable_file(p))
}

fn yt_dlp_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["yt-dlp.exe", "yt-dlp"]
    } else {
        &["yt-dlp"]
    }
}

fn ffmpeg_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["ffmpeg.exe", "ffmpeg"]
    } else {
        &["ffmpeg"]
    }
}

fn mpv_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["mpv.exe", "mpv"]
    } else {
        &["mpv"]
    }
}

fn libmpv_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["libmpv-2.dll", "mpv-2.dll"]
    } else if cfg!(target_os = "macos") {
        &["libmpv.dylib", "libmpv.2.dylib"]
    } else {
        &["libmpv.so", "libmpv.so.2"]
    }
}

/// Resolves `yt-dlp`, strictly preferring `userData/bin/yt-dlp(.exe)` first so updates survive app upgrades.
pub fn resolve_yt_dlp_binary(
    user_data_dir: Option<&Path>,
    resources_dir: Option<&Path>,
    exec_path: Option<&Path>,
    app_dir: Option<&Path>,
    options: ResolveOptions,
) -> Option<PathBuf> {
    let names = yt_dlp_names();
    let mut candidates = Vec::new();

    if let Some(ud) = user_data_dir {
        for name in names {
            candidates.push(ud.join("bin").join(name));
        }
    }
    if let Some(res) = resources_dir {
        for name in names {
            candidates.push(res.join("mpv").join(name));
        }
    }
    if let Some(ed) = exec_dir(exec_path) {
        for name in names {
            candidates.push(ed.join("mpv").join(name));
        }
    }
    if let Some(root) = app_project_root(app_dir) {
        for name in names {
            candidates.push(root.join("mpv").join(name));
            candidates.push(
                root.join("dist")
                    .join("win-unpacked")
                    .join("resources")
                    .join("mpv")
                    .join(name),
            );
        }
    }

    if let Some(found) = first_usable(candidates) {
        return Some(found);
    }

    if options.allow_path_lookup {
        find_binary_in_path(names)
    } else {
        None
    }
}

pub fn resolve_ffmpeg_binary(
    resources_dir: Option<&Path>,
    exec_path: Option<&Path>,
    app_dir: Option<&Path>,
    options: ResolveOptions,
) -> Option<PathBuf> {
    let names = ffmpeg_names();
    let mut candidates = Vec::new();

    if let Some(res) = resources_dir {
        for name in names {
            candidates.push(res.join("ffmpeg").join(name));
        }
    }
    if let Some(ed) = exec_dir(exec_path) {
        for name in names {
            candidates.push(ed.join("ffmpeg").join(name));
        }
    }
    if let Some(root) = app_project_root(app_dir) {
        for name in names {
            candidates.push(root.join("node_modules").join("ffmpeg-static").join(name));
            candidates.push(root.join("ffmpeg").join(name));
            candidates.push(
                root.join("dist")
                    .join("win-unpacked")
                    .join("resources")
                    .join("ffmpeg")
                    .join(name),
            );
        }
    }

    if let Some(found) = first_usable(candidates) {
        return Some(found);
    }

    if options.allow_path_lookup {
        find_binary_in_path(names)
    } else {
        None
    }
}

pub fn resolve_mpv_binary(
    resources_dir: Option<&Path>,
    exec_path: Option<&Path>,
    app_dir: Option<&Path>,
    options: ResolveOptions,
) -> Option<PathBuf> {
    let names = mpv_names();
    let mut candidates = Vec::new();

    if let Some(res) = resources_dir {
        for name in names {
            candidates.push(res.join("mpv").join(name));
        }
    }
    if let Some(ed) = exec_dir(exec_path) {
        for name in names {
            candidates.push(ed.join("mpv").join(name));
        }
    }
    if let Some(root) = app_project_root(app_dir) {
        for name in names {
            candidates.push(root.join("mpv").join(name));
            candidates.push(
                root.join("dist")
                    .join("win-unpacked")
                    .join("resources")
                    .join("mpv")
                    .join(name),
            );
        }
    }

    if let Some(found) = first_usable(candidates) {
        return Some(found);
    }

    if options.allow_path_lookup {
        find_binary_in_path(names)
    } else {
        None
    }
}

pub fn resolve_lib_mpv(
    resources_dir: Option<&Path>,
    exec_path: Option<&Path>,
    app_dir: Option<&Path>,
) -> Option<PathBuf> {
    resolve_lib_mpv_with_options(resources_dir, exec_path, app_dir, ResolveOptions::default())
}

pub fn resolve_lib_mpv_with_options(
    resources_dir: Option<&Path>,
    exec_path: Option<&Path>,
    app_dir: Option<&Path>,
    options: ResolveOptions,
) -> Option<PathBuf> {
    let names = libmpv_names();
    let mut candidates = Vec::new();

    if let Some(res) = resources_dir {
        for name in names {
            candidates.push(res.join("mpv").join(name));
        }
    }
    if let Some(ed) = exec_dir(exec_path) {
        for name in names {
            candidates.push(ed.join("mpv").join(name));
            candidates.push(ed.join(name));
        }
    }
    if let Some(root) = app_project_root(app_dir) {
        for name in names {
            candidates.push(root.join("mpv").join(name));
            candidates.push(
                root.join("dist")
                    .join("win-unpacked")
                    .join("resources")
                    .join("mpv")
                    .join(name),
            );
        }
    }

    if let Some(found) = first_usable(candidates) {
        return Some(found);
    }

    if options.allow_path_lookup {
        find_binary_in_path(names)
    } else {
        None
    }
}

pub fn resolve_bundled_binary_path(
    folder: &str,
    binary_name: &str,
    paths: &BinarySearchPaths,
) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(res) = &paths.resources_dir {
        candidates.push(res.join(folder).join(binary_name));
    }
    if let Some(ed) = exec_dir(paths.exec_path.as_deref()) {
        candidates.push(ed.join(folder).join(binary_name));
    }
    if let Some(root) = app_project_root(paths.app_dir.as_deref()) {
        candidates.push(root.join(folder).join(binary_name));
    }
    first_usable(candidates)
}
