use serde::{Deserialize, Serialize};

pub const AUDIO_SYNC_MIN_MS: i64 = -600_000;
pub const AUDIO_SYNC_MAX_MS: i64 = 600_000;

pub const SUBTITLE_DELAY_MIN_MS: i64 = -600_000;
pub const SUBTITLE_DELAY_MAX_MS: i64 = 600_000;
pub const SUBTITLE_SYNC_MIN_SEC: f64 = -600.0;
pub const SUBTITLE_SYNC_MAX_SEC: f64 = 600.0;

pub const SUBTITLE_FONT_SIZE_MIN: u16 = 12;
pub const SUBTITLE_FONT_SIZE_MAX: u16 = 96;
pub const DEFAULT_SUBTITLE_FONT_SIZE: u16 = 28;
pub const DEFAULT_SUBTITLE_FONT_FAMILY: &str = "Segoe UI";
pub const DEFAULT_SUBTITLE_FONT_COLOR: &str = "#ffffff";
pub const DEFAULT_SUBTITLE_BG_COLOR: &str = "#000000";
pub const DEFAULT_SUBTITLE_BG_OPACITY: f64 = 0.6;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaTrack {
    pub id: i64,
    #[serde(default)]
    pub lang: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub selected: bool,
    #[serde(default)]
    pub external: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubsDubsMode {
    Dubs,
    Subs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubsDubsSelection {
    pub mode: SubsDubsMode,
    pub audio_track_id: Option<i64>,
    pub subtitle_track_id: Option<i64>,
    pub subtitle_visible: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubtitleStyle {
    pub font_size: u16,
    pub font_family: String,
    pub font_color: String,
    pub bg_color: String,
    pub bg_opacity: f64,
}

impl Default for SubtitleStyle {
    fn default() -> Self {
        Self {
            font_size: DEFAULT_SUBTITLE_FONT_SIZE,
            font_family: DEFAULT_SUBTITLE_FONT_FAMILY.to_string(),
            font_color: DEFAULT_SUBTITLE_FONT_COLOR.to_string(),
            bg_color: DEFAULT_SUBTITLE_BG_COLOR.to_string(),
            bg_opacity: DEFAULT_SUBTITLE_BG_OPACITY,
        }
    }
}

impl SubtitleStyle {
    pub fn has_custom_style(&self) -> bool {
        self.font_size != DEFAULT_SUBTITLE_FONT_SIZE
            || self.font_family != DEFAULT_SUBTITLE_FONT_FAMILY
            || !self
                .font_color
                .eq_ignore_ascii_case(DEFAULT_SUBTITLE_FONT_COLOR)
            || !self
                .bg_color
                .eq_ignore_ascii_case(DEFAULT_SUBTITLE_BG_COLOR)
            || (self.bg_opacity - DEFAULT_SUBTITLE_BG_OPACITY).abs() > 1e-6
    }

    pub fn bg_color_with_alpha(&self) -> String {
        color_with_alpha(&self.bg_color, self.bg_opacity)
    }

    pub fn sanitized(mut self) -> Self {
        self.font_size = self
            .font_size
            .clamp(SUBTITLE_FONT_SIZE_MIN, SUBTITLE_FONT_SIZE_MAX);
        if self.font_family.trim().is_empty() {
            self.font_family = DEFAULT_SUBTITLE_FONT_FAMILY.to_string();
        }
        if !is_valid_hex_color(&self.font_color) {
            self.font_color = DEFAULT_SUBTITLE_FONT_COLOR.to_string();
        }
        if !is_valid_hex_color(&self.bg_color) {
            self.bg_color = DEFAULT_SUBTITLE_BG_COLOR.to_string();
        }
        if !self.bg_opacity.is_finite() {
            self.bg_opacity = DEFAULT_SUBTITLE_BG_OPACITY;
        } else {
            self.bg_opacity = self.bg_opacity.clamp(0.0, 1.0);
        }
        self
    }
}

fn is_valid_hex_color(hex: &str) -> bool {
    let hex = hex.trim();
    hex.len() == 7 && hex.starts_with('#') && hex[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// Converts a `#rrggbb` color + opacity `[0.0, 1.0]` into `#rrggbbaa` for mpv `sub-back-color`.
pub fn color_with_alpha(hex_color: &str, opacity: f64) -> String {
    let clamped = if opacity.is_finite() {
        opacity.clamp(0.0, 1.0)
    } else {
        DEFAULT_SUBTITLE_BG_OPACITY
    };
    let alpha = (clamped * 255.0).round() as u8;
    let base = if is_valid_hex_color(hex_color.trim()) {
        hex_color.trim()
    } else {
        DEFAULT_SUBTITLE_BG_COLOR
    };
    format!("{base}{alpha:02x}")
}

/// Clamps audio sync offset in milliseconds to `±600_000 ms` (±10 min).
pub fn clamp_audio_sync_ms(offset_ms: i64) -> i64 {
    offset_ms.clamp(AUDIO_SYNC_MIN_MS, AUDIO_SYNC_MAX_MS)
}

/// Converts subtitle delay input (seconds or milliseconds when `is_ms` or `|v| >= 1000`) to
/// mpv `sub-delay` seconds rounded to 0.01s and clamped to `[-600.0, 600.0]`.
pub fn normalize_subtitle_sync_sec(value: f64, is_ms: bool) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    let sec = if is_ms || value.abs() >= 1000.0 {
        value / 1000.0
    } else {
        value
    };
    let clamped = sec.clamp(SUBTITLE_SYNC_MIN_SEC, SUBTITLE_SYNC_MAX_SEC);
    (clamped * 100.0).round() / 100.0
}

pub fn subtitle_sec_to_ms(sec: f64) -> i64 {
    if !sec.is_finite() {
        return 0;
    }
    (sec * 1000.0)
        .round()
        .clamp(SUBTITLE_DELAY_MIN_MS as f64, SUBTITLE_DELAY_MAX_MS as f64) as i64
}

pub fn format_language_name(code: &str) -> String {
    let clean = code.trim().to_ascii_lowercase();
    if clean.is_empty() {
        return "Unknown".to_string();
    }
    match clean.as_str() {
        "en" | "eng" | "en-us" | "en-gb" => "English".to_string(),
        "ja" | "jpn" | "jp" => "Japanese".to_string(),
        "es" | "spa" | "es-la" | "es-419" => "Spanish".to_string(),
        "fr" | "fre" | "fra" => "French".to_string(),
        "de" | "ger" | "deu" => "German".to_string(),
        "it" | "ita" => "Italian".to_string(),
        "pt" | "por" | "pt-br" => "Portuguese".to_string(),
        "ru" | "rus" => "Russian".to_string(),
        "ko" | "kor" => "Korean".to_string(),
        "zh" | "chi" | "zho" | "zh-cn" | "zh-tw" => "Chinese".to_string(),
        "ar" | "ara" => "Arabic".to_string(),
        "hi" | "hin" => "Hindi".to_string(),
        "tr" | "tur" => "Turkish".to_string(),
        "nl" | "dut" | "nld" => "Dutch".to_string(),
        "pl" | "pol" => "Polish".to_string(),
        "sv" | "swe" => "Swedish".to_string(),
        "vi" | "vie" => "Vietnamese".to_string(),
        "th" | "tha" => "Thai".to_string(),
        "id" | "ind" => "Indonesian".to_string(),
        "und" => "Undetermined".to_string(),
        _ => clean.to_ascii_uppercase(),
    }
}

pub fn is_japanese_track(track: &MediaTrack) -> bool {
    let lang = track.lang.to_ascii_lowercase();
    let title = track.title.to_ascii_lowercase();
    matches!(lang.as_str(), "ja" | "jpn" | "jp")
        || title.contains("japanese")
        || title.contains("nihongo")
}

pub fn is_english_track(track: &MediaTrack) -> bool {
    let lang = track.lang.to_ascii_lowercase();
    let title = track.title.to_ascii_lowercase();
    lang == "en" || lang == "eng" || lang.starts_with("en-") || title.contains("english")
}

pub fn is_signs_or_songs_sub(track: &MediaTrack) -> bool {
    let title = track.title.to_ascii_lowercase();
    title.contains("sign") || title.contains("song") || title.contains("forced")
}

/// Computes the track pairing for `Subs / Dubs` toggle matching `player.js`:
/// - If currently Japanese audio -> switch to English audio (dub) and signs/songs English sub if available, else hide subs.
/// - Otherwise -> switch to Japanese audio (or first audio if only one) and full dialogue English sub (or any English / first sub).
pub fn compute_subs_dubs_toggle(
    audio_tracks: &[MediaTrack],
    sub_tracks: &[MediaTrack],
    current_audio_id: Option<i64>,
) -> Option<SubsDubsSelection> {
    if audio_tracks.is_empty() {
        return None;
    }

    let current_audio = current_audio_id
        .and_then(|id| audio_tracks.iter().find(|t| t.id == id))
        .or_else(|| audio_tracks.iter().find(|t| t.selected))
        .or_else(|| audio_tracks.first());

    let currently_japanese = current_audio.map(is_japanese_track).unwrap_or(false);

    if currently_japanese {
        let eng_audio = audio_tracks.iter().find(|t| is_english_track(t));
        let signs_sub = sub_tracks
            .iter()
            .find(|t| is_english_track(t) && is_signs_or_songs_sub(t))
            .or_else(|| sub_tracks.iter().find(|t| is_signs_or_songs_sub(t)));

        if let Some(dub) = eng_audio {
            return Some(SubsDubsSelection {
                mode: SubsDubsMode::Dubs,
                audio_track_id: Some(dub.id),
                subtitle_track_id: signs_sub.map(|s| s.id),
                subtitle_visible: signs_sub.is_some(),
            });
        }
        return None;
    }

    let jpn_audio = audio_tracks.iter().find(|t| is_japanese_track(t));
    let dialogue_sub = sub_tracks
        .iter()
        .find(|t| is_english_track(t) && !is_signs_or_songs_sub(t))
        .or_else(|| sub_tracks.iter().find(|t| is_english_track(t)))
        .or_else(|| sub_tracks.first());

    let target_audio = jpn_audio.or_else(|| audio_tracks.first());
    Some(SubsDubsSelection {
        mode: SubsDubsMode::Subs,
        audio_track_id: target_audio.map(|a| a.id),
        subtitle_track_id: dialogue_sub.map(|s| s.id),
        subtitle_visible: dialogue_sub.is_some(),
    })
}

/// Explicitly selects either `Subs` (Japanese audio + dialogue subtitle) or `Dubs` (English audio + signs/songs subtitle).
pub fn select_subs_dubs_mode(
    audio_tracks: &[MediaTrack],
    sub_tracks: &[MediaTrack],
    mode: SubsDubsMode,
) -> Option<SubsDubsSelection> {
    if audio_tracks.is_empty() {
        return None;
    }
    match mode {
        SubsDubsMode::Dubs => {
            let eng_audio = audio_tracks
                .iter()
                .find(|t| is_english_track(t))
                .or_else(|| audio_tracks.first());
            let signs_sub = sub_tracks
                .iter()
                .find(|t| is_english_track(t) && is_signs_or_songs_sub(t))
                .or_else(|| sub_tracks.iter().find(|t| is_signs_or_songs_sub(t)));
            Some(SubsDubsSelection {
                mode: SubsDubsMode::Dubs,
                audio_track_id: eng_audio.map(|a| a.id),
                subtitle_track_id: signs_sub.map(|s| s.id),
                subtitle_visible: signs_sub.is_some(),
            })
        }
        SubsDubsMode::Subs => {
            let jpn_audio = audio_tracks
                .iter()
                .find(|t| is_japanese_track(t))
                .or_else(|| audio_tracks.first());
            let dialogue_sub = sub_tracks
                .iter()
                .find(|t| is_english_track(t) && !is_signs_or_songs_sub(t))
                .or_else(|| sub_tracks.iter().find(|t| is_english_track(t)))
                .or_else(|| sub_tracks.first());
            Some(SubsDubsSelection {
                mode: SubsDubsMode::Subs,
                audio_track_id: jpn_audio.map(|a| a.id),
                subtitle_track_id: dialogue_sub.map(|s| s.id),
                subtitle_visible: dialogue_sub.is_some(),
            })
        }
    }
}

/// Cycles to the next audio track and returns the newly selected track if cycling is possible.
pub fn next_audio_track(
    audio_tracks: &[MediaTrack],
    current_audio_id: Option<i64>,
) -> Option<&MediaTrack> {
    if audio_tracks.len() < 2 {
        return audio_tracks.first();
    }
    let current_idx = current_audio_id
        .and_then(|id| audio_tracks.iter().position(|t| t.id == id))
        .or_else(|| audio_tracks.iter().position(|t| t.selected))
        .unwrap_or(0);
    let next_idx = (current_idx + 1) % audio_tracks.len();
    audio_tracks.get(next_idx)
}

/// Cycles to the next subtitle track (returning `None` after the last track to turn subtitles off).
pub fn next_subtitle_track(
    sub_tracks: &[MediaTrack],
    current_sub_id: Option<i64>,
) -> Option<&MediaTrack> {
    if sub_tracks.is_empty() {
        return None;
    }
    match current_sub_id {
        None => sub_tracks.first(),
        Some(id) => {
            if let Some(pos) = sub_tracks.iter().position(|t| t.id == id) {
                sub_tracks.get(pos + 1)
            } else {
                sub_tracks.first()
            }
        }
    }
}
