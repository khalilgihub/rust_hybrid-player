use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShortcutCatalogEntry {
    pub id: &'static str,
    pub action: &'static str,
    pub label: &'static str,
    pub category: &'static str,
    pub default_combo: &'static str,
}

pub const SHORTCUT_CATEGORIES: [&str; 6] = [
    "Screenshots & Recording",
    "Playback",
    "Seeking & Navigation",
    "Audio & Subtitles",
    "View & Interface",
    "File & Application",
];

pub const VALID_SCREENSHOT_MOUSE_BUTTONS: [i32; 4] = [-1, 1, 3, 4];

pub const SHORTCUT_CATALOG: [ShortcutCatalogEntry; 52] = [
    // Screenshots & Recording (7)
    ShortcutCatalogEntry {
        id: "screenshot",
        action: "screenshot",
        label: "Take Screenshot (Hold for Burst)",
        category: "Screenshots & Recording",
        default_combo: "KeyS",
    },
    ShortcutCatalogEntry {
        id: "screenshot-png",
        action: "screenshot-png",
        label: "Take Screenshot (PNG)",
        category: "Screenshots & Recording",
        default_combo: "Shift+KeyS",
    },
    ShortcutCatalogEntry {
        id: "delete-latest-screenshot",
        action: "delete-latest-screenshot",
        label: "Delete Latest Screenshot",
        category: "Screenshots & Recording",
        default_combo: "KeyD",
    },
    ShortcutCatalogEntry {
        id: "restore-latest-screenshot",
        action: "restore-latest-screenshot",
        label: "Restore Latest Screenshot",
        category: "Screenshots & Recording",
        default_combo: "KeyZ",
    },
    ShortcutCatalogEntry {
        id: "toggle-screenshot-carousel",
        action: "toggle-screenshot-carousel",
        label: "Delete Selected Screenshot",
        category: "Screenshots & Recording",
        default_combo: "Ctrl+KeyD",
    },
    ShortcutCatalogEntry {
        id: "toggle-restore-carousel",
        action: "toggle-restore-carousel",
        label: "Restore Selected Screenshot",
        category: "Screenshots & Recording",
        default_combo: "Ctrl+KeyZ",
    },
    ShortcutCatalogEntry {
        id: "toggle-recording",
        action: "toggle-recording",
        label: "Record Video Clip",
        category: "Screenshots & Recording",
        default_combo: "Ctrl+KeyS",
    },
    // Playback (11)
    ShortcutCatalogEntry {
        id: "toggle-play",
        action: "toggle-play",
        label: "Play / Pause",
        category: "Playback",
        default_combo: "Space",
    },
    ShortcutCatalogEntry {
        id: "toggle-play-alt",
        action: "toggle-play",
        label: "Play / Pause (Secondary)",
        category: "Playback",
        default_combo: "KeyK",
    },
    ShortcutCatalogEntry {
        id: "stop",
        action: "stop",
        label: "Stop Playback",
        category: "Playback",
        default_combo: "",
    },
    ShortcutCatalogEntry {
        id: "previous-track",
        action: "previous-track",
        label: "Previous Track",
        category: "Playback",
        default_combo: "KeyP",
    },
    ShortcutCatalogEntry {
        id: "next-track",
        action: "next-track",
        label: "Next Track",
        category: "Playback",
        default_combo: "KeyN",
    },
    ShortcutCatalogEntry {
        id: "speed-down",
        action: "speed-down",
        label: "Decrease Speed (-0.25x)",
        category: "Playback",
        default_combo: "BracketLeft",
    },
    ShortcutCatalogEntry {
        id: "speed-up",
        action: "speed-up",
        label: "Increase Speed (+0.25x)",
        category: "Playback",
        default_combo: "BracketRight",
    },
    ShortcutCatalogEntry {
        id: "frame-backward",
        action: "frame-backward",
        label: "Previous Frame",
        category: "Playback",
        default_combo: "Comma",
    },
    ShortcutCatalogEntry {
        id: "frame-forward",
        action: "frame-forward",
        label: "Next Frame",
        category: "Playback",
        default_combo: "Period",
    },
    ShortcutCatalogEntry {
        id: "toggle-repeat",
        action: "toggle-repeat",
        label: "Cycle Repeat Mode",
        category: "Playback",
        default_combo: "KeyL",
    },
    ShortcutCatalogEntry {
        id: "toggle-ab-loop",
        action: "toggle-ab-loop",
        label: "A-B Loop (Set A / Set B / Clear)",
        category: "Playback",
        default_combo: "KeyR",
    },
    // Seeking & Navigation (12)
    ShortcutCatalogEntry {
        id: "seek-back-5",
        action: "seek-back-5",
        label: "Seek Backward",
        category: "Seeking & Navigation",
        default_combo: "ArrowLeft",
    },
    ShortcutCatalogEntry {
        id: "seek-forward-5",
        action: "seek-forward-5",
        label: "Seek Forward",
        category: "Seeking & Navigation",
        default_combo: "ArrowRight",
    },
    ShortcutCatalogEntry {
        id: "seek-0",
        action: "seek-0",
        label: "Jump to 0%",
        category: "Seeking & Navigation",
        default_combo: "Digit0",
    },
    ShortcutCatalogEntry {
        id: "seek-10",
        action: "seek-10",
        label: "Jump to 10%",
        category: "Seeking & Navigation",
        default_combo: "Digit1",
    },
    ShortcutCatalogEntry {
        id: "seek-20",
        action: "seek-20",
        label: "Jump to 20%",
        category: "Seeking & Navigation",
        default_combo: "Digit2",
    },
    ShortcutCatalogEntry {
        id: "seek-30",
        action: "seek-30",
        label: "Jump to 30%",
        category: "Seeking & Navigation",
        default_combo: "Digit3",
    },
    ShortcutCatalogEntry {
        id: "seek-40",
        action: "seek-40",
        label: "Jump to 40%",
        category: "Seeking & Navigation",
        default_combo: "Digit4",
    },
    ShortcutCatalogEntry {
        id: "seek-50",
        action: "seek-50",
        label: "Jump to 50%",
        category: "Seeking & Navigation",
        default_combo: "Digit5",
    },
    ShortcutCatalogEntry {
        id: "seek-60",
        action: "seek-60",
        label: "Jump to 60%",
        category: "Seeking & Navigation",
        default_combo: "Digit6",
    },
    ShortcutCatalogEntry {
        id: "seek-70",
        action: "seek-70",
        label: "Jump to 70%",
        category: "Seeking & Navigation",
        default_combo: "Digit7",
    },
    ShortcutCatalogEntry {
        id: "seek-80",
        action: "seek-80",
        label: "Jump to 80%",
        category: "Seeking & Navigation",
        default_combo: "Digit8",
    },
    ShortcutCatalogEntry {
        id: "seek-90",
        action: "seek-90",
        label: "Jump to 90%",
        category: "Seeking & Navigation",
        default_combo: "Digit9",
    },
    // Audio & Subtitles (8)
    ShortcutCatalogEntry {
        id: "volume-up",
        action: "volume-up",
        label: "Volume Up (+5%)",
        category: "Audio & Subtitles",
        default_combo: "ArrowUp",
    },
    ShortcutCatalogEntry {
        id: "volume-down",
        action: "volume-down",
        label: "Volume Down (-5%)",
        category: "Audio & Subtitles",
        default_combo: "ArrowDown",
    },
    ShortcutCatalogEntry {
        id: "toggle-mute",
        action: "toggle-mute",
        label: "Mute / Unmute",
        category: "Audio & Subtitles",
        default_combo: "KeyM",
    },
    ShortcutCatalogEntry {
        id: "cycle-audio",
        action: "cycle-audio",
        label: "Cycle Audio Track (Subs / Dubs)",
        category: "Audio & Subtitles",
        default_combo: "KeyA",
    },
    ShortcutCatalogEntry {
        id: "toggle-equalizer",
        action: "toggle-equalizer",
        label: "Audio Equalizer",
        category: "Audio & Subtitles",
        default_combo: "KeyE",
    },
    ShortcutCatalogEntry {
        id: "toggle-subtitles",
        action: "toggle-subtitles",
        label: "Subtitles Panel",
        category: "Audio & Subtitles",
        default_combo: "KeyC",
    },
    ShortcutCatalogEntry {
        id: "sub-delay-down",
        action: "sub-delay-down",
        label: "Subtitle Delay -0.1s",
        category: "Audio & Subtitles",
        default_combo: "KeyG",
    },
    ShortcutCatalogEntry {
        id: "sub-delay-up",
        action: "sub-delay-up",
        label: "Subtitle Delay +0.1s",
        category: "Audio & Subtitles",
        default_combo: "KeyH",
    },
    // View & Interface (8)
    ShortcutCatalogEntry {
        id: "toggle-fullscreen",
        action: "toggle-fullscreen",
        label: "Toggle Fullscreen",
        category: "View & Interface",
        default_combo: "KeyF",
    },
    ShortcutCatalogEntry {
        id: "exit-fullscreen",
        action: "exit-fullscreen",
        label: "Exit Fullscreen",
        category: "View & Interface",
        default_combo: "Escape",
    },
    ShortcutCatalogEntry {
        id: "toggle-playlist",
        action: "toggle-playlist",
        label: "Toggle Playlist Sidebar",
        category: "View & Interface",
        default_combo: "Ctrl+KeyL",
    },
    ShortcutCatalogEntry {
        id: "toggle-stats",
        action: "toggle-stats",
        label: "Stats for Nerds",
        category: "View & Interface",
        default_combo: "KeyI",
    },
    ShortcutCatalogEntry {
        id: "toggle-time-format",
        action: "toggle-time-format",
        label: "Toggle Time Format",
        category: "View & Interface",
        default_combo: "KeyT",
    },
    ShortcutCatalogEntry {
        id: "toggle-bg-settings",
        action: "toggle-bg-settings",
        label: "Welcome Background Settings",
        category: "View & Interface",
        default_combo: "KeyB",
    },
    ShortcutCatalogEntry {
        id: "unlock-ui",
        action: "unlock-ui",
        label: "Unlock Controls (When Locked)",
        category: "View & Interface",
        default_combo: "Ctrl+KeyU",
    },
    ShortcutCatalogEntry {
        id: "toggle-lock",
        action: "toggle-lock",
        label: "Lock / Unlock Controls",
        category: "View & Interface",
        default_combo: "",
    },
    // File & Application (6)
    ShortcutCatalogEntry {
        id: "open-file",
        action: "open-file",
        label: "Open File",
        category: "File & Application",
        default_combo: "Ctrl+KeyO",
    },
    ShortcutCatalogEntry {
        id: "open-multiple-files",
        action: "open-multiple-files",
        label: "Open Multiple Files",
        category: "File & Application",
        default_combo: "Ctrl+Shift+KeyO",
    },
    ShortcutCatalogEntry {
        id: "open-folder",
        action: "open-folder",
        label: "Open Folder",
        category: "File & Application",
        default_combo: "Ctrl+KeyF",
    },
    ShortcutCatalogEntry {
        id: "open-network-stream",
        action: "open-network-stream",
        label: "Open Network Stream / URL",
        category: "File & Application",
        default_combo: "Ctrl+KeyN",
    },
    ShortcutCatalogEntry {
        id: "toggle-settings",
        action: "toggle-settings",
        label: "Open Settings",
        category: "File & Application",
        default_combo: "Ctrl+Comma",
    },
    ShortcutCatalogEntry {
        id: "quit",
        action: "quit",
        label: "Quit Player",
        category: "File & Application",
        default_combo: "Ctrl+KeyQ",
    },
];

const MODIFIER_CODES: &[&str] = &[
    "ControlLeft",
    "ControlRight",
    "ShiftLeft",
    "ShiftRight",
    "AltLeft",
    "AltRight",
    "MetaLeft",
    "MetaRight",
    "OSLeft",
    "OSRight",
];

pub fn is_modifier_code(code: &str) -> bool {
    MODIFIER_CODES.contains(&code)
}

pub fn normalize_key_token(raw_token: &str) -> String {
    let token = raw_token.trim();
    if token.is_empty() {
        return String::new();
    }

    if let Some(rest) = token.strip_prefix("Key") {
        if rest.len() == 1 && rest.as_bytes()[0].is_ascii_uppercase() {
            return token.to_string();
        }
    }
    if let Some(rest) = token.strip_prefix("Digit") {
        if rest.len() == 1 && rest.as_bytes()[0].is_ascii_digit() {
            return token.to_string();
        }
    }
    if let Some(rest) = token.strip_prefix("Numpad") {
        if rest.len() == 1 && rest.as_bytes()[0].is_ascii_digit() {
            return token.to_string();
        }
    }
    if let Some(rest) = token.strip_prefix('F') {
        if let Ok(n) = rest.parse::<u8>() {
            if (1..=12).contains(&n) && rest == n.to_string() {
                return token.to_string();
            }
        }
    }

    if token.len() == 1 && token.as_bytes()[0].is_ascii_alphabetic() {
        return format!("Key{}", token.to_ascii_uppercase());
    }
    if token.len() == 1 && token.as_bytes()[0].is_ascii_digit() {
        return format!("Digit{token}");
    }

    let lower = token.to_ascii_lowercase();
    match lower.as_str() {
        "space" | "spacebar" => return "Space".to_string(),
        "esc" | "escape" => return "Escape".to_string(),
        "enter" | "return" => return "Enter".to_string(),
        "tab" => return "Tab".to_string(),
        "backspace" => return "Backspace".to_string(),
        "delete" | "del" => return "Delete".to_string(),
        "insert" | "ins" => return "Insert".to_string(),
        "home" => return "Home".to_string(),
        "end" => return "End".to_string(),
        "pageup" | "pgup" => return "PageUp".to_string(),
        "pagedown" | "pgdn" => return "PageDown".to_string(),
        "arrowleft" | "left" => return "ArrowLeft".to_string(),
        "arrowright" | "right" => return "ArrowRight".to_string(),
        "arrowup" | "up" => return "ArrowUp".to_string(),
        "arrowdown" | "down" => return "ArrowDown".to_string(),
        "[" | "bracketleft" => return "BracketLeft".to_string(),
        "]" | "bracketright" => return "BracketRight".to_string(),
        "," | "comma" => return "Comma".to_string(),
        "." | "period" => return "Period".to_string(),
        "/" | "slash" => return "Slash".to_string(),
        "\\" | "backslash" => return "Backslash".to_string(),
        ";" | "semicolon" => return "Semicolon".to_string(),
        "'" | "quote" => return "Quote".to_string(),
        "`" | "backquote" => return "Backquote".to_string(),
        "-" | "minus" => return "Minus".to_string(),
        "=" | "equal" => return "Equal".to_string(),
        "numpadenter" => return "NumpadEnter".to_string(),
        "numpadadd" => return "NumpadAdd".to_string(),
        "numpadsubtract" => return "NumpadSubtract".to_string(),
        "numpadmultiply" => return "NumpadMultiply".to_string(),
        "numpaddivide" => return "NumpadDivide".to_string(),
        "numpaddecimal" => return "NumpadDecimal".to_string(),
        "numpadequal" => return "NumpadEqual".to_string(),
        "capslock" => return "CapsLock".to_string(),
        "numlock" => return "NumLock".to_string(),
        "scrolllock" => return "ScrollLock".to_string(),
        "printscreen" => return "PrintScreen".to_string(),
        "pause" => return "Pause".to_string(),
        "contextmenu" => return "ContextMenu".to_string(),
        _ => {}
    }

    if let Some(rest) = lower.strip_prefix("key") {
        if rest.len() == 1 && rest.as_bytes()[0].is_ascii_alphabetic() {
            return format!("Key{}", rest.to_ascii_uppercase());
        }
    }
    if let Some(rest) = lower.strip_prefix("digit") {
        if rest.len() == 1 && rest.as_bytes()[0].is_ascii_digit() {
            return format!("Digit{rest}");
        }
    }
    if let Some(rest) = lower.strip_prefix('f') {
        if let Ok(n) = rest.parse::<u8>() {
            if (1..=12).contains(&n) && rest == n.to_string() {
                return format!("F{n}");
            }
        }
    }

    token.to_string()
}

pub fn normalize_combo(raw_combo: &str) -> String {
    let trimmed = raw_combo.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let parts: Vec<&str> = trimmed
        .split('+')
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        return String::new();
    }

    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut key_code = String::new();

    for (i, part) in parts.iter().enumerate() {
        let lower = part.to_ascii_lowercase();
        if i + 1 < parts.len() {
            match lower.as_str() {
                "ctrl" | "control" | "cmd" | "meta" | "command" => {
                    ctrl = true;
                    continue;
                }
                "alt" | "option" => {
                    alt = true;
                    continue;
                }
                "shift" => {
                    shift = true;
                    continue;
                }
                _ => {}
            }
        }
        key_code = normalize_key_token(part);
    }

    if key_code.is_empty() || is_modifier_code(&key_code) {
        return String::new();
    }

    let mut out = Vec::new();
    if ctrl {
        out.push("Ctrl".to_string());
    }
    if alt {
        out.push("Alt".to_string());
    }
    if shift {
        out.push("Shift".to_string());
    }
    out.push(key_code);
    out.join("+")
}

pub fn format_combo_for_display(combo: &str) -> String {
    let normalized = normalize_combo(combo);
    if normalized.is_empty() {
        return "Unbound".to_string();
    }

    normalized
        .split('+')
        .map(|part| match part {
            "Ctrl" | "Alt" | "Shift" => part.to_string(),
            "Space" => "Space".to_string(),
            "ArrowLeft" => "←".to_string(),
            "ArrowRight" => "→".to_string(),
            "ArrowUp" => "↑".to_string(),
            "ArrowDown" => "↓".to_string(),
            "Escape" => "Esc".to_string(),
            "Enter" => "Enter".to_string(),
            "NumpadEnter" => "Num Enter".to_string(),
            "Tab" => "Tab".to_string(),
            "Backspace" => "Backspace".to_string(),
            "Delete" => "Del".to_string(),
            "Insert" => "Ins".to_string(),
            "Home" => "Home".to_string(),
            "End" => "End".to_string(),
            "PageUp" => "PgUp".to_string(),
            "PageDown" => "PgDn".to_string(),
            "BracketLeft" => "[".to_string(),
            "BracketRight" => "]".to_string(),
            "Comma" => ",".to_string(),
            "Period" => ".".to_string(),
            "Slash" => "/".to_string(),
            "Backslash" => "\\".to_string(),
            "Semicolon" => ";".to_string(),
            "Quote" => "'".to_string(),
            "Backquote" => "`".to_string(),
            "Minus" => "-".to_string(),
            "Equal" => "=".to_string(),
            p if p.starts_with("Key") && p.len() == 4 => p[3..].to_string(),
            p if p.starts_with("Digit") && p.len() == 6 => p[5..].to_string(),
            p if p.starts_with("Numpad") => format!("Num {}", &p[6..]),
            p => p.to_string(),
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

pub fn format_combo_compact(combo: &str) -> String {
    let normalized = normalize_combo(combo);
    if normalized.is_empty() {
        return String::new();
    }
    format_combo_for_display(&normalized).replace(" + ", "+")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetShortcutOutcome {
    pub action_id: String,
    pub label: String,
    pub combo: String,
    pub display_combo: String,
    pub previous_combo: String,
    pub displaced_action_id: Option<String>,
    pub displaced_action_label: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ShortcutManager {
    bindings: BTreeMap<String, String>,
    combo_to_action: BTreeMap<String, String>,
    screenshot_mouse_button: i32,
}

impl Default for ShortcutManager {
    fn default() -> Self {
        let mut bindings = BTreeMap::new();
        for item in SHORTCUT_CATALOG {
            bindings.insert(item.id.to_string(), item.default_combo.to_string());
        }
        let mut mgr = Self {
            bindings,
            combo_to_action: BTreeMap::new(),
            screenshot_mouse_button: 3,
        };
        mgr.rebuild_lookup_maps();
        mgr
    }
}

impl ShortcutManager {
    pub fn catalog() -> &'static [ShortcutCatalogEntry] {
        &SHORTCUT_CATALOG
    }

    pub fn get_binding(&self, action_id: &str) -> &str {
        self.bindings
            .get(action_id)
            .map(|s| s.as_str())
            .unwrap_or("")
    }

    pub fn get_default_binding(action_id: &str) -> &'static str {
        SHORTCUT_CATALOG
            .iter()
            .find(|item| item.id == action_id)
            .map(|item| item.default_combo)
            .unwrap_or("")
    }

    pub fn screenshot_mouse_button(&self) -> i32 {
        self.screenshot_mouse_button
    }

    pub fn set_screenshot_mouse_button(&mut self, button: i32) -> i32 {
        let normalized = if VALID_SCREENSHOT_MOUSE_BUTTONS.contains(&button) {
            button
        } else {
            3
        };
        self.screenshot_mouse_button = normalized;
        normalized
    }

    pub fn get_custom_overrides(&self) -> BTreeMap<String, String> {
        let mut overrides = BTreeMap::new();
        for item in SHORTCUT_CATALOG {
            let current = self.bindings.get(item.id).map(|s| s.as_str()).unwrap_or("");
            if current != item.default_combo {
                overrides.insert(item.id.to_string(), current.to_string());
            }
        }
        overrides
    }

    pub fn apply_custom_shortcuts(&mut self, custom_map: &BTreeMap<String, String>) {
        self.bindings.clear();
        for item in SHORTCUT_CATALOG {
            self.bindings
                .insert(item.id.to_string(), item.default_combo.to_string());
        }

        for (action_id, raw_combo) in custom_map {
            if !self.bindings.contains_key(action_id) {
                continue;
            }
            let normalized = normalize_combo(raw_combo);
            if !normalized.is_empty() {
                for item in SHORTCUT_CATALOG {
                    if item.id != action_id
                        && self.bindings.get(item.id).map(|s| s.as_str())
                            == Some(normalized.as_str())
                    {
                        self.bindings.insert(item.id.to_string(), String::new());
                    }
                }
            }
            self.bindings.insert(action_id.clone(), normalized);
        }
        self.rebuild_lookup_maps();
    }

    pub fn set_shortcut(
        &mut self,
        action_id: &str,
        raw_combo: &str,
    ) -> Result<SetShortcutOutcome, &'static str> {
        let entry = SHORTCUT_CATALOG
            .iter()
            .find(|item| item.id == action_id)
            .ok_or("Unknown shortcut action")?;

        let combo = normalize_combo(raw_combo);
        let previous_combo = self.bindings.get(action_id).cloned().unwrap_or_default();

        let mut displaced_action_id = None;
        let mut displaced_action_label = None;

        if !combo.is_empty() {
            for item in SHORTCUT_CATALOG {
                if item.id != action_id
                    && self.bindings.get(item.id).map(|s| s.as_str()) == Some(combo.as_str())
                {
                    self.bindings.insert(item.id.to_string(), String::new());
                    displaced_action_id = Some(item.id.to_string());
                    displaced_action_label = Some(item.label.to_string());
                }
            }
        }

        self.bindings.insert(action_id.to_string(), combo.clone());
        self.rebuild_lookup_maps();

        Ok(SetShortcutOutcome {
            action_id: action_id.to_string(),
            label: entry.label.to_string(),
            display_combo: format_combo_for_display(&combo),
            combo,
            previous_combo,
            displaced_action_id,
            displaced_action_label,
        })
    }

    pub fn reset_shortcut(&mut self, action_id: &str) -> Result<SetShortcutOutcome, &'static str> {
        let entry = SHORTCUT_CATALOG
            .iter()
            .find(|item| item.id == action_id)
            .ok_or("Unknown shortcut action")?;
        self.set_shortcut(action_id, entry.default_combo)
    }

    pub fn reset_all_shortcuts(&mut self) {
        self.bindings.clear();
        for item in SHORTCUT_CATALOG {
            self.bindings
                .insert(item.id.to_string(), item.default_combo.to_string());
        }
        self.rebuild_lookup_maps();
    }

    pub fn resolve_action(&self, combo: &str) -> Option<&str> {
        let normalized = normalize_combo(combo);
        if normalized.is_empty() {
            return None;
        }
        if let Some(action) = self.combo_to_action.get(&normalized) {
            return Some(action.as_str());
        }
        if normalized == "F11"
            || (normalized == "Shift+KeyF"
                && self
                    .combo_to_action
                    .get("KeyF")
                    .is_some_and(|a| a == "toggle-fullscreen"))
        {
            return Some("toggle-fullscreen");
        }
        None
    }

    pub fn is_action_allowed_when_locked(action: &str) -> bool {
        matches!(
            action,
            "unlock-ui" | "toggle-lock" | "screenshot" | "screenshot-png"
        )
    }

    pub fn is_repeatable_action(action: &str) -> bool {
        matches!(
            action,
            "seek-back-5" | "seek-forward-5" | "volume-up" | "volume-down"
        )
    }

    pub fn requires_media(action: &str) -> bool {
        matches!(
            action,
            "toggle-play"
                | "cycle-audio"
                | "stop"
                | "seek-back-5"
                | "seek-forward-5"
                | "seek-0"
                | "seek-10"
                | "seek-20"
                | "seek-30"
                | "seek-40"
                | "seek-50"
                | "seek-60"
                | "seek-70"
                | "seek-80"
                | "seek-90"
                | "speed-up"
                | "speed-down"
                | "frame-forward"
                | "frame-backward"
                | "toggle-stats"
                | "toggle-subtitles"
                | "toggle-equalizer"
                | "sub-delay-down"
                | "sub-delay-up"
                | "screenshot"
                | "screenshot-png"
                | "delete-latest-screenshot"
                | "restore-latest-screenshot"
                | "toggle-time-format"
                | "toggle-ab-loop"
                | "toggle-lock"
        )
    }

    fn rebuild_lookup_maps(&mut self) {
        self.combo_to_action.clear();
        for item in SHORTCUT_CATALOG {
            if let Some(combo) = self.bindings.get(item.id) {
                if !combo.is_empty() {
                    self.combo_to_action
                        .insert(combo.clone(), item.action.to_string());
                }
            }
        }
    }
}
