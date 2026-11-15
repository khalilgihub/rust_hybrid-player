use once_cell_like::LazyRegex;
use std::cmp::Ordering;
use std::collections::{HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const FOLDER_SCAN_MAX_DEPTH: usize = 8;
pub const FOLDER_SCAN_MAX_FILES: usize = 2000;
pub const FOLDER_SCAN_MAX_DIRS: usize = 5000;

/// Authoritative 30 supported media & playlist extensions from `hybrid-player/src/main/main.js`.
pub const MEDIA_EXTENSIONS: [&str; 30] = [
    ".mp4", ".mkv", ".avi", ".mov", ".webm", ".flv", ".m4v", ".wmv", ".mpg", ".mpeg", ".ts",
    ".m2ts", ".mts", ".vob", ".ogv", ".3gp", ".mp3", ".flac", ".wav", ".aac", ".ogg", ".m4a",
    ".wma", ".opus", ".aiff", ".alac", ".m3u", ".m3u8", ".pls", ".mpd",
];

const DOS_DEVICE_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

const GENERIC_PARENT_FOLDERS: &[&str] = &[
    "downloads",
    "desktop",
    "videos",
    "video",
    "vid",
    "movies",
    "series",
    "anime",
    "temp",
    "tmp",
    "media",
    "files",
    "hybrid-player",
    "screenshots",
    "new volume",
];

const PRESERVE_UPPERCASE_TOKENS: &[&str] = &[
    "I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX", "X", "OVA", "OAD", "SP", "PV", "MV",
    "OP", "ED", "TV", "HD", "4K",
];

mod once_cell_like {
    use regex::Regex;
    use std::sync::OnceLock;

    pub struct LazyRegex(OnceLock<Regex>, &'static str);

    impl LazyRegex {
        pub const fn new(pattern: &'static str) -> Self {
            Self(OnceLock::new(), pattern)
        }

        pub fn get(&self) -> &Regex {
            self.0
                .get_or_init(|| Regex::new(self.1).expect("valid static regex"))
        }
    }
}

#[derive(Debug, Error)]
pub enum MediaScanError {
    #[error("UNC/SMB network paths are not allowed: {0}")]
    UncPathRejected(String),
    #[error("root path does not exist or is not a directory: {0}")]
    InvalidRoot(String),
}

pub fn is_unc_or_network_path(raw: &str) -> bool {
    let trimmed = raw.trim();
    trimmed.starts_with(r"\\") || trimmed.starts_with("//")
}

pub fn is_media_extension(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    let dot_ext = format!(".{}", ext.to_ascii_lowercase());
    MEDIA_EXTENSIONS.contains(&dot_ext.as_str())
}

/// Natural case-insensitive numeric comparison matching JS `localeCompare(..., { numeric: true, sensitivity: 'base' })`.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut ai = a.as_bytes();
    let mut bi = b.as_bytes();

    while !ai.is_empty() && !bi.is_empty() {
        let a_digit = ai[0].is_ascii_digit();
        let b_digit = bi[0].is_ascii_digit();

        if a_digit && b_digit {
            let a_len = ai.iter().take_while(|c| c.is_ascii_digit()).count();
            let b_len = bi.iter().take_while(|c| c.is_ascii_digit()).count();

            let a_num_str = &ai[..a_len];
            let b_num_str = &bi[..b_len];
            let a_trim = a_num_str
                .iter()
                .position(|&c| c != b'0')
                .map_or(&[][..], |idx| &a_num_str[idx..]);
            let b_trim = b_num_str
                .iter()
                .position(|&c| c != b'0')
                .map_or(&[][..], |idx| &b_num_str[idx..]);

            let cmp = a_trim
                .len()
                .cmp(&b_trim.len())
                .then_with(|| a_trim.cmp(b_trim));
            if cmp != Ordering::Equal {
                return cmp;
            }
            // Tie-breaker by leading zeros count so deterministic ordering holds
            let len_cmp = a_len.cmp(&b_len);
            if len_cmp != Ordering::Equal {
                return len_cmp;
            }
            ai = &ai[a_len..];
            bi = &bi[b_len..];
        } else {
            // Decode UTF-8 char by char cleanly
            let a_str = std::str::from_utf8(ai).unwrap_or("");
            let b_str = std::str::from_utf8(bi).unwrap_or("");
            let Some(ac) = a_str.chars().next() else {
                break;
            };
            let Some(bc) = b_str.chars().next() else {
                break;
            };
            let cmp = ac
                .to_lowercase()
                .cmp(bc.to_lowercase())
                .then_with(|| ac.cmp(&bc));
            if cmp != Ordering::Equal {
                return cmp;
            }
            ai = &ai[ac.len_utf8()..];
            bi = &bi[bc.len_utf8()..];
        }
    }

    ai.len().cmp(&bi.len())
}

#[derive(Debug, Clone)]
pub struct ScanStats {
    pub files: Vec<PathBuf>,
    pub dirs_visited: usize,
    pub hit_file_limit: bool,
    pub hit_dir_limit: bool,
}

/// Scans `root` recursively up to `FOLDER_SCAN_MAX_DEPTH` (8), `FOLDER_SCAN_MAX_FILES` (2000),
/// and `FOLDER_SCAN_MAX_DIRS` (5000). Rejects UNC/SMB paths, skips symlinks and canonical duplicate
/// directories, and returns matching media paths sorted naturally.
pub fn scan_folder_for_media(root: &Path) -> Result<ScanStats, MediaScanError> {
    let raw = root.to_string_lossy();
    if is_unc_or_network_path(&raw) {
        return Err(MediaScanError::UncPathRejected(raw.into_owned()));
    }

    let root_meta =
        fs::symlink_metadata(root).map_err(|_| MediaScanError::InvalidRoot(raw.to_string()))?;
    if root_meta.file_type().is_symlink() || !root_meta.is_dir() {
        return Err(MediaScanError::InvalidRoot(raw.into_owned()));
    }

    let mut visited_dirs: HashSet<PathBuf> = HashSet::new();
    let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::new();
    if let Ok(canon) = fs::canonicalize(root) {
        visited_dirs.insert(canon);
    } else {
        visited_dirs.insert(root.to_path_buf());
    }
    queue.push_back((root.to_path_buf(), 0));

    let mut files = Vec::new();
    let mut dirs_visited = 0usize;
    let mut hit_file_limit = false;
    let mut hit_dir_limit = false;

    while let Some((dir, depth)) = queue.pop_front() {
        if dirs_visited >= FOLDER_SCAN_MAX_DIRS {
            hit_dir_limit = true;
            break;
        }
        dirs_visited += 1;

        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };

        for entry in entries.flatten() {
            if files.len() >= FOLDER_SCAN_MAX_FILES {
                hit_file_limit = true;
                break;
            }
            let path = entry.path();
            let Ok(meta) = fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                if depth < FOLDER_SCAN_MAX_DEPTH {
                    let key = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
                    if visited_dirs.insert(key) {
                        queue.push_back((path, depth + 1));
                    }
                }
            } else if meta.is_file() && is_media_extension(&path) {
                files.push(path);
            }
        }

        if hit_file_limit {
            break;
        }
    }

    files.sort_by(|a, b| natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));

    Ok(ScanStats {
        files,
        dirs_visited,
        hit_file_limit,
        hit_dir_limit,
    })
}

pub fn sanitize_windows_name(input: &str) -> String {
    if input.is_empty() {
        return String::new();
    }
    static RE_SEP: LazyRegex = LazyRegex::new(r"[:|\\/]+");
    static RE_ILLEGAL: LazyRegex = LazyRegex::new(r#"[<>?"*]"#);
    static RE_WS: LazyRegex = LazyRegex::new(r"\s+");
    static RE_DUP_DASH: LazyRegex = LazyRegex::new(r" - (?:[- ]+)+");

    let mut clean = input.replace([':', '|'], " - ");
    clean = clean.replace(['\\', '/'], " - ");
    let _ = &RE_SEP;
    clean = RE_ILLEGAL.get().replace_all(&clean, "").into_owned();
    clean = RE_WS.get().replace_all(&clean, " ").into_owned();
    while RE_DUP_DASH.get().is_match(&clean) {
        clean = RE_DUP_DASH.get().replace_all(&clean, " - ").into_owned();
    }
    clean = clean.trim().to_string();
    clean = clean
        .trim_start_matches([' ', '-'])
        .trim_end_matches([' ', '-', '.'])
        .to_string();
    clean = clean.chars().take(80).collect();

    if DOS_DEVICE_NAMES.contains(&clean.to_ascii_uppercase().as_str()) {
        clean.push('_');
    }
    clean
}

pub fn to_title_case(input: &str) -> String {
    if input.is_empty() {
        return String::new();
    }
    static RE_WORD: LazyRegex = LazyRegex::new(r"\b[a-zA-Z0-9']+\b");

    RE_WORD
        .get()
        .replace_all(input, |caps: &regex::Captures| {
            let word = &caps[0];
            let upper = word.to_ascii_uppercase();
            if PRESERVE_UPPERCASE_TOKENS.contains(&upper.as_str()) {
                return upper;
            }
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => {
                    let mut out = first.to_uppercase().to_string();
                    out.push_str(&chars.as_str().to_lowercase());
                    out
                }
            }
        })
        .into_owned()
}

fn is_four_digit_year_paren(inside: &str) -> bool {
    inside.len() == 4 && inside.bytes().all(|b| b.is_ascii_digit())
}

/// Strips `[...]` and non-`(YYYY)` `(...)` tags without regex lookahead (Rust `regex` has no lookahead).
pub fn strip_bracket_tags(raw: &str) -> String {
    if raw.is_empty() {
        return String::new();
    }
    static RE_CRC: LazyRegex = LazyRegex::new(r"\[[0-9a-fA-F]{8}\]");
    static RE_SCENE: LazyRegex = LazyRegex::new(
        r"(?i)\b(?:1080p|720p|480p|2160p|4k|hevc|x264|x265|bluray|web-dl|aac|flac|dual-audio|multi-subs)\b",
    );

    let mut cleaned: String = raw.chars().take(300).collect();

    // 1. Strip all leading [...] and non-(YYYY) (...)
    loop {
        let trimmed = cleaned.trim_start();
        if trimmed.starts_with('[') {
            if let Some(end) = trimmed.find(']') {
                cleaned = trimmed[end + 1..].to_string();
                continue;
            }
        } else if trimmed.starts_with('(') {
            if let Some(end) = trimmed.find(')') {
                let inside = &trimmed[1..end];
                if !is_four_digit_year_paren(inside) {
                    cleaned = trimmed[end + 1..].to_string();
                    continue;
                }
            }
        }
        break;
    }

    // 2. Strip trailing [...] and trailing non-(YYYY) (...)
    loop {
        let trimmed = cleaned.trim_end();
        if trimmed.ends_with(']') {
            if let Some(start) = trimmed.rfind('[') {
                cleaned = trimmed[..start].to_string();
                continue;
            }
        } else if trimmed.ends_with(')') {
            if let Some(start) = trimmed.rfind('(') {
                let inside = &trimmed[start + 1..trimmed.len() - 1];
                if !is_four_digit_year_paren(inside) {
                    cleaned = trimmed[..start].to_string();
                    continue;
                }
            }
        }
        break;
    }

    // 3. Strip standalone CRC32 hashes like [7F8A1B2C]
    cleaned = RE_CRC.get().replace_all(&cleaned, "").into_owned();

    // 4. Strip common scene resolution and codec tags
    cleaned = RE_SCENE.get().replace_all(&cleaned, "").into_owned();

    cleaned.trim().to_string()
}

fn roman_to_num(s: &str) -> Option<u32> {
    match s.to_ascii_uppercase().as_str() {
        "I" => Some(1),
        "II" => Some(2),
        "III" => Some(3),
        "IV" => Some(4),
        "V" => Some(5),
        "VI" => Some(6),
        "VII" => Some(7),
        "VIII" => Some(8),
        "IX" => Some(9),
        "X" => Some(10),
        _ => None,
    }
}

fn ordinal_to_num(s: &str) -> Option<u32> {
    match s.to_ascii_lowercase().as_str() {
        "1st" | "first" => Some(1),
        "2nd" | "second" => Some(2),
        "3rd" | "third" => Some(3),
        "4th" | "fourth" => Some(4),
        "5th" | "fifth" => Some(5),
        "final" => Some(99),
        _ => None,
    }
}

fn split_path_components_cross_platform(path_str: &str) -> (String, String) {
    // Handle both Windows `\` and Unix `/` separators cleanly for tests and runtime.
    let trimmed = path_str.trim();
    let parts: Vec<&str> = trimmed
        .split(['\\', '/'])
        .filter(|s| !s.is_empty())
        .collect();
    let file_part = parts.last().copied().unwrap_or("");
    let stem = match file_part.rfind('.') {
        Some(idx) if idx > 0 => &file_part[..idx],
        _ => file_part,
    };
    let parent = if parts.len() >= 2 {
        parts[parts.len() - 2]
    } else {
        ""
    };
    (stem.to_string(), parent.to_string())
}

pub fn resolve_screenshot_subfolder(file_path: &str, media_title: &str) -> PathBuf {
    static RE_URL: LazyRegex = LazyRegex::new(r"(?i)^(?:https?|rtmp|ytdl):");
    static RE_STREAM_SUFFIX: LazyRegex =
        LazyRegex::new(r"(?i)\s*[-|]\s*(?:YouTube|Crunchyroll|Netflix|Funimation)\s*$");
    static RE_STREAM_SERIES: LazyRegex = LazyRegex::new(
        r"(?i)^(.+?)\s+[-_:]?\s*(?:Season\s*(\d+)|S(\d+)|(2nd|3rd|4th|final)\s+Season)\s*(?:[-_:]\s*(.+))?$",
    );
    static RE_TRAILER: LazyRegex = LazyRegex::new(r"(?i)\b(?:trailer|pv|teaser|preview)\b");

    static RE_GENERIC_RECORDING: LazyRegex =
        LazyRegex::new(r"(?i)^(?:VID_\d|recording_\d|capture_\d|screenshot|image|video_\d|output)");
    static RE_SPECIAL: LazyRegex = LazyRegex::new(r"(?i)\b(?:OVA|OAD|SP\d*|Special|NCOP|NCED)\b");
    static RE_MOVIE: LazyRegex = LazyRegex::new(r"(?i)\b(?:Movie|The Movie|Film)\b");
    static RE_PATTERN_1: LazyRegex =
        LazyRegex::new(r"(?i)^(.*?)[-_\s]+S0*(\d{1,2})(?:E\d+|(?:-|_)?(?:v\d+)|\b)(.*)$");
    static RE_PATTERN_2: LazyRegex = LazyRegex::new(
        r"(?i)^(.*?)[-_\s]+(?:season|series)\s*0*(\d{1,2})(?:[-_\s]+(?:episode|ep)\s*\d+)?(.*)$",
    );
    static RE_PATTERN_3: LazyRegex =
        LazyRegex::new(r"(?i)^(.*?)\s+(I{1,3}|IV|V|VI)\s*[-_:]?\s*(?:\d+.*)?$");
    static RE_PATTERN_4: LazyRegex =
        LazyRegex::new(r"(?i)^(.*?)[-_\s]+(1st|2nd|3rd|4th|5th|final)\s+season\b(.*)$");
    static RE_PATTERN_5: LazyRegex =
        LazyRegex::new(r"(?i)^(.*?)[-_\s]+(?:E|EP|Episode|#)?\s*\d{1,3}(?:v\d+)?$");
    static RE_PARENT_SEASON: LazyRegex = LazyRegex::new(r"(?i)\b(?:Season|S)\s*0*(\d{1,2})\b");
    static RE_PARENT_FALLBACK: LazyRegex =
        LazyRegex::new(r"(?i)^(.*?)\s*[-_:(]?\s*(?:Season\s*(\d+)|S(\d+))\)?");
    static RE_MOVIE_YEAR: LazyRegex = LazyRegex::new(r"^(.+?\s*\(\d{4}\))$");
    static RE_BASE_SPLIT: LazyRegex = LazyRegex::new(r"^(.*?)\s*[-:]\s*.+$");

    let trimmed_path = file_path.trim();

    // Case A: Web Streams / YouTube
    if RE_URL.get().is_match(trimmed_path) {
        let raw_title = media_title.trim();
        if raw_title.is_empty() {
            return PathBuf::from("Streams");
        }
        let clean_title = RE_STREAM_SUFFIX
            .get()
            .replace(raw_title, "")
            .trim()
            .to_string();

        if let Some(caps) = RE_STREAM_SERIES.get().captures(&clean_title) {
            let raw_show = caps.get(1).map_or("", |m| m.as_str());
            let show_name = sanitize_windows_name(&to_title_case(&strip_bracket_tags(raw_show)));
            let season_num = caps
                .get(2)
                .or_else(|| caps.get(3))
                .and_then(|m| m.as_str().parse::<u32>().ok())
                .or_else(|| caps.get(4).and_then(|m| ordinal_to_num(m.as_str())));
            let extra = caps.get(5).map_or("", |m| m.as_str());
            let is_trailer =
                RE_TRAILER.get().is_match(extra) || RE_TRAILER.get().is_match(&clean_title);

            if !show_name.is_empty() {
                if is_trailer {
                    return PathBuf::from(show_name).join("Trailers");
                }
                if let Some(season) = season_num {
                    return PathBuf::from(show_name).join(format!("Season {season}"));
                }
                return PathBuf::from(show_name);
            }
        }

        let safe_title = sanitize_windows_name(&clean_title);
        return if safe_title.is_empty() {
            PathBuf::from("YouTube")
        } else {
            PathBuf::from("YouTube").join(safe_title)
        };
    }

    // Case B: Local Media Files
    if trimmed_path.is_empty() {
        return PathBuf::new();
    }

    let (raw_file_name, parent_folder) = split_path_components_cross_platform(trimmed_path);
    if raw_file_name.is_empty() || RE_GENERIC_RECORDING.get().is_match(&raw_file_name) {
        return PathBuf::new();
    }

    let mut working_name = strip_bracket_tags(&raw_file_name);
    if working_name.contains('.') && !working_name.contains(' ') {
        working_name = working_name.replace('.', " ");
    }

    let is_special = RE_SPECIAL.get().is_match(&raw_file_name);
    let is_movie = RE_MOVIE.get().is_match(&raw_file_name);

    let mut detected_season: Option<u32> = None;
    let mut clean_show_name = String::new();

    if let Some(caps) = RE_PATTERN_1.get().captures(&working_name) {
        clean_show_name = caps.get(1).map_or("", |m| m.as_str()).trim().to_string();
        detected_season = caps.get(2).and_then(|m| m.as_str().parse::<u32>().ok());
    }

    if detected_season.is_none() {
        if let Some(caps) = RE_PATTERN_2.get().captures(&working_name) {
            clean_show_name = caps.get(1).map_or("", |m| m.as_str()).trim().to_string();
            detected_season = caps.get(2).and_then(|m| m.as_str().parse::<u32>().ok());
        }
    }

    if detected_season.is_none() {
        if let Some(caps) = RE_PATTERN_3.get().captures(&working_name) {
            clean_show_name = caps.get(1).map_or("", |m| m.as_str()).trim().to_string();
            detected_season = caps.get(2).and_then(|m| roman_to_num(m.as_str()));
        }
    }

    if detected_season.is_none() {
        if let Some(caps) = RE_PATTERN_4.get().captures(&working_name) {
            clean_show_name = caps.get(1).map_or("", |m| m.as_str()).trim().to_string();
            detected_season = caps.get(2).and_then(|m| ordinal_to_num(m.as_str()));
        }
    }

    if detected_season.is_none() {
        if let Some(caps) = RE_PATTERN_5.get().captures(&working_name) {
            clean_show_name = caps.get(1).map_or("", |m| m.as_str()).trim().to_string();
            let parent_clean = strip_bracket_tags(&parent_folder);
            if let Some(p_caps) = RE_PARENT_SEASON.get().captures(&parent_clean) {
                detected_season = p_caps.get(1).and_then(|m| m.as_str().parse::<u32>().ok());
            } else {
                detected_season = Some(1);
            }
        }
    }

    if clean_show_name.is_empty() && !parent_folder.is_empty() {
        let clean_parent = strip_bracket_tags(&parent_folder);
        let parent_lower = clean_parent.to_ascii_lowercase();
        if !GENERIC_PARENT_FOLDERS.contains(&parent_lower.as_str()) {
            if let Some(caps) = RE_PARENT_FALLBACK.get().captures(&clean_parent) {
                if caps.get(2).is_some() || caps.get(3).is_some() {
                    clean_show_name = caps.get(1).map_or("", |m| m.as_str()).trim().to_string();
                    if detected_season.is_none() {
                        detected_season = caps
                            .get(2)
                            .or_else(|| caps.get(3))
                            .and_then(|m| m.as_str().parse::<u32>().ok());
                    }
                } else {
                    clean_show_name = clean_parent;
                }
            } else {
                clean_show_name = clean_parent;
            }
        }
    }

    let source_name = if clean_show_name.is_empty() {
        working_name.as_str()
    } else {
        clean_show_name.as_str()
    };
    let final_show_name = sanitize_windows_name(&to_title_case(&strip_bracket_tags(source_name)));

    if final_show_name.chars().count() < 2 {
        return PathBuf::new();
    }

    if detected_season.is_none() {
        if let Some(caps) = RE_MOVIE_YEAR.get().captures(&final_show_name) {
            return PathBuf::from(caps[1].trim());
        }
    }

    if is_special {
        let base_show = RE_BASE_SPLIT
            .get()
            .captures(&final_show_name)
            .map_or(final_show_name.as_str(), |c| {
                c.get(1).map_or("", |m| m.as_str()).trim()
            });
        return PathBuf::from(base_show).join("Specials");
    }

    if is_movie {
        let base_show = RE_BASE_SPLIT
            .get()
            .captures(&final_show_name)
            .map_or(final_show_name.as_str(), |c| {
                c.get(1).map_or("", |m| m.as_str()).trim()
            });
        return PathBuf::from(base_show).join("Movies");
    }

    if let Some(season) = detected_season {
        return PathBuf::from(final_show_name).join(format!("Season {season}"));
    }

    PathBuf::from(final_show_name)
}
