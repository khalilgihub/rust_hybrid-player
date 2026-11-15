use serde::de::{MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MEMORY_MAP_LIMIT: usize = 1000;
pub const HISTORY_LIMIT: usize = 100;
pub const RECENT_FILES_LIMIT: usize = 50;
pub const PLAYLIST_ITEMS_LIMIT: usize = 1000;
pub const SUBTITLE_DELAY_MIN_MS: f64 = -10.0 * 60.0 * 1000.0;
pub const SUBTITLE_DELAY_MAX_MS: f64 = 10.0 * 60.0 * 1000.0;
pub const MAX_PLAYBACK_SECONDS: f64 = 30.0 * 24.0 * 60.0 * 60.0;

static TMP_SEQ: AtomicU64 = AtomicU64::new(1);

pub const WELCOME_BACKGROUNDS: &[&str] = &["none", "dither", "particles", "faulty", "gridmotion"];
pub const WELCOME_QUALITIES: &[&str] = &["low", "medium", "high", "custom"];
pub const MOTION_PROFILES: &[&str] = &["reduced", "balanced", "showcase"];
pub const THEMES: &[&str] = &["dark", "oled", "light"];
pub const EQ_PRESETS: &[&str] = &[
    "flat",
    "bass-boost",
    "treble-boost",
    "vocal",
    "rock",
    "pop",
    "jazz",
    "classical",
    "electronic",
    "custom",
];
pub const BG_OPTION_PREFS: &[&str] = &[
    "bgOpts_dither",
    "bgOpts_particles",
    "bgOpts_faulty",
    "bgOpts_gridmotion",
];

pub const ALL_PREFERENCE_KEYS: &[&str] = &[
    "theme",
    "accentColor",
    "autoResume",
    "brandFontEnabled",
    "autoOrganizeScreenshots",
    "doubleClickFullscreen",
    "showUnlockButton",
    "volumeBoost",
    "volume",
    "subFontSize",
    "subBgOpacity",
    "subFontColor",
    "subBgColor",
    "equalizerPreset",
    "equalizerBands",
    "motionProfile",
    "welcomeBackground",
    "welcomeQuality",
    "screenshotDir",
    "screenshotFormat",
    "seekStep",
    "customShortcuts",
    "screenshotMouseButton",
    "bgOpts_dither",
    "bgOpts_particles",
    "bgOpts_faulty",
    "bgOpts_gridmotion",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartupDiagnostic {
    pub level: String,
    pub code: String,
    pub message: String,
    pub detail: Option<String>,
    pub timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub path: String,
    pub name: String,
    pub duration: f64,
    #[serde(default)]
    pub timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaylistItemEntry {
    pub path: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedPlaylist {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub items: Vec<PlaylistItemEntry>,
    #[serde(default)]
    pub created: u64,
}

/// Insertion-ordered bounded map matching JS Object key-order LRU eviction (`MEMORY_MAP_LIMIT = 1000`).
#[derive(Debug, Clone, PartialEq)]
pub struct BoundedMemoryMap<V> {
    entries: Vec<(String, V)>,
    limit: usize,
}

impl<V> Default for BoundedMemoryMap<V> {
    fn default() -> Self {
        Self::new(MEMORY_MAP_LIMIT)
    }
}

impl<V> BoundedMemoryMap<V> {
    pub fn new(limit: usize) -> Self {
        Self {
            entries: Vec::new(),
            limit: limit.max(1),
        }
    }

    pub fn set(&mut self, key: impl Into<String>, value: V) {
        let key = key.into();
        if key.is_empty() {
            return;
        }
        if let Some(pos) = self.entries.iter().position(|(k, _)| k == &key) {
            self.entries.remove(pos);
        }
        self.entries.push((key, value));
        if self.entries.len() > self.limit {
            let overflow = self.entries.len() - self.limit;
            self.entries.drain(0..overflow);
        }
    }

    pub fn get(&self, key: &str) -> Option<&V> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn remove(&mut self, key: &str) -> bool {
        if let Some(pos) = self.entries.iter().position(|(k, _)| k == key) {
            self.entries.remove(pos);
            true
        } else {
            false
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &V)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v))
    }
}

impl<V: Serialize> Serialize for BoundedMemoryMap<V> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.entries.len()))?;
        for (k, v) in &self.entries {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}

impl<'de, V: Deserialize<'de>> Deserialize<'de> for BoundedMemoryMap<V> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct MapVisitor<V>(std::marker::PhantomData<V>);

        impl<'de, V: Deserialize<'de>> Visitor<'de> for MapVisitor<V> {
            type Value = BoundedMemoryMap<V>;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a JSON object map")
            }

            fn visit_map<M: MapAccess<'de>>(self, mut access: M) -> Result<Self::Value, M::Error> {
                let mut map = BoundedMemoryMap::default();
                while let Some((k, v)) = access.next_entry::<String, V>()? {
                    map.set(k, v);
                }
                Ok(map)
            }
        }

        deserializer.deserialize_map(MapVisitor(std::marker::PhantomData))
    }
}

fn default_theme() -> String {
    "dark".to_string()
}
fn default_accent_color() -> String {
    "#e50914".to_string()
}
fn default_true() -> bool {
    true
}
fn default_volume() -> f64 {
    1.0
}
fn default_sub_font_size() -> f64 {
    28.0
}
fn default_sub_font_color() -> String {
    "#ffffff".to_string()
}
fn default_sub_bg_color() -> String {
    "#000000".to_string()
}
fn default_sub_bg_opacity() -> f64 {
    0.6
}
fn default_eq_preset() -> String {
    "flat".to_string()
}
fn default_eq_bands() -> Vec<f64> {
    vec![0.0; 10]
}
fn default_motion_profile() -> String {
    "balanced".to_string()
}
fn default_welcome_background() -> String {
    "dither".to_string()
}
fn default_welcome_quality() -> String {
    "medium".to_string()
}
fn default_screenshot_format() -> String {
    "jpg".to_string()
}
fn default_seek_step() -> u32 {
    5
}
fn default_screenshot_mouse_button() -> i32 {
    3
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preferences {
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_accent_color")]
    pub accent_color: String,
    #[serde(default = "default_true")]
    pub auto_resume: bool,
    #[serde(default = "default_volume")]
    pub volume: f64,
    #[serde(default)]
    pub volume_boost: bool,
    #[serde(default = "default_sub_font_size")]
    pub sub_font_size: f64,
    #[serde(default = "default_sub_font_color")]
    pub sub_font_color: String,
    #[serde(default = "default_sub_bg_color")]
    pub sub_bg_color: String,
    #[serde(default = "default_sub_bg_opacity")]
    pub sub_bg_opacity: f64,
    #[serde(default = "default_eq_preset")]
    pub equalizer_preset: String,
    #[serde(default = "default_eq_bands")]
    pub equalizer_bands: Vec<f64>,
    #[serde(default = "default_motion_profile")]
    pub motion_profile: String,
    #[serde(default = "default_true")]
    pub brand_font_enabled: bool,
    #[serde(default)]
    pub screenshot_dir: String,
    #[serde(default = "default_screenshot_format")]
    pub screenshot_format: String,
    #[serde(default = "default_true")]
    pub auto_organize_screenshots: bool,
    #[serde(default = "default_seek_step")]
    pub seek_step: u32,
    #[serde(default = "default_true")]
    pub double_click_fullscreen: bool,
    #[serde(default = "default_true")]
    pub show_unlock_button: bool,
    #[serde(default = "default_welcome_background")]
    pub welcome_background: String,
    #[serde(default = "default_welcome_quality")]
    pub welcome_quality: String,
    #[serde(default)]
    pub custom_shortcuts: BTreeMap<String, String>,
    #[serde(default = "default_screenshot_mouse_button")]
    pub screenshot_mouse_button: i32,
    #[serde(
        rename = "bgOpts_dither",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub bg_opts_dither: Option<String>,
    #[serde(
        rename = "bgOpts_particles",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub bg_opts_particles: Option<String>,
    #[serde(
        rename = "bgOpts_faulty",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub bg_opts_faulty: Option<String>,
    #[serde(
        rename = "bgOpts_gridmotion",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub bg_opts_gridmotion: Option<String>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            accent_color: default_accent_color(),
            auto_resume: true,
            volume: 1.0,
            volume_boost: false,
            sub_font_size: 28.0,
            sub_font_color: default_sub_font_color(),
            sub_bg_color: default_sub_bg_color(),
            sub_bg_opacity: 0.6,
            equalizer_preset: default_eq_preset(),
            equalizer_bands: default_eq_bands(),
            motion_profile: default_motion_profile(),
            brand_font_enabled: true,
            screenshot_dir: String::new(),
            screenshot_format: default_screenshot_format(),
            auto_organize_screenshots: true,
            seek_step: 5,
            double_click_fullscreen: true,
            show_unlock_button: true,
            welcome_background: default_welcome_background(),
            welcome_quality: default_welcome_quality(),
            custom_shortcuts: BTreeMap::new(),
            screenshot_mouse_button: 3,
            bg_opts_dither: None,
            bg_opts_particles: None,
            bg_opts_faulty: None,
            bg_opts_gridmotion: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Database {
    #[serde(default)]
    pub preferences: Preferences,
    #[serde(default)]
    pub history: Vec<HistoryEntry>,
    #[serde(default)]
    pub resume_positions: BoundedMemoryMap<f64>,
    #[serde(default)]
    pub playlists: Vec<SavedPlaylist>,
    #[serde(default)]
    pub speed_memory: BoundedMemoryMap<f64>,
    #[serde(default)]
    pub subtitle_delay_memory: BoundedMemoryMap<f64>,
    #[serde(default)]
    pub recent_files: Vec<String>,
}

pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn clamp_number(value: f64, min: f64, max: f64, fallback: f64) -> f64 {
    if !value.is_finite() {
        return fallback;
    }
    value.clamp(min, max)
}

pub fn is_hex_color(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 7 && bytes[0] == b'#' && bytes[1..].iter().all(|b| b.is_ascii_hexdigit())
}

pub fn normalize_string(value: &str, max: usize, trim: bool) -> Option<String> {
    let text = if trim { value.trim() } else { value };
    if text.is_empty() || text.chars().count() > max {
        return None;
    }
    Some(text.to_string())
}

pub fn sanitize_media_key(value: &str) -> Option<String> {
    normalize_string(value, 4096, true)
}

fn value_as_bool(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|v| v != 0.0 && v.is_finite()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
        Value::Null => false,
    }
}

fn value_as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64().filter(|v| v.is_finite()),
        Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                trimmed.parse::<f64>().ok().filter(|v| v.is_finite())
            }
        }
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

pub fn sanitize_equalizer_bands(value: &Value) -> Option<Vec<f64>> {
    let arr = value.as_array()?;
    let mut out = Vec::with_capacity(arr.len().min(10));
    for item in arr.iter().take(10) {
        let num = value_as_f64(item).unwrap_or(0.0);
        out.push(clamp_number(num, -12.0, 12.0, 0.0));
    }
    Some(out)
}

pub fn sanitize_background_options(value: &Value, key: &str) -> Option<String> {
    match value {
        Value::String(raw) => {
            let max_len = if key == "bgOpts_gridmotion" {
                5 * 1024 * 1024
            } else {
                8192
            };
            if raw.len() > max_len {
                return None;
            }
            let parsed: Value = serde_json::from_str(raw).ok()?;
            if !parsed.is_object() {
                return None;
            }
            serde_json::to_string(&parsed).ok()
        }
        Value::Object(_) => serde_json::to_string(value).ok(),
        _ => None,
    }
}

pub fn sanitize_custom_shortcuts(value: &Value) -> Option<BTreeMap<String, String>> {
    let parsed_owned;
    let obj = match value {
        Value::String(raw) => {
            if raw.len() > 16384 {
                return None;
            }
            parsed_owned = serde_json::from_str::<Value>(raw).ok()?;
            parsed_owned.as_object()?
        }
        Value::Object(map) => map,
        _ => return None,
    };

    let mut clean = BTreeMap::new();
    for (act_key, bind_val) in obj.iter().take(120) {
        let trimmed_key = act_key.trim();
        if trimmed_key.is_empty() || trimmed_key.len() > 80 {
            continue;
        }
        let Some(bind_str) = bind_val.as_str() else {
            continue;
        };
        if bind_str.len() > 80 {
            continue;
        }
        clean.insert(trimmed_key.to_string(), bind_str.trim().to_string());
    }
    Some(clean)
}

/// Validates and normalizes any preference key/value matching `ipc-handlers.js:56-61,113-184`.
pub fn sanitize_preference(key: &str, value: &Value) -> Option<Value> {
    let pref_key = normalize_string(key, 80, true)?;

    if BG_OPTION_PREFS.contains(&pref_key.as_str()) {
        return sanitize_background_options(value, &pref_key).map(Value::String);
    }

    match pref_key.as_str() {
        "theme" => value
            .as_str()
            .filter(|s| THEMES.contains(s))
            .map(|s| Value::String(s.to_string())),
        "accentColor" | "subFontColor" | "subBgColor" => value
            .as_str()
            .filter(|s| is_hex_color(s))
            .map(|s| Value::String(s.to_string())),
        "autoResume"
        | "brandFontEnabled"
        | "autoOrganizeScreenshots"
        | "doubleClickFullscreen"
        | "showUnlockButton"
        | "volumeBoost" => Some(Value::Bool(value_as_bool(value))),
        "volume" => {
            let numeric_vol = match value_as_f64(value) {
                Some(v) => v,
                None => return serde_json::to_value(1.0).ok(),
            };
            let normalized = if numeric_vol > 2.0 {
                numeric_vol / 100.0
            } else {
                numeric_vol
            };
            serde_json::to_value(clamp_number(normalized, 0.0, 2.0, 1.0)).ok()
        }
        "subFontSize" => {
            let num = value_as_f64(value).unwrap_or(f64::NAN);
            serde_json::to_value(clamp_number(num, 12.0, 96.0, 28.0)).ok()
        }
        "subBgOpacity" => {
            let num = value_as_f64(value).unwrap_or(f64::NAN);
            serde_json::to_value(clamp_number(num, 0.0, 1.0, 0.6)).ok()
        }
        "equalizerPreset" => value
            .as_str()
            .filter(|s| EQ_PRESETS.contains(s))
            .map(|s| Value::String(s.to_string())),
        "equalizerBands" => {
            sanitize_equalizer_bands(value).and_then(|v| serde_json::to_value(v).ok())
        }
        "motionProfile" => value
            .as_str()
            .filter(|s| MOTION_PROFILES.contains(s))
            .map(|s| Value::String(s.to_string())),
        "welcomeBackground" => value
            .as_str()
            .filter(|s| WELCOME_BACKGROUNDS.contains(s))
            .map(|s| Value::String(s.to_string())),
        "welcomeQuality" => value
            .as_str()
            .filter(|s| WELCOME_QUALITIES.contains(s))
            .map(|s| Value::String(s.to_string())),
        "screenshotDir" => {
            let s = value.as_str()?;
            Some(Value::String(
                normalize_string(s, 4096, true).unwrap_or_default(),
            ))
        }
        "screenshotFormat" => {
            let s = value.as_str()?;
            let lower = s.to_lowercase();
            match lower.as_str() {
                "jpg" | "jpeg" => Some(Value::String("jpg".to_string())),
                "png" | "webp" => Some(Value::String(lower)),
                _ => None,
            }
        }
        "seekStep" => {
            let num = value_as_f64(value)?;
            if num == 3.0 || num == 5.0 || num == 10.0 {
                Some(Value::from(num as u32))
            } else {
                None
            }
        }
        "customShortcuts" => {
            sanitize_custom_shortcuts(value).and_then(|m| serde_json::to_value(m).ok())
        }
        "screenshotMouseButton" => {
            let num = value_as_f64(value)?;
            if num == -1.0 || num == 1.0 || num == 3.0 || num == 4.0 {
                Some(Value::from(num as i32))
            } else {
                None
            }
        }
        _ => None,
    }
}

impl Preferences {
    pub fn get(&self, key: &str) -> Option<Value> {
        let pref_key = normalize_string(key, 80, true)?;
        match pref_key.as_str() {
            "theme" => Some(Value::String(self.theme.clone())),
            "accentColor" => Some(Value::String(self.accent_color.clone())),
            "autoResume" => Some(Value::Bool(self.auto_resume)),
            "volume" => serde_json::to_value(self.volume).ok(),
            "volumeBoost" => Some(Value::Bool(self.volume_boost)),
            "subFontSize" => serde_json::to_value(self.sub_font_size).ok(),
            "subFontColor" => Some(Value::String(self.sub_font_color.clone())),
            "subBgColor" => Some(Value::String(self.sub_bg_color.clone())),
            "subBgOpacity" => serde_json::to_value(self.sub_bg_opacity).ok(),
            "equalizerPreset" => Some(Value::String(self.equalizer_preset.clone())),
            "equalizerBands" => serde_json::to_value(&self.equalizer_bands).ok(),
            "motionProfile" => Some(Value::String(self.motion_profile.clone())),
            "brandFontEnabled" => Some(Value::Bool(self.brand_font_enabled)),
            "screenshotDir" => Some(Value::String(self.screenshot_dir.clone())),
            "screenshotFormat" => Some(Value::String(self.screenshot_format.clone())),
            "autoOrganizeScreenshots" => Some(Value::Bool(self.auto_organize_screenshots)),
            "seekStep" => Some(Value::from(self.seek_step)),
            "doubleClickFullscreen" => Some(Value::Bool(self.double_click_fullscreen)),
            "showUnlockButton" => Some(Value::Bool(self.show_unlock_button)),
            "welcomeBackground" => Some(Value::String(self.welcome_background.clone())),
            "welcomeQuality" => Some(Value::String(self.welcome_quality.clone())),
            "customShortcuts" => serde_json::to_value(&self.custom_shortcuts).ok(),
            "screenshotMouseButton" => Some(Value::from(self.screenshot_mouse_button)),
            "bgOpts_dither" => self
                .bg_opts_dither
                .as_ref()
                .map(|s| Value::String(s.clone())),
            "bgOpts_particles" => self
                .bg_opts_particles
                .as_ref()
                .map(|s| Value::String(s.clone())),
            "bgOpts_faulty" => self
                .bg_opts_faulty
                .as_ref()
                .map(|s| Value::String(s.clone())),
            "bgOpts_gridmotion" => self
                .bg_opts_gridmotion
                .as_ref()
                .map(|s| Value::String(s.clone())),
            _ => None,
        }
    }

    pub fn set(&mut self, key: &str, value: &Value) -> bool {
        let Some(sanitized) = sanitize_preference(key, value) else {
            return false;
        };
        let Some(pref_key) = normalize_string(key, 80, true) else {
            return false;
        };
        match pref_key.as_str() {
            "theme" => {
                if let Some(s) = sanitized.as_str() {
                    self.theme = s.to_string();
                }
            }
            "accentColor" => {
                if let Some(s) = sanitized.as_str() {
                    self.accent_color = s.to_string();
                }
            }
            "autoResume" => self.auto_resume = sanitized.as_bool().unwrap_or(self.auto_resume),
            "volume" => self.volume = sanitized.as_f64().unwrap_or(self.volume),
            "volumeBoost" => self.volume_boost = sanitized.as_bool().unwrap_or(self.volume_boost),
            "subFontSize" => self.sub_font_size = sanitized.as_f64().unwrap_or(self.sub_font_size),
            "subFontColor" => {
                if let Some(s) = sanitized.as_str() {
                    self.sub_font_color = s.to_string();
                }
            }
            "subBgColor" => {
                if let Some(s) = sanitized.as_str() {
                    self.sub_bg_color = s.to_string();
                }
            }
            "subBgOpacity" => {
                self.sub_bg_opacity = sanitized.as_f64().unwrap_or(self.sub_bg_opacity)
            }
            "equalizerPreset" => {
                if let Some(s) = sanitized.as_str() {
                    self.equalizer_preset = s.to_string();
                }
            }
            "equalizerBands" => {
                if let Ok(bands) = serde_json::from_value::<Vec<f64>>(sanitized) {
                    self.equalizer_bands = bands;
                }
            }
            "motionProfile" => {
                if let Some(s) = sanitized.as_str() {
                    self.motion_profile = s.to_string();
                }
            }
            "brandFontEnabled" => {
                self.brand_font_enabled = sanitized.as_bool().unwrap_or(self.brand_font_enabled)
            }
            "screenshotDir" => {
                if let Some(s) = sanitized.as_str() {
                    self.screenshot_dir = s.to_string();
                }
            }
            "screenshotFormat" => {
                if let Some(s) = sanitized.as_str() {
                    self.screenshot_format = s.to_string();
                }
            }
            "autoOrganizeScreenshots" => {
                self.auto_organize_screenshots = sanitized
                    .as_bool()
                    .unwrap_or(self.auto_organize_screenshots)
            }
            "seekStep" => {
                if let Some(v) = sanitized.as_u64() {
                    self.seek_step = v as u32;
                }
            }
            "doubleClickFullscreen" => {
                self.double_click_fullscreen =
                    sanitized.as_bool().unwrap_or(self.double_click_fullscreen)
            }
            "showUnlockButton" => {
                self.show_unlock_button = sanitized.as_bool().unwrap_or(self.show_unlock_button)
            }
            "welcomeBackground" => {
                if let Some(s) = sanitized.as_str() {
                    self.welcome_background = s.to_string();
                }
            }
            "welcomeQuality" => {
                if let Some(s) = sanitized.as_str() {
                    self.welcome_quality = s.to_string();
                }
            }
            "customShortcuts" => {
                if let Ok(map) = serde_json::from_value::<BTreeMap<String, String>>(sanitized) {
                    self.custom_shortcuts = map;
                }
            }
            "screenshotMouseButton" => {
                if let Some(v) = sanitized.as_i64() {
                    self.screenshot_mouse_button = v as i32;
                }
            }
            "bgOpts_dither" => self.bg_opts_dither = sanitized.as_str().map(str::to_string),
            "bgOpts_particles" => self.bg_opts_particles = sanitized.as_str().map(str::to_string),
            "bgOpts_faulty" => self.bg_opts_faulty = sanitized.as_str().map(str::to_string),
            "bgOpts_gridmotion" => self.bg_opts_gridmotion = sanitized.as_str().map(str::to_string),
            _ => return false,
        }
        true
    }

    pub fn apply_patch(&mut self, patch: &serde_json::Map<String, Value>) {
        for (k, v) in patch {
            self.set(k, v);
        }
    }
}

impl Database {
    pub fn add_history(&mut self, path: &str, name: Option<&str>, duration: f64) -> bool {
        let Some(clean_path) = sanitize_media_key(path) else {
            return false;
        };
        let fallback_name = Path::new(&clean_path)
            .file_name()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("Untitled")
            .to_string();
        let clean_name = name
            .and_then(|n| normalize_string(n, 260, true))
            .unwrap_or(fallback_name);
        let clean_duration = clamp_number(duration, 0.0, MAX_PLAYBACK_SECONDS, 0.0);

        self.history.retain(|h| h.path != clean_path);
        self.history.insert(
            0,
            HistoryEntry {
                path: clean_path.clone(),
                name: clean_name,
                duration: clean_duration,
                timestamp: now_millis(),
            },
        );
        if self.history.len() > HISTORY_LIMIT {
            self.history.truncate(HISTORY_LIMIT);
        }

        self.recent_files.retain(|r| r != &clean_path);
        self.recent_files.insert(0, clean_path);
        if self.recent_files.len() > RECENT_FILES_LIMIT {
            self.recent_files.truncate(RECENT_FILES_LIMIT);
        }
        true
    }

    pub fn save_resume_position(&mut self, file_path: &str, time: f64) -> bool {
        let Some(key) = sanitize_media_key(file_path) else {
            return false;
        };
        self.resume_positions
            .set(key, clamp_number(time, 0.0, MAX_PLAYBACK_SECONDS, 0.0));
        true
    }

    pub fn get_resume_position(&self, file_path: &str) -> f64 {
        sanitize_media_key(file_path)
            .and_then(|k| self.resume_positions.get(&k).copied())
            .unwrap_or(0.0)
    }

    pub fn clear_resume_position(&mut self, file_path: &str) -> bool {
        let Some(key) = sanitize_media_key(file_path) else {
            return false;
        };
        self.resume_positions.remove(&key);
        true
    }

    pub fn save_speed(&mut self, file_path: &str, speed: f64) -> bool {
        let Some(key) = sanitize_media_key(file_path) else {
            return false;
        };
        self.speed_memory
            .set(key, clamp_number(speed, 0.1, 16.0, 1.0));
        true
    }

    pub fn get_speed(&self, file_path: &str) -> Option<f64> {
        sanitize_media_key(file_path).and_then(|k| self.speed_memory.get(&k).copied())
    }

    pub fn save_subtitle_delay(&mut self, file_path: &str, delay_ms: f64) -> bool {
        let Some(key) = sanitize_media_key(file_path) else {
            return false;
        };
        self.subtitle_delay_memory.set(
            key,
            clamp_number(delay_ms, SUBTITLE_DELAY_MIN_MS, SUBTITLE_DELAY_MAX_MS, 0.0),
        );
        true
    }

    pub fn get_subtitle_delay(&self, file_path: &str) -> f64 {
        sanitize_media_key(file_path)
            .and_then(|k| self.subtitle_delay_memory.get(&k).copied())
            .unwrap_or(0.0)
    }

    pub fn save_playlist(&mut self, playlist: SavedPlaylist) -> bool {
        let id = normalize_string(&playlist.id, 128, true)
            .unwrap_or_else(|| format!("{:x}", now_millis()));
        let name = normalize_string(&playlist.name, 200, true)
            .unwrap_or_else(|| "Untitled Playlist".to_string());
        let items: Vec<PlaylistItemEntry> = playlist
            .items
            .into_iter()
            .take(PLAYLIST_ITEMS_LIMIT)
            .filter_map(|item| {
                let item_path = sanitize_media_key(&item.path)?;
                let fallback = Path::new(&item_path)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("Untitled")
                    .to_string();
                let item_name = normalize_string(&item.name, 260, true).unwrap_or(fallback);
                Some(PlaylistItemEntry {
                    path: item_path,
                    name: item_name,
                })
            })
            .collect();

        let now = now_millis();
        let created = if playlist.created == 0 || playlist.created > now {
            now
        } else {
            playlist.created
        };

        let clean = SavedPlaylist {
            id,
            name,
            items,
            created,
        };

        if let Some(existing) = self.playlists.iter_mut().find(|p| p.id == clean.id) {
            *existing = clean;
        } else {
            self.playlists.push(clean);
        }
        true
    }
}

#[derive(Debug, Clone)]
pub struct LoadDatabaseResult {
    pub db: Database,
    pub diagnostic: Option<StartupDiagnostic>,
    pub backup_path: Option<PathBuf>,
}

pub fn backup_unreadable_database(db_path: &Path) -> Option<PathBuf> {
    if !db_path.exists() {
        return None;
    }
    let dir = db_path.parent()?;
    let stem = db_path.file_stem()?.to_string_lossy();
    let ext = db_path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let stamp = now_millis();
    let mut backup_path = dir.join(format!("{stem}.corrupt-{stamp}{ext}"));
    let mut suffix = 1u32;
    while backup_path.exists() {
        backup_path = dir.join(format!("{stem}.corrupt-{stamp}-{suffix}{ext}"));
        suffix += 1;
    }
    fs::rename(db_path, &backup_path).ok()?;
    Some(backup_path)
}

/// Loads `hybrid-player-db.json` with normalization and `.corrupt-<ts>` backup on parse failure.
pub fn load_database(db_path: &Path) -> LoadDatabaseResult {
    if !db_path.exists() {
        return LoadDatabaseResult {
            db: Database::default(),
            diagnostic: None,
            backup_path: None,
        };
    }

    match fs::read_to_string(db_path) {
        Ok(raw) => match serde_json::from_str::<Value>(raw.trim_start_matches('\u{FEFF}')) {
            Ok(Value::Object(root)) => {
                let mut db = Database::default();
                if let Some(Value::Object(prefs_obj)) = root.get("preferences") {
                    db.preferences.apply_patch(prefs_obj);
                }
                if let Some(history_val) = root.get("history") {
                    if let Ok(entries) =
                        serde_json::from_value::<Vec<HistoryEntry>>(history_val.clone())
                    {
                        db.history = entries.into_iter().take(HISTORY_LIMIT).collect();
                    }
                }
                if let Some(recent_val) = root.get("recentFiles") {
                    if let Ok(files) = serde_json::from_value::<Vec<String>>(recent_val.clone()) {
                        db.recent_files = files.into_iter().take(RECENT_FILES_LIMIT).collect();
                    }
                }
                if let Some(playlists_val) = root.get("playlists") {
                    if let Ok(pls) =
                        serde_json::from_value::<Vec<SavedPlaylist>>(playlists_val.clone())
                    {
                        db.playlists = pls;
                    }
                }
                if let Some(resume_val) = root.get("resumePositions") {
                    if let Ok(map) =
                        serde_json::from_value::<BoundedMemoryMap<f64>>(resume_val.clone())
                    {
                        db.resume_positions = map;
                    }
                }
                if let Some(speed_val) = root.get("speedMemory") {
                    if let Ok(map) =
                        serde_json::from_value::<BoundedMemoryMap<f64>>(speed_val.clone())
                    {
                        db.speed_memory = map;
                    }
                }
                if let Some(sub_val) = root.get("subtitleDelayMemory") {
                    if let Ok(map) =
                        serde_json::from_value::<BoundedMemoryMap<f64>>(sub_val.clone())
                    {
                        db.subtitle_delay_memory = map;
                    }
                }
                LoadDatabaseResult {
                    db,
                    diagnostic: None,
                    backup_path: None,
                }
            }
            _ => {
                let backup_path = backup_unreadable_database(db_path);
                let detail = backup_path
                    .as_ref()
                    .map(|p| format!("Backup: {}", p.display()))
                    .unwrap_or_else(|| "Backup could not be created.".to_string());
                LoadDatabaseResult {
                    db: Database::default(),
                    diagnostic: Some(StartupDiagnostic {
                        level: "warning".to_string(),
                        code: "DATABASE_REPAIRED".to_string(),
                        message: "Settings database could not be read. Hybrid Player loaded defaults and saved a backup of the old file.".to_string(),
                        detail: Some(detail),
                        timestamp: now_millis(),
                    }),
                    backup_path,
                }
            }
        },
        Err(_) => {
            let backup_path = backup_unreadable_database(db_path);
            let detail = backup_path
                .as_ref()
                .map(|p| format!("Backup: {}", p.display()))
                .unwrap_or_else(|| "Backup could not be created.".to_string());
            LoadDatabaseResult {
                db: Database::default(),
                diagnostic: Some(StartupDiagnostic {
                    level: "warning".to_string(),
                    code: "DATABASE_REPAIRED".to_string(),
                    message: "Settings database could not be read. Hybrid Player loaded defaults and saved a backup of the old file.".to_string(),
                    detail: Some(detail),
                    timestamp: now_millis(),
                }),
                backup_path,
            }
        }
    }
}

/// Atomically persists `Database` via temp-write + rename with Windows lock fallback (`main.js:1472-1488`).
pub fn save_database_atomic(db: &Database, db_path: &Path) -> std::io::Result<()> {
    let dir = db_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)?;

    let file_name = db_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("hybrid-player-db.json");
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp_path = dir.join(format!(
        ".{file_name}.{}.{}.{seq}.tmp",
        std::process::id(),
        now_millis()
    ));

    let serialized = serde_json::to_string_pretty(db)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    if let Err(err) = fs::write(&tmp_path, serialized.as_bytes()) {
        let _ = fs::remove_file(&tmp_path);
        return Err(err);
    }

    match fs::rename(&tmp_path, db_path) {
        Ok(()) => Ok(()),
        Err(rename_err) => {
            let is_windows_lock =
                matches!(
                    rename_err.kind(),
                    std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ResourceBusy
                ) || matches!(rename_err.raw_os_error(), Some(5) | Some(32) | Some(33));

            if is_windows_lock {
                let write_res = fs::write(db_path, serialized.as_bytes());
                let _ = fs::remove_file(&tmp_path);
                write_res
            } else {
                let _ = fs::remove_file(&tmp_path);
                Err(rename_err)
            }
        }
    }
}
