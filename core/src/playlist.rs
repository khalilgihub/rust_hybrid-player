use rand::RngExt;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

pub const PLAYLIST_VIRTUAL_THRESHOLD: usize = 80;
pub const PLAYLIST_ITEM_HEIGHT_PX: u32 = 40;
pub const PREVIOUS_RESTART_THRESHOLD_SECONDS: f64 = 3.0;

pub const SUBTITLE_EXTENSIONS: &[&str] = &[
    ".srt", ".ass", ".ssa", ".vtt", ".sub", ".idx", ".sup", ".smi", ".rt",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RepeatMode {
    #[default]
    None,
    One,
    All,
}

impl RepeatMode {
    pub fn cycle(self) -> Self {
        match self {
            RepeatMode::None => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaylistItem {
    pub id: String,
    pub title: String,
    pub path: String,
    #[serde(default)]
    pub is_url: bool,
    #[serde(default)]
    pub duration: Option<u64>,
    #[serde(default)]
    pub thumbnail: Option<String>,
}

impl PlaylistItem {
    pub fn from_path(path: impl Into<String>, is_url: bool) -> Self {
        let path = path.into();
        let title = if is_url {
            path.clone()
        } else {
            Path::new(&path)
                .file_name()
                .and_then(|n| n.to_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(&path)
                .to_string()
        };
        Self {
            id: format!("pl_{}", uuid_like_token(&path)),
            title,
            path,
            is_url,
            duration: None,
            thumbnail: None,
        }
    }
}

fn uuid_like_token(seed: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in seed.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let r: u32 = rand::rng().random();
    format!("{hash:016x}{r:08x}")
}

pub fn is_subtitle_file_path(path: &str) -> bool {
    let lower = path.trim().to_ascii_lowercase();
    SUBTITLE_EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
}

#[derive(Debug, Clone)]
pub struct PlaylistState {
    pub items: Vec<PlaylistItem>,
    pub current_index: isize,
    pub repeat: RepeatMode,
    pub shuffle: bool,
    pub search_query: String,
    played_shuffle_indices: HashSet<usize>,
}

impl Default for PlaylistState {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            current_index: -1,
            repeat: RepeatMode::None,
            shuffle: false,
            search_query: String::new(),
            played_shuffle_indices: HashSet::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavigationAction {
    PlayIndex(usize),
    RestartCurrent,
    None,
}

impl PlaylistState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds non-subtitle items, deduplicating by path unless already present.
    /// Returns the index of the first added item (or first matching item if already present).
    pub fn add_paths(&mut self, paths: impl IntoIterator<Item = (String, bool)>) -> Option<usize> {
        let mut first_idx = None;
        for (path, is_url) in paths {
            if !is_url && is_subtitle_file_path(&path) {
                continue;
            }
            if let Some(existing) = self.items.iter().position(|item| item.path == path) {
                if first_idx.is_none() {
                    first_idx = Some(existing);
                }
                continue;
            }
            let idx = self.items.len();
            self.items.push(PlaylistItem::from_path(path, is_url));
            if first_idx.is_none() {
                first_idx = Some(idx);
            }
        }
        first_idx
    }

    pub fn add_item(&mut self, item: PlaylistItem) -> Option<usize> {
        if !item.is_url && is_subtitle_file_path(&item.path) {
            return None;
        }
        if let Some(existing) = self.items.iter().position(|i| i.path == item.path) {
            return Some(existing);
        }
        let idx = self.items.len();
        self.items.push(item);
        Some(idx)
    }

    pub fn current_item(&self) -> Option<&PlaylistItem> {
        if self.current_index >= 0 {
            self.items.get(self.current_index as usize)
        } else {
            None
        }
    }

    pub fn set_current_index(&mut self, index: usize) -> Option<&PlaylistItem> {
        if index < self.items.len() {
            self.current_index = index as isize;
            if self.shuffle {
                self.played_shuffle_indices.insert(index);
            }
            self.items.get(index)
        } else {
            None
        }
    }

    pub fn set_shuffle(&mut self, enabled: bool) {
        self.shuffle = enabled;
        self.played_shuffle_indices.clear();
        if enabled && self.current_index >= 0 && (self.current_index as usize) < self.items.len() {
            self.played_shuffle_indices
                .insert(self.current_index as usize);
        }
    }

    pub fn toggle_shuffle(&mut self) -> bool {
        self.set_shuffle(!self.shuffle);
        self.shuffle
    }

    pub fn cycle_repeat(&mut self) -> RepeatMode {
        self.repeat = self.repeat.cycle();
        self.repeat
    }

    pub fn should_virtualize(&self) -> bool {
        self.filtered_indices().len() > PLAYLIST_VIRTUAL_THRESHOLD
    }

    pub fn filtered_indices(&self) -> Vec<usize> {
        let q = self.search_query.trim().to_ascii_lowercase();
        if q.is_empty() {
            return (0..self.items.len()).collect();
        }
        self.items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                item.title.to_ascii_lowercase().contains(&q)
                    || item.path.to_ascii_lowercase().contains(&q)
            })
            .map(|(idx, _)| idx)
            .collect()
    }

    pub fn play_next(&mut self, auto_advance: bool) -> NavigationAction {
        let len = self.items.len();
        if len == 0 {
            return NavigationAction::None;
        }

        if self.repeat == RepeatMode::One
            && auto_advance
            && self.current_index >= 0
            && (self.current_index as usize) < len
        {
            return NavigationAction::PlayIndex(self.current_index as usize);
        }

        if self.shuffle && len > 1 {
            if self.current_index >= 0 && (self.current_index as usize) < len {
                self.played_shuffle_indices
                    .insert(self.current_index as usize);
            }
            if self.played_shuffle_indices.len() >= len {
                if self.repeat == RepeatMode::None {
                    return NavigationAction::None;
                }
                self.played_shuffle_indices.clear();
                if self.current_index >= 0 && (self.current_index as usize) < len {
                    self.played_shuffle_indices
                        .insert(self.current_index as usize);
                }
            }

            let unplayed: Vec<usize> = (0..len)
                .filter(|idx| !self.played_shuffle_indices.contains(idx))
                .collect();
            let next_idx = if unplayed.is_empty() {
                0
            } else {
                let pick = rand::rng().random_range(0..unplayed.len());
                unplayed[pick]
            };
            self.current_index = next_idx as isize;
            self.played_shuffle_indices.insert(next_idx);
            return NavigationAction::PlayIndex(next_idx);
        }

        let current = if self.current_index < 0 {
            0
        } else {
            (self.current_index as usize) + 1
        };

        if current >= len {
            if self.repeat == RepeatMode::All || !auto_advance {
                self.current_index = 0;
                NavigationAction::PlayIndex(0)
            } else {
                NavigationAction::None
            }
        } else {
            self.current_index = current as isize;
            NavigationAction::PlayIndex(current)
        }
    }

    pub fn play_previous(&mut self, current_time_seconds: f64) -> NavigationAction {
        let len = self.items.len();
        if len == 0 {
            return NavigationAction::None;
        }
        if current_time_seconds > PREVIOUS_RESTART_THRESHOLD_SECONDS {
            return NavigationAction::RestartCurrent;
        }

        let prev = if self.current_index <= 0 {
            if self.repeat == RepeatMode::All {
                len - 1
            } else {
                return NavigationAction::RestartCurrent;
            }
        } else {
            (self.current_index as usize) - 1
        };
        self.current_index = prev as isize;
        NavigationAction::PlayIndex(prev)
    }

    pub fn remove(&mut self, index: usize) -> Option<PlaylistItem> {
        if index >= self.items.len() {
            return None;
        }
        let removed = self.items.remove(index);
        self.played_shuffle_indices.clear();
        if self.items.is_empty() {
            self.current_index = -1;
        } else if (index as isize) < self.current_index {
            self.current_index -= 1;
        } else if (index as isize) == self.current_index
            && (self.current_index as usize) >= self.items.len()
        {
            self.current_index = (self.items.len() as isize) - 1;
        }
        Some(removed)
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.current_index = -1;
        self.played_shuffle_indices.clear();
    }
}
